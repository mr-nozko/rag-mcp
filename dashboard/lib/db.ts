import Database from 'better-sqlite3';
import path from 'path';

// Singleton read-only connection to the RAGMcp SQLite database.
let db: Database.Database | null = null;

export function getDatabase(): Database.Database {
  if (!db) {
    // Resolve DB path: prefer DB_PATH env var, fall back to ../ragmcp.db
    // (the Rust server writes the DB one level above the dashboard directory).
    const dbPath = process.env.DB_PATH
      ? path.resolve(process.env.DB_PATH)
      : path.join(process.cwd(), '..', 'ragmcp.db');

    db = new Database(dbPath, {
      readonly: true,    // Never write from the dashboard — MCP server owns the DB
      fileMustExist: true // Fail fast with a clear error if DB has not been initialized yet
    });

    // Only log in development — avoid leaking filesystem paths in production logs
    if (process.env.NODE_ENV !== 'production') {
      console.log(`[DB] Connected to ${dbPath} (read-only)`);
    }
  }
  return db;
}

export function closeDatabase() {
  if (db) {
    db.close();
    db = null;
  }
}
