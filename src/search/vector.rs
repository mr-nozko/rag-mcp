use crate::cache::ChunkEmbeddingCache;
use crate::db::Db;
use crate::embeddings::OpenAIEmbedder;
use crate::error::{Result, RagmcpError};
use crate::search::SearchResult;
use std::sync::Arc;

/// Search for chunks using vector similarity (cosine similarity).
///
/// When `chunk_cache` is provided and loaded, scores in memory and fetches metadata
/// only for top-k chunk_ids (with namespace/agent filter). Otherwise does a full DB scan
/// with optional namespace/agent filter in SQL.
///
/// # Arguments
///
/// * `db` - Database connection wrapper
/// * `embedder` - OpenAI embedder instance
/// * `query` - Search query text
/// * `k` - Maximum number of results to return
/// * `min_score` - Minimum cosine similarity threshold (0.0-1.0)
/// * `namespace` - Optional namespace filter (documents.namespace = ?)
/// * `agent_filter` - Optional agent filter (documents.agent_name = ?)
/// * `chunk_cache` - Optional in-memory chunk embedding cache for fast path
pub async fn search_vector(
    db: &Db,
    embedder: &OpenAIEmbedder,
    query: &str,
    k: usize,
    min_score: f32,
    namespace: Option<&str>,
    agent_filter: Option<&str>,
    chunk_cache: Option<Arc<ChunkEmbeddingCache>>,
) -> Result<Vec<SearchResult>> {
    let _start = std::time::Instant::now();

    let embed_start = std::time::Instant::now();
    let query_vec = embedder.embed_with_cache(query, 3).await?;
    let embed_duration = embed_start.elapsed();
    log::debug!("Vector search: query embedding took {:?}", embed_duration);

    if query_vec.len() != 1536 {
        return Err(RagmcpError::Embedding(format!(
            "Unexpected embedding dimension: expected 1536, got {}",
            query_vec.len()
        )));
    }

    // Fast path: use chunk cache when available and loaded
    if let Some(ref cache) = chunk_cache {
        cache.load_if_needed(db).await?;
        if cache.is_loaded() && cache.len() > 0 {
            return search_vector_cached(db, &query_vec, k, min_score, namespace, agent_filter, cache).await;
        }
    }

    // Full-scan path: fetch all chunks with embeddings and filter by namespace/agent in SQL
    search_vector_full_scan(db, &query_vec, k, min_score, namespace, agent_filter).await
}

