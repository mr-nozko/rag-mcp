//! Incremental ingestion: skip unchanged files by comparing file hashes with database.
//!
//! This module provides hash-based classification so the ingest CLI only processes
//! new or modified files, avoiding redundant parsing, chunking, and DB writes.

use std::collections::{HashMap, HashSet};

use crate::db::Db;
use crate::error::{Result, RagmcpError};
use crate::ingest::{compute_file_hash, FileMetadata};

/// Result of classifying discovered files against the database.
#[derive(Debug, Default)]
pub struct FileClassification {
    /// Files not present in the database (need full processing).
    pub new_files: Vec<FileMetadata>,
    /// Files present in the database but with a different hash (need re-processing).
    pub modified_files: Vec<FileMetadata>,
    /// Files present in the database with the same hash (skip processing).
    pub unchanged_files: Vec<FileMetadata>,
}

/// Load all document paths and their stored file hashes from the database.
///
/// Returns a map of `doc_path` → `file_hash` for O(1) lookup during classification.
/// Paths are stored as in the database (no normalization).
pub async fn get_existing_hashes(db: &Db) -> Result<HashMap<String, String>> {
    db.with_connection(|conn| {
        let mut stmt = conn.prepare("SELECT doc_path, file_hash FROM documents")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (path, hash) = row?;
            map.insert(path, hash);
        }
        Ok::<HashMap<String, String>, RagmcpError>(map)
    })
    .await
}

/// Classify discovered files into new, modified, or unchanged relative to the database.
///
/// For each file, computes the current file hash and compares with `existing_hashes`.
/// Unchanged files can be skipped during ingestion.
pub fn classify_files(
    files: &[FileMetadata],
    existing_hashes: &HashMap<String, String>,
) -> Result<FileClassification> {
    let mut classification = FileClassification::default();

    for file in files {
        let current_hash = compute_file_hash(&file.absolute_path)?;
        let existing = existing_hashes.get(&file.relative_path);

        match existing {
            None => classification.new_files.push(file.clone()),
            Some(stored) if stored != &current_hash => classification.modified_files.push(file.clone()),
            Some(_) => classification.unchanged_files.push(file.clone()),
        }
    }

    Ok(classification)
}

/// Find document paths that exist in the database but no longer exist on the filesystem.
///
/// `existing_files` is the set of relative paths of files currently discovered on disk.
/// Returns doc_paths that are in the database but not in that set (candidates for cleanup).
pub async fn find_deleted_documents(
    db: &Db,
    existing_files: &HashSet<String>,
) -> Result<Vec<String>> {
    let db_paths: Vec<String> = db
        .with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT doc_path FROM documents")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut paths = Vec::new();
            for row in rows {
                paths.push(row?);
            }
            Ok::<Vec<String>, RagmcpError>(paths)
        })
        .await?;

    let deleted: Vec<String> = db_paths
        .into_iter()
        .filter(|p| !existing_files.contains(p))
        .collect();

    Ok(deleted)
}

