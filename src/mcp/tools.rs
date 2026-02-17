use crate::config::Config;
use crate::db::Db;
use crate::embeddings::OpenAIEmbedder;
use crate::error::{Result, RagmcpError};
use crate::mcp::types::{ContentItem, Tool, ToolsCallResult};
use crate::mcp::roots::PathValidator;
use crate::mcp::audit::log_operation;
use crate::cache::ChunkEmbeddingCache;
use crate::search::hybrid::search_hybrid;
use crate::graph::traverse_graph;
use crate::ingest::metadata::{compute_file_hash, extract_agent_name, extract_namespace};
use crate::ingest::parsers::ParserRegistry;
use crate::ingest::chunker::chunk_document;
use crate::ingest::db_writer::{insert_chunks, insert_document};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

/// Get all tool definitions for tools/list
pub fn get_tool_definitions() -> Vec<Tool> {
    vec![
        Tool {
            name: "ragmcp_search".to_string(),
            description: "Hybrid search across documentation using BM25 and vector similarity".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query text",
                        "minLength": 3
                    },
                    "k": {
                        "type": "integer",
                        "description": "Number of results to return",
                        "default": 5,
                        "minimum": 1,
                        "maximum": 20
                    },
                    "overfetch": {
                        "type": "integer",
                        "description": "Optional number of raw fused results to retrieve before applying score thresholds (advanced). When set, the search will fetch up to this many top-ranked results and disable score-based filtering.",
                        "minimum": 1,
                        "maximum": 100
                    },
                    "namespace": {
                        "type": "string",
                        "description": "Filter by namespace (derived from top-level directory name, e.g. 'guides', 'api', 'research'). Use 'all' to include all namespaces. Call ragmcp_list with list_type=namespaces to see available namespaces.",
                        "default": "all"
                    },
                    "agent_filter": {
                        "type": "string",
                        "description": "Filter by entity/agent name (second-level directory, e.g. 'my-module', 'api-v2'). Use ragmcp_list with list_type=agents to see available values."
                    },
                    "min_score": {
                        "type": "number",
                        "description": "Minimum relevance score (0-1)",
                        "default": 0.65,
                        "minimum": 0,
                        "maximum": 1
                    }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "ragmcp_get".to_string(),
            description: "Retrieve a specific document by path".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "doc_path": {
                        "type": "string",
                        "description": "Document path relative to rag_folder root (as shown in search results)"
                    },
                    "return_full_doc": {
                        "type": "boolean",
                        "default": false,
                        "description": "Return full document or just metadata"
                    },
                    "sections": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Specific sections to retrieve"
                    }
                },
                "required": ["doc_path"]
            }),
        },
        Tool {
            name: "ragmcp_list".to_string(),
            description: "List documentation structure: agents (second-level entities), namespaces (top-level dirs), or doc types".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "list_type": {
                        "type": "string",
                        "description": "Type of list to return",
                        "enum": ["agents", "system_docs", "namespaces", "doc_types"],
                    },
                    "agent_name": {
                        "type": "string",
                        "description": "Filter by entity/agent name (for system_docs list type)"
                    },
                    "include_metadata": {
                        "type": "boolean",
                        "default": true,
                        "description": "Include metadata in results"
                    }
                },
                "required": ["list_type"]
            }),
        },
        Tool {
            name: "ragmcp_related".to_string(),
            description: "Find related entities via knowledge graph. Relations are extracted from document content during ingestion.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "entity": {
                        "type": "string",
                        "description": "Entity identifier"
                    },
                    "relation_types": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Types of relations to traverse"
                    },
                    "max_depth": {
                        "type": "integer",
                        "default": 1,
                        "minimum": 1,
                        "maximum": 3,
                        "description": "Maximum traversal depth"
                    }
                },
                "required": ["entity"]
            }),
        },
        Tool {
            name: "ragmcp_explain".to_string(),
            description: "Get meta-information about RAGMcp index (stats, doc info, freshness)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "explain_what": {
                        "type": "string",
                        "description": "What to explain",
                        "enum": ["index_stats", "doc_info", "freshness"]
                    },
                    "doc_path": {
                        "type": "string",
                        "description": "Document path (required for doc_info)"
                    }
                },
                "required": ["explain_what"]
            }),
        },
        Tool {
            name: "ragmcp_create_doc".to_string(),
            description: "Create a new document with automatic parsing, chunking, and embedding. Creates parent directories if needed.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "doc_path": {
                        "type": "string",
                        "description": "Relative path from docs root (e.g. \"Namespace/subfolder/doc.md\")"
                    },
                    "content": {
                        "type": "string",
                        "description": "Full document content to write"
                    },
                    "doc_type": {
                        "type": "string",
                        "description": "Document type (optional, auto-detected if omitted). Can be any string value."
                    }
                },
                "required": ["doc_path", "content"]
            }),
        },
        Tool {
            name: "ragmcp_update_doc".to_string(),
            description: "Update an existing document, re-parsing and re-embedding automatically. Creates parent directories if needed.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "doc_path": {
                        "type": "string",
                        "description": "Relative path of document to update"
                    },
                    "content": {
                        "type": "string",
                        "description": "New content (full replacement)"
                    }
                },
                "required": ["doc_path", "content"]
            }),
        },
    ]
}

