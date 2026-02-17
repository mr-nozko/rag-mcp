use walkdir::WalkDir;
use std::path::{Path, PathBuf};
use crate::error::Result;

/// Metadata for a discovered file
#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub extension: String,
    pub file_size: u64,
    pub modified: std::time::SystemTime,
}

/// Discover all relevant files in the configured rag_folder (docs root directory).
///
/// Recursively walks the entire directory tree and indexes all common text-based
/// document formats. Binary files and unknown extensions are automatically skipped.
///
/// **Supported extensions** (case-insensitive):
/// - Documents: `.md`, `.txt`, `.xml`
/// - Data: `.yaml`, `.yml`, `.json`, `.toml`
/// - Code (optional — index your source too): `.rs`, `.py`, `.ts`, `.js`, `.go`
pub fn discover_files(root: &Path) -> Result<Vec<FileMetadata>> {
    let mut files = Vec::new();
    
    for entry in WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        
        if !path.is_file() {
            continue;
        }
        
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        
        // Filter for relevant text-based file types; binary files are skipped.
        // Add more extensions here if you want to index additional file types.
        if !matches!(
            extension.as_str(),
            // Documentation / markup
            "md" | "txt" | "xml" |
            // Data / configuration
            "yaml" | "yml" | "json" | "toml" |
            // Source code (optional — useful for code-knowledge RAG)
            "rs" | "py" | "ts" | "js" | "go"
        ) {
            continue;
        }
        
        let metadata = std::fs::metadata(path)
            .map_err(crate::error::RagmcpError::Io)?;
        
        let relative_path = path
            .strip_prefix(root)
            .map_err(|_| crate::error::RagmcpError::Config(
                format!("Failed to compute relative path for: {}", path.display())
            ))?
            .to_string_lossy()
            .to_string();
        
        files.push(FileMetadata {
            relative_path,
            absolute_path: path.to_path_buf(),
            extension,
            file_size: metadata.len(),
            modified: metadata.modified()
                .map_err(crate::error::RagmcpError::Io)?,
        });
    }
    
    log::info!("Discovered {} files in {}", files.len(), root.display());
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;
    
    #[test]
    fn test_discover_files() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();
        
        // Create a generic multi-level directory structure
        fs::create_dir_all(root.join("Guides/api")).unwrap();
        fs::write(root.join("overview.xml"), "<doc></doc>").unwrap();
        fs::write(root.join("config.yaml"), "key: value").unwrap();
        fs::write(root.join("schema.json"), "{}").unwrap();
        fs::write(root.join("README.md"), "# Docs").unwrap();
        fs::write(root.join("notes.txt"), "plain text note").unwrap();
        fs::write(root.join("Guides/api/endpoints.md"), "# API endpoints").unwrap();
        fs::write(root.join("image.png"), b"\x89PNG\r\n\x1a\n").unwrap(); // Binary PNG — should be skipped
        
        let files = discover_files(root).unwrap();
        
        // xml, yaml, json, md, txt, md (nested) = 6 relevant files; .png is skipped
        assert_eq!(files.len(), 6);
        assert!(files.iter().any(|f| f.relative_path.contains("overview.xml")));
        assert!(files.iter().any(|f| f.relative_path.contains("config.yaml")));
        assert!(files.iter().any(|f| f.relative_path.contains("schema.json")));
        assert!(files.iter().any(|f| f.relative_path.contains("README.md")));
        assert!(files.iter().any(|f| f.relative_path.contains("notes.txt")));
        assert!(files.iter().any(|f| f.relative_path.contains("endpoints.md")));
        assert!(!files.iter().any(|f| f.relative_path.contains("image.png")));
    }
    
    #[test]
    fn test_discover_files_empty() {
        let temp_dir = TempDir::new().unwrap();
        let files = discover_files(temp_dir.path()).unwrap();
        assert_eq!(files.len(), 0);
    }
}
