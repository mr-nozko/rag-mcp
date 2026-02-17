pub mod xml;
pub mod yaml;
pub mod json;
pub mod markdown;
pub mod plaintext;

use crate::error::Result;

/// A parsed document with sections
#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub content: String,
    pub sections: Vec<Section>,
    pub doc_type: String,
}

/// A section within a document
#[derive(Debug, Clone)]
pub struct Section {
    pub header: String,
    pub content: String,
    pub section_type: Option<String>,
}

/// Trait for document parsers
pub trait Parser {
    /// Check if this parser can handle the given file extension
    fn can_parse(&self, extension: &str) -> bool;
    
    /// Parse document content into structured sections
    fn parse(&self, content: &str, path: &str) -> Result<ParsedDocument>;
}

/// Parser registry that selects appropriate parser by extension
pub struct ParserRegistry {
    parsers: Vec<Box<dyn Parser>>,
}

impl ParserRegistry {
    /// Create a new parser registry with all built-in parsers
    pub fn new() -> Self {
        let mut registry = Self {
            parsers: Vec::new(),
        };
        
        registry.register(Box::new(xml::XmlParser));
        registry.register(Box::new(yaml::YamlParser));
        registry.register(Box::new(json::JsonParser));
        registry.register(Box::new(markdown::MarkdownParser));
        
        registry
    }
    
    /// Register a parser
    pub fn register(&mut self, parser: Box<dyn Parser>) {
        self.parsers.push(parser);
    }
    
    /// Find a parser that can handle the given extension
    pub fn find_parser(&self, extension: &str) -> Option<&dyn Parser> {
        self.parsers
            .iter()
            .find(|p| p.can_parse(extension))
            .map(|p| p.as_ref())
    }
    
    /// Parse content using the appropriate parser for the extension
    /// 
    /// If the primary parser fails (e.g., due to syntax errors), falls back
    /// to plain text parsing to ensure the file can still be ingested.
    pub fn parse(&self, content: &str, path: &str, extension: &str) -> Result<ParsedDocument> {
        let parser = self.find_parser(extension)
            .ok_or_else(|| crate::error::RagmcpError::Parse(
                format!("No parser found for extension: {}", extension)
            ))?;
        
        // Try primary parser first
        match parser.parse(content, path) {
            Ok(doc) => Ok(doc),
            Err(e) => {
                // Log the parse error but fall back to plain text
                log::warn!(
                    "Parser failed for {} ({}), falling back to plain text: {}",
                    path,
                    extension,
                    e
                );
                
                // Use plain text parser as fallback
                let plaintext_parser = plaintext::PlainTextParser;
                plaintext_parser.parse(content, path)
            }
        }
    }
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parser_registry() {
        let registry = ParserRegistry::new();
        
        assert!(registry.find_parser("xml").is_some());
        assert!(registry.find_parser("yaml").is_some());
        assert!(registry.find_parser("yml").is_some());
        assert!(registry.find_parser("json").is_some());
        assert!(registry.find_parser("md").is_some());
        assert!(registry.find_parser("txt").is_none());
    }
    
    #[test]
    fn test_parser_fallback_to_plaintext() {
        let registry = ParserRegistry::new();
        
        // Invalid JSON that should fail parsing
        let invalid_json = r#"{"key": "value", invalid}"#;
        
        // Should fall back to plain text instead of erroring
        let result = registry.parse(invalid_json, "test.json", "json");
        assert!(result.is_ok());
        
        let doc = result.unwrap();
        assert_eq!(doc.doc_type, "json_plaintext");
        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.sections[0].header, "content");
        assert!(doc.sections[0].content.contains("invalid"));
    }
}
