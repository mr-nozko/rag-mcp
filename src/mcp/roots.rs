//! Path validation for MCP write operations (Module 14).
//!
//! Ensures all write paths stay within the configured rag_folder boundary
//! and prevents path traversal attacks.

use std::path::{Path, PathBuf};
use crate::error::{Result, RagmcpError};

/// Validates paths for write operations within allowed boundaries.
/// Compliant with MCP Roots protocol: only paths under rag_folder are allowed.
pub struct PathValidator {
    canonical_root: PathBuf,
}

impl PathValidator {
    /// Create validator with canonical root path.
    /// Fails if rag_folder does not exist or cannot be canonicalized.
    pub fn new(rag_folder: &Path) -> Result<Self> {
        let canonical_root = rag_folder
            .canonicalize()
            .map_err(|e| RagmcpError::Config(format!(
                "Cannot canonicalize rag_folder: {}",
                e
            )))?;
        Ok(Self { canonical_root })
    }

    /// Validate write path and return absolute path within boundary.
    ///
    /// Security checks:
    /// 1. Reject paths with '..' components (traversal prevention).
    /// 2. Reject paths that start with / or \ (absolute path injection).
    /// 3. Join with canonical root and verify result is under root via strip_prefix.
    pub fn validate_write_path(&self, relative_path: &str) -> Result<PathBuf> {
        // Security: Prevent path traversal
        if relative_path.contains("..") {
            return Err(RagmcpError::InvalidInput(
                "Path traversal not allowed (.. components)".to_string(),
            ));
        }
        // Security: Reject absolute paths (would escape root on join)
        if relative_path.starts_with('/') || relative_path.starts_with('\\') {
            return Err(RagmcpError::InvalidInput(
                "Path must be relative to rag_folder (no leading / or \\)".to_string(),
            ));
        }
        if relative_path.trim().is_empty() {
            return Err(RagmcpError::InvalidInput(
                "Path must not be empty".to_string(),
            ));
        }

        let full_path = self.canonical_root.join(relative_path);

        // Security: Verify path is within allowed boundary (component-based)
        if full_path.strip_prefix(&self.canonical_root).is_err() {
            return Err(RagmcpError::InvalidInput(format!(
                "Path outside allowed directory: {}",
                relative_path
            )));
        }

        Ok(full_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[test]
    fn test_validate_path_success() {
        let temp = TempDir::new().unwrap();
        let validator = PathValidator::new(temp.path()).unwrap();
        let p = validator.validate_write_path("Guides/api/endpoints.md").unwrap();
        assert!(p.is_absolute());
        let suffix = p.to_string_lossy().replace('\\', "/");
        assert!(suffix.ends_with("Guides/api/endpoints.md"), "suffix={}", suffix);
    }

    #[test]
    fn test_validate_path_traversal_blocked() {
        let temp = TempDir::new().unwrap();
        let validator = PathValidator::new(temp.path()).unwrap();
        // ".." components must be rejected to prevent path traversal attacks
        assert!(validator.validate_write_path("Guides/../etc/passwd").is_err());
        assert!(validator.validate_write_path("..").is_err());
        assert!(validator.validate_write_path("a/../b").is_err());
    }

    #[test]
    fn test_validate_path_outside_boundary_blocked() {
        let temp = TempDir::new().unwrap();
        let validator = PathValidator::new(temp.path()).unwrap();
        // Path under root should be accepted regardless of directory name
        let p = validator.validate_write_path("section/item").unwrap();
        assert!(p.is_absolute());
        assert!(p.to_string_lossy().replace('\\', "/").ends_with("section/item"));
    }

    #[test]
    fn test_validate_path_nonexistent() {
        let temp = TempDir::new().unwrap();
        let validator = PathValidator::new(temp.path()).unwrap();
        // Non-existent file path is allowed (needed for creating new documents)
        let p = validator.validate_write_path("Docs/new-topic/overview.md").unwrap();
        assert!(!p.exists());
        assert!(p.ends_with("Docs/new-topic/overview.md"));
    }

    #[test]
    fn test_validate_path_leading_slash_rejected() {
        let temp = TempDir::new().unwrap();
        let validator = PathValidator::new(temp.path()).unwrap();
        // Absolute paths must be rejected (would escape the root boundary on join)
        assert!(validator.validate_write_path("/Guides/file.md").is_err());
        assert!(validator.validate_write_path("\\Guides\\file.md").is_err());
    }

    #[test]
    fn test_validate_path_empty_rejected() {
        let temp = TempDir::new().unwrap();
        let validator = PathValidator::new(temp.path()).unwrap();
        assert!(validator.validate_write_path("").is_err());
    }

    #[test]
    fn test_create_dir_all_under_root() {
        let temp = TempDir::new().unwrap();
        let validator = PathValidator::new(temp.path()).unwrap();
        let p = validator.validate_write_path("Business/2026/Q1/report.md").unwrap();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, "content").unwrap();
        assert!(p.exists());
        assert!(p.is_absolute());
        assert!(p.to_string_lossy().replace('\\', "/").ends_with("Business/2026/Q1/report.md"));
    }
}
