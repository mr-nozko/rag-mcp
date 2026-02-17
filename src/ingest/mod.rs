pub mod walker;
pub mod metadata;
pub mod parsers;
pub mod chunker;
pub mod db_writer;
pub mod incremental;

pub use walker::{FileMetadata, discover_files};
pub use incremental::{
    FileClassification, classify_files, delete_documents, find_deleted_documents, get_existing_hashes,
};
pub use metadata::{compute_file_hash, extract_namespace, extract_agent_name};
pub use parsers::{ParserRegistry, ParsedDocument, Section};
pub use chunker::{Chunk, chunk_document, estimate_tokens};
pub use db_writer::{insert_document, insert_chunks};

/// Convenience function to ingest a single file
/// 
/// Orchestrates the full pipeline: parse → chunk → insert
pub async fn ingest_file(
    db: &crate::db::Db,
    file: &FileMetadata,
    parser_registry: &ParserRegistry,
    config: &crate::Config,
) -> crate::error::Result<(usize, usize)> {
    // Read file content
    let content = std::fs::read_to_string(&file.absolute_path)
        .map_err(crate::error::RagmcpError::Io)?;
    
    // Compute file hash
    let file_hash = compute_file_hash(&file.absolute_path)?;
    
    // Extract metadata
    let namespace = extract_namespace(&file.relative_path);
    let agent_name = extract_agent_name(&file.relative_path);
    
    // Parse document
    let parsed = parser_registry.parse(
        &content,
        &file.relative_path,
        &file.extension,
    )?;
    
    // Chunk document
    let chunks = chunk_document(&parsed, &config.performance)?;
    
    // Estimate total tokens
    let total_tokens = chunks.iter().map(|c| c.tokens).sum::<usize>();
    
    // Insert document
    let doc_id = insert_document(
        db,
        &file.relative_path,
        &parsed.doc_type,
        &namespace,
        agent_name.as_deref(),
        &parsed.content,
        total_tokens,
        &file_hash,
        file.modified,
    ).await?;
    
    // Insert chunks
    let chunk_count = insert_chunks(db, &doc_id, chunks).await?;
    
    Ok((chunk_count, total_tokens))
}
