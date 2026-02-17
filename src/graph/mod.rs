//! Knowledge graph module: relation extraction and BFS traversal.
//!
//! Extracts relationships from document content (e.g. agent routing chains)
//! and traverses the entity_relations graph with depth limits.

mod extraction;
mod traversal;

pub use extraction::extract_routing_relations;
pub use traversal::traverse_graph;

use serde::{Deserialize, Serialize};

/// A single relation in the knowledge graph (source --relation_type--> target).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    /// Unique identifier (UUID v4).
    pub relation_id: String,
    /// Source entity, e.g. `agent:example`.
    pub source_entity: String,
    /// Relation type, e.g. `routes_to`, `uses_playbook`.
    pub relation_type: String,
    /// Target entity, e.g. `agent:target`.
    pub target_entity: String,
    /// Optional JSON metadata.
    pub metadata_json: Option<String>,
}
