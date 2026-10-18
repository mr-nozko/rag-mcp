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
use crate::ingest::{compute_file_hash, ingest_file, normalize_doc_path, FileMetadata, ParserRegistry};
use sha2::{Digest, Sha256};

/// Build FileMetadata from an absolute path and the qm_os root.
/// Returns None if the path is outside root or has an unsupported extension.
const ALLOWED_EXTENSIONS: &[&str] = &["xml", "yaml", "yml", "json", "md", "txt"];

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

    let relative_path = normalize_doc_path(
        &absolute_path
            .strip_prefix(&root)
            .map_err(|_| RagmcpError::Config("strip_prefix".to_string()))?
            .to_string_lossy(),
    );

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
/// Uses normalized path matching so `\` and `/` variants resolve.
/// Returns `(file_hash, doc_id)` when a row exists.
/// `pub(crate)` so MCP reindex can share the same lookup without duplicating SQL.
pub(crate) async fn get_stored_hash_and_id_for_path(
    db: &Db,
    doc_path: &str,
) -> Result<Option<(String, String)>> {
    let doc_path = normalize_doc_path(doc_path);
    let out = db
        .with_connection(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT file_hash, doc_id FROM documents WHERE REPLACE(doc_path, '\\', '/') = ?1 ORDER BY last_modified DESC LIMIT 1",
            )?;
            let mut rows = stmt.query([&doc_path])?;
            if let Some(row) = rows.next()? {
                let h: String = row.get(0)?;
                let id: String = row.get(1)?;
                return Ok(Some((h, id)));
            }
            Ok(None)
        })
        .await?;
    Ok(out)
}

/// Outcome of hash-check + optional ingest + embed for one file.
/// Shared by the file watcher and `ragmcp_reindex` so both paths stay identical.
#[derive(Debug, Clone)]
pub struct IndexFileStatus {
    pub doc_id: String,
    pub relative_path: String,
    /// False when the on-disk SHA-256 already matched `documents.file_hash` (ingest skipped).
    pub ingested: bool,
    /// Chunk rows created by ingest; 0 when ingest was skipped.
    pub chunks_created: usize,
    /// Embeddings written for previously-NULL chunks (may be >0 even when ingest skipped).
    pub embeddings_stored: usize,
}

/// Index a known file: skip re-ingest when hash matches, always backfill NULL embeddings.
/// Callers with a resolved `FileMetadata` (watcher or MCP reindex) share this path.
pub async fn index_file_with_status(
    db: &Db,
    config: &Config,
    file: &FileMetadata,
    parser_registry: &ParserRegistry,
    embedder: &OpenAIEmbedder,
) -> Result<IndexFileStatus> {
    // Prefer DB doc_id when present so slash-variant upserts embed the correct rows
    let current_hash = compute_file_hash(&file.absolute_path)?;
    let stored = get_stored_hash_and_id_for_path(db, &file.relative_path).await?;
    let hash_unchanged = stored
        .as_ref()
        .map(|(h, _)| h.as_str() == current_hash.as_str())
        .unwrap_or(false);

    if hash_unchanged {
        let doc_id = stored
            .as_ref()
            .map(|(_, id)| id.clone())
            .unwrap_or_else(|| format!("{:x}", Sha256::digest(file.relative_path.as_bytes())));
        // Skip re-ingestion; still backfill any chunks that have NULL embedding for this doc
        let stored_count =
            embed_missing_for_doc(db, embedder, &doc_id, config.embeddings.batch_size).await?;
        return Ok(IndexFileStatus {
            doc_id,
            relative_path: file.relative_path.clone(),
            ingested: false,
            chunks_created: 0,
            embeddings_stored: stored_count,
        });
    }

    // New or modified: re-ingest then embed using the upserted doc_id
    let (doc_id, chunk_count, _tokens) =
        ingest_file(db, file, parser_registry, config).await?;
    let stored_count =
        embed_missing_for_doc(db, embedder, &doc_id, config.embeddings.batch_size).await?;
    Ok(IndexFileStatus {
        doc_id,
        relative_path: file.relative_path.clone(),
        ingested: true,
        chunks_created: chunk_count,
        embeddings_stored: stored_count,
    })
}

/// Embed all chunks for `doc_id` that still have NULL embeddings.
/// Shared by the file watcher and MCP create/update tools (one OpenAI batch path).
pub async fn embed_missing_for_doc(
    db: &Db,
    embedder: &OpenAIEmbedder,
    doc_id: &str,
    batch_size: usize,
) -> Result<usize> {
    let chunks = get_chunks_without_embedding_for_doc(db, doc_id).await?;
    if chunks.is_empty() {
        return Ok(0);
    }
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
    Ok(stored)
}

