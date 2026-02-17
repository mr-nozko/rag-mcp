use crate::config::Config;
use crate::db::Db;
use crate::embeddings::OpenAIEmbedder;
use crate::error::{Result, RagmcpError};
use crate::mcp::server::McpServer;
use crate::mcp::types::*;
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response, sse::{Event, KeepAlive, Sse}, Redirect},
    routing::{get, post},
    Json, Router, Form,
};
use std::sync::Arc;
use std::convert::Infallible;
use std::collections::HashMap;
use std::sync::Mutex;
use futures_util::{Stream, stream};
use tokio_stream::{StreamExt as TokioStreamExt, wrappers::IntervalStream};
use tokio::sync::mpsc;
use tower::ServiceBuilder;
use tower_http::cors::{Any, AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use sha2::{Sha256, Digest};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use uuid::Uuid;

/// Check if a port is available by attempting to bind to it
async fn check_port_available(port: u16) -> bool {
    match tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await {
        Ok(_) => true,
        Err(_) => false,
    }
}

/// HTTP MCP Server wrapper
pub struct HttpMcpServer {
    server: Arc<McpServer>,
    api_key: String,
    allowed_origins: Vec<String>,
    config: Config,
}

impl HttpMcpServer {
    /// Create a new HTTP MCP server
    pub fn new(
        db: Db,
        embedder: OpenAIEmbedder,
        config: Config,
        chunk_cache: Option<std::sync::Arc<crate::cache::ChunkEmbeddingCache>>,
    ) -> Result<Self> {
        // API key is optional if authless mode is enabled
        let api_key = if config.http_server.authless {
            String::new() // Empty string for authless mode
        } else {
            std::env::var(&config.http_server.api_key_env)
                .map_err(|_| RagmcpError::Config(format!(
                    "Environment variable {} not set. Set it in your .env file or as an environment variable, or enable authless mode.",
                    config.http_server.api_key_env
                )))?
        };

        let server = Arc::new(McpServer::new(db, embedder, config.clone(), chunk_cache));

        Ok(Self {
            server,
            api_key,
            allowed_origins: config.http_server.allowed_origins.clone(),
            config,
        })
    }

    /// Run the HTTP server
    pub async fn run(&self, port: u16) -> Result<()> {
        let app = self.create_router();

        let addr = format!("127.0.0.1:{}", port);
        log::info!("Starting HTTP MCP server on http://{}", addr);
        log::info!("MCP endpoint: http://{}/mcp", addr);

        // Check if port is available before attempting to bind
        if !check_port_available(port).await {
            return Err(RagmcpError::Config(format!(
                "Port {} is already in use. Another process (possibly a previous ragmcp instance) is using this port.\n\
                To fix this:\n\
                1. Find the process: netstat -ano | findstr :{}\n\
                2. Kill it: taskkill /PID <pid> /F\n\
                3. Or use a different port by setting http_server.port in config.toml",
                port, port
            )));
        }

        // Try to bind to the address
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| {
                // Windows error 10048 = WSAEADDRINUSE (port already in use)
                let error_msg = if e.raw_os_error() == Some(10048) {
                    format!(
                        "Port {} is already in use. Another process (possibly a previous ragmcp instance) is using this port.\n\
                        To fix this:\n\
                        1. Find the process: netstat -ano | findstr :{}\n\
                        2. Kill it: taskkill /PID <pid> /F\n\
                        3. Or use a different port by setting http_server.port in config.toml",
                        port, port
                    )
                } else {
                    format!("Failed to bind to {}: {}", addr, e)
                };

                RagmcpError::Io(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    error_msg
                ))
            })?;

        axum::serve(listener, app)
            .await
            .map_err(|e| RagmcpError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("HTTP server error: {}", e)
            )))?;

        Ok(())
    }

    /// Create the axum router
    fn create_router(&self) -> Router {
        let server_state = Arc::clone(&self.server);
        let api_key = self.api_key.clone();
        let allowed_origins = self.allowed_origins.clone();
        let authless = self.config.http_server.authless;

        // Build CORS layer.
        // - If allowed_origins is configured: set it explicitly so preflight responses are consistent
        //   with the origin validation we do in request handlers.
        // - If empty (local dev / authless): allow Any for convenience.
        let cors = if allowed_origins.is_empty() {
            // No restriction configured — allow all origins (local dev or authless mode)
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        } else {
            // Restrict to explicitly configured origins so CORS preflight matches enforcement
            let origins: Vec<axum::http::HeaderValue> = allowed_origins
                .iter()
                .filter_map(|o| o.parse().ok())
                .collect();
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(origins))
                .allow_methods(Any)
                .allow_headers(Any)
        };

        Router::new()
            .route("/sse", get(handle_sse))  // SSE endpoint for Claude
            .route("/mcp", post(handle_post))  // POST endpoint for JSON-RPC requests
            .route("/.well-known/mcp-server", get(handle_discovery))  // Discovery endpoint
            .route("/.well-known/mcp.json", get(handle_discovery))  // Alternative discovery endpoint
            .route("/health", get(handle_health))
            .layer(
                ServiceBuilder::new()
                    .layer(TraceLayer::new_for_http())
                    .layer(cors)
            )
            .route("/.well-known/oauth-authorization-server", get(handle_oauth_discovery))
            .route("/authorize", get(handle_authorize))
            .route("/token", post(handle_token))
            .with_state(AppState::new(server_state, api_key, allowed_origins, authless))
    }
}

