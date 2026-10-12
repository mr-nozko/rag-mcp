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
use crate::ingest::metadata::{compute_file_hash, extract_agent_name, extract_namespace, normalize_doc_path};
use crate::ingest::parsers::ParserRegistry;
use crate::ingest::chunker::chunk_document;
use crate::ingest::db_writer::{insert_chunks, insert_document};
use crate::ingest::delete_documents_normalized;
use crate::watch::embed_missing_for_doc;
use crate::storage::R2Client;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

/// After a successful local write, mirror the object to R2 and upload a SQLite snapshot.
/// Returns whether the snapshot upload succeeded (`true` when R2 is None / local mode).
/// Search/get/watcher never call this — only create/update/delete write-through.
async fn r2_write_through_put(
    r2: &Option<Arc<R2Client>>,
    db_path: &Path,
    relative_path: &str,
    absolute_path: &Path,
) -> bool {
    // Local mode: no remote snapshot to fail — treat as success so callers stay green.
    let Some(client) = r2 else { return true };
    match fs::read(absolute_path) {
        Ok(bytes) => {
            if let Err(e) = client.put_document(relative_path, &bytes).await {
                log::error!("R2 write-through put failed for {}: {}", relative_path, e);
                // Object not mirrored — report snapshot as failed (not attempted).
                return false;
            }
        }
        Err(e) => {
            log::error!(
                "R2 write-through: cannot read {} for upload: {}",
                absolute_path.display(),
                e
            );
            return false;
        }
    }
    match client.upload_snapshot(db_path).await {
        Ok(()) => true,
        Err(e) => {
            log::error!("R2 snapshot upload after put failed: {}", e);
            false
        }
    }
}

/// After a successful local delete, remove the R2 object and upload a SQLite snapshot.
/// Retries object delete once; on object failure logs but always attempts snapshot.
/// Returns whether the snapshot upload succeeded (`true` when R2 is None / local mode).
async fn r2_write_through_delete(
    r2: &Option<Arc<R2Client>>,
    db_path: &Path,
    relative_path: &str,
) -> bool {
    // Local mode: no remote snapshot to fail — treat as success so callers stay green.
    let Some(client) = r2 else { return true };
    if let Err(e) = client.delete_document(relative_path).await {
        log::error!(
            "R2 write-through delete failed for {}: {}; retrying once",
            relative_path,
            e
        );
        if let Err(e2) = client.delete_document(relative_path).await {
            // Do not return early — snapshot must still be attempted so remote DB stays current.
            log::error!(
                "R2 write-through delete retry failed for {}: {}",
                relative_path,
                e2
            );
        }
    }
    match client.upload_snapshot(db_path).await {
        Ok(()) => true,
        Err(e) => {
            log::error!("R2 snapshot upload after delete failed: {}", e);
            false
        }
    }
}

/// Get all tool definitions for tools/list
pub fn get_tool_definitions(pageindex_enabled: bool) -> Vec<Tool> {
    let mut tools = vec![
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
            description: "Update an existing document by replacing specific snippets of text, re-parsing and re-embedding automatically. Creates parent directories if needed.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "doc_path": {
                        "type": "string",
                        "description": "Relative path of document to update"
                    },
                    "updates": {
                        "type": "array",
                        "description": "List of text replacements to apply. Each update replaces 'target_text' with 'replacement_text'. Must provide exact existing text to match.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "target_text": { "type": "string", "description": "Exact text in the document to replace" },
                                "replacement_text": { "type": "string", "description": "New text to insert in place of target_text" }
                            },
                            "required": ["target_text", "replacement_text"]
                        }
                    }
                },
                "required": ["doc_path", "updates"]
            }),
        },
        Tool {
            name: "ragmcp_delete_doc".to_string(),
            description: "Delete a document file under rag_folder and remove it from the search index (chunks, embeddings, PageIndex). Path must be relative to the docs root.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "doc_path": {
                        "type": "string",
                        "description": "Relative path of document to delete (e.g. \"Namespace/subfolder/doc.md\")"
                    }
                },
                "required": ["doc_path"]
            }),
        },
    ];

    if pageindex_enabled {
        tools.push(Tool {
            name: "ragmcp_reason".to_string(),
            description: "Advanced reasoning-based retrieval mapping for long or nested documents. Use this when simple keyword search (ragmcp_search) fails to find complex structural answers.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The complex question to reason about within the document structure"
                    },
                    "doc_path": {
                        "type": "string",
                        "description": "Optional specific document path to reason within. If omitted, will attempt to identify target via search first."
                    },
                    "max_iterations": {
                        "type": "integer",
                        "description": "Maximum reasoning steps",
                        "default": 5,
                        "minimum": 1,
                        "maximum": 10
                    },
                    "model": {
                        "type": "string",
                        "description": "OpenAI model to use (default: gpt-4o-mini)",
                        "default": "gpt-4o-mini"
                    }
                },
                "required": ["query"]
            }),
        });
    }

    tools
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
    /// Section filter — part of the MCP API contract; section-level retrieval
    /// is not yet implemented. Reserved for future partial-document responses.
    #[allow(dead_code)]
    sections: Option<Vec<String>>,
}

