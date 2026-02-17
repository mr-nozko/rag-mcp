use ragmcp::Config;
use ragmcp::cache::{ChunkEmbeddingCache, EmbeddingCache};
use ragmcp::db::{Db, migrate};
use ragmcp::embeddings::OpenAIEmbedder;
use ragmcp::mcp::{HttpMcpServer, McpServer};
use std::path::Path;
use std::sync::Arc;
use anyhow::Result;

/// Build a configured embedder with an optional LRU query-embedding cache.
/// Extracted to avoid duplicating this setup between serve and serve-http paths.
fn build_embedder(config: &Config) -> Result<OpenAIEmbedder> {
    let api_key = std::env::var(&config.embeddings.api_key_env)
        .map_err(|_| anyhow::anyhow!(
            "Environment variable {} not set. Set it in your .env file or as an environment variable.",
            config.embeddings.api_key_env
        ))?;

    // Wrap in an LRU cache if cache_capacity > 0 (avoids re-embedding repeated queries)
    let cache = if config.embeddings.cache_capacity > 0 {
        Some(Arc::new(EmbeddingCache::new(config.embeddings.cache_capacity)))
    } else {
        None
    };

    Ok(OpenAIEmbedder::new_with_cache(
        api_key,
        config.embeddings.model.clone(),
        config.embeddings.batch_size,
        cache,
    ))
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger from environment variable or default to info level
    // For MCP server mode, we'll log to stderr (per MCP spec)
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .filter_or("RUST_LOG", "info")
    ).init();
    
    // Parse command-line arguments
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("verify");
    
    match command {
        "serve" => {
            // MCP server mode (stdio transport)
            run_mcp_server().await?;
        }
        "serve-http" => {
            // HTTP server mode (for custom connectors)
            run_http_server().await?;
        }
        "verify" | _ => {
            // Default: verify database schema
            run_schema_verification().await?;
        }
    }
    
    Ok(())
}

/// Run MCP server (stdio transport)
async fn run_mcp_server() -> Result<()> {
    // Load configuration
    let config = Config::load()?;
    
    // Initialize database
    let db = Db::new(config.db_path());
    
    // Run migrations
    let migrations_dir = Path::new("migrations");
    db.with_connection(|conn| {
        migrate::run_migrations(conn, migrations_dir)
    }).await?;
    
    let embedder = build_embedder(&config)?;
    let chunk_cache = Some(Arc::new(ChunkEmbeddingCache::new()));

    // Create and run MCP server (stdio transport)
    let mut server = McpServer::new(db, embedder, config, chunk_cache);
    server.run().await?;
    
    Ok(())
}

/// Run HTTP MCP server
async fn run_http_server() -> Result<()> {
    log::info!("Starting RAGMcp HTTP Server v{}", env!("CARGO_PKG_VERSION"));
    
    // Load configuration
    let config = Config::load()?;
    
    // Initialize database
    let db = Db::new(config.db_path());
    
    // Run migrations
    let migrations_dir = Path::new("migrations");
    db.with_connection(|conn| {
        migrate::run_migrations(conn, migrations_dir)
    }).await?;
    
    log::info!("Database initialized successfully");

    let embedder = build_embedder(&config)?;
    let chunk_cache = Some(Arc::new(ChunkEmbeddingCache::new()));

    // Create and run HTTP MCP server (custom connector / Cloudflare Tunnel transport)
    let http_server = HttpMcpServer::new(db, embedder, config.clone(), chunk_cache)?;
    http_server.run(config.http_server.port).await?;
    
    Ok(())
}