/// Application state shared across handlers
#[derive(Clone)]
struct AppState {
    server: Arc<McpServer>,
    api_key: String,
    allowed_origins: Vec<String>,
    auth_codes: Arc<Mutex<HashMap<String, AuthCodeData>>>,
    authless: bool,
    // Session management: map session ID to response sender
    sessions: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<JsonRpcResponse>>>>,
}

/// OAuth authorization code data
#[derive(Clone, Debug)]
struct AuthCodeData {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    state: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

impl AppState {
    fn new(server: Arc<McpServer>, api_key: String, allowed_origins: Vec<String>, authless: bool) -> Self {
        Self {
            server,
            api_key,
            allowed_origins,
            auth_codes: Arc::new(Mutex::new(HashMap::new())),
            authless,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Handle POST requests (JSON-RPC requests)
/// Per MCP spec, responses are sent via SSE message events, not POST response body.
async fn handle_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    body: axum::body::Bytes,
) -> Response {
    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid JSON: {}", e)})),
            )
                .into_response();
        }
    };
    // Validate authentication (skip if authless mode)
    if !state.authless {
        if let Err(response) = validate_auth(&headers, &state.api_key) {
            return response;
        }
    }

    // Validate Origin header (skip if authless mode)
    if !state.authless {
        if let Err(response) = validate_origin(&headers, &state.allowed_origins) {
            return response;
        }
    }

    // Process request (HTTP transport doesn't maintain initialization state per connection)
    // Each request is independent, so we track initialization per request session
    // For HTTP transport, we allow all operations without requiring initialization
    // since each request is stateless and independent
    // We start with initialized=false so initialize can succeed, but we don't enforce it
    let mut initialized = false; // HTTP requests are stateless - start false but don't enforce

    // Get session ID from query params
    let session_id = params.get("session_id").cloned().unwrap_or_default();
    
    let method = request.method.clone();
    let result = state.server.process_mcp_request(request, &mut initialized).await;
    
    match result {
        Ok(Some(response)) => {
            // Per MCP spec: send response via SSE message event, not POST response body
            // Find the session's response channel and send the response
            let sessions = state.sessions.lock().unwrap();
            if let Some(tx) = sessions.get(&session_id) {
                let _ = tx.send(response.clone());
            } else {
                // Fallback: if no session, send in POST response (for backwards compatibility)
                return (StatusCode::OK, Json(response)).into_response();
            }
            // Return 202 Accepted per MCP spec - response sent via SSE
            StatusCode::ACCEPTED.into_response()
        }
        Ok(None) => {
            // Notification - for notifications/initialized, return 202 Accepted per MCP spec
            if method == "notifications/initialized" {
                return StatusCode::ACCEPTED.into_response();
            }
            // Other notifications return 204 No Content
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            log::error!("Error processing MCP request: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Internal server error",
                    "details": e.to_string()
                }))
            ).into_response()
        }
    }
}

