use clap::Parser;
use ragmcp::Config;
use ragmcp::db::{Db, migrate};
use ragmcp::embeddings::{OpenAIEmbedder, store_embedding};
use std::path::Path;
use anyhow::Result;

#[derive(Parser, Debug)]
#[command(name = "embed")]
#[command(about = "Generate embeddings for chunks (incremental: only chunks without embeddings by default)")]
struct Args {
    /// Re-embed all chunks (ignore existing embeddings)
    #[arg(short, long)]
    force: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .filter_or("RUST_LOG", "info")
    ).init();
    
    let args = Args::parse();
    
    log::info!("Starting RAGMcp embedding generation");
    log::info!(
        "Embedding strategy: {}",
        if args.force { "FORCE (all chunks)" } else { "INCREMENTAL (new chunks only)" }
    );
    
    // Load configuration
    let config = Config::load()?;
    log::info!("Configuration loaded successfully");
    log::info!("Database path: {}", config.db_path().display());
    
    // Validate OpenAI API key is set
    let api_key = std::env::var(&config.embeddings.api_key_env)
        .map_err(|_| anyhow::anyhow!(
            "Environment variable {} not set. Set it in your .env file or as an environment variable.",
            config.embeddings.api_key_env
        ))?;
    
    // Initialize database
    let db = Db::new(config.db_path());
    
    // Run migrations (in case database is new)
    let migrations_dir = Path::new("migrations");
    db.with_connection(|conn| {
        migrate::run_migrations(conn, migrations_dir)
    }).await?;
    
    log::info!("Database initialized");
    
    // Create embedder
    let embedder = OpenAIEmbedder::new(
        api_key,
        config.embeddings.model.clone(),
        config.embeddings.batch_size,
    );
    
    log::info!(
        "Embedder configured: model={}, batch_size={}",
        config.embeddings.model,
        config.embeddings.batch_size
    );
    
    // Get chunks to embed: all chunks if --force, else only those without embeddings
    let query = if args.force {
        "SELECT chunk_id, chunk_text FROM chunks"
    } else {
        "SELECT chunk_id, chunk_text FROM chunks WHERE embedding IS NULL"
    };
    log::info!("Querying chunks...");
    let chunks = db.with_connection(|conn| {
        let mut stmt = conn.prepare(query)?;
        
        let mut rows = stmt.query([])?;
        let mut chunks = Vec::new();
        
        while let Some(row) = rows.next()? {
            let chunk_id: String = row.get(0)?;
            let chunk_text: String = row.get(1)?;
            chunks.push((chunk_id, chunk_text));
        }
        
        Ok::<Vec<(String, String)>, ragmcp::error::RagmcpError>(chunks)
    }).await?;
    
    let total_chunks = chunks.len();
    
    if total_chunks == 0 {
        log::info!("No chunks need embedding. All chunks already have embeddings.");
        return Ok(());
    }
    
    log::info!("Found {} chunks to embed", total_chunks);
    
    // Process chunks in batches
    let mut completed = 0;
    let mut failed = 0;
    
    for batch in chunks.chunks(config.embeddings.batch_size) {
        let texts: Vec<String> = batch.iter().map(|(_, text)| text.clone()).collect();
        let chunk_ids: Vec<String> = batch.iter().map(|(id, _)| id.clone()).collect();
        
        // Generate embeddings for this batch
        match embedder.embed_batch(texts).await {
            Ok(embeddings) => {
                // Store embeddings
                for (chunk_id, embedding) in chunk_ids.iter().zip(embeddings.iter()) {
                    match store_embedding(&db, chunk_id, embedding).await {
                        Ok(_) => {
                            completed += 1;
                            
                            // Progress reporting every 100 chunks
                            if completed % 100 == 0 {
                                let percentage = (completed as f64 / total_chunks as f64) * 100.0;
                                log::info!(
                                    "Embedding progress: {}/{} chunks ({:.1}%)",
                                    completed,
                                    total_chunks,
                                    percentage
                                );
                            }
                        }
                        Err(e) => {
                            failed += 1;
                            log::error!("Failed to store embedding for chunk {}: {}", chunk_id, e);
                        }
                    }
                }
            }
            Err(e) => {
                failed += batch.len();
                log::error!("Failed to generate embeddings for batch: {}", e);
                log::warn!("Continuing with next batch...");
            }
        }
    }
    
    // Final summary
    log::info!("Embedding generation complete!");
    log::info!("Successfully embedded: {} chunks", completed);
    if failed > 0 {
        log::warn!("Failed to embed: {} chunks", failed);
    }
    
    Ok(())
}
