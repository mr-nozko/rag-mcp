//! Evaluation framework: test query dataset, metrics (P@K, R@K, MRR), and CLI.

pub mod metrics;
pub mod query;

pub use metrics::{mean_reciprocal_rank, precision_at_k, recall_at_k};
pub use query::EvalQuery;
