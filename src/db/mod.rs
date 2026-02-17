use rusqlite::Connection;
use std::path::Path;
use tokio::task;
use crate::error::{Result, RagmcpError};

/// Database connection wrapper
pub struct Db {
    path: std::path::PathBuf,
}

impl Db {
    /// Create a new database connection manager
    pub fn new<P: AsRef<Path>>(db_path: P) -> Self {
        Self {
            path: db_path.as_ref().to_path_buf(),
        }
    }
    
    /// Open a new database connection with optimized pragmas
    pub fn open_connection(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path)
            .map_err(RagmcpError::Database)?;
        
        // Set SQLite pragmas for performance
        // WAL mode for better concurrency, NORMAL sync for speed, foreign keys for integrity
        // temp_store = MEMORY for faster temp operations
        // cache_size = -65536 (64MB cache) for better read performance
        // mmap_size = 268435456 (256MB) for memory-mapped I/O on large databases
        // wal_autocheckpoint = 1000 (default) to control WAL file growth
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; \
             PRAGMA synchronous = NORMAL; \
             PRAGMA foreign_keys = ON; \
             PRAGMA temp_store = MEMORY; \
             PRAGMA cache_size = -65536; \
             PRAGMA mmap_size = 268435456; \
             PRAGMA wal_autocheckpoint = 1000;"
        )?;
        
        Ok(conn)
    }
    
    /// Execute a closure with a database connection in a blocking task
    pub async fn with_connection<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let path = self.path.clone();
        task::spawn_blocking(move || {
            let mut conn = Connection::open(&path)
                .map_err(RagmcpError::Database)?;
            
            // Set pragmas for performance (same as open_connection)
            conn.execute_batch(
                "PRAGMA journal_mode = WAL; \
                 PRAGMA synchronous = NORMAL; \
                 PRAGMA foreign_keys = ON; \
                 PRAGMA temp_store = MEMORY; \
                 PRAGMA cache_size = -65536; \
                 PRAGMA mmap_size = 268435456; \
                 PRAGMA wal_autocheckpoint = 1000;"
            )?;
            
            f(&mut conn)
        })
        .await
        .map_err(|_e| {
            RagmcpError::Database(rusqlite::Error::InvalidParameterCount(0, 0))
        })?
    }
}

pub mod migrate;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_db_connection() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db = Db::new(&db_path);
        
        let result = db.with_connection(|conn| {
            conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)", [])
                .map_err(RagmcpError::Database)?;
            Ok(())
        }).await;
        
        assert!(result.is_ok());
        assert!(db_path.exists());
    }
    
    #[tokio::test]
    async fn test_pragmas_set() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db = Db::new(&db_path);
        
        db.with_connection(|conn| {
            let journal_mode: String = conn.query_row(
                "PRAGMA journal_mode",
                [],
                |row| row.get(0)
            )?;
            assert_eq!(journal_mode.to_uppercase(), "WAL");
            
            let foreign_keys: i32 = conn.query_row(
                "PRAGMA foreign_keys",
                [],
                |row| row.get(0)
            )?;
            assert_eq!(foreign_keys, 1);
            
            Ok::<(), RagmcpError>(())
        }).await.unwrap();
    }
}