/// Handle a single file change: hash check, ingest if changed, then embed chunks that have no embedding.
/// Thin wrapper around [`index_file_with_status`] that keeps watcher log messages unchanged.
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

    let status = index_file_with_status(db, config, &file, parser_registry, embedder).await?;

    if !status.ingested {
        if status.embeddings_stored == 0 {
            log::info!("watch: {} skip (unchanged, all embedded)", status.relative_path);
            return Ok(());
        }
        log::info!(
            "watch: {} skip ingest, backfilled {} embeddings in {:?}",
            status.relative_path,
            status.embeddings_stored,
            start.elapsed()
        );
        return Ok(());
    }

    if status.embeddings_stored == 0 {
        log::info!(
            "watch: {} ingested, no new chunks to embed in {:?}",
            status.relative_path,
            start.elapsed()
        );
        return Ok(());
    }

    log::info!(
        "watch: {} ingested, {} chunks embedded in {:?}",
        status.relative_path,
        status.embeddings_stored,
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
        // .docx is not in the watcher allow-list (unlike .txt which is supported)
        let file_path = root.path().join("readme.docx");
        fs::write(&file_path, "binary-ish").unwrap();

        let meta = file_metadata_from_path(&file_path, root.path()).unwrap();
        assert!(meta.is_none());
    }

    /// Hash match + embeddings already present → ingested false, embeddings_stored 0 (no network).
    #[tokio::test]
    async fn test_index_file_with_status_unchanged_when_hash_matches() {
        use crate::config::{
            Config, EmbeddingsConfig, PerformanceConfig, RagmcpConfig, SearchConfig,
        };
        use crate::db::migrate;
        use crate::embeddings::store_embeddings_batch;
        use crate::ingest::chunker::Chunk;
        use crate::ingest::db_writer::{insert_chunks, insert_document};
        use crate::ingest::compute_file_hash;
        use std::path::Path;

        let temp_dir = TempDir::new().unwrap();
        let rag = temp_dir.path().to_path_buf();
        let sub = rag.join("agents");
        fs::create_dir_all(&sub).unwrap();
        let file_path = sub.join("unchanged.md");
        let content = "# Unchanged doc\n\nBody text for hash match.\n";
        fs::write(&file_path, content).unwrap();

        let file_hash = compute_file_hash(&file_path).unwrap();
        let relative_path = "agents/unchanged.md";

        let db_path = rag.join("test.db");
        let db = Db::new(&db_path);
        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        db.with_connection(move |conn| migrate::run_migrations(conn, &migrations_dir))
            .await
            .unwrap();

        let doc_id = insert_document(
            &db,
            relative_path,
            "markdown",
            "agents",
            None,
            content,
            10,
            &file_hash,
            std::time::SystemTime::now(),
        )
        .await
        .unwrap();

        let chunks = vec![Chunk {
            text: "Body text for hash match.".to_string(),
            tokens: 5,
            section_header: Some("Unchanged doc".to_string()),
            chunk_type: Some("markdown".to_string()),
        }];
        insert_chunks(&db, &doc_id, chunks).await.unwrap();

        // Pre-fill embeddings so embed_missing finds nothing (no OpenAI call).
        let without = get_chunks_without_embedding_for_doc(&db, &doc_id)
            .await
            .unwrap();
        let dummy: Vec<f32> = (0..1536).map(|i| i as f32 * 0.001).collect();
        let pairs: Vec<(String, Vec<f32>)> = without
            .iter()
            .map(|(id, _)| (id.clone(), dummy.clone()))
            .collect();
        store_embeddings_batch(&db, pairs).await.unwrap();

        let config = Config {
            ragmcp: RagmcpConfig {
                rag_folder: rag.clone(),
                db_path: db_path.clone(),
                log_level: "info".to_string(),
            },
            embeddings: EmbeddingsConfig {
                provider: "openai".to_string(),
                model: "text-embedding-3-small".to_string(),
                api_key_env: "OPENAI_API_KEY".to_string(),
                batch_size: 100,
                dimensions: 1536,
                cache_capacity: 0,
            },
            search: SearchConfig {
                default_k: 5,
                min_score: 0.65,
                hybrid_bm25_weight: 0.5,
                hybrid_vector_weight: 0.5,
            },
            performance: PerformanceConfig {
                max_latency_ms: 1000,
                chunk_size_tokens: 300,
                chunk_overlap_tokens: 50,
            },
            http_server: Default::default(),
            pageindex: Default::default(),
            storage: None,
        };

        let meta = fs::metadata(&file_path).unwrap();
        let file = FileMetadata {
            relative_path: relative_path.to_string(),
            absolute_path: file_path,
            extension: "md".to_string(),
            file_size: meta.len(),
            modified: meta.modified().unwrap(),
        };

        // Dummy key — must not hit the network when embeddings already exist.
        let embedder = OpenAIEmbedder::new("dummy".into(), "text-embedding-3-small".into(), 10);
        let registry = ParserRegistry::new();
        let status = index_file_with_status(&db, &config, &file, &registry, &embedder)
            .await
            .unwrap();

        assert!(!status.ingested, "hash match must skip ingest");
        assert_eq!(status.chunks_created, 0);
        assert_eq!(
            status.embeddings_stored, 0,
            "no NULL embeddings → zero stores, no network"
        );
        assert_eq!(status.doc_id, doc_id);
    }
}