/// Run database schema verification
async fn run_schema_verification() -> Result<()> {
    log::info!("Starting RAGMcp v{}", env!("CARGO_PKG_VERSION"));
    
    // Load configuration
    let config = Config::load()?;
    log::info!("Configuration loaded successfully");
    log::info!("Docs root: {}", config.rag_folder().display());
    log::info!("Database path: {}", config.db_path().display());
    log::info!("Embedding model: {}", config.embeddings.model);
    
    // Initialize database
    let db = Db::new(config.db_path());
    
    // Run migrations
    let migrations_dir = Path::new("migrations");
    db.with_connection(|conn| {
        migrate::run_migrations(conn, migrations_dir)
    }).await?;
    
    log::info!("Database initialized successfully");
    
    // Verify schema
    verify_database_schema(&db).await?;
    
    log::info!("Ready for Phase 2: Document Ingestion");
    
    Ok(())
}

/// Verify that all expected database objects exist
async fn verify_database_schema(db: &ragmcp::db::Db) -> Result<()> {
    use ragmcp::db::migrate;
    use ragmcp::error::RagmcpError;
    
    db.with_connection(|conn| {
        // Check tables
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
        let tables: Vec<String> = stmt.query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;
        
        let expected_tables = vec!["chunks", "documents", "entity_relations", "query_logs", "schema_migrations"];
        let mut all_tables_exist = true;
        
        for table in &expected_tables {
            if !tables.iter().any(|t| t == table) {
                log::error!("Missing table: {}", table);
                all_tables_exist = false;
            } else {
                log::debug!("✓ Table exists: {}", table);
            }
        }
        
        if !all_tables_exist {
            return Err(RagmcpError::Config("Not all required tables exist".to_string()));
        }
        
        // Check FTS5 virtual table
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='chunks_fts'")?;
        let fts_exists: bool = stmt.exists([])?;
        if !fts_exists {
            return Err(RagmcpError::Config("FTS5 virtual table 'chunks_fts' does not exist".to_string()));
        }
        log::debug!("✓ FTS5 virtual table exists");
        
        // Check triggers
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='trigger' ORDER BY name")?;
        let triggers: Vec<String> = stmt.query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;
        
        let expected_triggers = vec!["chunks_fts_insert", "chunks_fts_delete", "chunks_fts_update"];
        let mut all_triggers_exist = true;
        
        for trigger in &expected_triggers {
            if !triggers.iter().any(|t| t.contains(trigger)) {
                log::error!("Missing trigger: {}", trigger);
                all_triggers_exist = false;
            } else {
                log::debug!("✓ Trigger exists: {}", trigger);
            }
        }
        
        if !all_triggers_exist {
            return Err(RagmcpError::Config("Not all required triggers exist".to_string()));
        }
        
        // Check migrations
        let applied = migrate::get_applied_migrations(conn)?;
        if applied.len() < 3 {
            return Err(RagmcpError::Config(format!("Expected at least 3 migrations, found {}", applied.len())));
        }
        log::debug!("✓ {} migrations applied", applied.len());
        
        // Check performance indexes (from migration 004)
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%' ORDER BY name")?;
        let indexes: Vec<String> = stmt.query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;
        
        let expected_indexes = vec![
            "idx_chunks_embedding_filter",
            "idx_documents_namespace_agent_type",
            "idx_logs_timestamp_method",
        ];
        
        for index_name in &expected_indexes {
            if indexes.iter().any(|i| i == index_name) {
                log::debug!("✓ Performance index exists: {}", index_name);
            } else {
                log::warn!("Performance index not found: {} (migration 004 may not be applied)", index_name);
            }
        }
        
        // Check pragmas
        let journal_mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        if journal_mode.to_uppercase() != "WAL" {
            return Err(RagmcpError::Config(format!("Journal mode is not WAL: {}", journal_mode)));
        }
        log::debug!("✓ Journal mode: WAL");
        
        let foreign_keys: i32 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        if foreign_keys != 1 {
            return Err(RagmcpError::Config("Foreign keys not enabled".to_string()));
        }
        log::debug!("✓ Foreign keys enabled");
        
        // Integrity check
        let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(RagmcpError::Config(format!("Database integrity check failed: {}", integrity)));
        }
        log::info!("✓ Database integrity: OK");
        
        Ok(())
    }).await?;
    
    log::info!("✓ Database schema verification complete");
    Ok(())
}
