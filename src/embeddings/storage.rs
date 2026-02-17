use crate::db::Db;
use crate::error::{Result, RagmcpError};
use rusqlite::params;

/// Store an embedding for a chunk in the database
/// 
/// # Arguments
/// 
/// * `db` - Database connection wrapper
/// * `chunk_id` - Chunk identifier
/// * `embedding` - Embedding vector (1536 dimensions for text-embedding-3-small)
/// 
/// # Returns
/// 
/// Ok(()) on success, error if chunk not found or database operation fails
pub async fn store_embedding(
    db: &Db,
    chunk_id: &str,
    embedding: &[f32],
) -> Result<()> {
    // Convert Vec<f32> to BLOB (raw bytes, little-endian)
    let bytes: Vec<u8> = embedding
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    
    let chunk_id_clone = chunk_id.to_string();
    
    db.with_connection(move |conn| {
        let rows_affected = conn.execute(
            "UPDATE chunks SET embedding = ? WHERE chunk_id = ?",
            params![bytes, chunk_id_clone],
        )?;
        
        if rows_affected == 0 {
            return Err(RagmcpError::ChunkNotFound(chunk_id_clone));
        }
        
        Ok::<(), RagmcpError>(())
    })
    .await?;
    
    Ok(())
}

/// Retrieve an embedding for a chunk from the database
/// 
/// # Arguments
/// 
/// * `db` - Database connection wrapper
/// * `chunk_id` - Chunk identifier
/// 
/// # Returns
/// 
/// Embedding vector (1536 dimensions) or error if chunk not found or has no embedding
pub async fn get_embedding(db: &Db, chunk_id: &str) -> Result<Vec<f32>> {
    let chunk_id_clone = chunk_id.to_string();
    
    let embedding = db
        .with_connection(move |conn| {
            let mut stmt = conn.prepare("SELECT embedding FROM chunks WHERE chunk_id = ?")?;
            
            let mut rows = stmt.query(params![chunk_id_clone])?;
            
            if let Some(row) = rows.next()? {
                let blob: Option<Vec<u8>> = row.get(0)?;
                
                if let Some(blob) = blob {
                    // Convert BLOB back to Vec<f32>
                    let mut floats = Vec::new();
                    for bytes in blob.chunks(4) {
                        let arr: [u8; 4] = bytes.try_into().map_err(|_| {
                            RagmcpError::Embedding("Invalid embedding BLOB length".to_string())
                        })?;
                        floats.push(f32::from_le_bytes(arr));
                    }
                    
                    Ok(Some(floats))
                } else {
                    Ok(None)
                }
            } else {
                Ok(None)
            }
        })
        .await?;
    
    embedding.ok_or_else(|| RagmcpError::ChunkNotFound(chunk_id.to_string()))
}

