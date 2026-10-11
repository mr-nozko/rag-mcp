//! Optional remote storage backends (Cloudflare R2 / S3-compatible).
//!
//! Local mode (no `[storage]` or `backend != "r2"`) never touches this module
//! at runtime beyond `Config` parsing defaults.

pub mod r2;

pub use r2::{
    assert_cache_dir_safe, hydrate_docs, is_internal_r2_key, pull_missing_to_folder,
    R2Client, CACHE_MARKER_NAME, DEFAULT_SNAPSHOT_KEY, INTERNAL_PREFIX,
};

use crate::config::Config;
use crate::db::Db;
use crate::embeddings::{store_embedding, OpenAIEmbedder};
use crate::error::{RagmcpError, Result};
use crate::ingest::{
    classify_files, discover_files, get_existing_hashes, ingest_file, ParserRegistry,
};

/// Run one incremental ingest pass + embed any chunks missing embeddings.
/// Used on R2 cold boot when no SQLite snapshot exists yet.
pub async fn bootstrap_index(db: &Db, config: &Config, embedder: &OpenAIEmbedder) -> Result<()> {
    let root = config.rag_folder();
    log::info!("R2 bootstrap: incremental ingest from {}", root.display());

    let files = discover_files(root)?;
    if files.is_empty() {
        log::warn!("R2 bootstrap: no documents found under {}", root.display());
        return Ok(());
    }

    let existing_hashes = get_existing_hashes(db).await?;
    let classification = classify_files(&files, &existing_hashes)?;
    let to_process: Vec<_> = classification
        .new_files
        .into_iter()
        .chain(classification.modified_files.into_iter())
        .collect();

    let registry = ParserRegistry::new();
    let mut ok = 0usize;
    let mut err = 0usize;
    for file in &to_process {
        match ingest_file(db, file, &registry, config).await {
            Ok(_) => ok += 1,
            Err(e) => {
                err += 1;
                log::error!("R2 bootstrap ingest failed for {}: {}", file.relative_path, e);
            }
        }
    }
    log::info!("R2 bootstrap ingest: {} ok, {} errors", ok, err);

    // Embed chunks that still lack vectors (same query shape as embed CLI).
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
        return Ok(());
    }

    let batch_size = config.embeddings.batch_size.max(1);
    log::info!("R2 bootstrap: embedding {} chunks (batch={})", chunks.len(), batch_size);
    for batch in chunks.chunks(batch_size) {
        let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
        let embeddings = embedder.embed_batch(texts).await?;
        for ((chunk_id, _), embedding) in batch.iter().zip(embeddings.into_iter()) {
            store_embedding(db, chunk_id, &embedding).await?;
        }
    }

    Ok(())
}
