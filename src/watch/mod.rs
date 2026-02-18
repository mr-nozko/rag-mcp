//! File watcher: re-ingest and re-embed when files under rag_folder change.
//!
//! Uses the notify crate to watch the directory, debounces events, and for each
//! changed file runs the existing ingest pipeline plus embedding for that doc.

mod watcher;

use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::config::Config;
use crate::db::Db;
use crate::embeddings::{get_chunks_without_embedding_for_doc, store_embeddings_batch, OpenAIEmbedder};
use crate::error::{Result, RagmcpError};
use crate::ingest::{compute_file_hash, ingest_file, FileMetadata, ParserRegistry};
use sha2::{Digest, Sha256};

/// Build FileMetadata from an absolute path and the qm_os root.
/// Returns None if the path is outside root or has an unsupported extension.
const ALLOWED_EXTENSIONS: &[&str] = &["xml", "yaml", "yml", "json", "md"];

pub fn file_metadata_from_path(absolute_path: &Path, root: &Path) -> Result<Option<FileMetadata>> {
    let root = root
        .canonicalize()
        .map_err(|e| RagmcpError::Config(format!("root canonicalize: {}", e)))?;
    let absolute_path = absolute_path
        .canonicalize()
        .map_err(|e| RagmcpError::Config(format!("path canonicalize: {}", e)))?;

    if !absolute_path.starts_with(&root) {
        return Ok(None);
    }

    let relative_path = absolute_path
        .strip_prefix(&root)
        .map_err(|_| RagmcpError::Config("strip_prefix".to_string()))?
        .to_string_lossy()
        .replace('\\', "/");

    let extension = absolute_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if !ALLOWED_EXTENSIONS.contains(&extension.as_str()) {
        return Ok(None);
    }

    if !absolute_path.is_file() {
        return Ok(None);
    }

    let metadata = std::fs::metadata(&absolute_path).map_err(RagmcpError::Io)?;
    Ok(Some(FileMetadata {
        relative_path,
        absolute_path,
        extension,
        file_size: metadata.len(),
        modified: metadata.modified().map_err(RagmcpError::Io)?,
    }))
}

/// Check if the document at doc_path has the given hash in the database.
async fn get_stored_hash_for_path(db: &Db, doc_path: &str) -> Result<Option<String>> {
    let doc_path = doc_path.to_string();
    let out = db
        .with_connection(move |conn| {
            let mut stmt = conn.prepare("SELECT file_hash FROM documents WHERE doc_path = ?1")?;
            let mut rows = stmt.query([&doc_path])?;
            if let Some(row) = rows.next()? {
                let h: String = row.get(0)?;
                return Ok(Some(h));
            }
            Ok(None)
        })
        .await?;
    Ok(out)
}

/// Handle a single file change: hash check, ingest if changed, then embed chunks that have no embedding.
/// When file hash is unchanged we skip re-ingestion but still backfill any chunks with NULL embeddings.
pub async fn handle_file_change(
    db: &Db,
    config: &Config,
    root: &Path,
    path: &Path,
    parser_registry: &ParserRegistry,
    embedder: &OpenAIEmbedder,
) -> Result<()> {
    let start = std::time::Instant::now();

    let file = match file_metadata_from_path(path, root)? {
        Some(f) => f,
        None => return Ok(()),
    };

    // doc_id must match insert_document: SHA256 of doc_path (relative_path)
    let doc_id = format!("{:x}", Sha256::digest(file.relative_path.as_bytes()));
    let current_hash = compute_file_hash(&file.absolute_path)?;
    let stored_hash = get_stored_hash_for_path(db, &file.relative_path).await?;

    let hash_unchanged = stored_hash.as_deref() == Some(current_hash.as_str());

    if hash_unchanged {
        // Skip re-ingestion; still backfill any chunks that have NULL embedding for this doc
        let chunks = get_chunks_without_embedding_for_doc(db, &doc_id).await?;
        if chunks.is_empty() {
            log::info!("watch: {} skip (unchanged, all embedded)", file.relative_path);
            return Ok(());
        }
        let batch_size = config.embeddings.batch_size;
        let mut stored = 0;
        for batch in chunks.chunks(batch_size) {
            let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
            let embeddings = embedder.embed_batch(texts).await?;
            let pairs: Vec<(String, Vec<f32>)> = batch
                .iter()
                .map(|(id, _)| id.clone())
                .zip(embeddings)
                .collect();
            stored += store_embeddings_batch(db, pairs).await?;
        }
        log::info!(
            "watch: {} skip ingest, backfilled {} embeddings in {:?}",
            file.relative_path,
            stored,
            start.elapsed()
        );
        return Ok(());
    }

    // New or modified: re-ingest then embed only chunks without embeddings
    ingest_file(db, &file, parser_registry, config).await?;

    let chunks = get_chunks_without_embedding_for_doc(db, &doc_id).await?;
    if chunks.is_empty() {
        log::info!(
            "watch: {} ingested, no new chunks to embed in {:?}",
            file.relative_path,
            start.elapsed()
        );
        return Ok(());
    }

    let batch_size = config.embeddings.batch_size;
    let mut stored = 0;
    for batch in chunks.chunks(batch_size) {
        let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
        let embeddings = embedder.embed_batch(texts).await?;
        let pairs: Vec<(String, Vec<f32>)> = batch
            .iter()
            .map(|(id, _)| id.clone())
            .zip(embeddings)
            .collect();
        stored += store_embeddings_batch(db, pairs).await?;
    }

    log::info!(
        "watch: {} ingested, {} chunks embedded in {:?}",
        file.relative_path,
        stored,
        start.elapsed()
    );
    Ok(())
}

