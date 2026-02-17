//! Evaluation metrics: Precision@K, Recall@K, and Mean Reciprocal Rank (MRR).

use crate::eval::EvalQuery;
use crate::search::SearchResult;
use std::collections::HashSet;

/// Precision at K: proportion of top-K results that are relevant.
/// Returns (relevant count in top-K) / K. If k is 0, returns 0.0.
pub fn precision_at_k(
    results: &[SearchResult],
    relevant_chunk_ids: &[String],
    k: usize,
) -> f32 {
    if k == 0 {
        return 0.0;
    }
    let relevant: HashSet<&str> = relevant_chunk_ids.iter().map(String::as_str).collect();
    let top_k = results.iter().take(k);
    let relevant_count = top_k
        .filter(|r| relevant.contains(r.chunk_id.as_str()))
        .count();
    relevant_count as f32 / k as f32
}

/// Recall at K: proportion of all relevant chunks that appear in top-K.
/// Returns (relevant retrieved in top-K) / |relevant_chunk_ids|. If there are no relevant
/// chunks (denominator 0), returns 0.0.
pub fn recall_at_k(
    results: &[SearchResult],
    relevant_chunk_ids: &[String],
    k: usize,
) -> f32 {
    if relevant_chunk_ids.is_empty() {
        return 0.0;
    }
    let relevant: HashSet<&str> = relevant_chunk_ids.iter().map(String::as_str).collect();
    let top_k = results.iter().take(k);
    let retrieved_relevant = top_k
        .filter(|r| relevant.contains(r.chunk_id.as_str()))
        .count();
    retrieved_relevant as f32 / relevant_chunk_ids.len() as f32
}

/// Mean Reciprocal Rank: average of 1/rank of the first relevant result per query.
/// For each query, finds the first result where query.is_relevant(r), adds 1/(rank+1);
/// if no relevant result, adds 0. Returns sum / queries.len(). If queries is empty, returns 0.0.
pub fn mean_reciprocal_rank(
    queries: &[EvalQuery],
    results: &[Vec<SearchResult>],
) -> f32 {
    if queries.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0;
    for (query, result_set) in queries.iter().zip(results.iter()) {
        for (rank, result) in result_set.iter().enumerate() {
            if query.is_relevant(result) {
                sum += 1.0 / (rank + 1) as f32;
                break;
            }
        }
    }
    sum / queries.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(chunk_id: &str, rank: usize) -> SearchResult {
        SearchResult {
            chunk_id: chunk_id.to_string(),
            doc_path: String::new(),
            doc_type: String::new(),
            agent_name: None,
            section: None,
            chunk_text: String::new(),
            score: 0.0,
            rank,
        }
    }

    #[test]
    fn precision_at_k_all_relevant() {
        let results = vec![
            make_result("a", 1),
            make_result("b", 2),
            make_result("c", 3),
        ];
        let relevant = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert!((precision_at_k(&results, &relevant, 3) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn precision_at_k_partial() {
        let results = vec![
            make_result("a", 1),
            make_result("b", 2),
            make_result("x", 3),
        ];
        let relevant = vec!["a".to_string(), "b".to_string()];
        assert!((precision_at_k(&results, &relevant, 3) - 2.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn precision_at_k_zero_k() {
        let results = vec![make_result("a", 1)];
        let relevant = vec!["a".to_string()];
        assert_eq!(precision_at_k(&results, &relevant, 0), 0.0);
    }

    #[test]
    fn recall_at_k_all_retrieved() {
        let results = vec![
            make_result("a", 1),
            make_result("b", 2),
        ];
        let relevant = vec!["a".to_string(), "b".to_string()];
        assert!((recall_at_k(&results, &relevant, 10) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn recall_at_k_partial() {
        let results = vec![make_result("a", 1), make_result("x", 2)];
        let relevant = vec!["a".to_string(), "b".to_string()];
        assert!((recall_at_k(&results, &relevant, 10) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn recall_at_k_empty_relevant() {
        let results = vec![make_result("a", 1)];
        let relevant: Vec<String> = vec![];
        assert_eq!(recall_at_k(&results, &relevant, 10), 0.0);
    }

    #[test]
    fn mrr_first_rank() {
        let query = EvalQuery {
            query: String::new(),
            category: String::new(),
            expected_doc: Some("doc.xml".to_string()),
            expected_section: None,
            expected_entities: None,
            min_rank: None,
            relevant_chunk_ids: None,
        };
        let first = SearchResult {
            chunk_id: "x".to_string(),
            doc_path: "doc.xml".to_string(),
            doc_type: String::new(),
            agent_name: None,
            section: None,
            chunk_text: String::new(),
            score: 0.0,
            rank: 0,
        };
        assert!(query.is_relevant(&first));
        let results = vec![vec![first, make_result("y", 1)]];
        let mrr = mean_reciprocal_rank(&[query], &results);
        assert!((mrr - 1.0).abs() < 1e-6, "MRR should be 1.0 when first result relevant");
    }

    #[test]
    fn mrr_second_rank() {
        let query = EvalQuery {
            query: String::new(),
            category: String::new(),
            expected_doc: Some("doc.xml".to_string()),
            expected_section: None,
            expected_entities: None,
            min_rank: None,
            relevant_chunk_ids: None,
        };
        let results = vec![vec![
            make_result("a", 0),
            SearchResult {
                chunk_id: "b".to_string(),
                doc_path: "doc.xml".to_string(),
                doc_type: String::new(),
                agent_name: None,
                section: None,
                chunk_text: String::new(),
                score: 0.0,
                rank: 1,
            },
        ]];
        let mrr = mean_reciprocal_rank(&[query], &results);
        assert!((mrr - 0.5).abs() < 1e-6, "MRR should be 1/2 when second result relevant");
    }

    #[test]
    fn mrr_empty_queries() {
        let results = vec![vec![make_result("a", 0)]];
        assert_eq!(mean_reciprocal_rank(&[], &results), 0.0);
    }
}
