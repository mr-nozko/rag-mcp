use crate::cache::EmbeddingCache;
use crate::error::{Result, RagmcpError};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Request structure for OpenAI embeddings API
#[derive(Serialize)]
struct EmbeddingRequest {
    model: String,
    input: Vec<String>,
}

/// Response structure from OpenAI embeddings API
#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

/// Individual embedding data in API response
#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

/// OpenAI embeddings client
/// 
/// Handles batch embedding generation with retry logic and rate limiting.
/// Optionally supports caching for query embeddings to reduce API calls.
pub struct OpenAIEmbedder {
    client: Client,
    api_key: String,
    model: String,
    batch_size: usize,
    cache: Option<Arc<EmbeddingCache>>,
}

impl OpenAIEmbedder {
    /// Create a new OpenAI embedder
    /// 
    /// # Arguments
    /// 
    /// * `api_key` - OpenAI API key
    /// * `model` - Model name (e.g., "text-embedding-3-small")
    /// * `batch_size` - Maximum number of texts to send per API request (max 2048)
    /// 
    /// # Panics
    /// 
    /// Panics if HTTP client cannot be created (should not happen in normal operation)
    pub fn new(api_key: String, model: String, batch_size: usize) -> Self {
        // Validate batch size doesn't exceed OpenAI limits
        let batch_size = batch_size.min(2048);
        
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");
        
        Self {
            client,
            api_key,
            model,
            batch_size,
            cache: None,
        }
    }
    
    /// Create a new OpenAI embedder with caching enabled
    /// 
    /// # Arguments
    /// 
    /// * `api_key` - OpenAI API key
    /// * `model` - Model name (e.g., "text-embedding-3-small")
    /// * `batch_size` - Maximum number of texts to send per API request (max 2048)
    /// * `cache` - Optional embedding cache for query embeddings
    /// 
    /// # Panics
    /// 
    /// Panics if HTTP client cannot be created (should not happen in normal operation)
    pub fn new_with_cache(
        api_key: String,
        model: String,
        batch_size: usize,
        cache: Option<Arc<EmbeddingCache>>,
    ) -> Self {
        // Validate batch size doesn't exceed OpenAI limits
        let batch_size = batch_size.min(2048);
        
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");
        
        Self {
            client,
            api_key,
            model,
            batch_size,
            cache,
        }
    }
    
    /// Embed a batch of texts, automatically splitting into smaller batches if needed
    /// 
    /// # Arguments
    /// 
    /// * `texts` - Vector of text strings to embed
    /// 
    /// # Returns
    /// 
    /// Vector of embeddings, one per input text, in the same order
    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        
        let mut all_embeddings = Vec::new();
        
        // Process in batches
        for chunk in texts.chunks(self.batch_size) {
            let embeddings = self.embed_batch_internal(chunk.to_vec()).await?;
            all_embeddings.extend(embeddings);
            
            // Rate limiting: small delay between batches to avoid hitting rate limits
            if chunk.len() == self.batch_size {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        
        Ok(all_embeddings)
    }
    
    /// Internal method to make a single API request
    /// 
    /// # Arguments
    /// 
    /// * `texts` - Vector of texts to embed in one request
    /// 
    /// # Returns
    /// 
    /// Vector of embeddings corresponding to input texts
    async fn embed_batch_internal(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let request = EmbeddingRequest {
            model: self.model.clone(),
            input: texts,
        };
        
        let response = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| RagmcpError::Embedding(format!("Network error: {}", e)))?;
        
        let status = response.status();
        
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error response".to_string());
            
            return Err(RagmcpError::Embedding(format!(
                "OpenAI API error {}: {}",
                status, body
            )));
        }
        