/// Run the file watcher: spawn watcher thread, then async loop that receives paths
/// and calls handle_file_change. Runs until the watcher thread exits (e.g. receiver dropped).
pub async fn run_watcher(
    db: Db,
    config: Config,
    embedder: OpenAIEmbedder,
    debounce_ms: u64,
) -> Result<()> {
    let root = config.rag_folder().to_path_buf();
    let (tx, rx) = mpsc::channel();
    let rx = Arc::new(Mutex::new(rx));

    std::thread::spawn(move || {
        if let Err(e) = watcher::run_watcher_thread(&root, debounce_ms, tx) {
            log::error!("watcher thread error: {}", e);
        }
    });

    let parser_registry = ParserRegistry::new();
    let root_ref = config.rag_folder().to_path_buf();

    loop {
        let rx_clone = rx.clone();
        let path = tokio::task::spawn_blocking(move || rx_clone.lock().unwrap().recv())
            .await
            .map_err(|e| RagmcpError::Config(format!("watcher task join: {}", e)))?;

        let path = match path {
            Ok(p) => p,
            Err(_) => break,
        };

        if let Err(e) = handle_file_change(
            &db,
            &config,
            &root_ref,
            &path,
            &parser_registry,
            &embedder,
        )
        .await
        {
            log::error!("watch handle_file_change {}: {}", path.display(), e);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Validates that doc_id derived in the watcher (SHA256 of relative_path) matches
    /// insert_document, and that get_chunks_without_embedding_for_doc + store_embeddings_batch
    /// correctly backfill NULL embeddings for that doc (no OpenAI API).
    #[tokio::test]
    async fn test_watcher_doc_id_consistency_and_backfill_embeddings() {
        use crate::db::migrate;
        use crate::embeddings::store_embeddings_batch;
        use crate::ingest::chunker::Chunk;
        use crate::ingest::db_writer::{insert_document, insert_chunks};
        use std::path::Path;

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db = Db::new(&db_path);
        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        db.with_connection(move |conn| migrate::run_migrations(conn, &migrations_dir))
            .await
            .unwrap();

        let relative_path = "agents/watcher_test.xml";
        let doc_id_from_insert = insert_document(
            &db,
            relative_path,
            "agent_prompt",
            "agents",
            Some("watcher_test"),
            "Content for embedding backfill",
            50,
            "hash1",
            std::time::SystemTime::now(),
        )
        .await
        .unwrap();

        let chunks = vec![
            Chunk {
                text: "Chunk one".to_string(),
                tokens: 2,
                section_header: Some("A".to_string()),
                chunk_type: Some("test".to_string()),
            },
            Chunk {
                text: "Chunk two".to_string(),
                tokens: 2,
                section_header: Some("B".to_string()),
                chunk_type: Some("test".to_string()),
            },
        ];
        insert_chunks(&db, &doc_id_from_insert, chunks).await.unwrap();

        // Watcher uses the same doc_id derivation as insert_document (SHA256 of doc_path)
        let doc_id_watcher = format!("{:x}", Sha256::digest(relative_path.as_bytes()));
        assert_eq!(
            doc_id_from_insert, doc_id_watcher,
            "doc_id from insert_document must match watcher derivation"
        );

        let without_before = get_chunks_without_embedding_for_doc(&db, &doc_id_watcher).await.unwrap();
        assert_eq!(without_before.len(), 2, "both chunks should have NULL embedding before backfill");

        // Backfill with dummy embeddings (1536 dims to match schema; no API call)
        let dummy_embedding: Vec<f32> = (0..1536).map(|i| i as f32 * 0.001).collect();
        let pairs: Vec<(String, Vec<f32>)> = without_before
            .iter()
            .map(|(id, _)| (id.clone(), dummy_embedding.clone()))
            .collect();
        let stored = store_embeddings_batch(&db, pairs).await.unwrap();
        assert_eq!(stored, 2);

        let without_after = get_chunks_without_embedding_for_doc(&db, &doc_id_watcher).await.unwrap();
        assert!(without_after.is_empty(), "no chunks should lack embedding after backfill");
    }

    #[test]
    fn test_file_metadata_from_path_under_root_allowed_extension() {
        let root = TempDir::new().unwrap();
        let sub = root.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        let file_path = sub.join("doc.xml");
        fs::write(&file_path, "<root/>").unwrap();

        let meta = file_metadata_from_path(&file_path, root.path()).unwrap();
        let meta = meta.expect("expected Some(FileMetadata)");
        assert_eq!(meta.relative_path, "sub/doc.xml");
        assert_eq!(meta.extension, "xml");
        assert!(meta.file_size > 0);
    }

    #[test]
    fn test_file_metadata_from_path_outside_root_returns_none() {
        let root = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        let file_path = other.path().join("doc.xml");
        fs::write(&file_path, "<root/>").unwrap();

        let meta = file_metadata_from_path(&file_path, root.path()).unwrap();
        assert!(meta.is_none());
    }

    #[test]
    fn test_file_metadata_from_path_unsupported_extension_returns_none() {
        let root = TempDir::new().unwrap();
        let file_path = root.path().join("readme.txt");
        fs::write(&file_path, "text").unwrap();

        let meta = file_metadata_from_path(&file_path, root.path()).unwrap();
        assert!(meta.is_none());
    }
}
