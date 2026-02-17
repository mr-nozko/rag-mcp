//! In-memory cache of chunk embeddings for fast vector search.
//!
//! Loads all chunk_id -> embedding pairs once from the database; vector search
//! then scores in memory and fetches metadata only for top-k chunks.

use crate::db::Db;
use crate::error::{Result, RagmcpError};
use std::collections::HashMap;
use std::sync::RwLock;

/// In-memory cache of chunk embeddings. Load once, then vector search
/// scores against this map and fetches metadata only for top-k.
pub struct ChunkEmbeddingCache {
    /// None = not loaded; Some = map of chunk_id -> embedding (1536-dim)
    inner: RwLock<Option<HashMap<String, Vec<f32>>>>,
}

fn parse_embedding_blob(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.len() % 4 != 0 {
        return None;
    }
    blob.chunks(4)
        .map(|bytes| {
            let arr: [u8; 4] = bytes.try_into().ok()?;
            Some(f32::from_le_bytes(arr))
        })
        .collect()
}

impl ChunkEmbeddingCache {
    /// Create an empty cache (not loaded).
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }

    /// Return true if the cache has been loaded.
    pub fn is_loaded(&self) -> bool {
        self.inner.read().unwrap().is_some()
    }

    /// Number of chunk embeddings in the cache (0 if not loaded).
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .unwrap()
            .as_ref()
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// Load all chunk_id, embedding from the database. Idempotent: reloads if already loaded.
    pub async fn load_from_db(&self, db: &Db) -> Result<()> {
        let rows = db
            .with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT chunk_id, embedding FROM chunks WHERE embedding IS NOT NULL",
                )?;
                let mut rows = stmt.query([])?;
                let mut map = HashMap::new();
                while let Some(row) = rows.next()? {
                    let chunk_id: String = row.get(0)?;
                    let blob: Option<Vec<u8>> = row.get(1)?;
                    if let Some(blob) = blob {
                        if let Some(embedding) = parse_embedding_blob(&blob) {
                            if embedding.len() == 1536 {
                                map.insert(chunk_id, embedding);
                            }
                        }
                    }
                }
                Ok::<HashMap<String, Vec<f32>>, RagmcpError>(map)
            })
            .await?;
        *self.inner.write().unwrap() = Some(rows);
        log::info!(
            "Chunk embedding cache loaded: {} embeddings",
            self.inner.read().unwrap().as_ref().unwrap().len()
        );
        Ok(())
    }

    /// Ensure cache is loaded; no-op if already loaded.
    pub async fn load_if_needed(&self, db: &Db) -> Result<()> {
        if !self.is_loaded() {
            self.load_from_db(db).await?;
        }
        Ok(())
    }

    /// Clear the cache (e.g. after re-ingestion). Next search will reload.
    pub fn clear(&self) {
        *self.inner.write().unwrap() = None;
    }

    /// Get embedding for a chunk, if loaded.
    pub fn get(&self, chunk_id: &str) -> Option<Vec<f32>> {
        self.inner
            .read()
            .unwrap()
            .as_ref()
            .and_then(|m| m.get(chunk_id).cloned())
    }

    /// Score query vector against all cached embeddings; return top `limit` (score, chunk_id)
    /// with score >= min_score, sorted by score descending.
    pub fn top_k_chunk_ids(
        &self,
        query_vec: &[f32],
        limit: usize,
        min_score: f32,
    ) -> Vec<(f32, String)> {
        let guard = self.inner.read().unwrap();
        let map = match guard.as_ref() {
            Some(m) => m,
            None => return Vec::new(),
        };
        let mut scored: Vec<(f32, String)> = Vec::with_capacity(map.len());
        for (chunk_id, emb) in map.iter() {
            if emb.len() != query_vec.len() {
                continue;
            }
            let sim = cosine_similarity(query_vec, emb);
            if sim >= min_score {
                scored.push((sim, chunk_id.clone()));
            }
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(limit).collect()
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        0.0
    } else {
        dot / (mag_a * mag_b)
    }
}

impl Default for ChunkEmbeddingCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_embedding_blob() {
        let blob: Vec<u8> = vec![1.0f32, 2.0f32, 3.0f32, 4.0f32]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let parsed = parse_embedding_blob(&blob);
        assert!(parsed.is_some());
        assert_eq!(parsed.unwrap().len(), 4);
    }

    #[test]
    fn test_cache_new_not_loaded() {
        let cache = ChunkEmbeddingCache::new();
        assert!(!cache.is_loaded());
        assert_eq!(cache.len(), 0);
        assert!(cache.get("any").is_none());
        assert!(cache.top_k_chunk_ids(&[1.0; 1536], 5, 0.0).is_empty());
    }

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }
}