/// Search parameters
#[derive(Debug, Deserialize)]
struct SearchParams {
    query: String,
    #[serde(default = "default_k")]
    k: usize,
    /// Optional overfetch count. When set, the search will retrieve up to this many
    /// fused results before any score thresholding is applied. This is useful for
    /// advanced RAG flows that want access to a larger candidate set.
    #[serde(default)]
    overfetch: Option<usize>,
    #[serde(default = "default_namespace")]
    namespace: String,
    agent_filter: Option<String>,
    #[serde(default = "default_min_score")]
    min_score: f32,
}

fn default_k() -> usize { 5 }
fn default_namespace() -> String { "all".to_string() }
fn default_min_score() -> f32 { 0.65 }

/// Handle ragmcp_search tool
pub async fn handle_search(
    db: &Db,
    embedder: &OpenAIEmbedder,
    config: &Config,
    arguments: &Value,
    chunk_cache: Option<Arc<ChunkEmbeddingCache>>,
) -> Result<ToolsCallResult> {
    let start = std::time::Instant::now();
    
    // Parse parameters
    let params: SearchParams = serde_json::from_value(arguments.clone())
        .map_err(|e| RagmcpError::Config(format!("Invalid search params: {}", e)))?;

    if params.query.trim().len() < 3 {
        return Ok(ToolsCallResult {
            content: vec![ContentItem {
                content_type: "text".to_string(),
                text: "Error: Query must be at least 3 characters".to_string(),
            }],
            is_error: Some(true),
        });
    }

    // Convert namespace="all" to None (search all namespaces)
    let namespace_filter = if params.namespace == "all" {
        None
    } else {
        Some(params.namespace.as_str())
    };

    // Determine effective search parameters.
    // If overfetch is provided, we:
    // - Use overfetch as the internal k for search_hybrid (how many fused results to retrieve)
    // - Disable score-based filtering inside search_hybrid by setting min_score = 0.0
    //   (this gives the caller access to the raw fused candidate set).
    let effective_k = params.overfetch.unwrap_or(params.k);
    let effective_min_score = if params.overfetch.is_some() {
        0.0
    } else {
        params.min_score
    };
    
    let agent_filter = params.agent_filter.as_deref();

    // Execute hybrid search (namespace and agent filter applied in vector SQL)
    let results = match search_hybrid(
        db,
        embedder,
        &params.query,
        namespace_filter,
        agent_filter,
        effective_k,
        effective_min_score,
        config.search.hybrid_bm25_weight,
        config.search.hybrid_vector_weight,
        chunk_cache,
    )
    .await
    {
        Ok(results) => results,
        Err(e) => {
            return Err(e);
        }
    };

    let latency_ms = start.elapsed().as_millis() as i64;

    // Log query to database
    log_query(db, &params.query, "hybrid", &results, latency_ms).await?;

    // Format results
    let mut result_text = format!(
        "Found {} results for query: \"{}\"\n\n",
        results.len(),
        params.query
    );

    for (idx, result) in results.iter().enumerate() {
        result_text.push_str(&format!(
            "{}. [{}] {} (score: {:.3})\n",
            idx + 1,
            result.doc_type,
            result.doc_path,
            result.score
        ));
        if let Some(section) = &result.section {
            result_text.push_str(&format!("   Section: {}\n", section));
        }
        if let Some(agent) = &result.agent_name {
            result_text.push_str(&format!("   Agent: {}\n", agent));
        }
        // Truncate chunk text for display (must not split multi-byte UTF-8 chars)
        let preview_len = 200.min(result.chunk_text.len());
        let safe_end = (0..=preview_len)
            .rev()
            .find(|&i| result.chunk_text.is_char_boundary(i))
            .unwrap_or(0);
        result_text.push_str(&format!(
            "   Content: {}\n\n",
            &result.chunk_text[..safe_end]
        ));
    }

    result_text.push_str(&format!("Latency: {}ms\n", latency_ms));

    Ok(ToolsCallResult {
        content: vec![ContentItem {
            content_type: "text".to_string(),
            text: result_text,
        }],
        is_error: None,
    })
}

