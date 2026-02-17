-- FTS5 virtual table for full-text search
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
    chunk_id UNINDEXED,
    chunk_text,
    section_header,
    tokenize = 'porter unicode61'
);

-- Triggers to keep FTS5 in sync with chunks table
CREATE TRIGGER IF NOT EXISTS chunks_fts_insert AFTER INSERT ON chunks BEGIN
    INSERT INTO chunks_fts(chunk_id, chunk_text, section_header)
    VALUES (new.chunk_id, new.chunk_text, new.section_header);
END;

CREATE TRIGGER IF NOT EXISTS chunks_fts_delete AFTER DELETE ON chunks BEGIN
    DELETE FROM chunks_fts WHERE chunk_id = old.chunk_id;
END;

CREATE TRIGGER IF NOT EXISTS chunks_fts_update AFTER UPDATE ON chunks BEGIN
    UPDATE chunks_fts 
    SET chunk_text = new.chunk_text,
        section_header = new.section_header
    WHERE chunk_id = new.chunk_id;
END;
