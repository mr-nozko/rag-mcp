use rusqlite::params;
use chrono::{Utc, TimeZone};
use sha2::{Sha256, Digest};
use crate::error::{Result, RagmcpError};
use crate::db::Db;
use crate::graph::extract_routing_relations;
use super::chunker::Chunk;

/// Insert or update a document in the database
/// 
/// Returns the document ID (SHA256 hash of doc_path).
/// Uses ON CONFLICT to update existing documents based on doc_path.
pub async fn insert_document(
    db: &Db,
    doc_path: &str,
    doc_type: &str,
    namespace: &str,
    agent_name: Option<&str>,
    content: &str,
    tokens: usize,
    file_hash: &str,
    last_modified: std::time::SystemTime,
) -> Result<String> {
    // Compute doc_id as SHA256 hash of doc_path
    let mut hasher = Sha256::new();
    hasher.update(doc_path.as_bytes());
    let doc_id = format!("{:x}", hasher.finalize());
    
    // Convert SystemTime to chrono::DateTime
    let modified_dt = last_modified
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| RagmcpError::Config("Invalid file modification time".to_string()))?;
    let modified_chrono = Utc.timestamp_opt(
        modified_dt.as_secs() as i64,
        modified_dt.subsec_nanos(),
    )
    .single()
    .ok_or_else(|| RagmcpError::Config("Failed to convert timestamp".to_string()))?;
    
    // Clone values to move into closure
    let doc_id_clone = doc_id.clone();
    let doc_path_clone = doc_path.to_string();
    let doc_type_clone = doc_type.to_string();
    let namespace_clone = namespace.to_string();
    let agent_name_clone = agent_name.map(|s| s.to_string());
    let content_clone = content.to_string();
    let tokens_clone = tokens;
    let modified_str = modified_chrono.to_rfc3339();
    let file_hash_clone = file_hash.to_string();
    
    db.with_connection(move |conn| {
        // Delete old chunks if document exists (CASCADE should handle this, but be explicit)
        conn.execute(
            "DELETE FROM chunks WHERE doc_id = ?1",
            params![doc_id_clone],
        )?;
        
        // Insert or update document
        conn.execute(
            r#"
            INSERT INTO documents (
                doc_id, doc_path, doc_type, namespace, agent_name,
                content_text, content_tokens, last_modified, file_hash
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(doc_path) DO UPDATE SET
                doc_id = excluded.doc_id,
                doc_type = excluded.doc_type,
                namespace = excluded.namespace,
                agent_name = excluded.agent_name,
                content_text = excluded.content_text,
                content_tokens = excluded.content_tokens,
                last_modified = excluded.last_modified,
                file_hash = excluded.file_hash
            "#,
            params![
                doc_id_clone,
                doc_path_clone,
                doc_type_clone,
                namespace_clone,
                agent_name_clone,
                content_clone,
                tokens_clone as i64,
                modified_str,
                file_hash_clone,
            ],
        )?;
        
        Ok::<(), RagmcpError>(())
    }).await?;

    // Extract and store knowledge-graph relations from documents that have an entity name
    // (i.e. any file nested at least 2 directories deep, regardless of doc_type).
    // Relations are detected from arrow patterns in content: "Entity-A → Entity-B".
    if let Some(entity) = agent_name {
        let relations = extract_routing_relations(entity, content);
        if !relations.is_empty() {
            let entity_for_meta = entity.to_string();
            let relations_clone = relations.clone();
            db.with_connection(move |conn| {
                // Remove previously extracted relations for this entity to avoid duplicates on re-ingestion
                let pattern = format!("%\"extracted_from\":\"agent:{}\"%", entity_for_meta);
                conn.execute("DELETE FROM entity_relations WHERE metadata_json LIKE ?1", params![pattern])?;
                for rel in &relations_clone {
                    conn.execute(
                        "INSERT OR REPLACE INTO entity_relations (relation_id, source_entity, relation_type, target_entity, metadata_json) \
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![rel.relation_id, rel.source_entity, rel.relation_type, rel.target_entity, rel.metadata_json],
                    )?;
                }
                Ok::<(), RagmcpError>(())
            }).await?;
            log::debug!("Extracted {} relations from entity:{}", relations.len(), entity);
        }
    }

    Ok(doc_id.clone())
}