/// Get parameters
#[derive(Debug, Deserialize)]
struct GetParams {
    doc_path: String,
    #[serde(default)]
    return_full_doc: bool,
    sections: Option<Vec<String>>,
}

/// Handle ragmcp_get tool
pub async fn handle_get(
    db: &Db,
    arguments: &Value,
) -> Result<ToolsCallResult> {
    let params: GetParams = serde_json::from_value(arguments.clone())
        .map_err(|e| RagmcpError::Config(format!("Invalid get params: {}", e)))?;

    // Normalize for comparison: use forward slash so we match DB regardless of stored separator (Windows \ vs /)
    let doc_path_param = params.doc_path.replace('\\', "/").trim_matches('/').to_string();
    let doc = db.with_connection(move |conn| {
        let mut stmt = conn.prepare(
            r#"
            SELECT 
                doc_id, doc_path, doc_type, namespace, agent_name,
                content_text, content_tokens, last_modified, file_hash, metadata_json
            FROM documents
            WHERE REPLACE(doc_path, '\', '/') = ?
            "#
        )?;

        let row = stmt.query_row(params![doc_path_param.clone()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        }).map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => RagmcpError::DocumentNotFound(doc_path_param),
            other => RagmcpError::Database(other),
        })?;

        Ok::<_, RagmcpError>(row)
    }).await?;

    let (_doc_id, doc_path, doc_type, namespace, agent_name, content_text, content_tokens, last_modified, file_hash, _metadata_json) = doc;

    let mut result_text = format!("Document: {}\n", doc_path);
    result_text.push_str(&format!("Type: {}\n", doc_type));
    result_text.push_str(&format!("Namespace: {}\n", namespace));
    if let Some(agent) = agent_name {
        result_text.push_str(&format!("Agent: {}\n", agent));
    }
    result_text.push_str(&format!("Tokens: {}\n", content_tokens));
    result_text.push_str(&format!("Last Modified: {}\n", last_modified));
    result_text.push_str(&format!("Hash: {}\n\n", file_hash));

    if params.return_full_doc {
        result_text.push_str("Full Content:\n");
        result_text.push_str(&content_text);
    } else {
        result_text.push_str("(Use return_full_doc=true to see full content)\n");
    }

    Ok(ToolsCallResult {
        content: vec![ContentItem {
            content_type: "text".to_string(),
            text: result_text,
        }],
        is_error: None,
    })
}

/// List parameters
#[derive(Debug, Deserialize)]
struct ListParams {
    list_type: String,
    agent_name: Option<String>,
    #[serde(default = "default_true")]
    include_metadata: bool,
}

fn default_true() -> bool { true }

