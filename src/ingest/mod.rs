pub mod walker;
pub mod metadata;
pub mod parsers;
pub mod chunker;
pub mod db_writer;
pub mod incremental;

pub use walker::{FileMetadata, discover_files, is_indexable_extension};
pub use incremental::{
    FileClassification, DeleteDocumentsResult, classify_files, delete_documents,
    delete_documents_normalized, find_deleted_documents, get_existing_hashes,
};
pub use metadata::{compute_file_hash, extract_agent_name, extract_namespace, normalize_doc_path};
pub use parsers::{ParserRegistry, ParsedDocument, Section};
pub use chunker::{Chunk, chunk_document, estimate_tokens};
pub use db_writer::{insert_document, insert_chunks};

/// Convenience function to ingest a single file
/// 
/// Orchestrates the full pipeline: parse → chunk → insert.
/// Returns `(doc_id, chunk_count, total_tokens)`.
pub async fn ingest_file(
    db: &crate::db::Db,
    file: &FileMetadata,
    parser_registry: &ParserRegistry,
    config: &crate::Config,
) -> crate::error::Result<(String, usize, usize)> {
    // Read file content
    let content = std::fs::read_to_string(&file.absolute_path)
        .map_err(crate::error::RagmcpError::Io)?;
    
    // Compute file hash
    let file_hash = compute_file_hash(&file.absolute_path)?;
    
    // Extract metadata from normalized relative path
    let relative_path = normalize_doc_path(&file.relative_path);
    let namespace = extract_namespace(&relative_path);
    let agent_name = extract_agent_name(&relative_path);
    
    // Parse document
    let parsed = parser_registry.parse(
        &content,
        &relative_path,
        &file.extension,
    )?;
    
    // Chunk document
    let chunks = chunk_document(&parsed, &config.performance)?;
    
    // Estimate total tokens
    let total_tokens = chunks.iter().map(|c| c.tokens).sum::<usize>();
    
    // Insert document (reuses existing doc_id when slash-variant row exists)
    let doc_id = insert_document(
        db,
        &relative_path,
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
    
    Ok((doc_id, chunk_count, total_tokens))
}
