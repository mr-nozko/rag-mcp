use thiserror::Error;

/// Main error type for RAGMcp
#[derive(Error, Debug)]
pub enum RagmcpError {
    /// Database-related errors
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    
    /// File system I/O errors
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    /// Configuration errors
    #[error("Configuration error: {0}")]
    Config(String),
    
    /// Embedding API errors
    #[error("Embedding API error: {0}")]
    Embedding(String),
    
    /// Document not found
    #[error("Document not found: {0}")]
    DocumentNotFound(String),
    
    /// Chunk not found
    #[error("Chunk not found: {0}")]
    ChunkNotFound(String),
    
    /// Parse errors
    #[error("Parse error: {0}")]
    Parse(String),
    
    /// MCP protocol errors
    #[error("MCP protocol error: {0}")]
    McpProtocol(String),
    
    /// Search errors
    #[error("Search error: {0}")]
    Search(String),
    
    /// Invalid input
    #[error("Invalid input: {0}")]
    InvalidInput(String),
}

/// Convenient Result type using RagmcpError
pub type Result<T> = std::result::Result<T, RagmcpError>;

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_error_display() {
        let err = RagmcpError::Config("Test error".to_string());
        assert!(err.to_string().contains("Configuration error"));
        assert!(err.to_string().contains("Test error"));
    }
    
    #[test]
    fn test_error_from_rusqlite() {
        let rusqlite_err = rusqlite::Error::InvalidQuery;
        let ragmcp_err: RagmcpError = rusqlite_err.into();
        assert!(matches!(ragmcp_err, RagmcpError::Database(_)));
    }
    
    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let ragmcp_err: RagmcpError = io_err.into();
        assert!(matches!(ragmcp_err, RagmcpError::Io(_)));
    }
}
