pub mod config;
pub mod error;
pub mod db;
pub mod ingest;
pub mod search;
pub mod embeddings;
pub mod mcp;
pub mod cache;
pub mod graph;
pub mod watch;
pub mod eval;

pub use config::Config;
pub use error::{RagmcpError, Result};
pub use graph::{Relation, extract_routing_relations, traverse_graph};
