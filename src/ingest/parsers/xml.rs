use super::{Parser, ParsedDocument, Section};
use quick_xml::Reader;
use quick_xml::events::Event;
use crate::error::{Result, RagmcpError};

/// XML parser for agent prompts and other XML documents
pub struct XmlParser;

impl Parser for XmlParser {
    fn can_parse(&self, extension: &str) -> bool {
        extension == "xml"
    }
    
    fn parse(&self, content: &str, path: &str) -> Result<ParsedDocument> {
        let mut reader = Reader::from_str(content);
        
        let mut sections = Vec::new();
        let mut buf = Vec::new();
        let mut current_section: Option<(String, String, Option<String>)> = None;
        let mut depth = 0;
        let mut root_tag = None;
        
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    
                    if root_tag.is_none() {
                        root_tag = Some(name.clone());
                    }
                    
                    // If we're starting a new top-level section (depth 1), save previous section
                    if depth == 1 && current_section.is_some() {
                        let (header, content, section_type) = current_section.take().unwrap();
                        if !content.trim().is_empty() {
                            sections.push(Section {
                                header: header.clone(),
                                content,
                                section_type: Some(section_type.unwrap_or_else(|| header.clone())),
                            });
                        }
                    }
                    
                    if depth == 1 {
                        // Top-level element becomes a section
                        let section_type = if name == "agent" {
                            None // Root agent tag, skip
                        } else {
                            Some(name.clone())
                        };
                        current_section = Some((name.clone(), String::new(), section_type));
                    }
                    
                    depth += 1;
                }
                Ok(Event::Text(e)) => {
                    if let Some((ref _header, ref mut content, _)) = current_section {
                        // Convert bytes to string, handling UTF-8
                        let text_str = String::from_utf8_lossy(e.as_ref());
                        content.push_str(&text_str);
                        content.push(' ');
                    }
                }
                Ok(Event::End(_)) => {
                    depth -= 1;
                    
                    // If we're closing a top-level section, finalize it
                    if depth == 1 && current_section.is_some() {
                        let (header, content, section_type) = current_section.take().unwrap();
                        if !content.trim().is_empty() {
                            sections.push(Section {
                                header: header.clone(),
                                content: content.trim().to_string(),
                                section_type: Some(section_type.unwrap_or_else(|| header.clone())),
                            });
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(e) => {
                    return Err(RagmcpError::Parse(format!(
                        "XML parse error in {}: {}",
                        path, e
                    )));
                }
            }
            buf.clear();
        }
        
        // Handle case where document has no sections (just root element)
        if sections.is_empty() {
            sections.push(Section {
                header: root_tag.unwrap_or_else(|| "root".to_string()),
                content: content.to_string(),
                section_type: None,
            });
        } else {
            // If we have sections but one is still open, finalize it
            if let Some((header, content, section_type)) = current_section.take() {
                if !content.trim().is_empty() {
                    sections.push(Section {
                        header: header.clone(),
                        content: content.trim().to_string(),
                        section_type: Some(section_type.unwrap_or_else(|| header.clone())),
                    });
                }
            }
        }
        
        let doc_type = if path.contains("prompt.xml") || path.contains("agent") {
            "agent_prompt".to_string()
        } else {
            "xml".to_string()
        };
        
        Ok(ParsedDocument {
            content: content.to_string(),
            sections,
            doc_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_xml_parser_can_parse() {
        let parser = XmlParser;
        assert!(parser.can_parse("xml"));
        assert!(!parser.can_parse("yaml"));
    }
    
    #[test]
    fn test_xml_parser_simple() {
        let parser = XmlParser;
        let content = r#"
            <agent>
                <Identity>Test Agent</Identity>
                <RoleStack>Role 1, Role 2</RoleStack>
            </agent>
        "#;
        
        let result = parser.parse(content, "test/agent/prompt.xml").unwrap();
        assert_eq!(result.doc_type, "agent_prompt");
        assert!(result.sections.len() >= 2);
        
        let identity_section = result.sections.iter()
            .find(|s| s.header == "Identity")
            .unwrap();
        assert!(identity_section.content.contains("Test Agent"));
    }
}
