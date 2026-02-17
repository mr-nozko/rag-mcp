//! BFS graph traversal over entity_relations.

use std::collections::{HashSet, VecDeque};

use crate::db::Db;
use crate::graph::Relation;
use crate::{Result, RagmcpError};

/// Traverse knowledge graph using BFS.
/// Returns all relations discovered within max_depth hops.
pub async fn traverse_graph(
    db: &Db,
    start_entity: &str,
    relation_types: Option<Vec<String>>,
    max_depth: usize,
) -> Result<Vec<Relation>> {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    let mut result = Vec::new();

    queue.push_back((start_entity.to_string(), 0));
    visited.insert(start_entity.to_string());

    while let Some((entity, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        let relation_types_clone = relation_types.clone();
        let entity_clone = entity.clone();
        let relations: Vec<Relation> = if let Some(ref types) = relation_types_clone {
            let placeholders = types.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let query = format!(
                "SELECT relation_id, source_entity, relation_type, target_entity, metadata_json \
                 FROM entity_relations \
                 WHERE source_entity = ? AND relation_type IN ({})",
                placeholders
            );
            let types_clone = types.clone();
            db.with_connection(move |conn| {
                let mut stmt = conn.prepare(&query).map_err(RagmcpError::Database)?;
                let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(entity_clone)];
                for t in &types_clone {
                    params.push(Box::new(t.clone()));
                }
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params), |row| {
                        Ok(Relation {
                            relation_id: row.get(0)?,
                            source_entity: row.get(1)?,
                            relation_type: row.get(2)?,
                            target_entity: row.get(3)?,
                            metadata_json: row.get(4)?,
                        })
                    })
                    .map_err(RagmcpError::Database)?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row.map_err(RagmcpError::Database)?);
                }
                Ok(out)
            })
            .await?
        } else {
            let entity_clone2 = entity.clone();
            db.with_connection(move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT relation_id, source_entity, relation_type, target_entity, metadata_json \
                         FROM entity_relations \
                         WHERE source_entity = ?1",
                    )
                    .map_err(RagmcpError::Database)?;
                let rows = stmt
                    .query_map([&entity_clone2], |row| {
                        Ok(Relation {
                            relation_id: row.get(0)?,
                            source_entity: row.get(1)?,
                            relation_type: row.get(2)?,
                            target_entity: row.get(3)?,
                            metadata_json: row.get(4)?,
                        })
                    })
                    .map_err(RagmcpError::Database)?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row.map_err(RagmcpError::Database)?);
                }
                Ok(out)
            })
            .await?
        };

        for rel in relations {
            if !visited.contains(&rel.target_entity) {
                visited.insert(rel.target_entity.clone());
                queue.push_back((rel.target_entity.clone(), depth + 1));
                result.push(rel);
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate;
    use std::path::Path;
    use tempfile::TempDir;
    use rusqlite::params;

    async fn setup_test_db_with_relations() -> (Db, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db = Db::new(&db_path);
        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        db.with_connection(move |conn| {
            migrate::run_migrations(conn, &migrations_dir)
        })
        .await
        .unwrap();
        // Insert sample relations: a -> b -> c, a -> d
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO entity_relations (relation_id, source_entity, relation_type, target_entity, metadata_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params!["r1", "agent:a", "routes_to", "agent:b", None::<String>],
            )?;
            conn.execute(
                "INSERT INTO entity_relations (relation_id, source_entity, relation_type, target_entity, metadata_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params!["r2", "agent:b", "routes_to", "agent:c", None::<String>],
            )?;
            conn.execute(
                "INSERT INTO entity_relations (relation_id, source_entity, relation_type, target_entity, metadata_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params!["r3", "agent:a", "routes_to", "agent:d", None::<String>],
            )?;
            Ok::<(), RagmcpError>(())
        })
        .await
        .unwrap();
        (db, temp_dir)
    }

    #[tokio::test]
    async fn test_traverse_single_hop() {
        let (db, _temp) = setup_test_db_with_relations().await;
        let relations = traverse_graph(&db, "agent:a", None, 1).await.unwrap();
        assert_eq!(relations.len(), 2); // a->b, a->d (depth 1 only)
        let targets: Vec<_> = relations.iter().map(|r| r.target_entity.as_str()).collect();
        assert!(targets.contains(&"agent:b"));
        assert!(targets.contains(&"agent:d"));
    }

    #[tokio::test]
    async fn test_traverse_multi_hop() {
        let (db, _temp) = setup_test_db_with_relations().await;
        let relations = traverse_graph(&db, "agent:a", None, 3).await.unwrap();
        assert_eq!(relations.len(), 3); // a->b, a->d, b->c
        let targets: Vec<_> = relations.iter().map(|r| r.target_entity.as_str()).collect();
        assert!(targets.contains(&"agent:b"));
        assert!(targets.contains(&"agent:d"));
        assert!(targets.contains(&"agent:c"));
    }

    #[tokio::test]
    async fn test_traverse_depth_limit() {
        let (db, _temp) = setup_test_db_with_relations().await;
        let relations = traverse_graph(&db, "agent:a", None, 0).await.unwrap();
        assert_eq!(relations.len(), 0); // depth 0: no expansion
    }

    #[tokio::test]
    async fn test_traverse_relation_type_filter() {
        let (db, _temp) = setup_test_db_with_relations().await;
        let relations = traverse_graph(
            &db,
            "agent:a",
            Some(vec!["routes_to".to_string()]),
            2,
        )
        .await
        .unwrap();
        assert!(relations.len() >= 2);
        assert!(relations.iter().all(|r| r.relation_type == "routes_to"));
    }

    #[tokio::test]
    async fn test_traverse_empty_entity() {
        let (db, _temp) = setup_test_db_with_relations().await;
        let relations = traverse_graph(&db, "agent:nonexistent", None, 2).await.unwrap();
        assert!(relations.is_empty());
    }

    #[tokio::test]
    async fn test_traverse_cycle_no_infinite_loop() {
        let (db, _temp) = setup_test_db_with_relations().await;
        // Add cycle: c -> a
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO entity_relations (relation_id, source_entity, relation_type, target_entity, metadata_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params!["r4", "agent:c", "routes_to", "agent:a", None::<String>],
            )?;
            Ok::<(), RagmcpError>(())
        })
        .await
        .unwrap();
        let relations = traverse_graph(&db, "agent:a", None, 5).await.unwrap();
        // Should still be finite: a->b, a->d, b->c (c->a would revisit a, skipped)
        assert!(relations.len() <= 4);
    }
}
