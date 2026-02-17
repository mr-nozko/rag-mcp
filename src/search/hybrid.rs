use crate::db::Db;
use crate::embeddings::OpenAIEmbedder;
use crate::error::Result;
use crate::search::{bm25, vector, SearchResult};
use std::collections::HashMap;
use std::sync::Arc;

use crate::cache::ChunkEmbeddingCache;

/// Search documents using hybrid approach combining BM25 and vector search
///
/// This function runs BM25 full-text search and vector similarity search in parallel,
/// then combines results using Reciprocal Rank Fusion (RRF). This approach leverages
/// the complementary strengths of lexical (BM25) and semantic (vector) retrieval.
///
/// # Arguments
///
/// * `db` - Database connection wrapper
/// * `embedder` - OpenAI embedder instance
/// * `query` - Search query text
/// * `namespace` - Optional namespace filter (directory-derived; e.g. agents, system, self, community); None = search all
/// * `agent_filter` - Optional agent name filter (documents.agent_name = ?)
/// * `k` - Maximum number of results to return
/// * `min_score` - Minimum RRF score threshold (0.0-1.0)
/// * `bm25_weight` - Weight for BM25 results in fusion (typically 0.3-0.7)
/// * `vector_weight` - Weight for vector results in fusion (typically 0.3-0.7)
/// * `chunk_cache` - Optional in-memory chunk embedding cache for faster vector search
///
/// # Returns
///
/// Vector of SearchResult structs, sorted by fused relevance score (highest first),
/// with ranks assigned (1-indexed).
///
/// # Implementation Details
///
/// - Over-fetching: Retrieves `k * 2` results from each method for better fusion quality
/// - Parallel execution: Runs both searches concurrently using `tokio::join!`
/// - RRF constant: K = 60.0 (standard default from research)
/// - Namespace and agent filtering are applied inside vector search SQL (no post-filter).
///
/// # Example
///
/// ```no_run
/// use ragmcp::{Config, db::Db, embeddings::OpenAIEmbedder, search::hybrid::search_hybrid};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = Config::load()?;
/// let db = Db::new(&config.ragmcp.db_path);
/// let embedder = OpenAIEmbedder::new(
///     std::env::var("OPENAI_API_KEY")?,
///     config.embeddings.model,
///     config.embeddings.batch_size,
/// );
///
/// let results = search_hybrid(
///     &db,
///     &embedder,
///     "What are the core concepts of module-alpha?",
///     None,  // namespace
///     None,  // agent_filter
///     5,
///     0.65,
///     0.5,
///     0.5,
///     None,  // chunk_cache
/// ).await?;
///
/// for result in results {
///     println!("{}: {} (score: {:.3})", result.rank, result.doc_path, result.score);
/// }
/// # Ok(())
/// # }
/// ```
pub async fn search_hybrid(
    db: &Db,
    embedder: &OpenAIEmbedder,
    query: &str,
    namespace: Option<&str>,
    agent_filter: Option<&str>,
    k: usize,
    min_score: f32,
    bm25_weight: f32,
    vector_weight: f32,
    chunk_cache: Option<Arc<ChunkEmbeddingCache>>,
) -> Result<Vec<SearchResult>> {
    let total_start = std::time::Instant::now();

    // Over-fetch from each method (k * 4) for better fusion quality in RAG use case
    let fetch_k = k * 4;

    // Run both searches in parallel; vector search applies namespace/agent filter in SQL
    let search_start = std::time::Instant::now();
    let (bm25_results, vector_results) = tokio::join!(
        bm25::search_bm25(db, query, namespace, None, fetch_k, 0.0),
        vector::search_vector(
            db,
            embedder,
            query,
            fetch_k,
            0.0,
            namespace,
            agent_filter,
            chunk_cache,
        )
    );
    let search_duration = search_start.elapsed();
    log::debug!("Hybrid search: BM25+vector parallel execution took {:?}", search_duration);

    let bm25_results = bm25_results?;
    let vector_results = vector_results?;

    // Apply Reciprocal Rank Fusion to combine results
    let fusion_start = std::time::Instant::now();
    let fused = reciprocal_rank_fusion(
        bm25_results,
        vector_results,
        k,
        bm25_weight,
        vector_weight,
    );
    let fusion_duration = fusion_start.elapsed();
    log::debug!("Hybrid search: RRF fusion took {:?}", fusion_duration);

    // CRITICAL FIX: RRF scores are rank-based (typically 0.01-0.1 range), NOT 0-1 normalized
    // Normalize RRF scores to 0-1 range using min-max normalization
    let max_rrf_score = fused.first().map(|r| r.score).unwrap_or(0.0);
    let min_rrf_score = if fused.is_empty() { 0.0 } else { fused.last().unwrap().score };
    let score_range = max_rrf_score - min_rrf_score;
    
    let normalized: Vec<SearchResult> = if score_range > 0.0 {
        fused.into_iter().map(|mut r| {
            // Normalize: (score - min) / (max - min)
            r.score = (r.score - min_rrf_score) / score_range;
            r
        }).collect()
    } else {
        fused
    };
    
    // RAG-optimized filtering: Use adaptive threshold for better recall
    // If score distribution is tight (small range), use lower threshold
    // This ensures we get comprehensive results even when all scores are similar
    let adaptive_threshold = if score_range < 0.1 {
        // Tight distribution: use lower threshold (0.2) for better recall
        min_score.min(0.2)
    } else {
        // Wide distribution: use configured threshold
        min_score
    };
    
    // Filter by adaptive threshold
    // The adaptive threshold (lowered to 0.2 for tight distributions) ensures good recall
    let filtered: Vec<SearchResult> = normalized
        .into_iter()
        .filter(|r| r.score >= adaptive_threshold)
        .collect();
    
    let total_duration = total_start.elapsed();
    log::debug!(
        "Hybrid search total: {:?} (search: {:?}, fusion: {:?}, results: {})",
        total_duration,
        search_duration,
        fusion_duration,
        filtered.len()
    );
    
    Ok(filtered)
}

