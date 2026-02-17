use super::{Parser, ParsedDocument, Section};
use crate::error::Result;

/// Plain text fallback parser
/// 
/// Treats the entire file as a single section. Used as a fallback
/// when structured parsers (XML, YAML, JSON) fail due to syntax errors.
pub struct PlainTextParser;

impl Parser for PlainTextParser {
    fn can_parse(&self, _extension: &str) -> bool {
        // This parser can handle any extension as a fallback
        true
    }
    
    fn parse(&self, content: &str, path: &str) -> Result<ParsedDocument> {
        // Determine doc_type from extension
        let doc_type = if path.ends_with(".yaml") || path.ends_with(".yml") {
            "yaml_plaintext"
        } else if path.ends_with(".json") {
            "json_plaintext"
        } else if path.ends_with(".xml") {
            "xml_plaintext"
        } else {
            "plaintext"
        };
        
        // Create a single section with the full content
        let sections = vec![Section {
            header: "content".to_string(),
            content: content.to_string(),
            section_type: None,
        }];
        
        Ok(ParsedDocument {
            content: content.to_string(),
            sections,
            doc_type: doc_type.to_string(),
        })
    }
}
