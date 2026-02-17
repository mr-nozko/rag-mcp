-- Documents table
CREATE TABLE IF NOT EXISTS documents (
    doc_id TEXT PRIMARY KEY,
    doc_path TEXT NOT NULL UNIQUE,
    doc_type TEXT NOT NULL,
    namespace TEXT NOT NULL,
    agent_name TEXT,
    content_text TEXT NOT NULL,
    content_tokens INTEGER NOT NULL,
    last_modified TIMESTAMP NOT NULL,
    file_hash TEXT NOT NULL,
    metadata_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_documents_namespace ON documents(namespace);
CREATE INDEX IF NOT EXISTS idx_documents_agent ON documents(agent_name);
CREATE INDEX IF NOT EXISTS idx_documents_type ON documents(doc_type);
CREATE INDEX IF NOT EXISTS idx_documents_path ON documents(doc_path);

-- Chunks table
CREATE TABLE IF NOT EXISTS chunks (
    chunk_id TEXT PRIMARY KEY,
    doc_id TEXT NOT NULL,
    chunk_index INTEGER NOT NULL,
    chunk_text TEXT NOT NULL,
    chunk_tokens INTEGER NOT NULL,
    section_header TEXT,
    chunk_type TEXT,
    embedding BLOB,
    FOREIGN KEY(doc_id) REFERENCES documents(doc_id) ON DELETE CASCADE,
    UNIQUE(doc_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_chunks_doc ON chunks(doc_id);
CREATE INDEX IF NOT EXISTS idx_chunks_type ON chunks(chunk_type);
CREATE INDEX IF NOT EXISTS idx_chunks_embedding ON chunks(doc_id) WHERE embedding IS NOT NULL;