/// Fast path: score in memory, then one metadata query for top-k chunk_ids (with namespace/agent).
async fn search_vector_cached(
    db: &Db,
    query_vec: &[f32],
    k: usize,
    min_score: f32,
    namespace: Option<&str>,
    agent_filter: Option<&str>,
    cache: &ChunkEmbeddingCache,
) -> Result<Vec<SearchResult>> {
    let top = cache.top_k_chunk_ids(query_vec, k, min_score);
    if top.is_empty() {
        return Ok(Vec::new());
    }

    let chunk_ids: Vec<String> = top.iter().map(|(_, id)| id.clone()).collect();
    let ns = namespace.map(String::from);
    let agent = agent_filter.map(String::from);

    let rows = db
        .with_connection(move |conn| {
            let placeholders = chunk_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            // Params: [chunk_id...], namespace, namespace, agent, agent (for IS NULL OR col = ?)
            let sql = format!(
                r#"
                SELECT c.chunk_id, c.chunk_text, c.section_header, d.doc_path, d.doc_type, d.agent_name
                FROM chunks c
                JOIN documents d ON c.doc_id = d.doc_id
                WHERE c.chunk_id IN ({})
                AND (? IS NULL OR d.namespace = ?)
                AND (? IS NULL OR d.agent_name = ?)
                "#,
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            for id in &chunk_ids {
                params.push(Box::new(id.clone()));
            }
            params.push(Box::new(ns.clone()));
            params.push(Box::new(ns.clone()));
            params.push(Box::new(agent.clone()));
            params.push(Box::new(agent.clone()));
            let mut rows = stmt.query(rusqlite::params_from_iter(params))?;
            let mut results = Vec::new();
            while let Some(row) = rows.next()? {
                results.push((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ));
            }
            Ok::<Vec<_>, RagmcpError>(results)
        })
        .await?;

    // Preserve order by score (top order); include only rows that passed namespace/agent filter
    let by_id: std::collections::HashMap<String, (String, Option<String>, String, String, Option<String>)> = rows
        .into_iter()
        .map(|(id, text, section, path, dtype, agent_name)| (id, (text, section, path, dtype, agent_name)))
        .collect();

    let results: Vec<SearchResult> = top
        .into_iter()
        .filter_map(|(score, chunk_id)| {
            by_id.get(&chunk_id).map(|(chunk_text, section_header, doc_path, doc_type, agent_name)| SearchResult {
                chunk_id,
                doc_path: doc_path.clone(),
                doc_type: doc_type.clone(),
                agent_name: agent_name.clone(),
                section: section_header.clone(),
                chunk_text: chunk_text.clone(),
                score,
                rank: 0,
            })
        })
        .enumerate()
        .map(|(idx, mut r)| {
            r.rank = idx + 1;
            r
        })
        .collect();

    Ok(results)
}

/// Full-scan path: one big query with namespace/agent in WHERE, then score in Rust.
async fn search_vector_full_scan(
    db: &Db,
    query_vec: &[f32],
    k: usize,
    min_score: f32,
    namespace: Option<&str>,
    agent_filter: Option<&str>,
) -> Result<Vec<SearchResult>> {
    let ns = namespace.map(String::from);
    let agent = agent_filter.map(String::from);

    let rows = db
        .with_connection(move |conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT
                    c.chunk_id,
                    c.chunk_text,
                    c.section_header,
                    c.embedding,
                    d.doc_path,
                    d.doc_type,
                    d.agent_name
                FROM chunks c
                JOIN documents d ON c.doc_id = d.doc_id
                WHERE c.embedding IS NOT NULL
                AND (?1 IS NULL OR d.namespace = ?1)
                AND (?2 IS NULL OR d.agent_name = ?2)
                "#,
            )?;
            let mut rows = stmt.query(rusqlite::params![ns, agent])?;
            let mut results = Vec::new();
            while let Some(row) = rows.next()? {
                let chunk_id: String = row.get(0)?;
                let chunk_text: String = row.get(1)?;
                let section_header: Option<String> = row.get(2)?;
                let embedding_blob: Option<Vec<u8>> = row.get(3)?;
                let doc_path: String = row.get(4)?;
                let doc_type: String = row.get(5)?;
                let agent_name: Option<String> = row.get(6)?;
                if let Some(blob) = embedding_blob {
                    results.push((
                        chunk_id,
                        chunk_text,
                        section_header,
                        blob,
                        doc_path,
                        doc_type,
                        agent_name,
                    ));
                }
            }
            Ok::<Vec<_>, RagmcpError>(results)
        })
        .await?;

    let mut scored_results: Vec<(f32, SearchResult)> = Vec::new();
    for (chunk_id, chunk_text, section_header, embedding_blob, doc_path, doc_type, agent_name) in rows {
        let embedding = match parse_embedding(&embedding_blob) {
            Some(e) => e,
            None => continue,
        };
        if embedding.len() != 1536 {
            continue;
        }
        let similarity = cosine_similarity(query_vec, &embedding);
        if similarity < min_score {
            continue;
        }
        scored_results.push((
            similarity,
            SearchResult {
                chunk_id,
                doc_path,
                doc_type,
                agent_name,
                section: section_header,
                chunk_text,
                score: similarity,
                rank: 0,
            },
        ));
    }

    scored_results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let results: Vec<SearchResult> = scored_results
        .into_iter()
        .take(k)
        .enumerate()
        .map(|(idx, (_, mut result))| {
            result.rank = idx + 1;
            result
        })
        .collect();

    Ok(results)
}

/// Parse embedding BLOB to Vec<f32>
/// 
/// # Arguments
/// 
/// * `blob` - BLOB bytes (little-endian f32 array)
/// 
/// # Returns
/// 
/// Some(Vec<f32>) if parsing succeeds, None otherwise
fn parse_embedding(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.len() % 4 != 0 {
        return None;
    }
    
    blob.chunks(4)
        .map(|bytes| {
            let arr: [u8; 4] = bytes.try_into().ok()?;
            Some(f32::from_le_bytes(arr))
        })
        .collect()
}

