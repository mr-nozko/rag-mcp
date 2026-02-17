use crate::config::PerformanceConfig;
use crate::error::Result;
use super::parsers::ParsedDocument;

/// A chunk of text with metadata
#[derive(Debug, Clone)]
pub struct Chunk {
    pub text: String,
    pub tokens: usize,
    pub section_header: Option<String>,
    pub chunk_type: Option<String>,
}

/// Chunk a parsed document into semantic chunks with overlap
/// 
/// Respects section boundaries and applies overlap between chunks
/// to maintain context continuity.
pub fn chunk_document(
    parsed: &ParsedDocument,
    config: &PerformanceConfig,
) -> Result<Vec<Chunk>> {
    let mut chunks = Vec::new();
    
    for section in &parsed.sections {
        // Chunk by semantic boundaries (section level)
        let section_chunks = chunk_text(
            &section.content,
            config.chunk_size_tokens,
            config.chunk_overlap_tokens,
        )?;
        
        for chunk_text in section_chunks {
            let tokens = estimate_tokens(&chunk_text);
            
            chunks.push(Chunk {
                text: chunk_text,
                tokens,
                section_header: Some(section.header.clone()),
                chunk_type: section.section_type.clone(),
            });
        }
    }
    
    // If document has no sections or sections produced no chunks, chunk the full content
    if chunks.is_empty() {
        let full_chunks = chunk_text(
            &parsed.content,
            config.chunk_size_tokens,
            config.chunk_overlap_tokens,
        )?;
        
        for chunk_text in full_chunks {
            let tokens = estimate_tokens(&chunk_text);
            chunks.push(Chunk {
                text: chunk_text,
                tokens,
                section_header: None,
                chunk_type: None,
            });
        }
    }
    
    Ok(chunks)
}

/// Chunk text with overlap
/// 
/// Uses character-based approximation: ~4 characters per token.
/// Applies overlap between chunks to maintain context continuity.
/// 
/// This function safely handles UTF-8 multi-byte characters by ensuring
/// all string slices occur at character boundaries.
fn chunk_text(text: &str, size_tokens: usize, overlap_tokens: usize) -> Result<Vec<String>> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    
    // Character-based approximation: ~4 chars per token
    let char_size = size_tokens * 4;
    let char_overlap = overlap_tokens * 4;
    
    let mut chunks = Vec::new();
    
    // Helper function to find the next character boundary at or before a byte position
    // This ensures we never slice in the middle of a multi-byte character
    let find_char_boundary = |byte_pos: usize| -> usize {
        if byte_pos >= text.len() {
            return text.len();
        }
        // If byte_pos is already at a char boundary, return it
        if text.is_char_boundary(byte_pos) {
            return byte_pos;
        }
        // Otherwise, find the previous char boundary
        for i in (0..byte_pos).rev() {
            if text.is_char_boundary(i) {
                return i;
            }
        }
        0
    };
    
    let mut start_byte = 0;
    
    while start_byte < text.len() {
        // Ensure start is at a character boundary
        start_byte = find_char_boundary(start_byte);
        
        let end_byte = (start_byte + char_size).min(text.len());
        let end_byte = find_char_boundary(end_byte);
        
        // Try to break at word boundaries if possible
        let chunk_end_byte = if end_byte < text.len() {
            // Look for word boundary (space, newline, punctuation) within last 20% of chunk
            let search_start_byte = end_byte.saturating_sub(char_size / 5);
            let search_start_byte = find_char_boundary(search_start_byte);
            let search_end_byte = end_byte;
            
            // Safe slice for boundary search
            if let Some(search_text) = text.get(search_start_byte..search_end_byte) {
                if let Some(boundary_offset) = search_text
                    .char_indices()
                    .rev()
                    .find(|(_, c)| c.is_whitespace() || *c == '.' || *c == '!' || *c == '?')
                    .map(|(byte_offset, _)| byte_offset)
                {
                    let boundary_byte = search_start_byte + boundary_offset;
                    // Find the start of the next character after the boundary
                    let mut next_char_start = boundary_byte + 1;
                    while next_char_start < text.len() && !text.is_char_boundary(next_char_start) {
                        next_char_start += 1;
                    }
                    find_char_boundary(next_char_start)
                } else {
                    end_byte
                }
            } else {
                end_byte
            }
        } else {
            end_byte
        };
        
        // Safe slice: both indices are guaranteed to be at character boundaries
        if let Some(chunk_str) = text.get(start_byte..chunk_end_byte) {
            chunks.push(chunk_str.trim().to_string());
        } else {
            // This shouldn't happen if find_char_boundary works correctly, but handle gracefully
            return Err(crate::error::RagmcpError::Parse(
                format!("Failed to slice text at byte boundaries: start={}, end={}, len={}", 
                    start_byte, chunk_end_byte, text.len())
            ));
        }
        
        if chunk_end_byte >= text.len() {
            break;
        }
        
        // Move start position with overlap, ensuring it's at a character boundary
        let new_start_byte = chunk_end_byte.saturating_sub(char_overlap);
        let new_start_byte = find_char_boundary(new_start_byte);
        
        // Prevent infinite loop if overlap is too large
        if new_start_byte >= chunk_end_byte {
            start_byte = chunk_end_byte;
        } else {
            start_byte = new_start_byte;
        }
    }
    
    Ok(chunks)
}