/// Handle ragmcp_list tool
pub async fn handle_list(
    db: &Db,
    arguments: &Value,
) -> Result<ToolsCallResult> {
    let params: ListParams = serde_json::from_value(arguments.clone())
        .map_err(|e| RagmcpError::Config(format!("Invalid list params: {}", e)))?;

    let result_text = match params.list_type.as_str() {
        "agents" => {
            let agents = db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT agent_name FROM documents WHERE agent_name IS NOT NULL ORDER BY agent_name"
                )?;
                let agents: Vec<String> = stmt.query_map([], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;
                Ok::<_, RagmcpError>(agents)
            }).await?;

            let mut text = format!("Found {} agents:\n\n", agents.len());
            for agent in agents {
                text.push_str(&format!("- {}\n", agent));
            }
            text
        }
        "system_docs" => {
            let agent_name_clone = params.agent_name.clone();
            let docs = db.with_connection(move |conn| {
                let mut query = "SELECT doc_path, doc_type, agent_name FROM documents WHERE namespace = 'system'".to_string();
                let mut params_vec: Vec<&dyn rusqlite::ToSql> = vec![];

                if let Some(agent) = &agent_name_clone {
                    query.push_str(" AND agent_name = ?");
                    params_vec.push(agent);
                }
                query.push_str(" ORDER BY doc_path");

                let mut stmt = conn.prepare(&query)?;
                let docs: Vec<(String, String, Option<String>)> = stmt.query_map(
                    rusqlite::params_from_iter(params_vec.iter()),
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    }
                )?
                .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;
                Ok::<_, RagmcpError>(docs)
            }).await?;

            let mut text = format!("Found {} system documents:\n\n", docs.len());
            for (path, doc_type, agent) in docs {
                text.push_str(&format!("- {} ({})", path, doc_type));
                if let Some(agent) = agent {
                    text.push_str(&format!(" [Agent: {}]", agent));
                }
                text.push('\n');
            }
            text
        }
        "namespaces" => {
            let namespaces = db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT namespace FROM documents ORDER BY namespace"
                )?;
                let namespaces: Vec<String> = stmt.query_map([], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;
                Ok::<_, RagmcpError>(namespaces)
            }).await?;

            let mut text = format!("Found {} namespaces:\n\n", namespaces.len());
            for ns in namespaces {
                text.push_str(&format!("- {}\n", ns));
            }
            text
        }
        "doc_types" => {
            let doc_types = db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT doc_type FROM documents ORDER BY doc_type"
                )?;
                let doc_types: Vec<String> = stmt.query_map([], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;
                Ok::<_, RagmcpError>(doc_types)
            }).await?;

            let mut text = format!("Found {} document types:\n\n", doc_types.len());
            for dt in doc_types {
                text.push_str(&format!("- {}\n", dt));
            }
            text
        }
        _ => {
            return Ok(ToolsCallResult {
                content: vec![ContentItem {
                    content_type: "text".to_string(),
                    text: format!("Error: Unknown list_type: {}", params.list_type),
                }],
                is_error: Some(true),
            });
        }
    };

    Ok(ToolsCallResult {
        content: vec![ContentItem {
            content_type: "text".to_string(),
            text: result_text,
        }],
        is_error: None,
    })
}

/// Related parameters
#[derive(Debug, Deserialize)]
struct RelatedParams {
    entity: String,
    relation_types: Option<Vec<String>>,
    #[serde(default = "default_max_depth")]
    max_depth: usize,
}

fn default_max_depth() -> usize { 1 }

/// Handle ragmcp_related tool (knowledge graph traversal)
pub async fn handle_related(
    db: &Db,
    arguments: &Value,
) -> Result<ToolsCallResult> {
    let params: RelatedParams = serde_json::from_value(arguments.clone())
        .map_err(|e| RagmcpError::Config(format!("Invalid related params: {}", e)))?;

    let relations = traverse_graph(
        db,
        &params.entity,
        params.relation_types.clone(),
        params.max_depth,
    )
    .await?;

    let result_json = json!({
        "entity": params.entity,
        "max_depth": params.max_depth,
        "relation_count": relations.len(),
        "relations": relations.iter().map(|r| {
            let metadata_parsed = r.metadata_json.as_ref().and_then(|s| serde_json::from_str::<Value>(s).ok());
            json!({
                "relation_id": r.relation_id,
                "source": r.source_entity,
                "type": r.relation_type,
                "target": r.target_entity,
                "metadata": metadata_parsed,
            })
        }).collect::<Vec<_>>(),
    });

    let text = serde_json::to_string_pretty(&result_json)
        .map_err(|e| RagmcpError::Config(format!("JSON serialization failed: {}", e)))?;

    Ok(ToolsCallResult {
        content: vec![ContentItem {
            content_type: "text".to_string(),
            text,
        }],
        is_error: None,
    })
}