/// Delete documents by path from the database.
///
/// Chunks are removed automatically via foreign key CASCADE.
/// Returns the number of documents deleted.
pub async fn delete_documents(db: &Db, doc_paths: &[String]) -> Result<usize> {
    if doc_paths.is_empty() {
        return Ok(0);
    }

    let count = doc_paths.len();
    let paths: Vec<String> = doc_paths.to_vec();

    db.with_connection(move |conn| {
        for doc_path in &paths {
            conn.execute("DELETE FROM documents WHERE doc_path = ?1", rusqlite::params![doc_path])?;
        }
        Ok::<usize, RagmcpError>(count)
    })
    .await?;

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file_meta(relative_path: &str, absolute_path: &str) -> FileMetadata {
        FileMetadata {
            relative_path: relative_path.to_string(),
            absolute_path: PathBuf::from(absolute_path),
            extension: PathBuf::from(relative_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_string(),
            file_size: 0,
            modified: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn test_classify_files_new_only() {
        use std::io::Write;
        let t1 = tempfile::NamedTempFile::new().unwrap();
        let t2 = tempfile::NamedTempFile::new().unwrap();
        t1.as_file().write_all(b"content1").unwrap();
        t2.as_file().write_all(b"content2").unwrap();
        t1.as_file().sync_all().unwrap();
        t2.as_file().sync_all().unwrap();
        let files = vec![
            file_meta("a.xml", t1.path().to_str().unwrap()),
            file_meta("b.yaml", t2.path().to_str().unwrap()),
        ];
        let existing: HashMap<String, String> = HashMap::new();
        let classification = classify_files(&files, &existing).unwrap();
        assert_eq!(classification.new_files.len(), 2);
        assert_eq!(classification.modified_files.len(), 0);
        assert_eq!(classification.unchanged_files.len(), 0);
    }

    #[test]
    fn test_classify_files_unchanged_uses_stored_hash() {
        use std::io::Write;
        let temp = tempfile::NamedTempFile::new().unwrap();
        temp.as_file().write_all(b"same content").unwrap();
        temp.as_file().sync_all().unwrap();
        let path = temp.path().to_path_buf();
        let hash = compute_file_hash(&path).unwrap();

        let files = vec![file_meta("x.md", path.to_str().unwrap())];
        let mut existing = HashMap::new();
        existing.insert("x.md".to_string(), hash);
        let classification = classify_files(&files, &existing).unwrap();
        assert_eq!(classification.new_files.len(), 0);
        assert_eq!(classification.modified_files.len(), 0);
        assert_eq!(classification.unchanged_files.len(), 1);
    }

    #[test]
    fn test_classify_files_modified_when_hash_differs() {
        use std::io::Write;
        let temp = tempfile::NamedTempFile::new().unwrap();
        temp.as_file().write_all(b"new content").unwrap();
        temp.as_file().sync_all().unwrap();
        let path = temp.path().to_path_buf();

        let files = vec![file_meta("y.xml", path.to_str().unwrap())];
        let mut existing = HashMap::new();
        existing.insert("y.xml".to_string(), "old_hash_placeholder".to_string());
        let classification = classify_files(&files, &existing).unwrap();
        assert_eq!(classification.new_files.len(), 0);
        assert_eq!(classification.modified_files.len(), 1);
        assert_eq!(classification.unchanged_files.len(), 0);
    }

    async fn setup_test_db() -> (crate::db::Db, tempfile::TempDir) {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db = crate::db::Db::new(&db_path);
        let migrations_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
        db.with_connection(move |conn| crate::db::migrate::run_migrations(conn, &migrations_dir))
            .await
            .unwrap();
        (db, temp_dir)
    }

    #[tokio::test]
    async fn test_get_existing_hashes() {
        let (db, _temp_dir) = setup_test_db().await;
        crate::ingest::db_writer::insert_document(
            &db,
            "agents/foo/prompt.xml",
            "agent_prompt",
            "agents",
            Some("foo"),
            "content",
            10,
            "abc123",
            std::time::SystemTime::now(),
        )
        .await
        .unwrap();
        let hashes = get_existing_hashes(&db).await.unwrap();
        assert_eq!(hashes.len(), 1);
        assert_eq!(hashes.get("agents/foo/prompt.xml"), Some(&"abc123".to_string()));
    }

    #[tokio::test]
    async fn test_find_deleted_documents() {
        let (db, _temp_dir) = setup_test_db().await;
        crate::ingest::db_writer::insert_document(
            &db,
            "only/in/db.xml",
            "agent_prompt",
            "agents",
            None,
            "x",
            1,
            "h1",
            std::time::SystemTime::now(),
        )
        .await
        .unwrap();
        let on_disk: HashSet<String> = ["on/disk.yaml".into()].into_iter().collect();
        let deleted = find_deleted_documents(&db, &on_disk).await.unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0], "only/in/db.xml");
        let on_disk_with_both: HashSet<String> =
            ["only/in/db.xml".into(), "on/disk.yaml".into()].into_iter().collect();
        let deleted_none = find_deleted_documents(&db, &on_disk_with_both).await.unwrap();
        assert!(deleted_none.is_empty());
    }

    #[tokio::test]
    async fn test_delete_documents() {
        let (db, _temp_dir) = setup_test_db().await;
        let doc_id = crate::ingest::db_writer::insert_document(
            &db,
            "to/delete.xml",
            "agent_prompt",
            "agents",
            None,
            "content",
            10,
            "hash",
            std::time::SystemTime::now(),
        )
        .await
        .unwrap();
        crate::ingest::db_writer::insert_chunks(
            &db,
            &doc_id,
            vec![crate::ingest::Chunk {
                text: "chunk".into(),
                tokens: 1,
                section_header: None,
                chunk_type: None,
            }],
        )
        .await
        .unwrap();
        let n = delete_documents(&db, &["to/delete.xml".to_string()]).await.unwrap();
        assert_eq!(n, 1);
        let hashes = get_existing_hashes(&db).await.unwrap();
        assert!(hashes.is_empty());
        let chunk_count: i64 = db
            .with_connection(|conn| {
                let c: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
                Ok::<i64, crate::error::RagmcpError>(c)
            })
            .await
            .unwrap();
        assert_eq!(chunk_count, 0);
    }
}
