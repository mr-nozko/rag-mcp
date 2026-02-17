use crate::config::Config;
use crate::db::Db;
use crate::embeddings::OpenAIEmbedder;
use crate::error::{Result, RagmcpError};
use crate::mcp::tools;
use crate::mcp::types::*;
use crate::cache::ChunkEmbeddingCache;
use serde_json::Value;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as AsyncBufReader};

/// MCP Server implementation
pub struct McpServer {
    db: Db,
    embedder: OpenAIEmbedder,
    config: Config,
    chunk_cache: Option<Arc<ChunkEmbeddingCache>>,
}

impl McpServer {
    /// Create a new MCP server
    pub fn new(
        db: Db,
        embedder: OpenAIEmbedder,
        config: Config,
        chunk_cache: Option<Arc<ChunkEmbeddingCache>>,
    ) -> Self {
        Self {
            db,
            embedder,
            config,
            chunk_cache,
        }
    }

    /// Process an MCP JSON-RPC request (transport-agnostic)
    /// 
    /// This function handles routing and processing of MCP protocol requests.
    /// It can be called from both stdio and HTTP transports.
    /// 
    /// # Arguments
    /// * `request` - The JSON-RPC request to process
    /// * `initialized` - Whether the client has been initialized (for stdio transport)
    /// 
    /// # Returns
    /// * `Ok(Some(response))` - Response to send back to client
    /// * `Ok(None)` - Notification (no response needed)
    /// * `Err(e)` - Error processing request
    pub async fn process_mcp_request(
        &self,
        request: JsonRpcRequest,
        initialized: &mut bool,
    ) -> Result<Option<JsonRpcResponse>> {
        // Handle notifications (no ID) - don't send response
        let id = match &request.id {
            Some(id) => id.clone(),
            None => {
                // Handle notifications
                if request.method == "notifications/initialized" {
                    *initialized = true;
                }
                return Ok(None);
            }
        };

        // Route request to appropriate handler
        // Note: For HTTP transport, initialization state is not enforced since requests are stateless
        // The initialized flag is only meaningful for stdio transport where state persists
        let response = match request.method.as_str() {
            "initialize" => {
                // Always allow initialize - for HTTP transport, each request is independent
                // For stdio transport, the initialized flag will prevent double initialization
                // but we don't have a way to distinguish transport here, so we allow it
                // The caller (stdio vs HTTP) should handle state management
                self.handle_initialize(&id, &request.params).await
            }
            "tools/list" => {
                // Always allow tools/list - HTTP transport is stateless
                self.handle_tools_list(&id).await
            }
            "tools/call" => {
                // Always allow tools/call - HTTP transport is stateless
                self.handle_tools_call(&id, &request.params).await
            }
            "shutdown" => {
                self.handle_shutdown(&id).await
            }
            _ => self.handle_error(
                &id,
                error_codes::METHOD_NOT_FOUND,
                &format!("Unknown method: {}", request.method),
            ),
        };

        match response {
            Ok(resp) => Ok(Some(resp)),
            Err(e) => {
                // Convert handler error to JSON-RPC error response
                Ok(Some(JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: id.into(),
                    payload: JsonRpcResponsePayload::Error {
                        error: JsonRpcError {
                            code: error_codes::INTERNAL_ERROR,
                            message: format!("Internal error: {}", e),
                            data: Some(serde_json::json!({ "details": e.to_string() })),
                        },
                    },
                }))
            }
        }
    }

    /// Run the MCP server (reads from stdin, writes to stdout)
    pub async fn run(&mut self) -> Result<()> {
        // Use async stdin/stdout for non-blocking I/O
        let stdin = tokio::io::stdin();
        let mut stdin_reader = AsyncBufReader::new(stdin);
        let mut stdout = tokio::io::stdout();
        let mut stderr = tokio::io::stderr();

        let mut line = String::new();
        let mut initialized = false;

        // Log to stderr (per MCP spec)
        let _ = stderr.write_all(
            format!("RAGMcp MCP Server v{} starting...\n", env!("CARGO_PKG_VERSION")).as_bytes()
        ).await;

        loop {
            line.clear();
            let bytes_read = stdin_reader.read_line(&mut line).await
                .map_err(|e| RagmcpError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to read from stdin: {}", e)
                )))?;

            // EOF - client disconnected
            if bytes_read == 0 {
                break;
            }

            // Trim newline
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Parse JSON-RPC message
            let request: JsonRpcRequest = match serde_json::from_str(trimmed) {
                Ok(req) => req,
                Err(e) => {
                    // Send parse error response if we have an ID
                    if let Some(id) = extract_id_from_line(trimmed) {
                        let error_response = JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id,
                            payload: JsonRpcResponsePayload::Error {
                                error: JsonRpcError {
                                    code: error_codes::PARSE_ERROR,
                                    message: format!("Parse error: {}", e),
                                    data: None,
                                },
                            },
                        };
                        send_response(&mut stdout, &error_response).await?;
                    }
                    continue;
                }
            };

            // Process request using common handler
            match self.process_mcp_request(request, &mut initialized).await {
                Ok(Some(response)) => {
                    send_response(&mut stdout, &response).await?;
                }
                Ok(None) => {
                    // Notification - no response needed
                    if initialized {
                        let _ = stderr.write_all(b"Client initialized\n").await;
                    }
                }
                Err(e) => {
                    // This shouldn't happen as process_mcp_request converts errors to responses
                    log::error!("Unexpected error in process_mcp_request: {}", e);
                }
            }
        }

        let _ = stderr.write_all(b"MCP server shutting down\n").await;
        Ok(())
    }

    /// Handle initialize request
    async fn handle_initialize(
        &self,
        id: &JsonRpcId,
        params: &Option<Value>,
    ) -> Result<JsonRpcResponse> {
        let params: InitializeParams = serde_json::from_value(
            params.clone().unwrap_or(serde_json::json!({}))
        )
        .map_err(|e| RagmcpError::Config(format!("Invalid initialize params: {}", e)))?;

        // Support protocol version 2024-11-05 and 2025-06-18
        let protocol_version = if params.protocol_version.starts_with("2024") 
            || params.protocol_version.starts_with("2025") {
            "2024-11-05".to_string() // Use stable version
        } else {
            params.protocol_version.clone()
        };

        let result = InitializeResult {
            protocol_version: protocol_version.clone(),
            capabilities: serde_json::json!({
                "tools": {}
            }),
            server_info: ServerInfo {
                name: "ragmcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        Ok(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: id.clone().into(),
            payload: JsonRpcResponsePayload::Result {
                result: serde_json::to_value(&result)
                    .map_err(|e| RagmcpError::Config(format!("JSON serialization error: {}", e)))?,
            },
        })
    }

    /// Handle tools/list request
    async fn handle_tools_list(&self, id: &JsonRpcId) -> Result<JsonRpcResponse> {
        let tools = tools::get_tool_definitions();
        let result = ToolsListResult { tools };

        Ok(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: id.clone().into(),
            payload: JsonRpcResponsePayload::Result {
                result: serde_json::to_value(&result)
                    .map_err(|e| RagmcpError::Config(format!("JSON serialization error: {}", e)))?,
            },
        })
    }

    /// Handle tools/call request
    async fn handle_tools_call(
        &self,
        id: &JsonRpcId,
        params: &Option<Value>,
    ) -> Result<JsonRpcResponse> {
        let params: ToolsCallParams = serde_json::from_value(
            params.clone().ok_or_else(|| {
                RagmcpError::Config("Missing params for tools/call".to_string())
            })?
        )
        .map_err(|e| RagmcpError::Config(format!("Invalid tools/call params: {}", e)))?;

        // Route to appropriate tool handler
        let result = match params.name.as_str() {
            "ragmcp_search" => {
                tools::handle_search(
                    &self.db,
                    &self.embedder,
                    &self.config,
                    &params.arguments,
                    self.chunk_cache.clone(),
                )
                .await?
            }
            "ragmcp_get" => {
                tools::handle_get(&self.db, &params.arguments).await?
            }
            "ragmcp_list" => {
                tools::handle_list(&self.db, &params.arguments).await?
            }
            "ragmcp_related" => {
                tools::handle_related(&self.db, &params.arguments).await?
            }
            "ragmcp_explain" => {
                tools::handle_explain(&self.db, &params.arguments).await?
            }
            "ragmcp_create_doc" => {
                tools::handle_create_doc(
                    &self.db,
                    &self.embedder,
                    &self.config,
                    self.chunk_cache.clone(),
                    &params.arguments,
                )
                .await?
            }
            "ragmcp_update_doc" => {
                tools::handle_update_doc(
                    &self.db,
                    &self.embedder,
                    &self.config,
                    self.chunk_cache.clone(),
                    &params.arguments,
                )
                .await?
            }
            _ => {
                return Ok(JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: id.clone().into(),
                    payload: JsonRpcResponsePayload::Error {
                        error: JsonRpcError {
                            code: error_codes::INVALID_PARAMS,
                            message: format!("Unknown tool: {}", params.name),
                            data: None,
                        },
                    },
                });
            }
        };

        Ok(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: id.clone().into(),
            payload: JsonRpcResponsePayload::Result {
                result: serde_json::to_value(&result)
                    .map_err(|e| RagmcpError::Config(format!("JSON serialization error: {}", e)))?,
            },
        })
    }

    /// Handle shutdown request
    async fn handle_shutdown(&self, id: &JsonRpcId) -> Result<JsonRpcResponse> {
        Ok(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: id.clone().into(),
            payload: JsonRpcResponsePayload::Result {
                result: serde_json::json!(null),
            },
        })
    }

    /// Create error response
    fn handle_error(
        &self,
        id: &JsonRpcId,
        code: i32,
        message: &str,
    ) -> Result<JsonRpcResponse> {
        Ok(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: id.clone().into(),
            payload: JsonRpcResponsePayload::Error {
                error: JsonRpcError {
                    code,
                    message: message.to_string(),
                    data: None,
                },
            },
        })
    }
}

