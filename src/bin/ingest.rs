use clap::Parser;
use ragmcp::Config;
use ragmcp::db::{Db, migrate};
use ragmcp::ingest::{
    discover_files, compute_file_hash, extract_namespace, extract_agent_name,
    ParserRegistry, chunk_document, insert_document, insert_chunks,
    get_existing_hashes, classify_files, find_deleted_documents, delete_documents,
};
use std::path::Path;
use std::collections::HashSet;
use std::time::Instant;
use anyhow::Result;

#[derive(Parser, Debug)]
#[command(name = "ingest")]
#[command(about = "Ingest documents into RAGMcp database (incremental by default)")]
struct Args {
    /// Force re-ingestion of all files (ignore hashes)
    #[arg(short, long)]
    force: bool,

    /// Clean up documents that no longer exist on filesystem
    #[arg(short, long)]
    cleanup: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .filter_or("RUST_LOG", "info")
    ).init();
    
    let args = Args::parse();
    
    log::info!("Starting RAGMcp ingestion");
    
    // Load configuration
    let config = Config::load()?;
    log::info!("Configuration loaded successfully");
    log::info!("Docs root: {}", config.rag_folder().display());
    log::info!("Database path: {}", config.db_path().display());
    
    // Initialize database
    let db = Db::new(config.db_path());
    
    // Run migrations
    let migrations_dir = Path::new("migrations");
    db.with_connection(|conn| {
        migrate::run_migrations(conn, migrations_dir)
    }).await?;
    
    log::info!("Database initialized");
    
    // Discover files
    log::info!("Discovering files in {}", config.rag_folder().display());
    let files = discover_files(config.rag_folder())?;
    log::info!("Found {} files to ingest", files.len());
    
    if files.is_empty() {
        log::warn!("No files found to ingest. Check rag_folder path in config.toml.");
        return Ok(());
    }
    
    // Incremental: classify files so we only process new or modified
    let force = args.force;
    let cleanup = args.cleanup;
    let existing_hashes = get_existing_hashes(&db).await?;
    let classification = classify_files(&files, &existing_hashes)?;
    
    let num_new = classification.new_files.len();
    let num_modified = classification.modified_files.len();
    let num_unchanged = classification.unchanged_files.len();
    
    let files_to_process: Vec<_> = if force {
        log::info!("Mode: full re-ingestion (all files)");
        files.iter().cloned().collect()
    } else {
        log::info!("Classification: new={}, modified={}, unchanged (skip)={}",
            num_new, num_modified, num_unchanged,
        );
        classification.new_files.into_iter()
            .chain(classification.modified_files.into_iter())
            .collect()
    };
    
    let total_to_process = files_to_process.len();
    if total_to_process == 0 {
        log::info!("No new or modified files to process. Ingestion complete.");
        // Optionally cleanup deleted documents
        let existing_file_paths: HashSet<String> = files.iter().map(|f| f.relative_path.clone()).collect();
        let deleted = find_deleted_documents(&db, &existing_file_paths).await?;
        if !deleted.is_empty() {
            log::info!("Found {} documents in DB that no longer exist on disk", deleted.len());
            if cleanup {
                let n = delete_documents(&db, &deleted).await?;
                log::info!("Cleaned up {} deleted documents", n);
            } else {
                log::info!("Run with --cleanup to remove them from the database.");
            }
        }
        return Ok(());
    }
    
    log::info!("Processing {} file(s)", total_to_process);
    let create_parser_registry = ParserRegistry::new();
    let parser_registry = &create_parser_registry;
    
    let start = Instant::now();
    let mut total_docs: usize = 0;
    let mut total_chunks: usize = 0;
    let mut total_tokens: usize = 0;
    let mut errors: usize = 0;
    
    for (idx, file) in files_to_process.iter().enumerate() {
        log::info!(
            "[{}/{}] Processing: {}",
            idx + 1,
            total_to_process,
            file.relative_path
        );
        
        match process_file(
            &db,
            file,
            parser_registry,
            &config,
        ).await {
            Ok((chunk_count, tokens)) => {
                total_docs += 1;
                total_chunks += chunk_count;
                total_tokens += tokens;
                log::info!(
                    "✓ {} ({} chunks, {} tokens)",
                    file.relative_path,
                    chunk_count,
                    tokens
                );
            }
            Err(e) => {
                errors += 1;
                log::error!("✗ {}: {}", file.relative_path, e);
            }
        }
    }
    
    let elapsed = start.elapsed();
    
    // Optional: cleanup documents that no longer exist on filesystem
    let existing_file_paths: HashSet<String> = files.iter().map(|f| f.relative_path.clone()).collect();
    let deleted = find_deleted_documents(&db, &existing_file_paths).await?;
    let deleted_count = if !deleted.is_empty() && cleanup {
        log::info!("Found {} deleted documents on disk", deleted.len());
        delete_documents(&db, &deleted).await?
    } else if !deleted.is_empty() {
        log::info!("Found {} documents in DB that no longer exist on disk (use --cleanup to remove)", deleted.len());
        0
    } else {
        0
    };
    
    // Report final statistics
    let skipped = files.len().saturating_sub(total_to_process);
    log::info!("=== Ingestion Complete ===");
    log::info!("Files discovered: {}", files.len());
    log::info!("  New: {}", num_new);
    log::info!("  Modified: {}", num_modified);
    log::info!("  Unchanged (skipped): {}", num_unchanged);
    log::info!("Files processed: {} (success: {}, errors: {})", total_docs, total_docs.saturating_sub(errors), errors);
    log::info!("Chunks created: {}", total_chunks);
    log::info!("Tokens indexed: {}", total_tokens);
    log::info!("Time: {:?}", elapsed);
    if skipped > 0 {
        // Rough estimate: ~0.5s per file saved by not parsing/chunking/inserting
        let estimated_saved_secs = (skipped as f64 * 0.5).round() as u64;
        log::info!("Skipped {} unchanged file(s) (estimated time saved: ~{}s)", skipped, estimated_saved_secs);
    }
    if deleted_count > 0 {
        log::info!("Documents cleaned up (deleted from disk): {}", deleted_count);
    }
    
    if errors > 0 {
        log::warn!("Some files failed to ingest. Check logs above for details.");
    }
    
    Ok(())
}