/// Combine ranked lists using Reciprocal Rank Fusion (RRF)
///
/// RRF is a rank-based fusion method that combines multiple ranked lists by
/// computing a score for each document based on its reciprocal rank in each list.
/// This approach is scale-invariant and doesn't require score normalization.
///
/// # Arguments
///
/// * `bm25_results` - Results from BM25 search (already ranked)
/// * `vector_results` - Results from vector search (already ranked)
/// * `k` - Number of results to return
/// * `bm25_weight` - Weight for BM25 results (0.0-1.0)
/// * `vector_weight` - Weight for vector results (0.0-1.0)
///
/// # Returns
///
/// Vector of SearchResult structs with combined RRF scores, sorted by score
/// (highest first), with final ranks assigned (1-indexed).
///
/// # Algorithm
///
/// For each document d in either list:
/// ```text
/// RRF_score(d) = Σ weight_i / (K + rank_i(d))
/// ```
/// where:
/// - K = 60.0 (standard constant from research)
/// - rank_i(d) = position of document d in list i (1-indexed)
/// - weight_i = importance weight for list i
///
/// Documents appearing in both lists accumulate scores from both.
///
/// # References
///
/// - Cormack et al. (2009): "Reciprocal Rank Fusion outperforms the best known automatic evaluation measures"
/// - OpenSearch, LanceDB, Marqo use K=60 as default (2024-2025 standard)
pub fn reciprocal_rank_fusion(
    bm25_results: Vec<SearchResult>,
    vector_results: Vec<SearchResult>,
    k: usize,
    bm25_weight: f32,
    vector_weight: f32,
) -> Vec<SearchResult> {
    // RRF constant - standard default from research
    // Sources: OpenSearch, LanceDB, Marqo, MariaDB all use K=60
    const K: f32 = 60.0;

    // Use HashMap to accumulate scores by chunk_id
    // Key: chunk_id, Value: (accumulated_score, SearchResult)
    let mut scores: HashMap<String, (f32, SearchResult)> = HashMap::new();

    // Add BM25 scores using RRF formula
    for (rank, result) in bm25_results.into_iter().enumerate() {
        // RRF score: weight / (K + rank)
        // rank is 0-indexed in enumerate, but RRF uses 1-indexed ranks
        let rrf_score = bm25_weight / (K + (rank + 1) as f32);

        scores.insert(result.chunk_id.clone(), (rrf_score, result));
    }

    // Add vector scores using RRF formula
    for (rank, result) in vector_results.into_iter().enumerate() {
        let rrf_score = vector_weight / (K + (rank + 1) as f32);

        scores
            .entry(result.chunk_id.clone())
            .and_modify(|(score, _)| *score += rrf_score) // Accumulate if already present
            .or_insert((rrf_score, result)); // Insert if new
    }

    // Convert HashMap to Vec and sort by combined score (descending)
    let mut ranked: Vec<_> = scores
        .into_iter()
        .map(|(_, (score, mut result))| {
            result.score = score;
            result
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Take top-k and assign final ranks (1-indexed)
    ranked
        .into_iter()
        .take(k)
        .enumerate()
        .map(|(idx, mut result)| {
            result.rank = idx + 1;
            result
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper function to create a test SearchResult
    fn create_result(chunk_id: &str, doc_path: &str, score: f32, rank: usize) -> SearchResult {
        SearchResult {
            chunk_id: chunk_id.to_string(),
            doc_path: doc_path.to_string(),
            doc_type: "test".to_string(),
            agent_name: None,
            section: None,
            chunk_text: "test content".to_string(),
            score,
            rank,
        }
    }

    #[test]
    fn test_rrf_basic_fusion_with_overlap() {
        // Test basic fusion where some chunks appear in both lists
        let bm25_results = vec![
            create_result("chunk1", "doc1.md", 0.9, 1),
            create_result("chunk2", "doc2.md", 0.8, 2),
            create_result("chunk3", "doc3.md", 0.7, 3),
        ];

        let vector_results = vec![
            create_result("chunk2", "doc2.md", 0.95, 1), // Same as BM25 rank 2
            create_result("chunk1", "doc1.md", 0.85, 2), // Same as BM25 rank 1
            create_result("chunk4", "doc4.md", 0.75, 3), // New chunk
        ];

        let fused = reciprocal_rank_fusion(bm25_results, vector_results, 5, 0.5, 0.5);

        // Chunk1 and chunk2 should rank highest (appear in both)
        assert_eq!(fused.len(), 4); // 4 unique chunks
        assert!(fused[0].chunk_id == "chunk1" || fused[0].chunk_id == "chunk2");
        assert!(fused[1].chunk_id == "chunk1" || fused[1].chunk_id == "chunk2");

        // Verify ranks are 1-indexed
        assert_eq!(fused[0].rank, 1);
        assert_eq!(fused[1].rank, 2);
        assert_eq!(fused[2].rank, 3);
        assert_eq!(fused[3].rank, 4);
    }

    #[test]
    fn test_rrf_no_overlap() {
        // Test fusion where there's no overlap between lists
        let bm25_results = vec![
            create_result("chunk1", "doc1.md", 0.9, 1),
            create_result("chunk2", "doc2.md", 0.8, 2),
        ];

        let vector_results = vec![
            create_result("chunk3", "doc3.md", 0.95, 1),
            create_result("chunk4", "doc4.md", 0.85, 2),
        ];

        let fused = reciprocal_rank_fusion(bm25_results, vector_results, 5, 0.5, 0.5);

        assert_eq!(fused.len(), 4); // All 4 unique chunks
        // Scores should be sorted descending
        for i in 1..fused.len() {
            assert!(fused[i - 1].score >= fused[i].score);
        }
    }

    #[test]
    fn test_rrf_empty_bm25() {
        // Test with empty BM25 results (only vector results)
        let bm25_results = vec![];
        let vector_results = vec![
            create_result("chunk1", "doc1.md", 0.9, 1),
            create_result("chunk2", "doc2.md", 0.8, 2),
        ];

        let fused = reciprocal_rank_fusion(bm25_results, vector_results, 5, 0.5, 0.5);

        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].chunk_id, "chunk1");
        assert_eq!(fused[1].chunk_id, "chunk2");
    }

    #[test]
    fn test_rrf_empty_vector() {
        // Test with empty vector results (only BM25 results)
        let bm25_results = vec![
            create_result("chunk1", "doc1.md", 0.9, 1),
            create_result("chunk2", "doc2.md", 0.8, 2),
        ];
        let vector_results = vec![];

        let fused = reciprocal_rank_fusion(bm25_results, vector_results, 5, 0.5, 0.5);

        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].chunk_id, "chunk1");
        assert_eq!(fused[1].chunk_id, "chunk2");
    }

    #[test]
    fn test_rrf_both_empty() {
        // Test with both empty (edge case)
        let bm25_results = vec![];
        let vector_results = vec![];

        let fused = reciprocal_rank_fusion(bm25_results, vector_results, 5, 0.5, 0.5);

        assert_eq!(fused.len(), 0);
    }

    #[test]
    fn test_rrf_weighted_fusion() {
        // Test that different weights affect ranking
        let bm25_results = vec![create_result("chunk1", "doc1.md", 0.9, 1)];
        let vector_results = vec![create_result("chunk2", "doc2.md", 0.9, 1)];

        // Heavily favor BM25
        let fused_bm25 = reciprocal_rank_fusion(
            bm25_results.clone(),
            vector_results.clone(),
            5,
            0.9, // High BM25 weight
            0.1, // Low vector weight
        );

        // Heavily favor vector
        let fused_vector = reciprocal_rank_fusion(
            bm25_results.clone(),
            vector_results.clone(),
            5,
            0.1, // Low BM25 weight
            0.9, // High vector weight
        );

        // chunk1 (BM25) should score higher with BM25 weight
        assert!(fused_bm25[0].chunk_id == "chunk1");
        assert!(fused_bm25[0].score > fused_bm25[1].score);

        // chunk2 (vector) should score higher with vector weight
        assert!(fused_vector[0].chunk_id == "chunk2");
        assert!(fused_vector[0].score > fused_vector[1].score);
    }

    #[test]
    fn test_rrf_top_k_limiting() {
        // Test that k parameter limits results
        let bm25_results = vec![
            create_result("chunk1", "doc1.md", 0.9, 1),
            create_result("chunk2", "doc2.md", 0.8, 2),
            create_result("chunk3", "doc3.md", 0.7, 3),
        ];
        let vector_results = vec![
            create_result("chunk4", "doc4.md", 0.95, 1),
            create_result("chunk5", "doc5.md", 0.85, 2),
        ];

        let fused = reciprocal_rank_fusion(bm25_results, vector_results, 3, 0.5, 0.5);

        assert_eq!(fused.len(), 3); // Limited to k=3
        assert_eq!(fused[0].rank, 1);
        assert_eq!(fused[1].rank, 2);
        assert_eq!(fused[2].rank, 3);
    }

    #[test]
    fn test_rrf_rank_assignment() {
        // Test that final ranks are correctly assigned (1-indexed, sequential)
        let bm25_results = vec![
            create_result("chunk1", "doc1.md", 0.9, 1),
            create_result("chunk2", "doc2.md", 0.8, 2),
        ];
        let vector_results = vec![create_result("chunk3", "doc3.md", 0.95, 1)];

        let fused = reciprocal_rank_fusion(bm25_results, vector_results, 10, 0.5, 0.5);

        // Verify ranks are sequential and 1-indexed
        for (idx, result) in fused.iter().enumerate() {
            assert_eq!(result.rank, idx + 1);
        }
    }

    #[test]
    fn test_rrf_score_accumulation() {
        // Test that scores properly accumulate for chunks in both lists
        let bm25_results = vec![create_result("chunk1", "doc1.md", 0.9, 1)];
        let vector_results = vec![create_result("chunk1", "doc1.md", 0.9, 1)];

        let fused = reciprocal_rank_fusion(bm25_results, vector_results, 5, 0.5, 0.5);

        assert_eq!(fused.len(), 1);
        // Score should be sum of both RRF contributions
        // RRF(rank=1, K=60, weight=0.5) = 0.5 / (60 + 1) ≈ 0.0082
        // Total ≈ 0.0082 + 0.0082 = 0.0164
        let expected_score = (0.5 / 61.0) + (0.5 / 61.0);
        assert!((fused[0].score - expected_score).abs() < 1e-6);
    }

    #[test]
    fn test_rrf_preserves_metadata() {
        // Test that SearchResult metadata is preserved through fusion
        let bm25_results = vec![SearchResult {
            chunk_id: "chunk1".to_string(),
            doc_path: "docs/module-alpha/overview.md".to_string(),
            doc_type: "agent_prompt".to_string(),
            agent_name: Some("module-alpha".to_string()),
            section: Some("Identity".to_string()),
            chunk_text: "Module Alpha provides core analysis capabilities".to_string(),
            score: 0.9,
            rank: 1,
        }];

        let vector_results = vec![];

        let fused = reciprocal_rank_fusion(bm25_results, vector_results, 5, 0.5, 0.5);

        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].chunk_id, "chunk1");
        assert_eq!(fused[0].doc_path, "docs/module-alpha/overview.md");
        assert_eq!(fused[0].doc_type, "agent_prompt");
        assert_eq!(fused[0].agent_name, Some("module-alpha".to_string()));
        assert_eq!(fused[0].section, Some("Identity".to_string()));
        assert_eq!(
            fused[0].chunk_text,
            "Module Alpha provides core analysis capabilities"
        );
    }
}
