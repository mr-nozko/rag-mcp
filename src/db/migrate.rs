use rusqlite::{Connection, params};
use std::fs;
use std::path::Path;
use crate::error::{Result, RagmcpError};

/// Migration metadata
struct Migration {
    version: u32,
    name: String,
    sql: String,
}

/// Create schema_migrations table if it doesn't exist
fn ensure_migrations_table(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;
    Ok(())
}

/// Get list of applied migrations
pub fn get_applied_migrations(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM schema_migrations ORDER BY version")?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()
        .map_err(RagmcpError::Database)?;
    Ok(names)
}

/// Load migration files from migrations directory
fn load_migrations(migrations_dir: &Path) -> Result<Vec<Migration>> {
    let mut migrations = Vec::new();
    
    let entries = fs::read_dir(migrations_dir)
        .map_err(RagmcpError::Io)?;
    
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("sql"))
        .collect();
    
    // Sort by filename
    files.sort_by_key(|e| e.file_name());
    
    for entry in files {
        let path = entry.path();
        let filename = path.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| RagmcpError::Config("Invalid migration filename".to_string()))?;
        
        // Parse version from filename (e.g., "001_core_tables.sql" -> 1)
        let version_str = filename
            .split('_')
            .next()
            .ok_or_else(|| RagmcpError::Config(format!("Invalid migration filename: {}", filename)))?;
        let version: u32 = version_str.parse()
            .map_err(|_| RagmcpError::Config(format!("Invalid migration version: {}", version_str)))?;
        
        let sql = fs::read_to_string(&path)
            .map_err(RagmcpError::Io)?;
        
        let name = filename.trim_end_matches(".sql").to_string();
        
        migrations.push(Migration { version, name, sql });
    }
    
    // Sort by version
    migrations.sort_by_key(|m| m.version);
    
    Ok(migrations)
}

/// Run all pending migrations
pub fn run_migrations(conn: &mut Connection, migrations_dir: &Path) -> Result<()> {
    ensure_migrations_table(conn)?;
    
    let applied = get_applied_migrations(conn)?;
    let migrations = load_migrations(migrations_dir)?;
    
    for migration in migrations {
        if applied.contains(&migration.name) {
            log::debug!("Migration {} already applied, skipping", migration.name);
            continue;
        }
        
        log::info!("Applying migration: {} (version {})", migration.name, migration.version);
        
        // Execute migration in a transaction
        let tx = conn.transaction()?;
        
        // Use execute_batch which properly handles multiple SQL statements
        // including those with semicolons in trigger definitions
        tx.execute_batch(&migration.sql)
            .map_err(|e| {
                RagmcpError::Database(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                    Some(format!("Failed to execute migration {}: {}", migration.name, e))
                ))
            })?;
        
        // Record migration as applied
        tx.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            params![migration.version, migration.name],
        )?;
        
        tx.commit()?;
        
        log::info!("Migration {} applied successfully", migration.name);
    }
    
    log::info!("All migrations completed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;
    
    #[test]
    fn test_migration_tracking() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let conn = Connection::open(&db_path).unwrap();
        
        ensure_migrations_table(&conn).unwrap();
        
        // Apply a test migration
        conn.execute("CREATE TABLE test (id INTEGER)", []).unwrap();
        conn.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            params![1, "001_test"],
        ).unwrap();
        
        let applied = get_applied_migrations(&conn).unwrap();
        assert!(applied.contains(&"001_test".to_string()));
    }
    
    #[test]
    fn test_load_migrations() {
        let temp_dir = TempDir::new().unwrap();
        let migrations_dir = temp_dir.path().join("migrations");
        fs::create_dir(&migrations_dir).unwrap();
        
        fs::write(
            migrations_dir.join("001_test.sql"),
            "CREATE TABLE test (id INTEGER);"
        ).unwrap();
        
        fs::write(
            migrations_dir.join("002_another.sql"),
            "CREATE TABLE another (id INTEGER);"
        ).unwrap();
        
        let migrations = load_migrations(&migrations_dir).unwrap();
        assert_eq!(migrations.len(), 2);
        assert_eq!(migrations[0].version, 1);
        assert_eq!(migrations[1].version, 2);
    }
    
    #[test]
    fn test_full_migration_schema() {
        // Test that migrations create all expected tables
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let mut conn = Connection::open(&db_path).unwrap();
        
        // Use actual migrations directory if it exists, otherwise create test migrations
        let migrations_dir = Path::new("migrations");
        if migrations_dir.exists() {
            run_migrations(&mut conn, migrations_dir).unwrap();
            
            // Verify all tables exist
            let tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()
                .unwrap();
            
            assert!(tables.contains(&"chunks".to_string()));
            assert!(tables.contains(&"documents".to_string()));
            assert!(tables.contains(&"entity_relations".to_string()));
            assert!(tables.contains(&"query_logs".to_string()));
            assert!(tables.contains(&"schema_migrations".to_string()));
            
            // Verify FTS5 virtual table exists
            let fts_tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='chunks_fts'")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()
                .unwrap();
            
            assert!(fts_tables.contains(&"chunks_fts".to_string()));
            
            // Verify triggers exist
            let triggers: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='trigger'")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()
                .unwrap();
            
            assert!(triggers.iter().any(|t| t.contains("chunks_fts_insert")));
            assert!(triggers.iter().any(|t| t.contains("chunks_fts_delete")));
            assert!(triggers.iter().any(|t| t.contains("chunks_fts_update")));
            
            // Verify performance indexes from migration 004 exist
            let indexes: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%' ORDER BY name")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()
                .unwrap();
            
            assert!(indexes.contains(&"idx_chunks_embedding_filter".to_string()), 
                "Performance index idx_chunks_embedding_filter should exist");
            assert!(indexes.contains(&"idx_documents_namespace_agent_type".to_string()),
                "Performance index idx_documents_namespace_agent_type should exist");
            assert!(indexes.contains(&"idx_logs_timestamp_method".to_string()),
                "Performance index idx_logs_timestamp_method should exist");
        }
    }
}