/// Process a single file: parse, chunk, and insert into database
async fn process_file(
    db: &Db,
    file: &ragmcp::ingest::FileMetadata,
    parser_registry: &ParserRegistry,
    config: &Config,
) -> Result<(usize, usize)> {
    // Read file content
    let content = std::fs::read_to_string(&file.absolute_path)
        .map_err(|e| anyhow::anyhow!("Failed to read file: {}", e))?;
    
    // Compute file hash
    let file_hash = compute_file_hash(&file.absolute_path)?;
    
    // Extract metadata
    let namespace = extract_namespace(&file.relative_path);
    let agent_name = extract_agent_name(&file.relative_path);
    
    // Parse document
    let parsed = parser_registry.parse(
        &content,
        &file.relative_path,
        &file.extension,
    )?;
    
    // Chunk document
    let chunks = chunk_document(&parsed, &config.performance)?;
    
    // Estimate total tokens
    let total_tokens = chunks.iter().map(|c| c.tokens).sum::<usize>();
    
    // Insert document
    let doc_id = insert_document(
        db,
        &file.relative_path,
        &parsed.doc_type,
        &namespace,
        agent_name.as_deref(),
        &parsed.content,
        total_tokens,
        &file_hash,
        file.modified,
    ).await?;
    
    // Insert chunks
    let chunk_count = insert_chunks(db, &doc_id, chunks).await?;
    
    Ok((chunk_count, total_tokens))
}
