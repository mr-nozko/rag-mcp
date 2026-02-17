use sha2::{Sha256, Digest};
use std::path::Path;
use crate::error::Result;

/// Compute SHA256 hash of file contents
pub fn compute_file_hash(path: &Path) -> Result<String> {
    let content = std::fs::read(path)
        .map_err(crate::error::RagmcpError::Io)?;
    let mut hasher = Sha256::new();
    hasher.update(&content);
    let hash = hasher.finalize();
    Ok(format!("{:x}", hash))
}

/// Extract namespace from relative file path.
///
/// Namespace is derived from the first path segment (top-level directory):
/// - Paths with no directory (e.g. `readme.md`) → `"all"` (root-level / unclassified).
/// - Otherwise the first segment is normalized: lowercase, spaces become a single hyphen.
///
/// Examples: `Guides/...` → `guides`, `Research/...` → `research`, `API-Docs/...` → `api-docs`,
/// `coding-systems/...` → `coding-systems`, `Some New Dir/...` → `some-new-dir`.
///
/// Handles both forward slashes (Unix/MCP style) and backslashes (Windows style).
pub fn extract_namespace(relative_path: &str) -> String {
    // Normalize path separators to forward slashes for consistent matching
    let normalized = relative_path.replace('\\', "/");

    // No directory: root-level or single-segment file → "all"
    if !normalized.contains('/') {
        return "all".to_string();
    }

    // First path segment is the top-level directory name
    let first = normalized
        .split('/')
        .next()
        .unwrap_or("")
        .trim();

    if first.is_empty() {
        return "all".to_string();
    }

    // Normalize: lowercase, collapse one or more spaces to a single hyphen
    let ns = first.to_lowercase();
    let ns = ns
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    ns
}

/// Extract entity name (second-level directory) from a relative file path.
///
/// Works for **any directory structure** — not tied to any specific folder name.
/// Returns the second path segment for files nested at least two directories deep,
/// which acts as a sub-namespace or entity identifier (e.g. agent name, module,
/// category, or any meaningful grouping in the user's docs folder).
///
/// # Examples
///
/// ```text
/// "Agents/my-agent/prompt.xml"   → Some("my-agent")
/// "Guides/api/endpoints.md"      → Some("api")
/// "Research/topic/notes.md"      → Some("topic")
/// "System/README.md"             → None  (only one directory level)
/// "readme.md"                    → None  (root-level file)
/// ```
///
/// Handles both forward slashes (Unix/MCP) and backslashes (Windows).
pub fn extract_agent_name(relative_path: &str) -> Option<String> {
    // Normalize path separators to forward slashes for consistent cross-platform handling
    let normalized = relative_path.replace('\\', "/");

    // Split into segments; filter out empty strings from leading/trailing slashes
    let segments: Vec<&str> = normalized
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    // Need at least: [top-level-dir, second-level-dir, filename]
    // i.e. segments.len() >= 3 means the file lives in a sub-sub-directory
    if segments.len() >= 3 {
        let entity = segments[1].trim();
        if !entity.is_empty() {
            return Some(entity.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;
    
    #[test]
    fn test_compute_file_hash() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "test content").unwrap();
        
        let hash = compute_file_hash(&file_path).unwrap();
        assert_eq!(hash.len(), 64); // SHA256 produces 64 hex chars
    }
    
    #[test]
    fn test_extract_namespace() {
        // Forward slashes (Unix/MCP style) — namespace = first path segment (lowercased)
        assert_eq!(extract_namespace("Agents/module-alpha/overview.md"), "agents");
        assert_eq!(extract_namespace("System/README.md"), "system");
        assert_eq!(extract_namespace("Business/rules.yaml"), "business");
        assert_eq!(extract_namespace("other/file.md"), "other");
        assert_eq!(extract_namespace("Self/notes.md"), "self");
        assert_eq!(extract_namespace("Community/guide.md"), "community");
        assert_eq!(extract_namespace("coding-systems/README.md"), "coding-systems");
        assert_eq!(extract_namespace("Deep/nested/file.md"), "deep");

        // Root-level (no directory) → "all"
        assert_eq!(extract_namespace("readme.md"), "all");
        assert_eq!(extract_namespace("foo.xml"), "all");

        // Backslashes (Windows style)
        assert_eq!(extract_namespace("Agents\\module-alpha\\overview.md"), "agents");
        assert_eq!(extract_namespace("System\\README.md"), "system");
        assert_eq!(extract_namespace("Business\\rules.yaml"), "business");
        assert_eq!(extract_namespace("other\\file.md"), "other");
    }
    
    #[test]
    fn test_extract_agent_name() {
        // extract_agent_name is generic: returns the second-level directory for ANY
        // path nested at least 3 segments deep (TopDir/SubDir/file).

        // Forward slashes (Unix/MCP style)
        assert_eq!(
            extract_agent_name("Agents/my-agent/prompt.xml"),
            Some("my-agent".to_string())
        );
        assert_eq!(
            extract_agent_name("Guides/api/endpoints.md"),
            Some("api".to_string())
        );
        assert_eq!(
            extract_agent_name("Research/topic-a/notes.md"),
            Some("topic-a".to_string())
        );
        assert_eq!(
            extract_agent_name("Docs/section/deep/nested/file.md"),
            Some("section".to_string()) // always the second segment
        );

        // Single directory level → None (no sub-namespace available)
        assert_eq!(extract_agent_name("System/README.md"), None);
        assert_eq!(extract_agent_name("other/file.md"), None);
        // Root-level file → None
        assert_eq!(extract_agent_name("readme.md"), None);

        // Backslashes (Windows style — normalized internally)
        assert_eq!(
            extract_agent_name("Guides\\api\\endpoints.md"),
            Some("api".to_string())
        );
        assert_eq!(
            extract_agent_name("Agents\\my-agent\\tools.yaml"),
            Some("my-agent".to_string())
        );
        assert_eq!(extract_agent_name("System\\README.md"), None);
    }
}
