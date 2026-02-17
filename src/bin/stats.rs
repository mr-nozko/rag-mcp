use ragmcp::{config::Config, db::Db, error::RagmcpError};

/// Calculate percentile from sorted values
fn percentile(sorted_values: &[i64], p: f64) -> i64 {
    if sorted_values.is_empty() {
        return 0;
    }
    let index = ((sorted_values.len() - 1) as f64 * p).ceil() as usize;
    sorted_values[index.min(sorted_values.len() - 1)]
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    
    let config = Config::load()?;
    let db = Db::new(&config.ragmcp.db_path);
    
    println!("\n=== RAGMcp Query Performance Statistics ===\n");
    
    // Query statistics by retrieval method for last 24 hours
    let stats = db.with_connection(|conn| {
        let mut stmt = conn.prepare(
            r#"
            SELECT 
                retrieval_method,
                COUNT(*) as count,
                AVG(latency_ms) as avg_latency,
                MIN(latency_ms) as min_latency,
                MAX(latency_ms) as max_latency,
                AVG(result_count) as avg_result_count,
                SUM(result_count) as total_results
            FROM query_logs
            WHERE timestamp > datetime('now', '-24 hours')
            GROUP BY retrieval_method
            ORDER BY count DESC
            "#
        )?;
        
        let mut rows = stmt.query([])?;
        let mut results = Vec::new();
        
        while let Some(row) = rows.next()? {
            results.push((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, i64>(1)?, // count
                row.get::<_, Option<f64>>(2)?, // avg_latency
                row.get::<_, Option<i64>>(3)?, // min_latency
                row.get::<_, Option<i64>>(4)?, // max_latency
                row.get::<_, Option<f64>>(5)?, // avg_result_count
                row.get::<_, Option<i64>>(6)?, // total_results
            ));
        }
        
        Ok::<Vec<_>, RagmcpError>(results)
    }).await?;
    
    if stats.is_empty() {
        println!("No queries found in the last 24 hours.");
        println!("\nRun some searches to generate statistics.");
        return Ok(());
    }
    
    println!("24-Hour Query Statistics by Retrieval Method:\n");
    println!("{:-<80}", "");
    println!(
        "{:<20} {:>8} {:>12} {:>10} {:>10} {:>12} {:>12}",
        "Method", "Count", "Avg (ms)", "Min (ms)", "Max (ms)", "Avg Results", "Total Results"
    );
    println!("{:-<80}", "");
    
    for (method, count, avg_latency, min_latency, max_latency, avg_results, total_results) in &stats {
        let method_str = method.as_deref().unwrap_or("unknown");
        let avg_lat = avg_latency.map(|v| v as i64).unwrap_or(0);
        let min_lat = min_latency.unwrap_or(0);
        let max_lat = max_latency.unwrap_or(0);
        let avg_res = avg_results.map(|v| v as i64).unwrap_or(0);
        let total_res = total_results.unwrap_or(0);
        
        println!(
            "{:<20} {:>8} {:>12} {:>10} {:>10} {:>12} {:>12}",
            method_str, count, avg_lat, min_lat, max_lat, avg_res, total_res
        );
    }
    println!("{:-<80}", "");
    
    // Calculate percentiles for all queries
    let all_latencies = db.with_connection(|conn| {
        let mut stmt = conn.prepare(
            r#"
            SELECT latency_ms
            FROM query_logs
            WHERE timestamp > datetime('now', '-24 hours')
                AND latency_ms IS NOT NULL
            ORDER BY latency_ms
            "#
        )?;
        
        let mut rows = stmt.query([])?;
        let mut latencies = Vec::new();
        
        while let Some(row) = rows.next()? {
            latencies.push(row.get::<_, i64>(0)?);
        }
        
        Ok::<Vec<i64>, RagmcpError>(latencies)
    }).await?;
    
    if !all_latencies.is_empty() {
        println!("\nLatency Percentiles (All Methods, Last 24 Hours):\n");
        println!("{:-<50}", "");
        println!("{:<15} {:>15}", "Percentile", "Latency (ms)");
        println!("{:-<50}", "");
        println!("{:<15} {:>15}", "P50", percentile(&all_latencies, 0.50));
        println!("{:<15} {:>15}", "P95", percentile(&all_latencies, 0.95));
        println!("{:<15} {:>15}", "P99", percentile(&all_latencies, 0.99));
        println!("{:-<50}", "");
        
        // Check against PRD targets
        let p50 = percentile(&all_latencies, 0.50);
        let p95 = percentile(&all_latencies, 0.95);
        
        println!("\nPRD Targets:");
        println!("  P50 < 500ms: {}", if p50 < 500 { "✅ PASS" } else { "❌ FAIL" });
        println!("  P95 < 1000ms: {}", if p95 < 1000 { "✅ PASS" } else { "❌ FAIL" });
    }
    
    // Recent query activity
    let recent_count = db.with_connection(|conn| {
        conn.query_row(
            r#"
            SELECT COUNT(*)
            FROM query_logs
            WHERE timestamp > datetime('now', '-1 hour')
            "#,
            [],
            |row| row.get::<_, i64>(0)
        ).map_err(RagmcpError::from)
    }).await?;
    
    println!("\nRecent Activity:");
    println!("  Queries in last hour: {}", recent_count);
    
    // Total statistics
    let total_stats = db.with_connection(|conn| {
        conn.query_row(
            r#"
            SELECT 
                COUNT(*) as total_queries,
                MIN(timestamp) as first_query,
                MAX(timestamp) as last_query
            FROM query_logs
            "#,
            [],
            |row| Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        ).map_err(RagmcpError::from)
    }).await?;
    
    println!("\nTotal Statistics:");
    println!("  Total queries logged: {}", total_stats.0);
    if let Some(first) = total_stats.1 {
        println!("  First query: {}", first);
    }
    if let Some(last) = total_stats.2 {
        println!("  Last query: {}", last);
    }
    
    println!();
    
    Ok(())
}
