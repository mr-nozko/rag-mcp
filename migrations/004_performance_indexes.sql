-- Performance optimization indexes for Module 8
-- These covering indexes improve query performance for common access patterns

-- Covering index for chunks with embeddings (used in vector search)
-- Includes chunk_type and section_header for filtering, only indexes rows with embeddings
CREATE INDEX IF NOT EXISTS idx_chunks_embedding_filter 
ON chunks(chunk_type, section_header) 
WHERE embedding IS NOT NULL;

-- Covering index for document filtering by namespace, agent, and type
-- Used in BM25 search and document listing operations
CREATE INDEX IF NOT EXISTS idx_documents_namespace_agent_type 
ON documents(namespace, agent_name, doc_type);

-- Composite index for query_logs analysis (timestamp + method)
-- Used by stats CLI for performance analysis queries
CREATE INDEX IF NOT EXISTS idx_logs_timestamp_method 
ON query_logs(timestamp DESC, retrieval_method);