/// Compute cosine similarity between two vectors
/// 
/// # Arguments
/// 
/// * `a` - First vector
/// * `b` - Second vector (must have same length as `a`)
/// 
/// # Returns
/// 
/// Cosine similarity score (0.0-1.0), or 0.0 if either vector has zero magnitude
/// 
/// # Panics
/// 
/// Panics if vectors have different lengths (should not happen in normal operation)
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(
        a.len(),
        b.len(),
        "Vectors must have same length for cosine similarity"
    );
    
    // Compute dot product
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    
    // Compute magnitudes
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    
    // Handle zero magnitude vectors
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    
    dot / (mag_a * mag_b)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let similarity = cosine_similarity(&a, &b);
        assert!((similarity - 1.0).abs() < 1e-6, "Identical vectors should have similarity 1.0");
    }
    
    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let similarity = cosine_similarity(&a, &b);
        assert!((similarity - 0.0).abs() < 1e-6, "Orthogonal vectors should have similarity 0.0");
    }
    
    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![-1.0, 0.0, 0.0];
        let similarity = cosine_similarity(&a, &b);
        assert!(
            (similarity - (-1.0)).abs() < 1e-6,
            "Opposite vectors should have similarity -1.0"
        );
    }
    
    #[test]
    fn test_cosine_similarity_zero_magnitude() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let similarity = cosine_similarity(&a, &b);
        assert_eq!(similarity, 0.0, "Zero magnitude vector should return 0.0");
    }
    
    #[test]
    fn test_parse_embedding_valid() {
        // Create a test embedding (4 floats = 16 bytes)
        let test_floats = vec![1.0f32, 2.0f32, 3.0f32, 4.0f32];
        let blob: Vec<u8> = test_floats
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        
        let parsed = parse_embedding(&blob);
        assert!(parsed.is_some());
        
        let parsed = parsed.unwrap();
        assert_eq!(parsed.len(), 4);
        for (original, parsed) in test_floats.iter().zip(parsed.iter()) {
            assert!((original - parsed).abs() < 1e-6);
        }
    }
    
    #[test]
    fn test_parse_embedding_invalid_length() {
        // Invalid length (not multiple of 4)
        let blob = vec![0u8, 1, 2, 3, 4]; // 5 bytes
        let parsed = parse_embedding(&blob);
        assert!(parsed.is_none());
    }
    
    #[test]
    fn test_parse_embedding_empty() {
        let blob = vec![];
        let parsed = parse_embedding(&blob);
        assert!(parsed.is_some());
        assert_eq!(parsed.unwrap().len(), 0);
    }
    
    #[test]
    fn test_cosine_similarity_normalized_vectors() {
        // Test with normalized unit vectors
        let a = vec![0.6, 0.8, 0.0];
        let b = vec![0.6, 0.8, 0.0];
        let similarity = cosine_similarity(&a, &b);
        assert!((similarity - 1.0).abs() < 1e-6, "Normalized identical vectors should have similarity 1.0");
    }
    
    #[test]
    fn test_cosine_similarity_different_magnitudes() {
        // Test that cosine similarity is magnitude-independent
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![2.0, 0.0, 0.0]; // Same direction, different magnitude
        let similarity = cosine_similarity(&a, &b);
        assert!((similarity - 1.0).abs() < 1e-6, "Vectors in same direction should have similarity 1.0 regardless of magnitude");
    }
    
    #[test]
    fn test_parse_embedding_1536_dimensions() {
        // Test parsing a full 1536-dimensional embedding (as used in production)
        let test_floats: Vec<f32> = (0..1536).map(|i| i as f32 * 0.001).collect();
        let blob: Vec<u8> = test_floats
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        
        let parsed = parse_embedding(&blob);
        assert!(parsed.is_some());
        
        let parsed = parsed.unwrap();
        assert_eq!(parsed.len(), 1536);
        for (original, parsed) in test_floats.iter().zip(parsed.iter()) {
            assert!((original - parsed).abs() < 1e-6);
        }
    }
    
    // Note: Integration tests for search_vector would require:
    // - Mock embedder or real API key
    // - Test database with embedded chunks
    // These should be run separately with proper test fixtures
}
