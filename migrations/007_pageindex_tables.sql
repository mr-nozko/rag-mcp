-- Migration 007: PageIndex Index and Telemetry Tables

-- PageIndex document index tracking
CREATE TABLE pageindex_index (
    doc_id TEXT PRIMARY KEY,
    doc_path TEXT NOT NULL,
    tree_path TEXT NOT NULL,           -- path to JSON ToC file
    tree_built_at TIMESTAMP NOT NULL,
    tree_node_count INT NOT NULL,
    model_used TEXT NOT NULL,
    FOREIGN KEY(doc_id) REFERENCES documents(doc_id) ON DELETE CASCADE
);

-- PageIndex query telemetry (separate from chunk-based query_logs)
CREATE TABLE pageindex_query_logs (
    query_id TEXT PRIMARY KEY,
    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    query_text TEXT NOT NULL,
    doc_path TEXT,
    iterations INT,
    retrieved_node_ids TEXT,           -- JSON array of node_ids
    latency_ms INT,
    model_used TEXT,
    user_feedback INT                  -- -1 | 0 | 1
);