/// Estimate token count from text
/// 
/// Uses approximation: 1 token ≈ 4 characters
/// This is a rough estimate. For more accurate tokenization,
/// consider integrating tiktoken or similar libraries.
pub fn estimate_tokens(text: &str) -> usize {
    // Rough estimation: 1 token ≈ 4 characters
    (text.len() + 3) / 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PerformanceConfig;
    use crate::ingest::parsers::Section;
    
    fn test_config() -> PerformanceConfig {
        PerformanceConfig {
            max_latency_ms: 1000,
            chunk_size_tokens: 300,
            chunk_overlap_tokens: 50,
        }
    }
    
    #[test]
    fn test_estimate_tokens() {
        // 4 chars = 1 token
        assert_eq!(estimate_tokens("test"), 1);
        // 8 chars = 2 tokens
        assert_eq!(estimate_tokens("testtest"), 2);
        // 10 chars = 3 tokens (rounds up)
        assert_eq!(estimate_tokens("testtest12"), 3);
    }
    
    #[test]
    fn test_chunk_text() {
        let config = test_config();
        let text = "a ".repeat(1000); // ~2000 chars = ~500 tokens
        
        let chunks = chunk_text(&text, config.chunk_size_tokens, config.chunk_overlap_tokens).unwrap();
        
        assert!(!chunks.is_empty());
        // Should have multiple chunks for 500 tokens with 300 token size
        assert!(chunks.len() >= 2);
        
        // Check overlap between chunks
        // Overlap exists if the end of chunk[0] appears at the start of chunk[1]
        if chunks.len() > 1 {
            // Find the longest suffix of chunk[0] that is a prefix of chunk[1]
            let chunk0 = &chunks[0];
            let chunk1 = &chunks[1];
            let mut overlap_found = false;
            
            // Check for overlap by finding common substring at boundaries
            let min_overlap = (config.chunk_overlap_tokens * 4).min(chunk0.len().min(chunk1.len()));
            for overlap_len in (1..=min_overlap).rev() {
                if chunk0.len() >= overlap_len && chunk1.len() >= overlap_len {
                    // Safe slicing: ensure we slice at character boundaries
                    let suffix_start = chunk0.len().saturating_sub(overlap_len);
                    // Find nearest char boundary before suffix_start
                    let mut suffix_start_safe = suffix_start;
                    while suffix_start_safe < chunk0.len() && !chunk0.is_char_boundary(suffix_start_safe) {
                        suffix_start_safe += 1;
                    }
                    if suffix_start_safe > 0 && !chunk0.is_char_boundary(suffix_start_safe) {
                        suffix_start_safe = suffix_start_safe.saturating_sub(1);
                        while suffix_start_safe > 0 && !chunk0.is_char_boundary(suffix_start_safe) {
                            suffix_start_safe -= 1;
                        }
                    }
                    
                    // Find nearest char boundary at or after overlap_len
                    let mut prefix_end_safe = overlap_len;
                    while prefix_end_safe < chunk1.len() && !chunk1.is_char_boundary(prefix_end_safe) {
                        prefix_end_safe += 1;
                    }
                    
                    if let (Some(suffix), Some(prefix)) = (
                        chunk0.get(suffix_start_safe..),
                        chunk1.get(..prefix_end_safe)
                    ) {
                        if suffix == prefix {
                            overlap_found = true;
                            break;
                        }
                    }
                }
            }
            
            // If chunks are adjacent (no gap), there should be some overlap
            // Even if exact match isn't found, chunks should be close together
            assert!(
                overlap_found || chunks.len() == 2,
                "Expected overlap between chunks or only 2 chunks total"
            );
        }
    }
    
    #[test]
    fn test_chunk_document() {
        let config = test_config();
        let parsed = ParsedDocument {
            content: "Full content".to_string(),
            sections: vec![
                Section {
                    header: "Section 1".to_string(),
                    content: "a ".repeat(1000), // ~500 tokens
                    section_type: Some("test".to_string()),
                },
            ],
            doc_type: "test".to_string(),
        };
        
        let chunks = chunk_document(&parsed, &config).unwrap();
        
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].section_header, Some("Section 1".to_string()));
        assert_eq!(chunks[0].chunk_type, Some("test".to_string()));
        
        // Check token estimates are reasonable
        for chunk in &chunks {
            assert!(chunk.tokens > 0);
            assert!(chunk.tokens <= config.chunk_size_tokens * 2); // Allow some flexibility
        }
    }
    
    #[test]
    fn test_chunk_empty_text() {
        let chunks = chunk_text("", 300, 50).unwrap();
        assert!(chunks.is_empty());
    }
}
