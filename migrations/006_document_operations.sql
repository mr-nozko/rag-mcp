-- Document write operations audit log (Module 14)
CREATE TABLE IF NOT EXISTS document_operations (
    operation_id TEXT PRIMARY KEY,
    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    operation_type TEXT NOT NULL,  -- 'create' | 'update'
    doc_path TEXT NOT NULL,
    doc_id TEXT,
    user_context TEXT,            -- MCP session info (optional)
    success BOOLEAN NOT NULL,
    error_message TEXT,
    metadata_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_operations_timestamp
ON document_operations(timestamp DESC);

CREATE INDEX IF NOT EXISTS idx_operations_doc_path
ON document_operations(doc_path);

CREATE INDEX IF NOT EXISTS idx_operations_type
ON document_operations(operation_type);