/// Handle SSE (Server-Sent Events) endpoint for Claude
/// This is the main endpoint Claude connects to for remote MCP servers
/// Claude uses this endpoint to receive MCP responses and notifications
async fn handle_sse(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    // Validate authentication and origin (skip if authless mode)
    // Note: For SSE, we continue even if validation fails to avoid breaking the stream
    // The POST endpoint will enforce validation for actual requests
    if !state.authless {
        let _ = validate_auth(&headers, &state.api_key);
        let _ = validate_origin(&headers, &state.allowed_origins);
    }

    // Per MCP spec (2024-11-05), SSE endpoint must immediately send an "endpoint" event
    // telling the client where to POST requests. Then send periodic keepalives.
    use tokio::time::{interval, Duration};
    
    // Generate session ID for this SSE connection
    let session_id = Uuid::new_v4().to_string();

    // Create channel for sending responses from POST handler to SSE stream
    let (tx, rx) = mpsc::unbounded_channel::<JsonRpcResponse>();
    
    // Store the sender in app state for POST handler to use
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions.insert(session_id.clone(), tx);
    }
    
    // First, send the endpoint event immediately when client connects
    // Per MCP spec, the data field must be a string URI with session ID
    // This tells Claude where to POST JSON-RPC requests
    let endpoint_uri = format!("/mcp?session_id={}", session_id);
    let endpoint_event = Event::default()
        .event("endpoint")
        .data(endpoint_uri);
    
    // Create stream for responses from POST handler (via channel)
    let response_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
        .map(|response| {
            // Send response as SSE message event per MCP spec
            let response_json = serde_json::to_string(&response).unwrap_or_default();
            std::result::Result::<Event, Infallible>::Ok(
                Event::default()
                    .event("message")
                    .data(response_json)
            )
        });
    
    // Then create a stream that sends periodic keepalive messages at proper intervals
    // Use tokio interval to throttle events properly (every 30 seconds)
    let mut interval_timer = interval(Duration::from_secs(30));
    interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    
    let keepalive_stream = IntervalStream::new(interval_timer)
        .map(move |_| {
            std::result::Result::<Event, Infallible>::Ok(Event::default().data(": keepalive"))
        });
    
    // Combine: send endpoint event first, then responses, then keepalives
    let endpoint_stream = stream::once(async move {
        std::result::Result::<Event, Infallible>::Ok(endpoint_event)
    });
    
    // Merge streams: endpoint first, then merge responses and keepalives
    // Use select to interleave response events with keepalives
    use futures_util::stream::select;
    let merged_stream = select(response_stream, keepalive_stream);
    let combined_stream = endpoint_stream.chain(merged_stream);

    // Configure keepalive to send pings every 15 seconds if no events
    let keepalive = KeepAlive::new()
        .interval(Duration::from_secs(15))
        .text("ping");

    Sse::new(combined_stream).keep_alive(keepalive)
}

/// Handle discovery endpoint (/.well-known/mcp-server or /.well-known/mcp.json)
/// Returns server metadata for Claude to discover capabilities
async fn handle_discovery(State(state): State<AppState>) -> Response {
    let mut discovery = serde_json::json!({
        "name": "ragmcp",
        "version": env!("CARGO_PKG_VERSION"),
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "ragmcp",
            "version": env!("CARGO_PKG_VERSION")
        },
        "transport": {
            "type": "sse",
            "endpoint": "/sse"
        }
    });

    // If authless mode, indicate no authentication required
    if state.authless {
        discovery["authentication"] = serde_json::json!({
            "type": "none"
        });
    }

    (StatusCode::OK, Json(discovery)).into_response()
}

/// Handle health check endpoint
async fn handle_health() -> Response {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "service": "ragmcp",
            "version": env!("CARGO_PKG_VERSION")
        }))
    ).into_response()
}

/// Handle OAuth discovery endpoint (/.well-known/oauth-authorization-server)
/// Returns OAuth 2.0 authorization server metadata per RFC 8414.
///
/// The issuer URL is derived dynamically from the incoming `Host` header so
/// this works correctly for any hostname (local, Cloudflare Tunnel, custom domain)
/// without requiring configuration.
async fn handle_oauth_discovery(
    headers: HeaderMap,
    State(_state): State<AppState>,
) -> Response {
    // Derive the issuer from the Host header sent by the client.
    // - Use HTTPS for any host that is not localhost/127.0.0.1 (likely public).
    // - Fall back to http://localhost:8081 if the header is missing (direct CLI calls).
    let issuer = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .map(|host| {
            let is_local = host.starts_with("localhost") || host.starts_with("127.0.0.1");
            if is_local {
                format!("http://{}", host)
            } else {
                format!("https://{}", host)
            }
        })
        .unwrap_or_else(|| "http://localhost:8081".to_string());

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{}/authorize", issuer),
            "token_endpoint": format!("{}/token", issuer),
            "grant_types_supported": ["authorization_code"],
            "response_types_supported": ["code"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post", "none"],
            "scopes_supported": ["claudeai"]
        }))
    ).into_response()
}