/// Explain parameters
#[derive(Debug, Deserialize)]
struct ExplainParams {
    explain_what: String,
    doc_path: Option<String>,
}

/// Handle ragmcp_explain tool
pub async fn handle_explain(
    db: &Db,
    arguments: &Value,
) -> Result<ToolsCallResult> {
    let params: ExplainParams = serde_json::from_value(arguments.clone())
        .map_err(|e| RagmcpError::Config(format!("Invalid explain params: {}", e)))?;

    let result_text = match params.explain_what.as_str() {
        "index_stats" => {
            let stats = db.with_connection(|conn| {
                let doc_count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM documents",
                    [],
                    |row| row.get(0)
                )?;
                let chunk_count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM chunks",
                    [],
                    |row| row.get(0)
                )?;
                let embedded_count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM chunks WHERE embedding IS NOT NULL",
                    [],
                    |row| row.get(0)
                )?;
                let last_update: Option<String> = conn.query_row(
                    "SELECT MAX(last_modified) FROM documents",
                    [],
                    |row| row.get(0)
                ).ok().flatten();
                Ok::<_, RagmcpError>((doc_count, chunk_count, embedded_count, last_update))
            }).await?;

            let (doc_count, chunk_count, embedded_count, last_update) = stats;
            format!(
                "Index Statistics:\n\n\
                Total Documents: {}\n\
                Total Chunks: {}\n\
                Chunks with Embeddings: {}\n\
                Embedding Coverage: {:.1}%\n\
                Last Update: {}\n",
                doc_count,
                chunk_count,
                embedded_count,
                if chunk_count > 0 {
                    (embedded_count as f64 / chunk_count as f64) * 100.0
                } else {
                    0.0
                },
                last_update.unwrap_or_else(|| "Unknown".to_string())
            )
        }
        "doc_info" => {
            let doc_path = params.doc_path.ok_or_else(|| {
                RagmcpError::Config("doc_path required for doc_info".to_string())
            })?;

            // Normalize path separators: convert forward slashes to backslashes to match database storage
            let doc_path = doc_path.replace('/', "\\");
            let doc_path_clone = doc_path.clone();
            let info = db.with_connection(move |conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT 
                        doc_path, doc_type, namespace, agent_name,
                        content_tokens, last_modified, file_hash
                    FROM documents
                    WHERE doc_path = ?
                    "#
                )?;

                let row = stmt.query_row(params![doc_path_clone], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })?;
                Ok::<_, RagmcpError>(row)
            }).await?;

            let (doc_path, doc_type, namespace, agent_name, content_tokens, last_modified, file_hash) = info;

            let mut text = format!("Document Information:\n\n");
            text.push_str(&format!("Path: {}\n", doc_path));
            text.push_str(&format!("Type: {}\n", doc_type));
            text.push_str(&format!("Namespace: {}\n", namespace));
            if let Some(agent) = agent_name {
                text.push_str(&format!("Agent: {}\n", agent));
            }
            text.push_str(&format!("Tokens: {}\n", content_tokens));
            text.push_str(&format!("Last Modified: {}\n", last_modified));
            text.push_str(&format!("Hash: {}\n", file_hash));

            // Get chunk count
            let doc_path_for_chunks = doc_path.clone();
            let chunk_count = db.with_connection(move |conn| {
                let doc_id: String = conn.query_row(
                    "SELECT doc_id FROM documents WHERE doc_path = ?",
                    params![doc_path_for_chunks],
                    |row| row.get(0)
                )?;
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM chunks WHERE doc_id = ?",
                    params![doc_id],
                    |row| row.get(0)
                )?;
                Ok::<_, RagmcpError>(count)
            }).await?;

            text.push_str(&format!("Chunks: {}\n", chunk_count));
            text
        }
        "freshness" => {
            let stale_docs = db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT doc_path, last_modified
                    FROM documents
                    WHERE datetime(last_modified) < datetime('now', '-7 days')
                    ORDER BY last_modified ASC
                    LIMIT 20
                    "#
                )?;
                let docs: Vec<(String, String)> = stmt.query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;
                Ok::<_, RagmcpError>(docs)
            }).await?;

            let mut text = format!("Stale Documents (>7 days old):\n\n");
            if stale_docs.is_empty() {
                text.push_str("No stale documents found.\n");
            } else {
                text.push_str(&format!("Found {} stale documents:\n\n", stale_docs.len()));
                for (path, modified) in stale_docs {
                    text.push_str(&format!("- {} (last modified: {})\n", path, modified));
                }
            }
            text
        }
        _ => {
            return Ok(ToolsCallResult {
                content: vec![ContentItem {
                    content_type: "text".to_string(),
                    text: format!("Error: Unknown explain_what: {}", params.explain_what),
                }],
                is_error: Some(true),
            });
        }
    };

    Ok(ToolsCallResult {
        content: vec![ContentItem {
            content_type: "text".to_string(),
            text: result_text,
        }],
        is_error: None,
    })
}

