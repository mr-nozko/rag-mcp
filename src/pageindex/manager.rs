use std::process::{Child, Command};
use std::time::Duration;
use anyhow::{Result, Context, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use log::{info, warn};

pub struct PageIndexManager {
    pub sidecar_url: String,
    process: Option<Child>,
    client: Client,
}

#[derive(Debug, Serialize)]
pub struct IndexRequest {
    pub doc_path: String,
    pub model: String,
    pub force_rebuild: bool,
}

#[derive(Debug, Serialize)]
pub struct QueryRequest {
    pub doc_path: String,
    pub query: String,
    pub model: String,
    pub max_iterations: u8,
}

#[derive(Debug, Deserialize)]
pub struct IndexResponse {
    pub status: String,
    pub tree_path: String,
    pub node_count: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct QueryResponse {
    pub answer: String,
    pub retrieved_sections: Vec<serde_json::Value>,
    pub iterations: u32,
    pub latency_ms: u64,
}

impl PageIndexManager {
    pub fn new(port: u16) -> Self {
        Self {
            sidecar_url: format!("http://127.0.0.1:{}", port),
            process: None,
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
        }
    }

    pub async fn start(&mut self, port: u16) -> Result<()> {
        info!("[pageindex] Launching PageIndex Sidecar in a new terminal tab...");
        
        let python_cmd = format!(
            "python pageindex_sidecar/pageindex_sidecar.py --port {}",
            port
        );
        
        // Windows specific command to open in new terminal window
        let child = if cfg!(windows) {
            Command::new("cmd")
                .args(["/C", "start", "powershell", "-NoExit", "-Command", &python_cmd])
                .spawn()
                .context("Failed to spawn PageIndex terminal window")?
        } else {
            // Linux/macOS fallback: might need xterm or similar, keep simple for now
            Command::new("python3")
                .args(["pageindex_sidecar/pageindex_sidecar.py", "--port", &port.to_string()])
                .spawn()
                .context("Failed to start sidecar subprocess")?
        };

        // We store the process handle (for windows 'start' returns the shell proc, not the python one)
        self.process = Some(child);

        // Poll /health to ensure readiness
        self.wait_for_health(15).await?;
        info!("[pageindex] Sidecar detected as healthy at {}", self.sidecar_url);
        
        Ok(())
    }

    async fn wait_for_health(&self, timeout_secs: u64) -> Result<()> {
        let url = format!("{}/health", self.sidecar_url);
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
        
        while std::time::Instant::now() < deadline {
            match self.client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                _ => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
        anyhow::bail!("[pageindex] Sidecar failed health-check within {}s", timeout_secs)
    }

    pub async fn query(&self, doc_path: &str, query: &str, model: &str, max_iter: u8) -> Result<QueryResponse> {
        let url = format!("{}/query", self.sidecar_url);
        let req = QueryRequest {
            doc_path: doc_path.to_string(),
            query: query.to_string(),
            model: model.to_string(),
            max_iterations: max_iter,
        };
        
        let resp = self.client.post(&url)
            .json(&req)
            .send()
            .await
            .context("Error calling PageIndex sidecar query endpoint")?;
            
        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
             return Err(anyhow!("Sidecar query returned {}: {}", status, err_text));
        }

        let result: QueryResponse = resp.json().await
            .context("Failed to parse sidecar query response")?;
        Ok(result)
    }

    pub async fn index_document(&self, doc_path: &str, model: &str) -> Result<IndexResponse> {
        let url = format!("{}/index", self.sidecar_url);
        let req = IndexRequest {
            doc_path: doc_path.to_string(),
            model: model.to_string(),
            force_rebuild: false,
        };
        
        let resp = self.client.post(&url)
            .json(&req)
            .send()
            .await?;
            
        if !resp.status().is_success() {
            let status = resp.status();
            warn!("[pageindex] Indexing request failed for {}: {}", doc_path, status);
            return Err(anyhow!("Indexing failed with status: {}", status));
        }
        
        let result: IndexResponse = resp.json().await
            .context("Failed to parse sidecar index response")?;
            
        Ok(result)
    }

    pub async fn shutdown(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            info!("[pageindex] Shutdown trigger sent to sidecar management process");
        }
    }
}