/// Handle OAuth authorization endpoint (/authorize)
/// Implements OAuth 2.0 authorization code flow with PKCE
async fn handle_authorize(
    State(app_state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    // Extract OAuth parameters
    let client_id = params.get("client_id").cloned().unwrap_or_default();
    let redirect_uri = params.get("redirect_uri").cloned().unwrap_or_default();
    let oauth_state = params.get("state").cloned().unwrap_or_default();
    let code_challenge = params.get("code_challenge").cloned().unwrap_or_default();
    let code_challenge_method = params.get("code_challenge_method").cloned().unwrap_or("S256".to_string());
    let response_type = params.get("response_type").cloned().unwrap_or_default();

    // Validate client_id
    if client_id != "ragmcp-client" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_client",
                "error_description": "Invalid client_id"
            }))
        ).into_response();
    }

    // Validate response_type
    if response_type != "code" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "unsupported_response_type",
                "error_description": "Only 'code' response type is supported"
            }))
        ).into_response();
    }

    // Validate redirect_uri (must be Claude's callback)
    if redirect_uri != "https://claude.ai/api/mcp/auth_callback" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_redirect_uri",
                "error_description": "Invalid redirect_uri"
            }))
        ).into_response();
    }

    // Generate authorization code
    let auth_code = uuid::Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now() + chrono::Duration::minutes(10);

    // Store authorization code
    let code_data = AuthCodeData {
        client_id: client_id.clone(),
        redirect_uri: redirect_uri.clone(),
        code_challenge,
        code_challenge_method,
        state: oauth_state.clone(),
        expires_at,
    };

    {
        let mut codes = app_state.auth_codes.lock().unwrap();
        codes.insert(auth_code.clone(), code_data);
    }

    // Redirect back to Claude with authorization code
    let mut redirect_url = url::Url::parse(&redirect_uri).unwrap();
    redirect_url.query_pairs_mut()
        .append_pair("code", &auth_code)
        .append_pair("state", &oauth_state);

    Redirect::to(redirect_url.as_str()).into_response()
}

/// Handle OAuth token endpoint (/token)
/// Exchanges authorization code for access token
async fn handle_token(
    State(app_state): State<AppState>,
    Form(params): Form<HashMap<String, String>>,
) -> Response {
    let grant_type = params.get("grant_type").cloned().unwrap_or_default();
    let code = params.get("code").cloned().unwrap_or_default();
    let client_id = params.get("client_id").cloned().unwrap_or_default();
    let client_secret = params.get("client_secret").cloned().unwrap_or_default();
    let code_verifier = params.get("code_verifier").cloned().unwrap_or_default();
    let redirect_uri = params.get("redirect_uri").cloned().unwrap_or_default();

    // Validate grant_type
    if grant_type != "authorization_code" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "unsupported_grant_type",
                "error_description": "Only 'authorization_code' grant type is supported"
            }))
        ).into_response();
    }

    // Validate client_id
    if client_id != "ragmcp-client" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_client",
                "error_description": "Invalid client_id"
            }))
        ).into_response();
    }

    // Validate client_secret matches API key
    if client_secret != app_state.api_key {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_client",
                "error_description": "Invalid client_secret"
            }))
        ).into_response();
    }

    // Retrieve and validate authorization code
    let code_data = {
        let mut codes = app_state.auth_codes.lock().unwrap();
        codes.remove(&code)
    };

    let code_data = match code_data {
        Some(data) => data,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_grant",
                    "error_description": "Invalid or expired authorization code"
                }))
            ).into_response();
        }
    };

    // Check expiration
    if code_data.expires_at < chrono::Utc::now() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "Authorization code expired"
            }))
        ).into_response();
    }

    // Validate redirect_uri matches
    if code_data.redirect_uri != redirect_uri {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "redirect_uri mismatch"
            }))
        ).into_response();
    }

    // Verify PKCE code_verifier
    if code_data.code_challenge_method == "S256" {
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let computed_challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());
        
        if computed_challenge != code_data.code_challenge {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_grant",
                    "error_description": "Invalid code_verifier (PKCE verification failed)"
                }))
            ).into_response();
        }
    }

    // Issue access token (for simplicity, we return the API key as the access token)
    // In production, you might want to generate a separate token
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "access_token": app_state.api_key,
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": "claudeai"
        }))
    ).into_response()
}

/// Validate Authorization header
fn validate_auth(headers: &HeaderMap, expected_key: &str) -> std::result::Result<(), Response> {
    let auth_header = headers.get("authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Missing Authorization header",
                    "message": "Use 'Authorization: Bearer <api-key>' header"
                }))
            ).into_response()
        })?;

    if !auth_header.starts_with("Bearer ") {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "Invalid Authorization header format",
                "message": "Use 'Authorization: Bearer <api-key>' header"
            }))
        ).into_response());
    }

    let provided_key = &auth_header[7..]; // Skip "Bearer "
    if provided_key != expected_key {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "Invalid API key"
            }))
        ).into_response());
    }

    Ok(())
}

/// Validate Origin header (prevents DNS rebinding attacks)
fn validate_origin(headers: &HeaderMap, allowed_origins: &[String]) -> std::result::Result<(), Response> {
    // If no origins are configured, allow all (for local development)
    if allowed_origins.is_empty() {
        return Ok(());
    }

    let origin = headers.get("origin")
        .and_then(|h| h.to_str().ok());

    // If no origin header, allow (direct requests, not browser)
    let origin = match origin {
        Some(o) => o,
        None => return Ok(()),
    };

    // Check if origin is in allowed list
    if allowed_origins.iter().any(|allowed| origin == allowed || origin.starts_with(&format!("{}://", allowed))) {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Origin not allowed",
                "message": format!("Origin '{}' is not in the allowed origins list", origin)
            }))
        ).into_response())
    }
}
