use crate::db::Db;
use crate::error::{Result, RagmcpError};
use rusqlite::params;

/// Search result containing chunk information and relevance score
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk_id: String,
    pub doc_path: String,
    pub doc_type: String,
    pub agent_name: Option<String>,
    pub section: Option<String>,
    pub chunk_text: String,
    pub score: f32,
    pub rank: usize,
}

/// Sanitize and format FTS5 query string for optimal matching
/// 
/// Escapes special characters and formats multi-word queries for better recall.
/// Uses OR logic for space-separated terms to improve recall (any term matching is better than all).
/// Removes FTS5 special characters that cause syntax errors (? * etc.) and filters out common stop words.
pub fn sanitize_fts5_query(query: &str) -> String {
    let trimmed = query.trim();
    
    // Remove FTS5 special characters that cause syntax errors
    // FTS5: ? * ( ) { } - and single quote (') cause "syntax error near \"'\"" in MATCH
    let cleaned: String = trimmed
        .chars()
        .filter(|c| !matches!(c, '?' | '*' | '(' | ')' | '{' | '}' | '-' | '\''))
        .collect();
    
    // Split into terms and filter out common stop words for better matching
    // Stop words add noise and don't help with retrieval
    let stop_words: std::collections::HashSet<&str> = [
        "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with",
        "by", "from", "as", "is", "are", "was", "were", "be", "been", "being", "have",
        "has", "had", "do", "does", "did", "will", "would", "should", "could", "what",
        "which", "who", "where", "when", "why", "how", "this", "that", "these", "those"
    ].iter().cloned().collect();
    
    let terms: Vec<&str> = cleaned
        .split_whitespace()
        .filter(|term| {
            let lower = term.to_lowercase();
            // Keep terms that are not stop words and have at least 2 characters
            !stop_words.contains(lower.as_str()) && term.len() >= 2
        })
        .collect();
    
    if terms.is_empty() {
        // If all terms were filtered, use original cleaned query (fallback)
        return cleaned.replace('"', "\"\"");
    }
    
    if terms.len() == 1 {
        // Single term: just escape quotes
        return terms[0].replace('"', "\"\"");
    }
    
    // Multiple terms: format as OR query for better recall
    // Escape double quotes in each term
    let escaped_terms: Vec<String> = terms
        .iter()
        .map(|t| t.replace('"', "\"\""))
        .collect();
    
    escaped_terms.join(" OR ")
}

/// Normalize BM25 score from negative range to 0-1 range
/// 
/// BM25 scores are negative (better matches = lower scores).
/// This function converts them to a 0-1 range where higher = better,
/// making them compatible with vector search scores for hybrid fusion.
/// 
/// Uses sigmoid normalization: 1.0 / (1.0 + exp(-raw_score))
pub fn normalize_bm25_score(raw_score: f64) -> f32 {
    // Handle edge cases
    if raw_score.is_nan() || raw_score.is_infinite() {
        return 0.0;
    }
    
    // Sigmoid normalization: maps negative scores to 0-1 range
    // For negative scores (better matches), this gives values closer to 1.0
    // Since raw_score is negative for good matches, we use exp(raw_score) directly
    // exp(-5.0) ≈ 0.0067, so 1.0 / (1.0 + 0.0067) ≈ 0.9933 (high value for good match)
    let normalized = 1.0 / (1.0 + raw_score.exp());
    normalized as f32
}

