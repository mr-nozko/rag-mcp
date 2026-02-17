-- Index on file_hash for faster incremental ingestion (Module 10)
-- get_existing_hashes() and change detection benefit from this index
CREATE INDEX IF NOT EXISTS idx_documents_hash
ON documents(file_hash);
