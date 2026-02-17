//! Evaluation CLI: run hybrid search over eval queries and report P@5, R@10, MRR.

use clap::Parser;
use ragmcp::{
    db::Db,
    embeddings::OpenAIEmbedder,
    eval::{mean_reciprocal_rank, precision_at_k, recall_at_k, EvalQuery},
    search::hybrid,
    Config,
};
use std::path::PathBuf;

/// Evaluation framework: run queries and report metrics.
#[derive(Parser, Debug)]
#[command(name = "eval")]
struct Args {
    /// Path to eval queries JSON (default: eval_queries.json).
    #[arg(long, default_value = "eval_queries.json")]
    queries: PathBuf,

    /// Search method (only hybrid supported).
    #[arg(long, default_value = "hybrid")]
    method: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let args = Args::parse();
    let config = Config::load()?;
    let db = Db::new(config.db_path());

    let api_key = std::env::var(&config.embeddings.api_key_env)?;
    let embedder = OpenAIEmbedder::new(
        api_key,
        config.embeddings.model.clone(),
        config.embeddings.batch_size,
    );

    let queries_json = std::fs::read_to_string(&args.queries)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", args.queries.display(), e))?;
    let queries: Vec<EvalQuery> =
        serde_json::from_str(&queries_json).map_err(|e| anyhow::anyhow!("Invalid queries JSON: {}", e))?;

    if queries.is_empty() {
        anyhow::bail!("No queries in {}", args.queries.display());
    }

    println!("Running evaluation on {} queries ({})\n", queries.len(), args.method);

    let k_retrieve = 10_usize.max(config.search.default_k);
    let mut all_results = Vec::with_capacity(queries.len());
    let mut precisions = Vec::with_capacity(queries.len());
    let mut recalls = Vec::with_capacity(queries.len());

    for query in &queries {
        let results = hybrid::search_hybrid(
            &db,
            &embedder,
            &query.query,
            None,
            None,
            k_retrieve,
            config.search.min_score,
            config.search.hybrid_bm25_weight,
            config.search.hybrid_vector_weight,
            None,
        )
        .await?;

        let relevant = query.relevant_chunk_ids(&db).await?;
        let precision = precision_at_k(&results, &relevant, 5);
        let recall = recall_at_k(&results, &relevant, 10);

        precisions.push(precision);
        recalls.push(recall);
        all_results.push(results);

        println!(
            "  {} (P@5: {:.2}, R@10: {:.2})",
            query.query,
            precision * 100.0,
            recall * 100.0
        );
    }

    let avg_precision = precisions.iter().sum::<f32>() / precisions.len() as f32;
    let avg_recall = recalls.iter().sum::<f32>() / recalls.len() as f32;
    let mrr = mean_reciprocal_rank(&queries, &all_results);

    println!("\n=== Evaluation Results ===");
    println!("Precision@5: {:.2}%", avg_precision * 100.0);
    println!("Recall@10:   {:.2}%", avg_recall * 100.0);
    println!("MRR:         {:.2}", mrr);

    const THRESHOLD_P: f32 = 0.85;
    const THRESHOLD_R: f32 = 0.90;
    const THRESHOLD_MRR: f32 = 0.80;

    if avg_precision >= THRESHOLD_P && avg_recall >= THRESHOLD_R && mrr >= THRESHOLD_MRR {
        println!("\nAll metrics pass (P@5 >= {:.0}%, R@10 >= {:.0}%, MRR >= {:.2}).", THRESHOLD_P * 100.0, THRESHOLD_R * 100.0, THRESHOLD_MRR);
        std::process::exit(0);
    } else {
        println!("\nMetrics below threshold (P@5 >= {:.0}%, R@10 >= {:.0}%, MRR >= {:.2}).", THRESHOLD_P * 100.0, THRESHOLD_R * 100.0, THRESHOLD_MRR);
        std::process::exit(1);
    }
}