/// Send JSON-RPC response to stdout (newline-delimited)
async fn send_response(
    stdout: &mut tokio::io::Stdout,
    response: &JsonRpcResponse,
) -> Result<()> {
    let json = serde_json::to_string(response)
        .map_err(|e| RagmcpError::Config(format!("JSON serialization error: {}", e)))?;
    stdout.write_all(json.as_bytes()).await
        .map_err(|e| RagmcpError::Io(e))?;
    stdout.write_all(b"\n").await
        .map_err(|e| RagmcpError::Io(e))?;
    stdout.flush().await
        .map_err(|e| RagmcpError::Io(e))?;
    Ok(())
}

/// Extract ID from JSON line (for error handling)
fn extract_id_from_line(line: &str) -> Option<Value> {
    // Try to extract ID field from malformed JSON
    if let Some(id_start) = line.find(r#""id":"#) {
        let id_str = &line[id_start + 5..];
        if let Some(id_end) = id_str.find(',') {
            let id_val = id_str[..id_end].trim();
            if id_val.starts_with('"') && id_val.ends_with('"') {
                return Some(Value::String(id_val[1..id_val.len()-1].to_string()));
            } else if let Ok(num) = id_val.parse::<i64>() {
                return Some(Value::Number(num.into()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_id_from_line() {
        // Test string ID
        let line = r#"{"jsonrpc":"2.0","id":"test-123","method":"test"}"#;
        let id = extract_id_from_line(line);
        assert!(id.is_some());
        if let Some(Value::String(s)) = id {
            assert_eq!(s, "test-123");
        }

        // Test numeric ID
        let line = r#"{"jsonrpc":"2.0","id":42,"method":"test"}"#;
        let id = extract_id_from_line(line);
        assert!(id.is_some());
        if let Some(Value::Number(n)) = id {
            assert_eq!(n.as_i64(), Some(42));
        }
    }

    #[test]
    fn test_json_rpc_request_parsing() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let request: std::result::Result<JsonRpcRequest, _> = serde_json::from_str(json);
        assert!(request.is_ok());
        let request = request.unwrap();
        assert_eq!(request.method, "initialize");
        assert_eq!(request.jsonrpc, "2.0");
    }
}