/// Store multiple embeddings in a batch for better performance
/// 
/// # Arguments
/// 
/// * `db` - Database connection wrapper
/// * `embeddings` - Vector of (chunk_id, embedding) tuples
/// 
/// # Returns
/// 
/// Number of embeddings successfully stored
pub async fn store_embeddings_batch(
    db: &Db,
    embeddings: Vec<(String, Vec<f32>)>,
) -> Result<usize> {
    if embeddings.is_empty() {
        return Ok(0);
    }
    
    let embeddings_clone = embeddings.clone();
    
    let count = db
        .with_connection(move |conn| {
            // Use a transaction for better performance and atomicity
            let tx = conn.transaction()?;
            
            let mut success_count = 0;
            
            for (chunk_id, embedding) in embeddings_clone {
                // Convert embedding to BLOB
                let bytes: Vec<u8> = embedding
                    .iter()
                    .flat_map(|f| f.to_le_bytes())
                    .collect();
                
                match tx.execute(
                    "UPDATE chunks SET embedding = ? WHERE chunk_id = ?",
                    params![bytes, chunk_id],
                ) {
                    Ok(rows_affected) => {
                        if rows_affected > 0 {
                            success_count += 1;
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to store embedding for chunk {}: {}", chunk_id, e);
                        // Continue with other embeddings
                    }
                }
            }
            
            tx.commit()?;
            Ok::<usize, RagmcpError>(success_count)
        })
        .await?;
    
    Ok(count)
}

/// Return (chunk_id, chunk_text) for all chunks of a document that have no embedding yet.
/// Used by the watch module to embed only new chunks after re-ingestion.
pub async fn get_chunks_without_embedding_for_doc(
    db: &Db,
    doc_id: &str,
) -> Result<Vec<(String, String)>> {
    let doc_id = doc_id.to_string();
    let chunks = db
        .with_connection(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT chunk_id, chunk_text FROM chunks WHERE doc_id = ?1 AND embedding IS NULL",
            )?;
            let rows = stmt.query_map([&doc_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok::<Vec<(String, String)>, RagmcpError>(out)
        })
        .await?;
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate;
    use crate::ingest::db_writer::{insert_document, insert_chunks};
    use crate::ingest::chunker::Chunk;
    use std::path::Path;
    use tempfile::TempDir;
    
    async fn setup_test_db() -> (Db, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db = Db::new(&db_path);
        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        db.with_connection(move |conn| migrate::run_migrations(conn, &migrations_dir))
            .await
            .unwrap();
        (db, temp_dir)
    }
    
    async fn insert_test_chunk(db: &Db, index: usize) -> String {
        // Insert a test document with unique path
        let doc_path = format!("test/doc_{}.xml", index);
        let doc_id = insert_document(
            db,
            &doc_path,
            "agent_prompt",
            "agents",
            Some("test_agent"),
            "Test content",
            100,
            &format!("test_hash_{}", index),
            std::time::SystemTime::now(),
        )
        .await
        .unwrap();
        
        // Insert a test chunk
        let chunks = vec![Chunk {
            text: format!("Test chunk text {}", index),
            tokens: 5,
            section_header: Some("Test".to_string()),
            chunk_type: Some("test".to_string()),
        }];
        
        insert_chunks(db, &doc_id, chunks).await.unwrap();
        
        // Get the chunk_id (format: {doc_id}::{index})
        format!("{}::0", doc_id)
    }
    
    #[tokio::test]
    async fn test_store_and_get_embedding() {
        let (db, _temp_dir) = setup_test_db().await;
        let chunk_id = insert_test_chunk(&db, 0).await;
        
        // Create a test embedding (1536 dimensions)
        let test_embedding: Vec<f32> = (0..1536).map(|i| i as f32 * 0.001).collect();
        
        // Store embedding
        store_embedding(&db, &chunk_id, &test_embedding)
            .await
            .unwrap();
        
        // Retrieve embedding
        let retrieved = get_embedding(&db, &chunk_id).await.unwrap();
        
        // Verify round-trip
        assert_eq!(retrieved.len(), test_embedding.len());
        for (i, (original, retrieved)) in test_embedding.iter().zip(retrieved.iter()).enumerate() {
            assert!(
                (original - retrieved).abs() < 1e-6,
                "Embedding value mismatch at index {}: {} != {}",
                i,
                original,
                retrieved
            );
        }
    }
    
    #[tokio::test]
    async fn test_get_embedding_not_found() {
        let (db, _temp_dir) = setup_test_db().await;
        
        // First insert a document and chunk to ensure table exists
        let _doc_id = insert_test_chunk(&db, 0).await;
        
        let result = get_embedding(&db, "nonexistent_chunk").await;
        assert!(result.is_err());
        // Could be ChunkNotFound or Database error if chunk doesn't exist
        let err = result.unwrap_err();
        assert!(
            matches!(err, RagmcpError::ChunkNotFound(_)) || matches!(err, RagmcpError::Database(_)),
            "Expected ChunkNotFound or Database error, got: {:?}", err
        );
    }
    
    #[tokio::test]
    async fn test_store_embedding_not_found() {
        let (db, _temp_dir) = setup_test_db().await;
        
        // First insert a document and chunk to ensure table exists
        let _doc_id = insert_test_chunk(&db, 0).await;
        
        let test_embedding: Vec<f32> = vec![1.0, 2.0, 3.0];
        let result = store_embedding(&db, "nonexistent_chunk", &test_embedding).await;
        
        assert!(result.is_err());
        // Should be ChunkNotFound since chunk doesn't exist (rows_affected == 0)
        let err = result.unwrap_err();
        assert!(
            matches!(err, RagmcpError::ChunkNotFound(_)),
            "Expected ChunkNotFound error, got: {:?}", err
        );
    }
    
    #[tokio::test]
    async fn test_store_embeddings_batch() {
        let (db, _temp_dir) = setup_test_db().await;
        
        // Create multiple test chunks with unique documents
        let chunk_ids = vec![
            insert_test_chunk(&db, 0).await,
            insert_test_chunk(&db, 1).await,
            insert_test_chunk(&db, 2).await,
        ];
        
        // Create test embeddings with distinct but simple values
        // Use a base value per chunk to make verification easier
        let embeddings: Vec<(String, Vec<f32>)> = chunk_ids
            .iter()
            .enumerate()
            .map(|(i, chunk_id)| {
                // Create embeddings with a constant base value per chunk for easy verification
                // First chunk: all 0.1, second: all 0.2, third: all 0.3
                let base_value = (i + 1) as f32 * 0.1;
                let embedding: Vec<f32> = vec![base_value; 1536];
                (chunk_id.clone(), embedding)
            })
            .collect();
        
        // Store batch
        let count = store_embeddings_batch(&db, embeddings.clone())
            .await
            .unwrap();
        
        assert_eq!(count, 3);
        
        // Verify all embeddings stored correctly
        for (idx, (chunk_id, original_embedding)) in embeddings.iter().enumerate() {
            let retrieved = get_embedding(&db, chunk_id).await.unwrap();
            assert_eq!(retrieved.len(), original_embedding.len());
            
            // Check first value to verify we got the right embedding
            let expected_base = (idx + 1) as f32 * 0.1;
            let retrieved_base = retrieved[0];
            
            assert!(
                (expected_base - retrieved_base).abs() < 1e-6,
                "Wrong embedding retrieved for chunk {} (index {}): expected base={}, got base={}",
                chunk_id, idx, expected_base, retrieved_base
            );
            
            // Verify all values match
            for (val_idx, (original, retrieved_val)) in original_embedding.iter().zip(retrieved.iter()).enumerate() {
                let diff = (original - retrieved_val).abs();
                if diff >= 1e-6 {
                    panic!(
                        "Embedding mismatch at index {} for chunk {}: original={}, retrieved={}, diff={}",
                        val_idx, chunk_id, original, retrieved_val, diff
                    );
                }
            }
        }
    }
    
    #[tokio::test]
    async fn test_store_embeddings_batch_empty() {
        let (db, _temp_dir) = setup_test_db().await;
        
        let count = store_embeddings_batch(&db, Vec::new()).await.unwrap();
        assert_eq!(count, 0);
    }
    
    #[tokio::test]
    async fn test_store_embedding_updates_existing() {
        let (db, _temp_dir) = setup_test_db().await;
        let chunk_id = insert_test_chunk(&db, 0).await;
        
        // Store first embedding
        let embedding1: Vec<f32> = (0..1536).map(|i| i as f32 * 0.001).collect();
        store_embedding(&db, &chunk_id, &embedding1).await.unwrap();
        
        // Update with new embedding
        let embedding2: Vec<f32> = (0..1536).map(|i| (i + 1000) as f32 * 0.001).collect();
        store_embedding(&db, &chunk_id, &embedding2).await.unwrap();
        
        // Verify new embedding is stored
        let retrieved = get_embedding(&db, &chunk_id).await.unwrap();
        assert_eq!(retrieved.len(), embedding2.len());
        for (original, retrieved) in embedding2.iter().zip(retrieved.iter()) {
            assert!((original - retrieved).abs() < 1e-6);
        }
    }
    
    #[tokio::test]
    async fn test_get_embedding_no_embedding() {
        let (db, _temp_dir) = setup_test_db().await;
        let chunk_id = insert_test_chunk(&db, 0).await;
        
        // Try to get embedding for chunk that has no embedding
        // get_embedding returns ChunkNotFound when embedding is None
        let result = get_embedding(&db, &chunk_id).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, RagmcpError::ChunkNotFound(_)),
            "Expected ChunkNotFound error when embedding is None, got: {:?}", err
        );
    }

    /// get_chunks_without_embedding_for_doc returns only chunks with NULL embedding for the given doc_id.
    #[tokio::test]
    async fn test_get_chunks_without_embedding_for_doc() {
        let (db, _temp_dir) = setup_test_db().await;
        let doc_path = "test/doc_multi.xml";
        let doc_id = insert_document(
            &db,
            doc_path,
            "agent_prompt",
            "agents",
            Some("test_agent"),
            "Content",
            50,
            "hash_multi",
            std::time::SystemTime::now(),
        )
        .await
        .unwrap();
        let chunks = vec![
            Chunk {
                text: "Chunk A".to_string(),
                tokens: 2,
                section_header: Some("A".to_string()),
                chunk_type: Some("test".to_string()),
            },
            Chunk {
                text: "Chunk B".to_string(),
                tokens: 2,
                section_header: Some("B".to_string()),
                chunk_type: Some("test".to_string()),
            },
            Chunk {
                text: "Chunk C".to_string(),
                tokens: 2,
                section_header: Some("C".to_string()),
                chunk_type: Some("test".to_string()),
            },
        ];
        insert_chunks(&db, &doc_id, chunks).await.unwrap();
        let chunk_a = format!("{}::0", doc_id);
        let chunk_b = format!("{}::1", doc_id);
        let _chunk_c = format!("{}::2", doc_id);
        // Store embedding for A and B only; C remains NULL
        let emb: Vec<f32> = vec![0.1; 1536];
        store_embedding(&db, &chunk_a, &emb).await.unwrap();
        store_embedding(&db, &chunk_b, &emb).await.unwrap();

        let without = get_chunks_without_embedding_for_doc(&db, &doc_id).await.unwrap();
        assert_eq!(without.len(), 1, "only one chunk without embedding");
        assert_eq!(without[0].0, format!("{}::2", doc_id));
        assert_eq!(without[0].1, "Chunk C");
    }
}
