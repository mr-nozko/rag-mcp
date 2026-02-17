//! Document write operations audit logging (Module 14).

use crate::db::Db;
use crate::error::Result;
use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;

/// Log a document write operation to the audit table.
///
/// Returns the generated operation_id (UUID).
pub async fn log_operation(
    db: &Db,
    operation_type: &str,
    doc_path: &str,
    doc_id: Option<&str>,
    success: bool,
    error_message: Option<&str>,
    metadata_json: Option<&str>,
) -> Result<String> {
    let operation_id = Uuid::new_v4().to_string();
    let timestamp = Utc::now().to_rfc3339();

    let op_type = operation_type.to_string();
    let path = doc_path.to_string();
    let id = doc_id.map(String::from);
    let err = error_message.map(String::from);
    let meta = metadata_json.map(String::from);
    let op_id = operation_id.clone();

    db.with_connection(move |conn| {
        conn.execute(
            r#"
            INSERT INTO document_operations (
                operation_id, timestamp, operation_type, doc_path,
                doc_id, success, error_message, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![op_id, timestamp, op_type, path, id, success, err, meta],
        )?;
        Ok::<(), crate::error::RagmcpError>(())
    })
    .await?;

    Ok(operation_id)
}
