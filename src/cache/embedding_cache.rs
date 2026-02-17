use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;

/// Thread-safe LRU cache for query embeddings
/// 
/// Caches embeddings for frequently-used queries to avoid redundant API calls.
/// Uses LRU eviction policy to maintain bounded memory usage.
pub struct EmbeddingCache {
    cache: Mutex<LruCache<String, Vec<f32>>>,
}

impl EmbeddingCache {
    /// Create a new embedding cache with the specified capacity
    /// 
    /// # Arguments
    /// 
    /// * `capacity` - Maximum number of embeddings to cache
    /// 
    /// # Panics
    /// 
    /// Panics if capacity is 0 (LRU cache requires non-zero capacity)
    pub fn new(capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1))
            .expect("Cache capacity must be at least 1");
        
        Self {
            cache: Mutex::new(LruCache::new(cap)),
        }
    }
    
    /// Get a cached embedding for a query
    /// 
    /// # Arguments
    /// 
    /// * `query` - Query text to look up
    /// 
    /// # Returns
    /// 
    /// Some(embedding) if found in cache, None otherwise
    pub fn get(&self, query: &str) -> Option<Vec<f32>> {
        self.cache
            .lock()
            .unwrap()
            .get(query)
            .cloned()
    }
    
    /// Store an embedding in the cache
    /// 
    /// # Arguments
    /// 
    /// * `query` - Query text (used as key)
    /// * `embedding` - Embedding vector to cache
    pub fn put(&self, query: String, embedding: Vec<f32>) {
        self.cache
            .lock()
            .unwrap()
            .put(query, embedding);
    }
    
    /// Get the current number of cached entries
    pub fn len(&self) -> usize {
        self.cache.lock().unwrap().len()
    }
    
    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.cache.lock().unwrap().is_empty()
    }
    
    /// Clear all entries from the cache
    pub fn clear(&self) {
        self.cache.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_cache_put_and_get() {
        let cache = EmbeddingCache::new(10);
        
        let query = "test query".to_string();
        let embedding = vec![1.0, 2.0, 3.0];
        
        cache.put(query.clone(), embedding.clone());
        
        let retrieved = cache.get(&query);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), embedding);
    }
    
    #[test]
    fn test_cache_miss() {
        let cache = EmbeddingCache::new(10);
        
        let retrieved = cache.get("nonexistent query");
        assert!(retrieved.is_none());
    }
    
    #[test]
    fn test_cache_eviction() {
        let cache = EmbeddingCache::new(2);
        
        // Fill cache to capacity
        cache.put("query1".to_string(), vec![1.0]);
        cache.put("query2".to_string(), vec![2.0]);
        
        // Add third entry - should evict query1 (LRU)
        cache.put("query3".to_string(), vec![3.0]);
        
        assert!(cache.get("query1").is_none()); // Evicted
        assert!(cache.get("query2").is_some()); // Still present
        assert!(cache.get("query3").is_some()); // New entry
    }
    
    #[test]
    fn test_cache_len() {
        let cache = EmbeddingCache::new(10);
        
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
        
        cache.put("query1".to_string(), vec![1.0]);
        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());
        
        cache.put("query2".to_string(), vec![2.0]);
        assert_eq!(cache.len(), 2);
    }
    
    #[test]
    fn test_cache_clear() {
        let cache = EmbeddingCache::new(10);
        
        cache.put("query1".to_string(), vec![1.0]);
        cache.put("query2".to_string(), vec![2.0]);
        
        assert_eq!(cache.len(), 2);
        
        cache.clear();
        
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
        assert!(cache.get("query1").is_none());
        assert!(cache.get("query2").is_none());
    }
    
    #[test]
    fn test_cache_get_updates_lru() {
        let cache = EmbeddingCache::new(2);
        
        cache.put("query1".to_string(), vec![1.0]);
        cache.put("query2".to_string(), vec![2.0]);
        
        // Access query1 to update its position in LRU
        let _ = cache.get("query1");
        
        // Add third entry - should evict query2 (not query1, since it was recently accessed)
        cache.put("query3".to_string(), vec![3.0]);
        
        assert!(cache.get("query1").is_some()); // Still present (recently accessed)
        assert!(cache.get("query2").is_none()); // Evicted
        assert!(cache.get("query3").is_some()); // New entry
    }
    
    #[test]
    fn test_cache_capacity_one() {
        let cache = EmbeddingCache::new(1);
        
        cache.put("query1".to_string(), vec![1.0]);
        assert_eq!(cache.len(), 1);
        
        cache.put("query2".to_string(), vec![2.0]);
        assert_eq!(cache.len(), 1);
        assert!(cache.get("query1").is_none()); // Evicted
        assert!(cache.get("query2").is_some()); // Present
    }
}
