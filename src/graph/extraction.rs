//! Relation extraction from document content (regex-based).

use regex::Regex;
use uuid::Uuid;

use super::Relation;

/// Extract agent routing relationships from document content.
/// Matches patterns like: "Agent-A → Agent-B → Agent-C"
pub fn extract_routing_relations(agent_name: &str, content: &str) -> Vec<Relation> {
    let mut relations = Vec::new();

    // Pattern matches: word → word (arrow syntax)
    let chains_regex =
        Regex::new(r"(\w+)\s*→\s*(\w+)").expect("Invalid regex pattern");

    for cap in chains_regex.captures_iter(content) {
        let from = cap.get(1).unwrap().as_str();
        let to = cap.get(2).unwrap().as_str();

        relations.push(Relation {
            relation_id: Uuid::new_v4().to_string(),
            source_entity: format!("agent:{}", from.to_lowercase()),
            relation_type: "routes_to".to_string(),
            target_entity: format!("agent:{}", to.to_lowercase()),
            metadata_json: Some(format!(
                "{{\"extracted_from\":\"agent:{}\"}}",
                agent_name
            )),
        });
    }

    relations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_routing_basic() {
        // Regex matches non-overlapping pairs; "A → B → C" yields one match (A→B)
        let content = "DefaultChains: Agent-A → Agent-B";
        let relations = extract_routing_relations("x", content);
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].source_entity, "agent:agent-a");
        assert_eq!(relations[0].target_entity, "agent:agent-b");
        assert_eq!(relations[0].relation_type, "routes_to");
    }

    #[test]
    fn test_extract_routing_multiple() {
        let content = "A → B and C → D";
        let relations = extract_routing_relations("agent1", content);
        assert_eq!(relations.len(), 2);
        assert_eq!(relations[0].source_entity, "agent:a");
        assert_eq!(relations[0].target_entity, "agent:b");
        assert_eq!(relations[1].source_entity, "agent:c");
        assert_eq!(relations[1].target_entity, "agent:d");
    }

    #[test]
    fn test_extract_routing_case_normalized() {
        let content = "AGENT-A → Agent-B";
        let relations = extract_routing_relations("x", content);
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].source_entity, "agent:agent-a");
        assert_eq!(relations[0].target_entity, "agent:agent-b");
    }

    #[test]
    fn test_extract_routing_no_matches() {
        let content = "No arrows here, just text.";
        let relations = extract_routing_relations("agent", content);
        assert!(relations.is_empty());
    }

    #[test]
    fn test_extract_routing_uuid_valid() {
        let content = "Foo → Bar";
        let relations = extract_routing_relations("a", content);
        assert_eq!(relations.len(), 1);
        // UUID v4 format: 8-4-4-4-12 hex
        let id = &relations[0].relation_id;
        assert!(id.len() == 36);
        assert!(id.chars().filter(|&c| c == '-').count() == 4);
    }

    #[test]
    fn test_extract_routing_metadata_contains_agent() {
        let content = "X → Y";
        let relations = extract_routing_relations("my_agent", content);
        assert_eq!(relations.len(), 1);
        let meta = relations[0].metadata_json.as_deref().unwrap();
        assert!(meta.contains("my_agent"));
    }
}