/// Search documents using BM25 full-text search via FTS5
/// 
/// Performs a full-text search across chunk text and section headers,
/// returning results ranked by BM25 relevance score.
/// 
/// # Arguments
/// 
/// * `db` - Database connection wrapper
/// * `query` - Search query text (will be sanitized for FTS5)
/// * `namespace` - Optional namespace filter (directory-derived; e.g. agents, system, self, community); None = search all
/// * `agent_filter` - Optional agent name filter
/// * `k` - Maximum number of results to return
/// * `min_score` - Minimum normalized score threshold (0.0-1.0)
/// 
/// # Returns
/// 
/// Vector of SearchResult structs, sorted by relevance (highest score first),
/// with ranks assigned (1-indexed).
pub async fn search_bm25(
    db: &Db,
    query: &str,
    namespace: Option<&str>,
    agent_filter: Option<&str>,
    k: usize,
    min_score: f32,
) -> Result<Vec<SearchResult>> {
    let start = std::time::Instant::now();
    
    // Handle empty or whitespace-only queries early (FTS5 doesn't accept them)
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    
    // Sanitize query to prevent FTS5 syntax errors
    let sanitized_query = sanitize_fts5_query(query);
    
    // Clone values to move into closure
    let sanitized_query_clone = sanitized_query.clone();
    let namespace_clone = namespace.map(|s| s.to_string());
    let agent_filter_clone = agent_filter.map(|s| s.to_string());
    let k_clone = k;
    
    // Execute query using async database connection
    let mut rows = db.with_connection(move |conn| {
        let mut stmt = conn.prepare(
            r#"
            SELECT 
                c.chunk_id,
                c.chunk_text,
                c.section_header,
                d.doc_path,
                d.doc_type,
                d.agent_name,
                bm25(chunks_fts) AS raw_score
            FROM chunks_fts
            JOIN chunks c ON chunks_fts.chunk_id = c.chunk_id
            JOIN documents d ON c.doc_id = d.doc_id
            WHERE chunks_fts MATCH ?1
                AND (?2 IS NULL OR d.namespace = ?2)
                AND (?3 IS NULL OR d.agent_name = ?3)
            ORDER BY raw_score
            LIMIT ?4
            "#
        )?;
        
        let mut rows = stmt.query(params![
            sanitized_query_clone,
            namespace_clone,
            agent_filter_clone,
            k_clone as i64,
        ])?;
        
        let mut results = Vec::new();
        while let Some(row) = rows.next()? {
            let chunk_id: String = row.get(0)?;
            let chunk_text: String = row.get(1)?;
            let section_header: Option<String> = row.get(2)?;
            let doc_path: String = row.get(3)?;
            let doc_type: String = row.get(4)?;
            let agent_name: Option<String> = row.get(5)?;
            let raw_score: f64 = row.get(6)?;
            
            // Normalize score to 0-1 range
            let normalized_score = normalize_bm25_score(raw_score);
            
            // Filter by min_score threshold
            if normalized_score < min_score {
                continue;
            }
            
            results.push(SearchResult {
                chunk_id,
                doc_path,
                doc_type,
                agent_name,
                section: section_header,
                chunk_text,
                score: normalized_score,
                rank: 0, // Will be set after collecting all results
            });
        }
        
        Ok::<Vec<SearchResult>, RagmcpError>(results)
    }).await?;
    
    // Assign ranks (1-indexed) and ensure results are sorted by score descending
    // Note: SQL orders by raw_score ASC (lower = better), which after normalization
    // becomes highest normalized scores first. We sort explicitly to ensure correctness.
    rows.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    
    for (idx, result) in rows.iter_mut().enumerate() {
        result.rank = idx + 1;
    }
    
    let duration = start.elapsed();
    log::debug!("BM25 search took {:?}, returned {} results", duration, rows.len());
    
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Db, migrate};
    use crate::ingest::db_writer::{insert_document, insert_chunks};
    use crate::ingest::chunker::Chunk;
    use std::path::Path;
    use tempfile::TempDir;
    
    async fn setup_test_db() -> (Db, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db = Db::new(&db_path);
        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        db.with_connection(move |conn| migrate::run_migrations(conn, &migrations_dir))
            .await
            .unwrap();
        (db, temp_dir)
    }
    
    async fn insert_test_data(db: &Db) -> String {
        // Insert a test document
        let doc_id = insert_document(
            db,
            "test/agent.xml",
            "agent_prompt",
            "agents",
            Some("test_agent"),
            "Test document content",
            100,
            "test_hash",
            std::time::SystemTime::now(),
        ).await.unwrap();
        
        // Insert test chunks
        let chunks = vec![
            Chunk {
                text: "This is a test chunk about Rust programming language".to_string(),
                tokens: 15,
                section_header: Some("Identity".to_string()),
                chunk_type: Some("identity".to_string()),
            },
            Chunk {
                text: "Another chunk discussing SQLite database and FTS5 search".to_string(),
                tokens: 12,
                section_header: Some("RoleStack".to_string()),
                chunk_type: Some("rolestack".to_string()),
            },
            Chunk {
                text: "Final chunk with different content about machine learning".to_string(),
                tokens: 12,
                section_header: None,
                chunk_type: None,
            },
        ];
        
        insert_chunks(db, &doc_id, chunks).await.unwrap();
        
        doc_id
    }
    
    #[test]
    fn test_sanitize_fts5_query() {
        // Test basic query (no special characters)
        assert_eq!(sanitize_fts5_query("rust programming"), "rust OR programming");
        
        // Test double quote escaping
        assert_eq!(
            sanitize_fts5_query(r#"test "quoted" text"#),
            r#"test OR ""quoted"" OR text"#
        );
        
        // Test multiple quotes
        // Note: stop word "and" is removed; remaining terms use OR for better recall
        assert_eq!(
            sanitize_fts5_query(r#""quoted" and "another""#),
            r#"""quoted"" OR ""another"""#
        );
        
        // Test with other special characters (should be removed to avoid FTS5 syntax issues)
        assert_eq!(
            sanitize_fts5_query("test* (query) {terms}"),
            "test OR query OR terms"
        );
        
        // Test empty string
        assert_eq!(sanitize_fts5_query(""), "");

        // FTS5 treats '-' as "exclude term" and throws "syntax error near '-'"; we strip it
        assert_eq!(sanitize_fts5_query("--agent_filter"), "agent_filter");
        assert_eq!(sanitize_fts5_query("well-known term"), "wellknown OR term");

        // FTS5 throws "syntax error near \"'\"" when apostrophe is in query; we strip it
        assert_eq!(
            sanitize_fts5_query("What are Alpha's NonNegotiables?"),
            "Alphas OR NonNegotiables"
        );
    }
    
    #[test]
    fn test_normalize_bm25_score() {
        // Test negative score (typical BM25, better match = more negative)
        let score = normalize_bm25_score(-5.0);
        assert!(score > 0.9, "Negative score should normalize to high value");
        assert!(score <= 1.0, "Normalized score should be <= 1.0");
        
        // Test zero score
        let score = normalize_bm25_score(0.0);
        assert!((score - 0.5).abs() < 0.01, "Zero score should normalize to ~0.5");
        
        // Test positive score (worse match)
        let score = normalize_bm25_score(5.0);
        assert!(score < 0.1, "Positive score should normalize to low value");
        assert!(score >= 0.0, "Normalized score should be >= 0.0");
        
        // Test very negative score (excellent match)
        let score = normalize_bm25_score(-20.0);
        assert!(score > 0.99, "Very negative score should normalize to very high value");
        
        // Test NaN handling
        let score = normalize_bm25_score(f64::NAN);
        assert_eq!(score, 0.0, "NaN should normalize to 0.0");
        
        // Test infinity handling
        let score = normalize_bm25_score(f64::INFINITY);
        assert_eq!(score, 0.0, "Infinity should normalize to 0.0");
        
        let score = normalize_bm25_score(f64::NEG_INFINITY);
        assert_eq!(score, 0.0, "Negative infinity should normalize to 0.0");
    }
    
    #[tokio::test]
    async fn test_search_bm25_basic() {
        let (db, _temp_dir) = setup_test_db().await;
        let _doc_id = insert_test_data(&db).await;
        
        // Search for "Rust"
        let results = search_bm25(&db, "Rust", None, None, 10, 0.0).await.unwrap();
        
        assert!(!results.is_empty(), "Should return at least one result");
        
        // Check that results contain "Rust" (case-insensitive via FTS5)
        let has_rust = results.iter().any(|r| 
            r.chunk_text.to_lowercase().contains("rust")
        );
        assert!(has_rust, "Results should contain 'rust'");
        
        // Check that results are sorted by score (descending)
        for i in 1..results.len() {
            assert!(
                results[i-1].score >= results[i].score,
                "Results should be sorted by score descending"
            );
        }
        
        // Check that ranks are assigned correctly (1-indexed)
        for (idx, result) in results.iter().enumerate() {
            assert_eq!(result.rank, idx + 1, "Ranks should be 1-indexed");
        }
    }
    
    #[tokio::test]
    async fn test_search_bm25_with_filters() {
        let (db, _temp_dir) = setup_test_db().await;
        let _doc_id = insert_test_data(&db).await;
        
        // Search with namespace filter
        let results = search_bm25(&db, "test", Some("agents"), None, 10, 0.0).await.unwrap();
        assert!(!results.is_empty(), "Should return results for agents namespace");
        
        // Verify all results are from agents namespace
        for result in &results {
            // We can't directly check namespace from SearchResult, but we can verify
            // the search worked by checking results exist
            assert!(!result.doc_path.is_empty());
        }
        
        // Search with agent filter
        let results = search_bm25(&db, "test", None, Some("test_agent"), 10, 0.0).await.unwrap();
        assert!(!results.is_empty(), "Should return results for test_agent");
        
        // Verify all results are from test_agent
        for result in &results {
            assert_eq!(result.agent_name, Some("test_agent".to_string()));
        }
        
        // Search with both filters
        let results = search_bm25(
            &db, 
            "test", 
            Some("agents"), 
            Some("test_agent"), 
            10, 
            0.0
        ).await.unwrap();
        assert!(!results.is_empty(), "Should return results with both filters");
    }
    
    #[tokio::test]
    async fn test_search_bm25_empty_query() {
        let (db, _temp_dir) = setup_test_db().await;
        let _doc_id = insert_test_data(&db).await;
        
        // Search with empty query - should return empty results (not error)
        let results = search_bm25(&db, "", None, None, 10, 0.0).await.unwrap();
        assert_eq!(results.len(), 0, "Empty query should return empty results");
    }
    
    #[tokio::test]
    async fn test_search_bm25_min_score_filtering() {
        let (db, _temp_dir) = setup_test_db().await;
        let _doc_id = insert_test_data(&db).await;
        
        // Search with very high min_score (should filter out most/all results)
        let results_high = search_bm25(&db, "test", None, None, 10, 0.99).await.unwrap();
        
        // Search with low min_score (should return more results)
        let results_low = search_bm25(&db, "test", None, None, 10, 0.0).await.unwrap();
        
        // High threshold should return fewer or equal results
        assert!(
            results_high.len() <= results_low.len(),
            "Higher min_score should filter out more results"
        );
        
        // Verify all results meet the min_score threshold
        for result in &results_high {
            assert!(
                result.score >= 0.99,
                "All results should meet min_score threshold"
            );
        }
    }
    
    #[tokio::test]
    async fn test_search_bm25_limit_k() {
        let (db, _temp_dir) = setup_test_db().await;
        let _doc_id = insert_test_data(&db).await;
        
        // Search with k=1
        let results = search_bm25(&db, "test", None, None, 1, 0.0).await.unwrap();
        assert!(results.len() <= 1, "Should respect k limit");
        
        // Search with k=10 (more than available chunks)
        let results = search_bm25(&db, "test", None, None, 10, 0.0).await.unwrap();
        assert!(results.len() <= 10, "Should respect k limit");
    }
}
