use super::{Parser, ParsedDocument, Section};
use crate::error::{Result, RagmcpError};
use serde_json::Value as JsonValue;

/// JSON parser for JSON Schema and other JSON files
pub struct JsonParser;

impl Parser for JsonParser {
    fn can_parse(&self, extension: &str) -> bool {
        extension == "json"
    }
    
    fn parse(&self, content: &str, path: &str) -> Result<ParsedDocument> {
        let json_value: JsonValue = serde_json::from_str(content)
            .map_err(|e| RagmcpError::Parse(format!("JSON parse error in {}: {}", path, e)))?;
        
        let mut sections = Vec::new();
        
        // Handle JSON Schema format
        if let JsonValue::Object(map) = &json_value {
            // Extract definitions if present (JSON Schema)
            if let Some(JsonValue::Object(definitions)) = map.get("definitions") {
                for (name, schema) in definitions {
                    let content = format_schema_definition(name, schema);
                    sections.push(Section {
                        header: format!("Definition: {}", name),
                        content,
                        section_type: Some("definition".to_string()),
                    });
                }
            }
            
            // Extract properties if present (JSON Schema)
            if let Some(JsonValue::Object(properties)) = map.get("properties") {
                for (name, prop) in properties {
                    let content = format_schema_property(name, prop);
                    sections.push(Section {
                        header: format!("Property: {}", name),
                        content,
                        section_type: Some("property".to_string()),
                    });
                }
            }
            
            // Extract top-level keys as sections if no definitions/properties
            if sections.is_empty() {
                for (key, value) in map {
                    sections.push(Section {
                        header: key.clone(),
                        content: json_value_to_text(value),
                        section_type: Some(key.clone()),
                    });
                }
            }
        } else {
            // Non-object JSON - treat as single section
            sections.push(Section {
                header: "content".to_string(),
                content: json_value_to_text(&json_value),
                section_type: None,
            });
        }
        
        // If no sections were created, create one with full content
        if sections.is_empty() {
            sections.push(Section {
                header: "root".to_string(),
                content: content.to_string(),
                section_type: None,
            });
        }
        
        Ok(ParsedDocument {
            content: content.to_string(),
            sections,
            doc_type: "json_schema".to_string(),
        })
    }
}

/// Format a JSON Schema definition as readable text
fn format_schema_definition(name: &str, schema: &JsonValue) -> String {
    let mut parts = vec![format!("Definition: {}", name)];
    
    if let JsonValue::Object(map) = schema {
        if let Some(JsonValue::String(desc)) = map.get("description") {
            parts.push(format!("Description: {}", desc));
        }
        
        if let Some(JsonValue::String(typ)) = map.get("type") {
            parts.push(format!("Type: {}", typ));
        }
        
        if let Some(JsonValue::Object(props)) = map.get("properties") {
            parts.push("Properties:".to_string());
            for (prop_name, prop_value) in props {
                parts.push(format!("  - {}: {}", prop_name, json_value_to_text(prop_value)));
            }
        }
    }
    
    parts.join("\n")
}

/// Format a JSON Schema property as readable text
fn format_schema_property(name: &str, prop: &JsonValue) -> String {
    let mut parts = vec![format!("Property: {}", name)];
    
    if let JsonValue::Object(map) = prop {
        if let Some(JsonValue::String(typ)) = map.get("type") {
            parts.push(format!("Type: {}", typ));
        }
        
        if let Some(JsonValue::String(desc)) = map.get("description") {
            parts.push(format!("Description: {}", desc));
        }
        
        if let Some(JsonValue::Array(enum_vals)) = map.get("enum") {
            parts.push(format!("Enum values: {:?}", enum_vals));
        }
    } else {
        parts.push(json_value_to_text(prop));
    }
    
    parts.join("\n")
}

/// Convert JSON value to readable text representation
fn json_value_to_text(value: &JsonValue) -> String {
    match value {
        JsonValue::String(s) => s.clone(),
        JsonValue::Number(n) => n.to_string(),
        JsonValue::Bool(b) => b.to_string(),
        JsonValue::Null => "null".to_string(),
        JsonValue::Array(arr) => {
            arr.iter()
                .map(json_value_to_text)
                .collect::<Vec<_>>()
                .join(", ")
        }
        JsonValue::Object(map) => {
            map.iter()
                .map(|(k, v)| format!("{}: {}", k, json_value_to_text(v)))
                .collect::<Vec<_>>()
                .join(", ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_json_parser_can_parse() {
        let parser = JsonParser;
        assert!(parser.can_parse("json"));
        assert!(!parser.can_parse("xml"));
    }
    
    #[test]
    fn test_json_parser_schema() {
        let parser = JsonParser;
        let content = r#"
{
  "definitions": {
    "TestType": {
      "type": "object",
      "description": "A test type",
      "properties": {
        "name": {"type": "string"}
      }
    }
  }
}
"#;
        
        let result = parser.parse(content, "test.json").unwrap();
        assert_eq!(result.doc_type, "json_schema");
        assert!(result.sections.len() >= 1);
        
        let def_section = result.sections.iter()
            .find(|s| s.header.contains("Definition"))
            .unwrap();
        assert!(def_section.content.contains("TestType"));
    }
}