/// Params for ragmcp_create_doc
#[derive(Debug, Deserialize)]
struct CreateDocParams {
    doc_path: String,
    content: String,
    doc_type: Option<String>,
}

/// Create a new document: validate path, create dirs, write file, parse, chunk, insert, audit.
pub async fn handle_create_doc(
    db: &Db,
    _embedder: &OpenAIEmbedder,
    config: &Config,
    _cache: Option<Arc<ChunkEmbeddingCache>>,
    arguments: &Value,
) -> Result<ToolsCallResult> {
    let params: CreateDocParams = serde_json::from_value(arguments.clone())
        .map_err(|e| RagmcpError::InvalidInput(format!("Invalid parameters: {}", e)))?;

    let validator = PathValidator::new(&config.ragmcp.rag_folder)?;
    let absolute_path = validator.validate_write_path(&params.doc_path)?;

    if absolute_path.exists() {
        let _ = log_operation(
            db,
            "create",
            &params.doc_path,
            None,
            false,
            Some("File already exists"),
            None,
        )
        .await;
        return Err(RagmcpError::InvalidInput(format!(
            "File already exists: {}",
            params.doc_path
        )));
    }

    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent).map_err(RagmcpError::Io)?;
    }
    fs::write(&absolute_path, &params.content).map_err(RagmcpError::Io)?;

    let file_hash = compute_file_hash(&absolute_path)?;
    let metadata = fs::metadata(&absolute_path).map_err(RagmcpError::Io)?;
    let last_modified = metadata.modified().map_err(RagmcpError::Io)?;

    // Parse and chunk in a block so ParserRegistry (non-Send) is dropped before any await.
    let (doc_type, namespace, agent_name, chunks, total_tokens, content) = {
        let extension = Path::new(&params.doc_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let registry = ParserRegistry::new();
        let parsed = registry
            .parse(&params.content, &params.doc_path, &extension)
            .map_err(|e| RagmcpError::Parse(e.to_string()))?;
        let doc_type = params
            .doc_type
            .unwrap_or_else(|| parsed.doc_type.clone());
        let namespace = extract_namespace(&params.doc_path);
        let agent_name = extract_agent_name(&params.doc_path);
        let chunks = chunk_document(&parsed, &config.performance)?;
        let total_tokens = chunks.iter().map(|c| c.tokens).sum::<usize>();
        (doc_type, namespace, agent_name, chunks, total_tokens, parsed.content)
    };

    let doc_id = insert_document(
        db,
        &params.doc_path,
        &doc_type,
        &namespace,
        agent_name.as_deref(),
        &content,
        total_tokens,
        &file_hash,
        last_modified,
    )
    .await?;

    let chunk_count = insert_chunks(db, &doc_id, chunks).await?;

    let meta_json = json!({
        "doc_type": doc_type,
        "chunk_count": chunk_count,
        "file_hash": file_hash
    })
    .to_string();
    let operation_id = log_operation(
        db,
        "create",
        &params.doc_path,
        Some(&doc_id),
        true,
        None,
        Some(&meta_json),
    )
    .await?;

    let text = json!({
        "success": true,
        "doc_id": doc_id,
        "doc_path": params.doc_path,
        "chunks_created": chunk_count,
        "operation_id": operation_id,
        "message": "Document created successfully"
    })
    .to_string();

    Ok(ToolsCallResult {
        content: vec![ContentItem {
            content_type: "text".to_string(),
            text,
        }],
        is_error: None,
    })
}

/// Params for ragmcp_update_doc
#[derive(Debug, Deserialize)]
struct UpdateDocParams {
    doc_path: String,
    content: String,
}

