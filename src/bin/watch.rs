//! Watch rag_folder for file changes; re-ingest and re-embed changed files automatically.

use clap::Parser;
use ragmcp::watch::run_watcher;
use ragmcp::{Config, db::Db, db::migrate, embeddings::OpenAIEmbedder};
use std::path::Path;
use anyhow::Result;

#[derive(Parser, Debug)]
#[command(name = "watch")]
#[command(about = "Watch rag_folder for changes and auto re-ingest + embed")]
struct Args {
    /// Debounce delay in milliseconds before processing a file change
    #[arg(long, default_value = "500")]
    debounce_ms: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().filter_or("RUST_LOG", "info"),
    )
    .init();

    let args = Args::parse();

    log::info!("Starting RAGMcp file watcher");
    let config = Config::load()?;
    log::info!("Docs root: {}", config.rag_folder().display());
    log::info!("Debounce: {} ms", args.debounce_ms);

    let db = Db::new(config.db_path());
    let migrations_dir = Path::new("migrations");
    db.with_connection(|conn| migrate::run_migrations(conn, migrations_dir)).await?;

    let api_key = std::env::var(&config.embeddings.api_key_env).map_err(|_| {
        anyhow::anyhow!(
            "Environment variable {} not set. Set it in your .env file or as an environment variable.",
            config.embeddings.api_key_env
        )
    })?;

    let embedder = OpenAIEmbedder::new(
        api_key,
        config.embeddings.model.clone(),
        config.embeddings.batch_size,
    );

    log::info!("Watching for changes (Ctrl+C to stop)");
    run_watcher(db, config, embedder, args.debounce_ms).await?;
    Ok(())
}
