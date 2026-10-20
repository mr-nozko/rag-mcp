//! Optional remote storage backends (Cloudflare R2 / S3-compatible).
//!
//! Local mode (no `[storage]` or `backend != "r2"`) never touches this module
//! at runtime beyond `Config` parsing defaults.

pub mod r2;

pub use r2::{
    assert_cache_dir_safe, delete_manifest_entry, hydrate_docs, is_internal_r2_key,
    normalize_etag, pull_missing_to_folder, schedule_snapshot_upload, upsert_manifest_entry,
    R2Client, CACHE_MARKER_NAME, DEFAULT_SNAPSHOT_KEY, INTERNAL_PREFIX, SNAPSHOT_DEBOUNCE_SECS,
};

use crate::config::Config;
use crate::db::Db;
use crate::embeddings::{store_embedding, OpenAIEmbedder};
use crate::error::{RagmcpError, Result};
use crate::ingest::{
    classify_files, discover_files, get_existing_hashes, ingest_file, ParserRegistry,
};

/// Outcome of an incremental ingest + embed bootstrap pass.
#[derive(Debug, Clone, Copy)]
pub struct BootstrapStatus {
    /// True when at least one file was ingested or at least one embedding was stored.
    pub db_changed: bool,
    /// Files successfully ingested in this pass.
    pub ingested_ok: usize,
    /// Per-file ingest failures (logged; does not abort the rest of the sweep).
    pub ingest_errors: usize,
    /// Chunk embeddings written for previously-NULL rows.
    pub embeddings_stored: usize,
}

/// Run one incremental ingest pass + embed any chunks missing embeddings.
/// Called after every R2 restore (reconcile after snapshot, or cold bootstrap when none exists).
pub async fn bootstrap_index(
    db: &Db,
    config: &Config,
    embedder: &OpenAIEmbedder,
) -> Result<BootstrapStatus> {
    let root = config.rag_folder();
    log::info!("R2 bootstrap: incremental ingest from {}", root.display());

    let files = discover_files(root)?;
    if files.is_empty() {
        // Empty corpus is a no-op — callers may still upload an initial snapshot.
        log::warn!("R2 bootstrap: no documents found under {}", root.display());
        return Ok(BootstrapStatus {
            db_changed: false,
            ingested_ok: 0,
            ingest_errors: 0,
            embeddings_stored: 0,
        });
    }

    let existing_hashes = get_existing_hashes(db).await?;
    let classification = classify_files(&files, &existing_hashes)?;
    // Skip unchanged hashes — only new/modified files are ingested.
    let to_process: Vec<_> = classification
        .new_files
        .into_iter()
        .chain(classification.modified_files.into_iter())
        .collect();

    let registry = ParserRegistry::new();
    let mut ingested_ok = 0usize;
    let mut ingest_errors = 0usize;
    for file in &to_process {
        match ingest_file(db, file, &registry, config).await {
            Ok(_) => ingested_ok += 1,
            Err(e) => {
                ingest_errors += 1;
                log::error!("R2 bootstrap ingest failed for {}: {}", file.relative_path, e);
            }
        }
    }
    log::info!(
        "R2 bootstrap ingest: {} ok, {} errors",
        ingested_ok,
        ingest_errors
    );

    // Embed only chunks that still lack vectors (same query shape as embed CLI).
    let chunks = db
        .with_connection(|conn| {
            let mut stmt =
                conn.prepare("SELECT chunk_id, chunk_text FROM chunks WHERE embedding IS NULL")?;
            let mut rows = stmt.query([])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push((row.get::<_, String>(0)?, row.get::<_, String>(1)?));
            }
            Ok::<_, RagmcpError>(out)
        })
        .await?;

    if chunks.is_empty() {
        log::info!("R2 bootstrap: all chunks already embedded");
        return Ok(BootstrapStatus {
            db_changed: ingested_ok > 0,
            ingested_ok,
            ingest_errors,
            embeddings_stored: 0,
        });
    }

    let batch_size = config.embeddings.batch_size.max(1);
    log::info!("R2 bootstrap: embedding {} chunks (batch={})", chunks.len(), batch_size);
    let mut embeddings_stored = 0usize;
    for batch in chunks.chunks(batch_size) {
        let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
        let embeddings = embedder.embed_batch(texts).await?;
        for ((chunk_id, _), embedding) in batch.iter().zip(embeddings.into_iter()) {
            store_embedding(db, chunk_id, &embedding).await?;
            embeddings_stored += 1;
        }
    }

    Ok(BootstrapStatus {
        db_changed: ingested_ok > 0 || embeddings_stored > 0,
        ingested_ok,
        ingest_errors,
        embeddings_stored,
    })
}