/// Insert chunks in batches
/// 
/// Inserts chunks in batches of 100 for efficiency.
/// FTS5 triggers automatically populate chunks_fts on insert.
pub async fn insert_chunks(
    db: &Db,
    doc_id: &str,
    chunks: Vec<Chunk>,
) -> Result<usize> {
    if chunks.is_empty() {
        return Ok(0);
    }
    
    // Clone doc_id to move into closure
    let doc_id = doc_id.to_string();
    let count = db.with_connection(move |conn| {
        let mut count = 0;
        const BATCH_SIZE: usize = 100;
        
        for batch in chunks.chunks(BATCH_SIZE) {
            // Use prepared statement for batch insert
            let mut stmt = conn.prepare(
                r#"
                INSERT INTO chunks (
                    chunk_id, doc_id, chunk_index, chunk_text,
                    chunk_tokens, section_header, chunk_type
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#
            )?;
            
            for (idx, chunk) in batch.iter().enumerate() {
                let chunk_id = format!("{}::{}", doc_id, count + idx);
                
                stmt.execute(params![
                    chunk_id,
                    doc_id,
                    (count + idx) as i64,
                    chunk.text,
                    chunk.tokens as i64,
                    chunk.section_header,
                    chunk.chunk_type,
                ])?;
            }
            
            count += batch.len();
        }
        
        Ok::<usize, RagmcpError>(count)
    }).await?;
    
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::graph::traverse_graph;
    use tempfile::TempDir;
    use std::path::Path;
    use crate::db::migrate;
    
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
    
    #[tokio::test]
    async fn test_insert_document() {
        let (db, _temp_dir) = setup_test_db().await;
        
        let doc_id = insert_document(
            &db,
            "test/path.xml",
            "agent_prompt",
            "agents",
            Some("test_agent"),
            "Test content",
            100,
            "test_hash",
            std::time::SystemTime::now(),
        ).await.unwrap();
        
        assert!(!doc_id.is_empty());
        
        // Verify document was inserted
        let doc_id_clone = doc_id.clone();
        db.with_connection(move |conn| {
            let mut stmt = conn.prepare("SELECT doc_path, doc_type FROM documents WHERE doc_id = ?1")?;
            let row = stmt.query_row(params![doc_id_clone], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            
            assert_eq!(row.0, "test/path.xml");
            assert_eq!(row.1, "agent_prompt");
            
            Ok::<(), RagmcpError>(())
        }).await.unwrap();
    }
    
    #[tokio::test]
    async fn test_insert_chunks() {
        let (db, _temp_dir) = setup_test_db().await;
        
        // Insert document first
        let doc_id = insert_document(
            &db,
            "test/path.xml",
            "agent_prompt",
            "agents",
            None,
            "Test content",
            100,
            "test_hash",
            std::time::SystemTime::now(),
        ).await.unwrap();
        
        // Insert chunks
        let chunks = vec![
            Chunk {
                text: "Chunk 1".to_string(),
                tokens: 10,
                section_header: Some("Section 1".to_string()),
                chunk_type: Some("test".to_string()),
            },
            Chunk {
                text: "Chunk 2".to_string(),
                tokens: 10,
                section_header: Some("Section 2".to_string()),
                chunk_type: None,
            },
        ];
        
        let count = insert_chunks(&db, &doc_id, chunks).await.unwrap();
        assert_eq!(count, 2);
        
        // Verify chunks were inserted
        let doc_id_clone = doc_id.clone();
        db.with_connection(move |conn| {
            let mut stmt = conn.prepare("SELECT COUNT(*) FROM chunks WHERE doc_id = ?1")?;
            let count: i64 = stmt.query_row(params![doc_id_clone], |row| row.get(0))?;
            assert_eq!(count, 2);
            
            // Verify FTS5 was populated
            let fts_pattern = format!("{}::%", doc_id_clone);
            let mut stmt = conn.prepare("SELECT COUNT(*) FROM chunks_fts WHERE chunk_id LIKE ?1")?;
            let fts_count: i64 = stmt.query_row(params![fts_pattern], |row| row.get(0))?;
            assert_eq!(fts_count, 2);
            
            Ok::<(), RagmcpError>(())
        }).await.unwrap();
    }
    
    #[tokio::test]
    async fn test_insert_document_upsert() {
        let (db, _temp_dir) = setup_test_db().await;
        
        let doc_id1 = insert_document(
            &db,
            "test/path.xml",
            "agent_prompt",
            "agents",
            None,
            "Original content",
            100,
            "hash1",
            std::time::SystemTime::now(),
        ).await.unwrap();
        
        // Update with new content
        let doc_id2 = insert_document(
            &db,
            "test/path.xml",
            "agent_prompt",
            "agents",
            None,
            "Updated content",
            200,
            "hash2",
            std::time::SystemTime::now(),
        ).await.unwrap();
        
        // doc_id should be the same (based on path)
        assert_eq!(doc_id1, doc_id2);
        
        // Verify content was updated
        let doc_id_clone = doc_id1.clone();
        db.with_connection(move |conn| {
            let mut stmt = conn.prepare("SELECT content_text, file_hash FROM documents WHERE doc_id = ?1")?;
            let row = stmt.query_row(params![doc_id_clone], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            
            assert_eq!(row.0, "Updated content");
            assert_eq!(row.1, "hash2");
            
            Ok::<(), RagmcpError>(())
        }).await.unwrap();
    }

    /// End-to-end: ingest agent doc with routing content, then traverse graph (Module 9).
    #[tokio::test]
    async fn test_ingest_extracts_relations_then_traverse() {
        let (db, _temp_dir) = setup_test_db().await;

        insert_document(
            &db,
            "docs/module-alpha/overview.md",
            "agent_prompt",
            "agents",
            Some("module-alpha"),
            "DefaultChains: Agent-A → Agent-B",
            50,
            "hash1",
            std::time::SystemTime::now(),
        )
        .await
        .unwrap();

        let relations = traverse_graph(&db, "agent:agent-a", None, 1).await.unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].target_entity, "agent:agent-b");
        assert_eq!(relations[0].relation_type, "routes_to");
    }
}
