-- Entity relations (knowledge graph)
CREATE TABLE IF NOT EXISTS entity_relations (
    relation_id TEXT PRIMARY KEY,
    source_entity TEXT NOT NULL,
    relation_type TEXT NOT NULL,
    target_entity TEXT NOT NULL,
    metadata_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_relations_source ON entity_relations(source_entity);
CREATE INDEX IF NOT EXISTS idx_relations_type ON entity_relations(relation_type);
CREATE INDEX IF NOT EXISTS idx_relations_target ON entity_relations(target_entity);

-- Query telemetry
CREATE TABLE IF NOT EXISTS query_logs (
    query_id TEXT PRIMARY KEY,
    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    query_text TEXT NOT NULL,
    namespace TEXT,
    retrieval_method TEXT,
    retrieved_chunk_ids TEXT,
    latency_ms INTEGER,
    result_count INTEGER,
    user_feedback INTEGER
);

CREATE INDEX IF NOT EXISTS idx_logs_timestamp ON query_logs(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_logs_method ON query_logs(retrieval_method);