/// Update an existing document: validate path, create dirs if needed, write file, re-parse, re-chunk, upsert, audit.
pub async fn handle_update_doc(
    db: &Db,
    _embedder: &OpenAIEmbedder,
    config: &Config,
    _cache: Option<Arc<ChunkEmbeddingCache>>,
    arguments: &Value,
) -> Result<ToolsCallResult> {
    let params: UpdateDocParams = serde_json::from_value(arguments.clone())
        .map_err(|e| RagmcpError::InvalidInput(format!("Invalid parameters: {}", e)))?;

    let validator = PathValidator::new(&config.ragmcp.rag_folder)?;
    let absolute_path = validator.validate_write_path(&params.doc_path)?;

    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent).map_err(RagmcpError::Io)?;
    }
    fs::write(&absolute_path, &params.content).map_err(RagmcpError::Io)?;

    let file_hash = compute_file_hash(&absolute_path)?;
    let metadata = fs::metadata(&absolute_path).map_err(RagmcpError::Io)?;
    let last_modified = metadata.modified().map_err(RagmcpError::Io)?;

    let (doc_type, namespace, agent_name, chunks, total_tokens, content) = {
        let extension = Path::new(&params.doc_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let registry = ParserRegistry::new();
        let parsed = registry
            .parse(&params.content, &params.doc_path, &extension)
            .map_err(|e| RagmcpError::Parse(e.to_string()))?;
        let doc_type = parsed.doc_type.clone();
        let namespace = extract_namespace(&params.doc_path);
        let agent_name = extract_agent_name(&params.doc_path);
        let chunks = chunk_document(&parsed, &config.performance)?;
        let total_tokens = chunks.iter().map(|c| c.tokens).sum::<usize>();
        (doc_type, namespace, agent_name, chunks, total_tokens, parsed.content)
    };

    let doc_id = insert_document(
        db,
        &params.doc_path,
        &doc_type,
        &namespace,
        agent_name.as_deref(),
        &content,
        total_tokens,
        &file_hash,
        last_modified,
    )
    .await?;

    let chunk_count = insert_chunks(db, &doc_id, chunks).await?;

    let meta_json = json!({
        "doc_type": doc_type,
        "chunk_count": chunk_count,
        "file_hash": file_hash
    })
    .to_string();
    let operation_id = log_operation(
        db,
        "update",
        &params.doc_path,
        Some(&doc_id),
        true,
        None,
        Some(&meta_json),
    )
    .await?;

    let text = json!({
        "success": true,
        "doc_id": doc_id,
        "doc_path": params.doc_path,
        "chunks_created": chunk_count,
        "operation_id": operation_id,
        "message": "Document updated successfully"
    })
    .to_string();

    Ok(ToolsCallResult {
        content: vec![ContentItem {
            content_type: "text".to_string(),
            text,
        }],
        is_error: None,
    })
}

/// Log query to query_logs table
async fn log_query(
    db: &Db,
    query_text: &str,
    retrieval_method: &str,
    results: &[crate::search::SearchResult],
    latency_ms: i64,
) -> Result<()> {
    let query_id = Uuid::new_v4().to_string();
    let chunk_ids: Vec<String> = results.iter().map(|r| r.chunk_id.clone()).collect();
    let chunk_ids_json = serde_json::to_string(&chunk_ids)
        .map_err(|e| RagmcpError::Config(format!("JSON serialization error: {}", e)))?;

    let query_text = query_text.to_string();
    let retrieval_method = retrieval_method.to_string();
    let query_id_clone = query_id.clone();
    let chunk_ids_json_clone = chunk_ids_json.clone();
    let result_count = results.len() as i64;

    db.with_connection(move |conn| {
        conn.execute(
            r#"
            INSERT INTO query_logs (
                query_id, query_text, retrieval_method,
                retrieved_chunk_ids, latency_ms, result_count
            ) VALUES (?, ?, ?, ?, ?, ?)
            "#,
            params![
                query_id_clone,
                query_text,
                retrieval_method,
                chunk_ids_json_clone,
                latency_ms,
                result_count
            ]
        )?;
        Ok::<_, RagmcpError>(())
    }).await?;

    Ok(())
}