        let result: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| RagmcpError::Embedding(format!("Failed to parse response: {}", e)))?;
        
        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }
    
    /// Embed a single text with caching and retry logic
    /// 
    /// Checks cache first, then calls API if cache miss.
    /// 
    /// # Arguments
    /// 
    /// * `text` - Text string to embed
    /// * `max_retries` - Maximum number of retry attempts
    /// 
    /// # Returns
    /// 
    /// Embedding vector (1536 dimensions for text-embedding-3-small)
    pub async fn embed_with_cache(&self, text: &str, max_retries: usize) -> Result<Vec<f32>> {
        // Check cache first if available
        if let Some(cache) = &self.cache {
            if let Some(cached) = cache.get(text) {
                log::debug!("Cache hit for query: {}", text);
                return Ok(cached);
            }
        }
        
        // Cache miss - call API
        let embedding = self.embed_with_retry_internal(text, max_retries).await?;
        
        // Store in cache if available
        if let Some(cache) = &self.cache {
            cache.put(text.to_string(), embedding.clone());
        }
        
        Ok(embedding)
    }
    
    /// Embed a single text with retry logic (internal, no caching)
    /// 
    /// # Arguments
    /// 
    /// * `text` - Text string to embed
    /// * `max_retries` - Maximum number of retry attempts
    /// 
    /// # Returns
    /// 
    /// Embedding vector (1536 dimensions for text-embedding-3-small)
    pub async fn embed_with_retry(&self, text: &str, max_retries: usize) -> Result<Vec<f32>> {
        self.embed_with_retry_internal(text, max_retries).await
    }
    
    /// Internal method for embedding with retry (no caching)
    async fn embed_with_retry_internal(&self, text: &str, max_retries: usize) -> Result<Vec<f32>> {
        let start = std::time::Instant::now();
        let mut attempt = 0;
        let mut delay = Duration::from_secs(1);
        
        loop {
            match self.embed_batch_internal(vec![text.to_string()]).await {
                Ok(mut embeddings) => {
                    if embeddings.is_empty() {
                        return Err(RagmcpError::Embedding(
                            "Empty response from OpenAI API".to_string(),
                        ));
                    }
                    let duration = start.elapsed();
                    log::debug!("Embedding API call took {:?} (attempt {})", duration, attempt + 1);
                    return Ok(embeddings.remove(0));
                }
                Err(e) if attempt < max_retries => {
                    // Check if it's a retryable error (429 rate limit or 5xx server error)
                    let should_retry = e.to_string().contains("429")
                        || e.to_string().contains("500")
                        || e.to_string().contains("502")
                        || e.to_string().contains("503")
                        || e.to_string().contains("504");
                    
                    if should_retry {
                        log::warn!(
                            "Retry {}/{} after error: {}",
                            attempt + 1,
                            max_retries,
                            e
                        );
                        tokio::time::sleep(delay).await;
                        delay *= 2; // Exponential backoff
                        attempt += 1;
                    } else {
                        // Non-retryable error, return immediately
                        return Err(e);
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_embedder_new() {
        let embedder = OpenAIEmbedder::new(
            "test-key".to_string(),
            "text-embedding-3-small".to_string(),
            100,
        );
        
        assert_eq!(embedder.model, "text-embedding-3-small");
        assert_eq!(embedder.batch_size, 100);
    }
    
    #[test]
    fn test_embedder_batch_size_limit() {
        // Test that batch size is capped at 2048
        let embedder = OpenAIEmbedder::new(
            "test-key".to_string(),
            "text-embedding-3-small".to_string(),
            5000, // Exceeds limit
        );
        
        assert_eq!(embedder.batch_size, 2048);
    }
    
    #[test]
    fn test_embedder_batch_size_exact_limit() {
        // Test that exact limit (2048) is preserved
        let embedder = OpenAIEmbedder::new(
            "test-key".to_string(),
            "text-embedding-3-small".to_string(),
            2048,
        );
        
        assert_eq!(embedder.batch_size, 2048);
    }
    
    #[test]
    fn test_embedder_batch_size_under_limit() {
        // Test that values under limit are preserved
        let embedder = OpenAIEmbedder::new(
            "test-key".to_string(),
            "text-embedding-3-small".to_string(),
            100,
        );
        
        assert_eq!(embedder.batch_size, 100);
    }
    
    // Note: Integration tests for actual API calls would require a real API key
    // and should be run separately with proper test fixtures
}
