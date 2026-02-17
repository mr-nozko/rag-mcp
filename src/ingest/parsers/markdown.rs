use super::{Parser, ParsedDocument, Section};
use crate::error::Result;
use pulldown_cmark::{Parser as CmarkParser, Event, Tag, CodeBlockKind, TagEnd};

/// Markdown parser for README files and documentation
pub struct MarkdownParser;

impl Parser for MarkdownParser {
    fn can_parse(&self, extension: &str) -> bool {
        extension == "md"
    }
    
    fn parse(&self, content: &str, _path: &str) -> Result<ParsedDocument> {
        let parser = CmarkParser::new(content);
        let mut sections = Vec::new();
        let mut current_section: Option<(String, String, usize)> = None; // (header, content, level)
        let mut current_content = String::new();
        let mut frontmatter: Option<String> = None;
        let mut in_frontmatter = false;
        
        // Check for YAML frontmatter
        if content.starts_with("---\n") {
            if let Some(end) = content[4..].find("---\n") {
                frontmatter = Some(content[4..end + 4].to_string());
                in_frontmatter = true;
            }
        }
        
        let events = parser.into_iter();
        let mut skip_until_header = in_frontmatter;
        
        for event in events {
            match event {
                Event::Start(Tag::Heading { level, .. }) => {
                    // Save previous section if exists
                    if let Some((header, content, _)) = current_section.take() {
                        if !content.trim().is_empty() {
                            sections.push(Section {
                                header: header.clone(),
                                content: content.trim().to_string(),
                                section_type: Some(format!("h{}", level as u32)),
                            });
                        }
                    }
                    
                    // Start new section
                    current_content.clear();
                    current_section = Some((String::new(), String::new(), level as usize));
                    skip_until_header = false;
                }
                Event::Text(text) => {
                    if skip_until_header {
                        continue;
                    }
                    
                    if let Some((ref mut header, ref mut content, _)) = current_section {
                        if header.is_empty() {
                            // This text is the header
                            *header = text.to_string();
                        } else {
                            // This text is content
                            content.push_str(&text);
                            content.push(' ');
                        }
                    } else {
                        // No current section, add to content
                        current_content.push_str(&text);
                        current_content.push(' ');
                    }
                }
                Event::Code(code) => {
                    if skip_until_header {
                        continue;
                    }
                    
                    if let Some((_, ref mut content, _)) = current_section {
                        content.push_str("```");
                        content.push_str(&code);
                        content.push_str("``` ");
                    } else {
                        current_content.push_str("```");
                        current_content.push_str(&code);
                        current_content.push_str("``` ");
                    }
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    if skip_until_header {
                        continue;
                    }
                    
                    let lang = match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    
                    if let Some((_, ref mut content, _)) = current_section {
                        content.push_str(&format!("```{}\n", lang));
                    } else {
                        current_content.push_str(&format!("```{}\n", lang));
                    }
                }
                Event::End(TagEnd::CodeBlock) => {
                    if skip_until_header {
                        continue;
                    }
                    
                    if let Some((_, ref mut content, _)) = current_section {
                        content.push_str("```\n");
                    } else {
                        current_content.push_str("```\n");
                    }
                }
                Event::SoftBreak | Event::HardBreak => {
                    if skip_until_header {
                        continue;
                    }
                    
                    if let Some((_, ref mut content, _)) = current_section {
                        content.push('\n');
                    } else {
                        current_content.push('\n');
                    }
                }
                _ => {}
            }
        }
        
        // Save final section
        if let Some((header, content, _)) = current_section.take() {
            if !content.trim().is_empty() {
                sections.push(Section {
                    header: header.clone(),
                    content: content.trim().to_string(),
                    section_type: Some("markdown".to_string()),
                });
            }
        }
        
        // If no sections were created, create one with full content
        if sections.is_empty() {
            sections.push(Section {
                header: "content".to_string(),
                content: if !current_content.is_empty() {
                    current_content.trim().to_string()
                } else {
                    content.to_string()
                },
                section_type: None,
            });
        }
        
        // Add frontmatter as first section if present
        if let Some(fm) = frontmatter {
            sections.insert(0, Section {
                header: "frontmatter".to_string(),
                content: fm,
                section_type: Some("frontmatter".to_string()),
            });
        }
        
        Ok(ParsedDocument {
            content: content.to_string(),
            sections,
            doc_type: "markdown".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_markdown_parser_can_parse() {
        let parser = MarkdownParser;
        assert!(parser.can_parse("md"));
        assert!(!parser.can_parse("txt"));
    }
    
    #[test]
    fn test_markdown_parser_simple() {
        let parser = MarkdownParser;
        let content = r#"
# Title

This is content.

## Subsection

More content.
"#;
        
        let result = parser.parse(content, "test.md").unwrap();
        assert_eq!(result.doc_type, "markdown");
        assert!(result.sections.len() >= 2);
        
        let title_section = result.sections.iter()
            .find(|s| s.header == "Title")
            .unwrap();
        assert!(title_section.content.contains("This is content"));
    }
}
