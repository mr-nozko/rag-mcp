use super::{Parser, ParsedDocument, Section};
use crate::error::{Result, RagmcpError};
use serde_yaml_ng::Value as YamlValue;

/// YAML parser for tools, guardrails, and configuration files
pub struct YamlParser;

impl Parser for YamlParser {
    fn can_parse(&self, extension: &str) -> bool {
        matches!(extension, "yaml" | "yml")
    }
    
    fn parse(&self, content: &str, path: &str) -> Result<ParsedDocument> {
        let yaml_value: YamlValue = serde_yaml_ng::from_str(content)
            .map_err(|e| RagmcpError::Parse(format!("YAML parse error in {}: {}", path, e)))?;
        
        let mut sections = Vec::new();
        
        match yaml_value {
            YamlValue::Mapping(map) => {
                // Extract top-level keys as sections
                for (key, value) in map {
                    let header = match key {
                        YamlValue::String(s) => s,
                        YamlValue::Number(n) => n.to_string(),
                        YamlValue::Bool(b) => b.to_string(),
                        _ => "unknown".to_string(),
                    };
                    
                    let content = yaml_value_to_text(&value);
                    
                    sections.push(Section {
                        header: header.clone(),
                        content,
                        section_type: Some(header),
                    });
                }
            }
            _ => {
                // Single value or other structure - treat as one section
                sections.push(Section {
                    header: "root".to_string(),
                    content: yaml_value_to_text(&yaml_value),
                    section_type: None,
                });
            }
        }
        
        // If no sections were created, create one with full content
        if sections.is_empty() {
            sections.push(Section {
                header: "content".to_string(),
                content: content.to_string(),
                section_type: None,
            });
        }
        
        Ok(ParsedDocument {
            content: content.to_string(),
            sections,
            doc_type: "yaml".to_string(),
        })
    }
}

/// Convert YAML value to readable text representation
fn yaml_value_to_text(value: &YamlValue) -> String {
    match value {
        YamlValue::String(s) => s.clone(),
        YamlValue::Number(n) => n.to_string(),
        YamlValue::Bool(b) => b.to_string(),
        YamlValue::Null => "null".to_string(),
        YamlValue::Sequence(seq) => {
            seq.iter()
                .map(yaml_value_to_text)
                .collect::<Vec<_>>()
                .join(", ")
        }
        YamlValue::Mapping(map) => {
            map.iter()
                .map(|(k, v)| {
                    let key = match k {
                        YamlValue::String(s) => s.clone(),
                        YamlValue::Number(n) => n.to_string(),
                        YamlValue::Bool(b) => b.to_string(),
                        _ => "key".to_string(),
                    };
                    format!("{}: {}", key, yaml_value_to_text(v))
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        YamlValue::Tagged(tagged) => {
            yaml_value_to_text(&tagged.value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_yaml_parser_can_parse() {
        let parser = YamlParser;
        assert!(parser.can_parse("yaml"));
        assert!(parser.can_parse("yml"));
        assert!(!parser.can_parse("json"));
    }
    
    #[test]
    fn test_yaml_parser_simple() {
        let parser = YamlParser;
        let content = r#"
key1: value1
key2: value2
nested:
  subkey: subvalue
"#;
        
        let result = parser.parse(content, "test.yaml").unwrap();
        assert_eq!(result.doc_type, "yaml");
        assert!(result.sections.len() >= 2);
        
        let key1_section = result.sections.iter()
            .find(|s| s.header == "key1")
            .unwrap();
        assert!(key1_section.content.contains("value1"));
    }
}