/// Handle ragmcp_get tool
pub async fn handle_get(
    db: &Db,
    config: &Config,
    arguments: &Value,
) -> Result<ToolsCallResult> {
    let params: GetParams = serde_json::from_value(arguments.clone())
        .map_err(|e| RagmcpError::Config(format!("Invalid get params: {}", e)))?;

    // Normalize for comparison: use forward slash so we match DB regardless of stored separator (Windows \ vs /)
    let doc_path_param = normalize_doc_path(&params.doc_path);
    let doc = db.with_connection({
        let doc_path_param = doc_path_param.clone();
        move |conn| {
        let mut stmt = conn.prepare(
            r#"
            SELECT 
                doc_id, doc_path, doc_type, namespace, agent_name,
                content_text, content_tokens, last_modified, file_hash, metadata_json
            FROM documents
            WHERE REPLACE(doc_path, '\', '/') = ?
            ORDER BY last_modified DESC
            LIMIT 1
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
    }}).await?;

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
        // Prefer live file on disk so MCP writes are visible even if index briefly lags
        let disk_path = config.rag_folder().join(&doc_path_param);
        let full_content = match fs::read_to_string(&disk_path) {
            Ok(text) => text,
            Err(_) => {
                // Fall back to indexed content_text if the file is missing
                result_text.push_str("(Disk file missing; showing indexed content_text)\n");
                content_text
            }
        };
        result_text.push_str("Full Content:\n");
        result_text.push_str(&full_content);
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
    /// Controls whether metadata (doc type, agent, etc.) is included in list
    /// results. Part of the MCP API contract; metadata enrichment is not yet
    /// applied — reserved for a richer list response in a future iteration.
    #[allow(dead_code)]
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

            // Normalize path separators to match documents stored with forward slashes
            let doc_path = normalize_doc_path(&doc_path);
            let doc_path_clone = doc_path.clone();
            let info = db.with_connection(move |conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT 
                        doc_path, doc_type, namespace, agent_name,
                        content_tokens, last_modified, file_hash
                    FROM documents
                    WHERE REPLACE(doc_path, '\', '/') = ?
                    ORDER BY last_modified DESC
                    LIMIT 1
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
                    "SELECT doc_id FROM documents WHERE REPLACE(doc_path, '\\', '/') = ? ORDER BY last_modified DESC LIMIT 1",
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

/// Create a new document: validate path, create dirs, write file, parse, chunk, insert, embed, audit.
pub async fn handle_create_doc(
    db: &Db,
    embedder: &OpenAIEmbedder,
    config: &Config,
    cache: Option<Arc<ChunkEmbeddingCache>>,
    arguments: &Value,
    pageindex: Option<Arc<crate::pageindex::PageIndexManager>>,
    r2: Option<Arc<R2Client>>,
) -> Result<ToolsCallResult> {
    let params: CreateDocParams = serde_json::from_value(arguments.clone())
        .map_err(|e| RagmcpError::InvalidInput(format!("Invalid parameters: {}", e)))?;

    // Canonical path identity before any DB write
    let doc_path = normalize_doc_path(&params.doc_path);

    let validator = PathValidator::new(&config.ragmcp.rag_folder)?;
    let absolute_path = validator.validate_write_path(&doc_path)?;

    if absolute_path.exists() {
        let _ = log_operation(
            db,
            "create",
            &doc_path,
            None,
            false,
            Some("File already exists"),
            None,
        )
        .await;
        return Err(RagmcpError::InvalidInput(format!(
            "File already exists: {}",
            doc_path
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
        let extension = Path::new(&doc_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let registry = ParserRegistry::new();
        let parsed = registry
            .parse(&params.content, &doc_path, &extension)
            .map_err(|e| RagmcpError::Parse(e.to_string()))?;
        let doc_type = params
            .doc_type
            .unwrap_or_else(|| parsed.doc_type.clone());
        let namespace = extract_namespace(&doc_path);
        let agent_name = extract_agent_name(&doc_path);
        let chunks = chunk_document(&parsed, &config.performance)?;
        let total_tokens = chunks.iter().map(|c| c.tokens).sum::<usize>();
        (doc_type, namespace, agent_name, chunks, total_tokens, parsed.content)
    };

    let doc_id = insert_document(
        db,
        &doc_path,
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

    // Embed immediately so search/get stay fresh without a separate embed CLI
    let embedded = embed_missing_for_doc(db, embedder, &doc_id, config.embeddings.batch_size).await?;
    log::info!(
        "create_doc: {} chunks created, {} embeddings stored",
        chunk_count,
        embedded
    );
    // Invalidate in-memory vectors so next search reloads from the updated DB.
    if let Some(ref cache) = cache {
        cache.clear();
    }

    let meta_json = json!({
        "doc_type": doc_type,
        "chunk_count": chunk_count,
        "embeddings_stored": embedded,
        "file_hash": file_hash
    })
    .to_string();
    let operation_id = log_operation(
        db,
        "create",
        &doc_path,
        Some(&doc_id),
        true,
        None,
        Some(&meta_json),
    )
    .await?;

    if let Some(pi) = pageindex {
        let pi_clone = pi.clone();
        let path_clone = doc_path.clone();
        let doc_id_clone = doc_id.clone();
        let db_path = config.db_path().to_path_buf();
        let model_clone = config.pageindex.default_model.clone();
        // Clone R2 into spawn so PageIndex metadata lands in the remote snapshot too.
        let r2_for_pi = r2.clone();
        tokio::spawn(async move {
            let db = crate::db::Db::new(&db_path);
            if let Ok(resp) = pi_clone.index_document(&path_clone, &model_clone).await {
                let node_count = resp.node_count.unwrap_or(0);
                let tree_path = resp.tree_path;
                let insert_ok = db.with_connection(move |conn| {
                    conn.execute(
                        "INSERT OR REPLACE INTO pageindex_index (doc_id, doc_path, tree_path, tree_built_at, tree_node_count, model_used) VALUES (?, ?, ?, CURRENT_TIMESTAMP, ?, ?)",
                        rusqlite::params![doc_id_clone, path_clone, tree_path, node_count, model_clone]
                    )?;
                    Ok::<_, RagmcpError>(())
                }).await;
                // After pageindex_index is durable, push a fresh snapshot when R2 is configured.
                if insert_ok.is_ok() {
                    if let Some(client) = r2_for_pi {
                        if let Err(e) = client.upload_snapshot(&db_path).await {
                            log::error!("R2 snapshot upload after pageindex (create) failed: {}", e);
                        }
                    }
                }
            }
        });
    }

    // R2 write-through: object + snapshot (no-op when r2 is None / local mode)
    let snapshot_ok = r2_write_through_put(&r2, config.db_path(), &doc_path, &absolute_path).await;
    // Live index succeeded; surface R2 snapshot failure so the client can retry/alert.
    let r2_snapshot_failed = r2.is_some() && !snapshot_ok;
    let text = if r2_snapshot_failed {
        json!({
            "success": true,
            "doc_id": doc_id,
            "doc_path": doc_path,
            "chunks_created": chunk_count,
            "embeddings_stored": embedded,
            "operation_id": operation_id,
            "r2_snapshot_ok": false,
            "message": "Document created in live index, but R2 snapshot upload failed"
        })
        .to_string()
    } else {
        json!({
            "success": true,
            "doc_id": doc_id,
            "doc_path": doc_path,
            "chunks_created": chunk_count,
            "embeddings_stored": embedded,
            "operation_id": operation_id,
            "message": "Document created successfully"
        })
        .to_string()
    };

    Ok(ToolsCallResult {
        content: vec![ContentItem {
            content_type: "text".to_string(),
            text,
        }],
        is_error: if r2_snapshot_failed { Some(true) } else { None },
    })
}

/// Params for ragmcp_update_doc
#[derive(Debug, Deserialize)]
struct DocUpdate {
    target_text: String,
    replacement_text: String,
}

#[derive(Debug, Deserialize)]
struct UpdateDocParams {
    doc_path: String,
    updates: Vec<DocUpdate>,
}

/// Update an existing document: validate path, create dirs if needed, write file, re-parse, re-chunk, upsert, embed, audit.
pub async fn handle_update_doc(
    db: &Db,
    embedder: &OpenAIEmbedder,
    config: &Config,
    cache: Option<Arc<ChunkEmbeddingCache>>,
    arguments: &Value,
    pageindex: Option<Arc<crate::pageindex::PageIndexManager>>,
    r2: Option<Arc<R2Client>>,
) -> Result<ToolsCallResult> {
    let params: UpdateDocParams = serde_json::from_value(arguments.clone())
        .map_err(|e| RagmcpError::InvalidInput(format!("Invalid parameters: {}", e)))?;

    // Canonical path identity before any DB write
    let doc_path = normalize_doc_path(&params.doc_path);

    let validator = PathValidator::new(&config.ragmcp.rag_folder)?;
    let absolute_path = validator.validate_write_path(&doc_path)?;

    // Read existing content
    let mut current_content = if absolute_path.exists() {
        fs::read_to_string(&absolute_path).unwrap_or_default()
    } else {
        return Err(RagmcpError::InvalidInput(format!("Cannot apply updates: File does not exist at {}", doc_path)));
    };

    for update in &params.updates {
        if !current_content.contains(&update.target_text) {
            // Return an error string with a preview of the target text that wasn't found
            let preview: String = update.target_text.chars().take(50).collect();
            return Err(RagmcpError::InvalidInput(format!(
                "Target text not found in document: '{}...'", preview
            )));
        }
        // Replace the first occurrence of target_text with replacement_text
        current_content = current_content.replacen(&update.target_text, &update.replacement_text, 1);
    }

    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent).map_err(RagmcpError::Io)?;
    }
    fs::write(&absolute_path, &current_content).map_err(RagmcpError::Io)?;

    let file_hash = compute_file_hash(&absolute_path)?;
    let metadata = fs::metadata(&absolute_path).map_err(RagmcpError::Io)?;
    let last_modified = metadata.modified().map_err(RagmcpError::Io)?;

    let (doc_type, namespace, agent_name, chunks, total_tokens, content) = {
        let extension = Path::new(&doc_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let registry = ParserRegistry::new();
        let parsed = registry
            .parse(&current_content, &doc_path, &extension)
            .map_err(|e| RagmcpError::Parse(e.to_string()))?;
        let doc_type = parsed.doc_type.clone();
        let namespace = extract_namespace(&doc_path);
        let agent_name = extract_agent_name(&doc_path);
        let chunks = chunk_document(&parsed, &config.performance)?;
        let total_tokens = chunks.iter().map(|c| c.tokens).sum::<usize>();
        (doc_type, namespace, agent_name, chunks, total_tokens, parsed.content)
    };

    let doc_id = insert_document(
        db,
        &doc_path,
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

    // Embed immediately so search stays fresh without a separate embed CLI
    let embedded = embed_missing_for_doc(db, embedder, &doc_id, config.embeddings.batch_size).await?;
    log::info!(
        "update_doc: {} chunks created, {} embeddings stored",
        chunk_count,
        embedded
    );
    // Invalidate in-memory vectors so next search reloads from the updated DB.
    if let Some(ref cache) = cache {
        cache.clear();
    }

    let meta_json = json!({
        "doc_type": doc_type,
        "chunk_count": chunk_count,
        "embeddings_stored": embedded,
        "file_hash": file_hash
    })
    .to_string();
    let operation_id = log_operation(
        db,
        "update",
        &doc_path,
        Some(&doc_id),
        true,
        None,
        Some(&meta_json),
    )
    .await?;

    if let Some(pi) = pageindex {
        let pi_clone = pi.clone();
        let path_clone = doc_path.clone();
        let doc_id_clone = doc_id.clone();
        let db_path = config.db_path().to_path_buf();
        let model_clone = config.pageindex.default_model.clone();
        // Clone R2 into spawn so PageIndex metadata lands in the remote snapshot too.
        let r2_for_pi = r2.clone();
        tokio::spawn(async move {
            let db = crate::db::Db::new(&db_path);
            if let Ok(resp) = pi_clone.index_document(&path_clone, &model_clone).await {
                let node_count = resp.node_count.unwrap_or(0);
                let tree_path = resp.tree_path;
                let insert_ok = db.with_connection(move |conn| {
                    conn.execute(
                        "INSERT OR REPLACE INTO pageindex_index (doc_id, doc_path, tree_path, tree_built_at, tree_node_count, model_used) VALUES (?, ?, ?, CURRENT_TIMESTAMP, ?, ?)",
                        rusqlite::params![doc_id_clone, path_clone, tree_path, node_count, model_clone]
                    )?;
                    Ok::<_, RagmcpError>(())
                }).await;
                // After pageindex_index is durable, push a fresh snapshot when R2 is configured.
                if insert_ok.is_ok() {
                    if let Some(client) = r2_for_pi {
                        if let Err(e) = client.upload_snapshot(&db_path).await {
                            log::error!("R2 snapshot upload after pageindex (update) failed: {}", e);
                        }
                    }
                }
            }
        });
    }

    // R2 write-through: object + snapshot (no-op when r2 is None / local mode)
    let snapshot_ok = r2_write_through_put(&r2, config.db_path(), &doc_path, &absolute_path).await;
    // Live index succeeded; surface R2 snapshot failure so the client can retry/alert.
    let r2_snapshot_failed = r2.is_some() && !snapshot_ok;
    let text = if r2_snapshot_failed {
        json!({
            "success": true,
            "doc_id": doc_id,
            "doc_path": doc_path,
            "chunks_created": chunk_count,
            "embeddings_stored": embedded,
            "operation_id": operation_id,
            "r2_snapshot_ok": false,
            "message": "Document updated in live index, but R2 snapshot upload failed"
        })
        .to_string()
    } else {
        json!({
            "success": true,
            "doc_id": doc_id,
            "doc_path": doc_path,
            "chunks_created": chunk_count,
            "embeddings_stored": embedded,
            "operation_id": operation_id,
            "message": "Document updated successfully"
        })
        .to_string()
    };

    Ok(ToolsCallResult {
        content: vec![ContentItem {
            content_type: "text".to_string(),
            text,
        }],
        is_error: if r2_snapshot_failed { Some(true) } else { None },
    })
}

/// Params for ragmcp_delete_doc
#[derive(Debug, Deserialize)]
struct DeleteDocParams {
    doc_path: String,
}

/// Delete a document file under rag_folder and remove it from the SQLite index.
pub async fn handle_delete_doc(
    db: &Db,
    config: &Config,
    arguments: &Value,
    cache: Option<Arc<ChunkEmbeddingCache>>,
    r2: Option<Arc<R2Client>>,
) -> Result<ToolsCallResult> {
    let params: DeleteDocParams = serde_json::from_value(arguments.clone())
        .map_err(|e| RagmcpError::InvalidInput(format!("Invalid parameters: {}", e)))?;

    // Canonical path identity before any FS or DB work
    let doc_path = normalize_doc_path(&params.doc_path);

    let validator = PathValidator::new(&config.ragmcp.rag_folder)?;
    let absolute_path = validator.validate_write_path(&doc_path)?;

    // Refuse directory deletes — this tool is file-only
    if absolute_path.is_dir() {
        return Err(RagmcpError::InvalidInput(format!(
            "Path is a directory; ragmcp_delete_doc only deletes files: {}",
            doc_path
        )));
    }

    // Delete from disk when the file exists
    let mut deleted_from_disk = false;
    if absolute_path.is_file() {
        fs::remove_file(&absolute_path).map_err(RagmcpError::Io)?;
        deleted_from_disk = true;
        log::info!("delete_doc: removed file {}", absolute_path.display());
    }

    // Remove index rows (slash-variant safe) + PageIndex + agent relations
    let index_result = delete_documents_normalized(db, &doc_path).await?;
    let deleted_from_index = index_result.deleted_count > 0;
    // After a successful index delete, drop cached embeddings so search cannot return ghosts.
    if deleted_from_index {
        if let Some(ref cache) = cache {
            cache.clear();
        }
    }

    if !deleted_from_disk && !deleted_from_index {
        let _ = log_operation(
            db,
            "delete",
            &doc_path,
            None,
            false,
            Some("Document not found on disk or in index"),
            None,
        )
        .await;
        return Err(RagmcpError::DocumentNotFound(doc_path));
    }

    let meta_json = json!({
        "deleted_from_disk": deleted_from_disk,
        "deleted_from_index": deleted_from_index,
        "index_rows_removed": index_result.deleted_count,
        "agent_name": index_result.agent_name,
    })
    .to_string();

    let operation_id = log_operation(
        db,
        "delete",
        &doc_path,
        index_result.doc_id.as_deref(),
        true,
        None,
        Some(&meta_json),
    )
    .await?;

    // R2 write-through: delete object + snapshot (no-op when r2 is None / local mode)
    let snapshot_ok = r2_write_through_delete(&r2, config.db_path(), &doc_path).await;
    // Live index succeeded; surface R2 snapshot failure so the client can retry/alert.
    let r2_snapshot_failed = r2.is_some() && !snapshot_ok;
    let text = if r2_snapshot_failed {
        json!({
            "success": true,
            "doc_path": doc_path,
            "doc_id": index_result.doc_id,
            "deleted_from_disk": deleted_from_disk,
            "deleted_from_index": deleted_from_index,
            "operation_id": operation_id,
            "r2_snapshot_ok": false,
            "message": "Document deleted from live index, but R2 snapshot upload failed"
        })
        .to_string()
    } else {
        json!({
            "success": true,
            "doc_path": doc_path,
            "doc_id": index_result.doc_id,
            "deleted_from_disk": deleted_from_disk,
            "deleted_from_index": deleted_from_index,
            "operation_id": operation_id,
            "message": "Document deleted successfully"
        })
        .to_string()
    };

    Ok(ToolsCallResult {
        content: vec![ContentItem {
            content_type: "text".to_string(),
            text,
        }],
        is_error: if r2_snapshot_failed { Some(true) } else { None },
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

/// Log pageindex query to pageindex_query_logs table
async fn log_pageindex_query(
    db: &Db,
    query_text: &str,
    doc_path: &str,
    result: &crate::pageindex::manager::QueryResponse,
    latency_ms: u64,
    model_used: &str,
) -> Result<()> {
    let query_id = Uuid::new_v4().to_string();
    
    // Extract node_ids from retrieved_sections
    let mut node_ids = Vec::new();
    for section in &result.retrieved_sections {
        if let Some(node_id) = section.get("node_id") {
            if let Some(s) = node_id.as_str() {
                node_ids.push(s.to_string());
            }
        }
    }
    let node_ids_json = serde_json::to_string(&node_ids)
        .map_err(|e| RagmcpError::Config(format!("JSON serialization error: {}", e)))?;

    let query_text = query_text.to_string();
    let doc_path = doc_path.to_string();
    let model_used = model_used.to_string();
    let query_id_clone = query_id.clone();
    let node_ids_json_clone = node_ids_json.clone();
    let iterations = result.iterations as i32;
    let latency_ms = latency_ms as i32;

    db.with_connection(move |conn| {
        conn.execute(
            r#"
            INSERT INTO pageindex_query_logs (
                query_id, query_text, doc_path,
                iterations, retrieved_node_ids, latency_ms, model_used
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
            rusqlite::params![
                query_id_clone,
                query_text,
                doc_path,
                iterations,
                node_ids_json_clone,
                latency_ms,
                model_used
            ]
        )?;
        Ok::<_, RagmcpError>(())
    }).await?;

    Ok(())
}


/// Params for ragmcp_reason
#[derive(Debug, Deserialize)]
struct ReasonParams {
    query: String,
    doc_path: Option<String>,
    #[serde(default = "default_max_iterations")]
    max_iterations: u8,
    #[serde(default = "default_reason_model")]
    model: String,
}

fn default_max_iterations() -> u8 { 5 }
fn default_reason_model() -> String { "gpt-4o-mini".to_string() }

/// Handle ragmcp_reason tool (PageIndex reasoning)
pub async fn handle_reason(
    db: &Db,
    embedder: &OpenAIEmbedder,
    _config: &Config,
    pi: Arc<crate::pageindex::PageIndexManager>,
    arguments: &Value,
    chunk_cache: Option<Arc<ChunkEmbeddingCache>>,
) -> Result<ToolsCallResult> {
    let params: ReasonParams = serde_json::from_value(arguments.clone())
        .map_err(|e| RagmcpError::Config(format!("Invalid reason params: {}", e)))?;

    let doc_path = if let Some(path) = params.doc_path {
        path
    } else {
        // Fallback: search for the best document candidate first
        log::info!("[pageindex] No doc_path provided, searching for candidate...");
        let search_results = search_hybrid(
            db,
            embedder,
            &params.query,
            None,
            None,
            1,
            0.5,
            0.5,
            0.5,
            chunk_cache,
        ).await?;

        if let Some(top) = search_results.first() {
            log::info!("[pageindex] Selected candidate: {}", top.doc_path);
            top.doc_path.clone()
        } else {
            return Ok(ToolsCallResult {
                content: vec![ContentItem {
                    content_type: "text".to_string(),
                    text: "No relevant documents found to reason within. Please provide a doc_path or a more specific query.".to_string(),
                }],
                is_error: Some(true),
            });
        }
    };

    let start = std::time::Instant::now();
    let result = pi.query(&doc_path, &params.query, &params.model, params.max_iterations).await
        .map_err(|e| RagmcpError::Config(format!("Reasoning failed: {}", e)))?;
    let duration = start.elapsed().as_millis() as u64;

    // Log the pageindex query telemetry
    let _ = log_pageindex_query(db, &params.query, &doc_path, &result, duration, &params.model).await;

    let result_text = format!(
        "Reasoning Result for: {}\n\n\"{}\"\n\nMethod: PageIndex Reasoning\nIterations: {}\nLatency: {}ms (Sidecar)\nTotal Latency: {}ms",
        doc_path,
        result.answer,
        result.iterations,
        result.latency_ms,
        duration
    );

    Ok(ToolsCallResult {
        content: vec![ContentItem {
            content_type: "text".to_string(),
            text: result_text,
        }],
        is_error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        Config, EmbeddingsConfig, PerformanceConfig, RagmcpConfig, SearchConfig,
    };
    use crate::db::migrate;
    use std::path::Path;
    use tempfile::TempDir;

    /// Every MCP tool must advertise a JSON Schema object so strict clients accept tools/list.
    #[test]
    fn test_all_tool_input_schemas_are_objects() {
        for pageindex_enabled in [false, true] {
            let tools = get_tool_definitions(pageindex_enabled);
            assert!(!tools.is_empty(), "expected at least one tool");
            for tool in tools {
                let schema_type = tool
                    .input_schema
                    .get("type")
                    .and_then(|v| v.as_str());
                assert_eq!(
                    schema_type,
                    Some("object"),
                    "tool {} inputSchema.type must be object",
                    tool.name
                );
                let properties = tool.input_schema.get("properties");
                assert!(
                    properties.map(|p| p.is_object()).unwrap_or(false),
                    "tool {} inputSchema.properties must be an object",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn test_delete_doc_listed_in_tool_definitions() {
        let tools = get_tool_definitions(false);
        assert!(
            tools.iter().any(|t| t.name == "ragmcp_delete_doc"),
            "ragmcp_delete_doc must appear in tools/list"
        );
    }

    fn test_config(rag_folder: &Path) -> Config {
        Config {
            ragmcp: RagmcpConfig {
                rag_folder: rag_folder.to_path_buf(),
                db_path: rag_folder.join("test.db"),
                log_level: "info".to_string(),
            },
            embeddings: EmbeddingsConfig {
                provider: "openai".to_string(),
                model: "text-embedding-3-small".to_string(),
                api_key_env: "OPENAI_API_KEY".to_string(),
                batch_size: 100,
                dimensions: 1536,
                cache_capacity: 0,
            },
            search: SearchConfig {
                default_k: 5,
                min_score: 0.65,
                hybrid_bm25_weight: 0.5,
                hybrid_vector_weight: 0.5,
            },
            performance: PerformanceConfig {
                max_latency_ms: 1000,
                chunk_size_tokens: 300,
                chunk_overlap_tokens: 50,
            },
            http_server: Default::default(),
            pageindex: Default::default(),
            storage: None,
        }
    }

    async fn setup_delete_test() -> (Db, Config, TempDir) {
        let temp = TempDir::new().unwrap();
        let rag = temp.path().to_path_buf();
        let db_path = rag.join("test.db");
        let db = Db::new(&db_path);
        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        db.with_connection(move |conn| migrate::run_migrations(conn, &migrations_dir))
            .await
            .unwrap();
        let config = test_config(&rag);
        (db, config, temp)
    }

    #[tokio::test]
    async fn test_handle_delete_doc_removes_file_and_index() {
        let (db, config, _temp) = setup_delete_test().await;
        let rel = "Guides/note.md";
        let abs = config.rag_folder().join("Guides");
        fs::create_dir_all(&abs).unwrap();
        let file = abs.join("note.md");
        fs::write(&file, "# hello\n").unwrap();

        insert_document(
            &db,
            rel,
            "markdown",
            "guides",
            None,
            "# hello\n",
            2,
            "hash1",
            std::time::SystemTime::now(),
        )
        .await
        .unwrap();

        let args = json!({ "doc_path": rel });
        let result = handle_delete_doc(&db, &config, &args, None, None).await.unwrap();
        assert!(result.is_error.is_none());
        assert!(!file.exists(), "file should be deleted from disk");

        let count: i64 = db
            .with_connection(|conn| {
                let c: i64 = conn.query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))?;
                Ok::<i64, RagmcpError>(c)
            })
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_handle_delete_doc_traversal_blocked() {
        let (db, config, _temp) = setup_delete_test().await;
        let args = json!({ "doc_path": "../outside.md" });
        let err = handle_delete_doc(&db, &config, &args, None, None).await.unwrap_err();
        match err {
            RagmcpError::InvalidInput(msg) => assert!(msg.contains("traversal") || msg.contains("..")),
            other => panic!("expected InvalidInput, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_handle_delete_doc_missing_errors() {
        let (db, config, _temp) = setup_delete_test().await;
        let args = json!({ "doc_path": "Missing/nowhere.md" });
        let err = handle_delete_doc(&db, &config, &args, None, None).await.unwrap_err();
        match err {
            RagmcpError::DocumentNotFound(p) => assert_eq!(p, "Missing/nowhere.md"),
            other => panic!("expected DocumentNotFound, got {:?}", other),
        }
    }
}
