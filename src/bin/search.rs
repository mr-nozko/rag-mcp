use ragmcp::{Config, db::Db, embeddings::OpenAIEmbedder, search::hybrid};
use std::time::Instant;

/// Parse CLI args: optional --namespace <val>, --agent_filter <val>; first positional is the query.
fn parse_search_args() -> anyhow::Result<(String, Option<String>, Option<String>)> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut query = None;
    let mut namespace = None;
    let mut agent_filter = None;
    let mut next_namespace = false;
    let mut next_agent = false;
    for arg in &args {
        if next_namespace {
            namespace = Some(arg.clone());
            next_namespace = false;
            continue;
        }
        if next_agent {
            agent_filter = Some(arg.clone());
            next_agent = false;
            continue;
        }
        if arg == "--namespace" {
            next_namespace = true;
            continue;
        }
        if arg == "--agent_filter" {
            next_agent = true;
            continue;
        }
        if arg.starts_with("--") {
            continue;
        }
        if query.is_none() {
            query = Some(arg.clone());
        }
    }
    let query = query.ok_or_else(|| anyhow::anyhow!(
        "Usage: search <query> [--namespace <ns>] [--agent_filter <agent>]\nExample: search \"module overview\" --agent_filter module-alpha"
    ))?;
    if query.trim().is_empty() {
        anyhow::bail!("Query cannot be empty");
    }
    Ok((query, namespace, agent_filter))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    env_logger::init();

    // Load configuration
    let config = Config::load()?;

    // Initialize database
    let db = Db::new(config.db_path());

    // Get API key from environment (loaded by config via dotenv)
    let api_key = std::env::var(&config.embeddings.api_key_env)?;

    // Create embedder
    let embedder = OpenAIEmbedder::new(
        api_key,
        config.embeddings.model.clone(),
        config.embeddings.batch_size,
    );

    let (query, namespace, agent_filter) = parse_search_args()?;

    let namespace_ref = namespace.as_deref();
    let agent_filter_ref = agent_filter.as_deref();

    // Measure search latency
    let start = Instant::now();

    // Execute hybrid search (optional namespace/agent filter; no chunk cache in CLI)
    let results = hybrid::search_hybrid(
        &db,
        &embedder,
        &query,
        namespace_ref,
        agent_filter_ref,
        config.search.default_k,
        config.search.min_score,
        config.search.hybrid_bm25_weight,
        config.search.hybrid_vector_weight,
        None,
    )
    .await?;

    let duration = start.elapsed();

    // Display results
    println!("\n╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║ RAGMcp Hybrid Search Results                                                ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!("\nQuery: \"{}\"\n", query);

    if results.is_empty() {
        println!("No results found.");
    } else {
        for result in &results {
            println!("─────────────────────────────────────────────────────────────────────────────");
            println!("Rank #{}: {} (score: {:.3})", result.rank, result.doc_path, result.score);
            
            if let Some(ref section) = result.section {
                println!("Section: {}", section);
            }
            
            if let Some(ref agent) = result.agent_name {
                println!("Agent: {}", agent);
            }
            
            println!("Type: {}", result.doc_type);
            
            // Display chunk preview (first 200 characters)
            let preview_len = result.chunk_text.len().min(200);
            let preview = &result.chunk_text[..preview_len];
            let ellipsis = if result.chunk_text.len() > 200 { "..." } else { "" };
            
            println!("\nContent:");
            println!("{}{}", preview, ellipsis);
            println!();
        }
        println!("─────────────────────────────────────────────────────────────────────────────");
    }

    // Display search statistics
    println!("\n╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║ Search Statistics                                                            ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!("Results: {}", results.len());
    println!("Latency: {:?}", duration);
    println!("BM25 weight: {:.2}", config.search.hybrid_bm25_weight);
    println!("Vector weight: {:.2}", config.search.hybrid_vector_weight);
    println!("Min score: {:.2}", config.search.min_score);
    
    // Performance check
    if duration.as_millis() > config.performance.max_latency_ms as u128 {
        println!("\n⚠️  Warning: Search latency exceeded target of {}ms", config.performance.max_latency_ms);
    } else {
        println!("\n✅ Search completed within target latency");
    }

    Ok(())
}
