use crate::config::Config;
use crate::db::Db;
use crate::embeddings::OpenAIEmbedder;
use crate::error::{Result, RagmcpError};
use crate::mcp::server::McpServer;
use crate::mcp::types::*;
use crate::storage::R2Client;
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode, Uri},
    response::{Html, IntoResponse, Response, sse::{Event, Sse}, Redirect},
    routing::{get, post},
    Json, Router, Form,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::convert::Infallible;
use std::collections::HashMap;
use std::sync::Mutex;
use futures_util::stream;
use tokio_stream::{StreamExt as TokioStreamExt, wrappers::IntervalStream};
use tokio::sync::mpsc;
use tower::ServiceBuilder;
use tower_http::cors::{Any, AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use sha2::{Sha256, Digest};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use uuid::Uuid;

/// Shared ready flag: false while R2 restore/hydrate runs; health always 200.
pub type ReadyState = Arc<AtomicBool>;

/// Check if a port is available by attempting to bind to it
async fn check_port_available(bind_host: &str, port: u16) -> bool {
    match tokio::net::TcpListener::bind(format!("{}:{}", bind_host, port)).await {
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
    ready: ReadyState,
}

impl HttpMcpServer {
    /// Create a new HTTP MCP server
    pub fn new(
        db: Db,
        embedder: OpenAIEmbedder,
        config: Config,
        chunk_cache: Option<std::sync::Arc<crate::cache::ChunkEmbeddingCache>>,
        pageindex: Option<std::sync::Arc<crate::pageindex::PageIndexManager>>,
    ) -> Result<Self> {
        // Local/default path: ready immediately (no R2 restore).
        Self::new_with_ready_r2(
            db,
            embedder,
            config,
            chunk_cache,
            pageindex,
            Arc::new(AtomicBool::new(true)),
            None,
        )
    }

    /// Create HTTP MCP server with explicit ready gate + optional R2 write-through.
    pub fn new_with_ready_r2(
        db: Db,
        embedder: OpenAIEmbedder,
        config: Config,
        chunk_cache: Option<std::sync::Arc<crate::cache::ChunkEmbeddingCache>>,
        pageindex: Option<std::sync::Arc<crate::pageindex::PageIndexManager>>,
        ready: ReadyState,
        r2: Option<Arc<R2Client>>,
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

        let server = Arc::new(McpServer::new_with_r2(
            db,
            embedder,
            config.clone(),
            chunk_cache,
            pageindex,
            r2,
        ));

        Ok(Self {
            server,
            api_key,
            allowed_origins: config.http_server.allowed_origins.clone(),
            config,
            ready,
        })
    }

    /// Run the HTTP server.
    /// When env `PORT` is set (Railway/Render), bind `0.0.0.0:$PORT`; else `127.0.0.1` + config port.
    pub async fn run(&self, port: u16) -> Result<()> {
        let app = self.create_router();

        // Platform PaaS inject PORT and require all-interfaces bind for health checks.
        let (bind_host, bind_port) = if let Ok(port_str) = std::env::var("PORT") {
            let p: u16 = port_str.parse().map_err(|_| {
                RagmcpError::Config(format!("Invalid PORT env value: {}", port_str))
            })?;
            ("0.0.0.0".to_string(), p)
        } else {
            ("127.0.0.1".to_string(), port)
        };

        let addr = format!("{}:{}", bind_host, bind_port);
        log::info!("Starting HTTP MCP server on http://{}", addr);
        log::info!("MCP endpoint: http://{}/mcp", addr);
        log::info!("Ready={}", self.ready.load(Ordering::SeqCst));

        // Check if port is available before attempting to bind
        if !check_port_available(&bind_host, bind_port).await {
            return Err(RagmcpError::Config(format!(
                "Port {} is already in use. Another process (possibly a previous ragmcp instance) is using this port.\n\
                To fix this:\n\
                1. Find the process: netstat -ano | findstr :{}\n\
                2. Kill it: taskkill /PID <pid> /F\n\
                3. Or use a different port by setting http_server.port in config.toml",
                bind_port, bind_port
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
                        bind_port, bind_port
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
        let ready = Arc::clone(&self.ready);

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

        // Core MCP routes: Streamable HTTP POST /mcp is primary; GET/POST /sse kept as legacy.
        // GET /mcp returns 405 + Allow: POST so misrouted clients get a clear hint.
        let mut router = Router::new()
            .route("/sse", get(handle_sse).post(handle_post))
            .route("/mcp", get(handle_mcp_method_not_allowed).post(handle_post))
            .route("/.well-known/mcp-server", get(handle_discovery))  // Discovery endpoint
            .route("/.well-known/mcp.json", get(handle_discovery))  // Alternative discovery endpoint
            .route("/health", get(handle_health))
            // Wake probe for cold-start platforms (no auth; 503 until ready)
            .route("/wake", get(handle_wake))
            .layer(
                ServiceBuilder::new()
                    .layer(TraceLayer::new_for_http())
                    .layer(cors)
            );

        // OAuth surface only when auth is required (authless=false).
        // Claude custom connectors discover PRM → AS metadata → DCR → authorize → token.
        if !authless {
            router = router
                // RFC 9728 PRM: root + /mcp (primary) + /sse (legacy) path forms
                .route("/.well-known/oauth-protected-resource", get(handle_protected_resource))
                .route("/.well-known/oauth-protected-resource/mcp", get(handle_protected_resource))
                .route("/.well-known/oauth-protected-resource/sse", get(handle_protected_resource))
                .route("/.well-known/oauth-authorization-server", get(handle_oauth_discovery))
                // GET shows password form when RAGMCP_AUTH_PASSWORD set; POST validates it
                .route("/authorize", get(handle_authorize_get).post(handle_authorize_post))
                .route("/token", post(handle_token))
                // RFC 7591 Dynamic Client Registration so Claude can auto-register
                .route("/register", post(handle_register));
        }

        router.with_state(AppState::new(server_state, api_key, allowed_origins, authless, ready))
    }
}

/// Fixed OAuth client ID shared by DCR responses and manual Claude Advanced settings.
const OAUTH_CLIENT_ID: &str = "ragmcp-client";

/// Claude hosted-surface redirect URIs (claude.ai and claude.com).
const CLAUDE_REDIRECT_URIS: &[&str] = &[
    "https://claude.ai/api/mcp/auth_callback",
    "https://claude.com/api/mcp/auth_callback",
];

/// Derive public issuer base URL from proxy/Host headers.
/// Prefer X-Forwarded-Proto + X-Forwarded-Host (Render/Cloudflare) so PRM URLs match the public origin.
fn issuer_from_headers(headers: &HeaderMap) -> String {
    // First value only — proxies may append a comma-separated chain.
    let forwarded_proto = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get("host"))
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match host {
        Some(host) => {
            let is_local = host.starts_with("localhost") || host.starts_with("127.0.0.1");
            let scheme = if let Some(proto) = forwarded_proto {
                proto
            } else if is_local {
                "http"
            } else {
                "https"
            };
            format!("{}://{}", scheme, host)
        }
        None => "http://localhost:8081".to_string(),
    }
}

/// MCP resource segment advertised in PRM / WWW-Authenticate (`mcp` primary, `sse` legacy).
/// Root `/.well-known/oauth-protected-resource` defaults to `mcp` (Streamable HTTP connector URL).
fn prm_resource_segment(request_path: &str) -> &'static str {
    // Path-form PRM …/oauth-protected-resource/sse or MCP route /sse → legacy SSE resource.
    if request_path.ends_with("/sse") || request_path.contains("/oauth-protected-resource/sse") {
        "sse"
    } else {
        "mcp"
    }
}

/// Pure gate order for MCP routes: unauthenticated → 401 before any ready/503 check.
/// Handlers mirror this order; unit tests lock the contract so cold-start never masks OAuth.
#[cfg_attr(not(test), allow(dead_code))]
fn mcp_gate_decision(authless: bool, authenticated: bool, ready: bool) -> &'static str {
    if !authless && !authenticated {
        "unauthorized"
    } else if !ready {
        "not_ready"
    } else {
        "ok"
    }
}

/// Standard OAuth token payload: access + refresh both equal RAGMCP_API_KEY (simplicity for Claude).
fn bearer_token_json(api_key: &str) -> serde_json::Value {
    serde_json::json!({
        "access_token": api_key,
        "refresh_token": api_key,
        "token_type": "Bearer",
        "expires_in": 3600,
        "scope": "claudeai"
    })
}

/// refresh_token grant: accept refresh_token == API key, re-issue the same bearer pair.
fn refresh_token_grant(
    refresh_token: &str,
    api_key: &str,
) -> std::result::Result<serde_json::Value, (&'static str, &'static str)> {
    if refresh_token.is_empty() || refresh_token != api_key {
        return Err((
            "invalid_grant",
            "Invalid refresh_token",
        ));
    }
    Ok(bearer_token_json(api_key))
}

/// Discovery document for /.well-known/mcp-server and mcp.json (Streamable HTTP primary).
fn discovery_document(authless: bool) -> serde_json::Value {
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
            "type": "streamable-http",
            "endpoint": "/mcp",
            "legacy_sse_endpoint": "/sse"
        }
    });

    if authless {
        discovery["authentication"] = serde_json::json!({ "type": "none" });
    } else {
        discovery["authentication"] = serde_json::json!({
            "type": "oauth2",
            "authorization_server": "/.well-known/oauth-authorization-server"
        });
    }
    discovery
}

/// True if redirect_uri is an allowed Claude MCP callback.
fn is_allowed_claude_redirect(redirect_uri: &str) -> bool {
    CLAUDE_REDIRECT_URIS.contains(&redirect_uri)
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
    /// False while R2 snapshot/hydrate is in progress (MCP routes return 503).
    ready: ReadyState,
}

/// OAuth authorization code data (PKCE flow).
/// `client_id` and `state` are stored but not yet validated at token exchange;
/// they are reserved for client-identity verification and CSRF protection
/// once the full OAuth token-exchange handler is implemented.
#[derive(Clone, Debug)]
struct AuthCodeData {
    #[allow(dead_code)]
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    #[allow(dead_code)]
    state: String,          // CSRF token — must be echoed back to client on redirect
    expires_at: chrono::DateTime<chrono::Utc>,
}

impl AppState {
    fn new(
        server: Arc<McpServer>,
        api_key: String,
        allowed_origins: Vec<String>,
        authless: bool,
        ready: ReadyState,
    ) -> Self {
        Self {
            server,
            api_key,
            allowed_origins,
            auth_codes: Arc::new(Mutex::new(HashMap::new())),
            authless,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            ready,
        }
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
}

/// 503 JSON used when MCP is still restoring from R2.
fn still_starting_response() -> Response {
    let mut resp = (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": "still starting",
            "message": "RAGMcp is restoring from R2. Retry shortly or poll GET /wake.",
            "ready": false
        })),
    )
        .into_response();
    resp.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from_static("5"),
    );
    resp
}

/// Handle POST requests (JSON-RPC) on /mcp (Streamable HTTP) and legacy /sse.
/// Auth + Origin run before the ready gate so cold-start never hides 401+PRM from Claude.
async fn handle_post(
    State(state): State<AppState>,
    uri: Uri,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    body: axum::body::Bytes,
) -> Response {
    let request_path = uri.path();

    // 1) Auth first when secured — unauthenticated always 401 + PRM (even if !ready).
    if !state.authless {
        if let Err(response) = validate_auth(&headers, &state.api_key, request_path) {
            return response;
        }
        if let Err(response) = validate_origin(&headers, &state.allowed_origins) {
            return response;
        }
    }

    // 2) Ready gate only after auth — authenticated (or authless) clients get 503 + Retry-After.
    if !state.is_ready() {
        return still_starting_response();
    }

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

    // HTTP requests are stateless — start uninitialized; do not enforce init across POSTs.
    let mut initialized = false;

    let session_id = params.get("session_id").cloned().unwrap_or_default();
    // Streamable HTTP primary: POST /mcp always returns JSON in the POST body.
    let prefer_sessionless_json = request_path == "/mcp";

    let method = request.method.clone();
    let result = state.server.process_mcp_request(request, &mut initialized).await;

    match result {
        Ok(Some(response)) => {
            if prefer_sessionless_json {
                return (StatusCode::OK, Json(response)).into_response();
            }
            // Legacy SSE path: push onto session channel when present.
            let sessions = state.sessions.lock().unwrap();
            if let Some(tx) = sessions.get(&session_id) {
                let _ = tx.send(response.clone());
                return StatusCode::ACCEPTED.into_response();
            }
            // No open SSE session — sessionless JSON fallback (Cursor Streamable probes).
            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(None) => {
            // Notifications: 202 for initialized, 204 otherwise (unchanged).
            if method == "notifications/initialized" {
                return StatusCode::ACCEPTED.into_response();
            }
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

/// Legacy SSE endpoint (deprecated compatibility). Prefer Streamable HTTP POST /mcp.
/// Auth before ready so unauthenticated cold-start probes still receive 401 + PRM.
async fn handle_sse(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    const SSE_PATH: &str = "/sse";

    // 1) Auth + Origin first when secured.
    if !state.authless {
        if let Err(response) = validate_auth(&headers, &state.api_key, SSE_PATH) {
            return response;
        }
        if let Err(response) = validate_origin(&headers, &state.allowed_origins) {
            return response;
        }
    }

    // 2) Ready gate after auth — do not open half-ready SSE sessions.
    if !state.is_ready() {
        return still_starting_response();
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
    
    // Cursor strictly JSON-parses SSE data; never emit comment-style ": keepalive"/": ping".
    // Named ping events with valid JSON data keep the TCP stream alive without breaking parsers.
    let mut interval_timer = interval(Duration::from_secs(15));
    interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let keepalive_stream = IntervalStream::new(interval_timer).map(move |_| {
        std::result::Result::<Event, Infallible>::Ok(
            Event::default().event("ping").data("{}"),
        )
    });

    // Combine: endpoint first, then interleave JSON-RPC message responses with pings
    let endpoint_stream = stream::once(async move {
        std::result::Result::<Event, Infallible>::Ok(endpoint_event)
    });

    use futures_util::stream::select;
    let merged_stream = select(response_stream, keepalive_stream);
    let combined_stream = endpoint_stream.chain(merged_stream);

    // Do not use axum KeepAlive — it emits SSE comments (`: ping`) that Cursor rejects.
    Sse::new(combined_stream).into_response()
}

/// Explicit 405 for GET /mcp — Streamable HTTP uses POST /mcp (legacy SSE remains on GET /sse).
async fn handle_mcp_method_not_allowed() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::ALLOW, "POST")],
        Json(serde_json::json!({
            "error": "Method Not Allowed",
            "hint": "Use Streamable HTTP POST /mcp for JSON-RPC (legacy GET /sse for SSE)"
        })),
    )
        .into_response()
}

/// Handle discovery endpoint (/.well-known/mcp-server or /.well-known/mcp.json)
async fn handle_discovery(State(state): State<AppState>) -> Response {
    (StatusCode::OK, Json(discovery_document(state.authless))).into_response()
}

/// Handle health check endpoint — always 200 (platform probes); includes ready flag.
async fn handle_health(State(state): State<AppState>) -> Response {
    let ready = state.is_ready();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "service": "ragmcp",
            "version": env!("CARGO_PKG_VERSION"),
            "ready": ready
        }))
    ).into_response()
}

/// Wake endpoint for cold-start platforms: no auth; 503 + Retry-After until ready.
async fn handle_wake(State(state): State<AppState>) -> Response {
    if !state.is_ready() {
        return still_starting_response();
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "awake",
            "ready": true,
            "service": "ragmcp",
            "version": env!("CARGO_PKG_VERSION")
        })),
    )
        .into_response()
}

/// RFC 9728 Protected Resource Metadata.
/// Root and `/mcp` PRM advertise `{issuer}/mcp`; `/sse` PRM keeps `{issuer}/sse`.
async fn handle_protected_resource(uri: Uri, headers: HeaderMap) -> Response {
    let issuer = issuer_from_headers(&headers);
    let segment = prm_resource_segment(uri.path());
    // resource must match the connector URL users paste into Claude (…/mcp primary).
    let resource = format!("{}/{}", issuer, segment);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "resource": resource,
            "authorization_servers": [issuer],
            "scopes_supported": ["claudeai"],
            "bearer_methods_supported": ["header"]
        }))
    ).into_response()
}

/// Handle OAuth discovery endpoint (/.well-known/oauth-authorization-server)
/// Returns OAuth 2.0 authorization server metadata per RFC 8414.
/// Includes registration_endpoint so Claude can perform Dynamic Client Registration.
async fn handle_oauth_discovery(
    headers: HeaderMap,
    State(_state): State<AppState>,
) -> Response {
    let issuer = issuer_from_headers(&headers);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{}/authorize", issuer),
            "token_endpoint": format!("{}/token", issuer),
            // RFC 7591 — Claude uses this when no Client ID is set in Advanced settings
            "registration_endpoint": format!("{}/register", issuer),
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "response_types_supported": ["code"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post", "none"],
            "scopes_supported": ["claudeai"]
        }))
    ).into_response()
}

/// RFC 7591 Dynamic Client Registration.
/// Returns the fixed public client so Claude can complete OAuth without a pre-filled Client ID.
async fn handle_register(
    Json(body): Json<serde_json::Value>,
) -> Response {
    // Prefer redirect_uris from the registration request when they are Claude callbacks;
    // otherwise fall back to the known Claude allowlist.
    let redirect_uris: Vec<String> = body
        .get("redirect_uris")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|u| u.as_str())
                .filter(|u| is_allowed_claude_redirect(u))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    let redirect_uris = if redirect_uris.is_empty() {
        CLAUDE_REDIRECT_URIS.iter().map(|s| (*s).to_string()).collect::<Vec<_>>()
    } else {
        redirect_uris
    };

    // Public client: Claude authenticates via PKCE only (token_endpoint_auth_method: none)
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "client_id": OAUTH_CLIENT_ID,
            "client_id_issued_at": chrono::Utc::now().timestamp(),
            "client_name": body.get("client_name").cloned().unwrap_or(serde_json::json!("Claude")),
            "redirect_uris": redirect_uris,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "code_challenge_methods": ["S256"]
        }))
    ).into_response()
}

/// Validate OAuth authorize query params (shared by GET/POST).
fn validate_authorize_params(
    params: &HashMap<String, String>,
) -> std::result::Result<(String, String, String, String, String), Response> {
    let client_id = params.get("client_id").cloned().unwrap_or_default();
    let redirect_uri = params.get("redirect_uri").cloned().unwrap_or_default();
    let oauth_state = params.get("state").cloned().unwrap_or_default();
    let code_challenge = params.get("code_challenge").cloned().unwrap_or_default();
    let code_challenge_method = params
        .get("code_challenge_method")
        .cloned()
        .unwrap_or_else(|| "S256".to_string());
    let response_type = params.get("response_type").cloned().unwrap_or_default();

    if client_id != OAUTH_CLIENT_ID {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_client",
                "error_description": "Invalid client_id"
            })),
        )
            .into_response());
    }
    if response_type != "code" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "unsupported_response_type",
                "error_description": "Only 'code' response type is supported"
            })),
        )
            .into_response());
    }
    if !is_allowed_claude_redirect(&redirect_uri) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_redirect_uri",
                "error_description": "Invalid redirect_uri"
            })),
        )
            .into_response());
    }

    Ok((
        client_id,
        redirect_uri,
        oauth_state,
        code_challenge,
        code_challenge_method,
    ))
}

/// Issue PKCE authorization code and redirect to Claude (current auto-approve path).
fn issue_authorize_redirect(
    app_state: &AppState,
    client_id: String,
    redirect_uri: String,
    oauth_state: String,
    code_challenge: String,
    code_challenge_method: String,
) -> Response {
    let auth_code = uuid::Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now() + chrono::Duration::minutes(10);

    let code_data = AuthCodeData {
        client_id,
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

    let mut redirect_url = url::Url::parse(&redirect_uri).unwrap();
    redirect_url
        .query_pairs_mut()
        .append_pair("code", &auth_code)
        .append_pair("state", &oauth_state);

    Redirect::to(redirect_url.as_str()).into_response()
}

fn html_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Simple password gate HTML when RAGMCP_AUTH_PASSWORD is set (public server).
fn authorize_password_form(params: &HashMap<String, String>) -> Response {
    let hidden: String = [
        "client_id",
        "redirect_uri",
        "state",
        "code_challenge",
        "code_challenge_method",
        "response_type",
        "scope",
    ]
    .iter()
    .filter_map(|k| {
        params.get(*k).map(|v| {
            format!(
                r#"<input type="hidden" name="{}" value="{}" />"#,
                k,
                html_escape_attr(v)
            )
        })
    })
    .collect::<Vec<_>>()
    .join("\n");

    let html = format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>RAGMcp Authorize</title>
<style>
body{{font-family:system-ui,sans-serif;max-width:360px;margin:4rem auto;padding:0 1rem}}
label{{display:block;margin-bottom:.5rem}}
input[type=password]{{width:100%;padding:.5rem;margin-bottom:1rem}}
button{{padding:.5rem 1rem}}
</style></head>
<body>
<h1>RAGMcp</h1>
<p>Enter the server password to authorize this connector.</p>
<form method="POST" action="/authorize">
{hidden}
<label for="password">Password</label>
<input id="password" name="password" type="password" required autofocus />
<button type="submit">Authorize</button>
</form>
</body></html>"#,
        hidden = hidden
    );
    (StatusCode::OK, Html(html)).into_response()
}

/// GET /authorize — password form when env set; otherwise auto-issue PKCE code (local/dev).
async fn handle_authorize_get(
    State(app_state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (client_id, redirect_uri, oauth_state, code_challenge, code_challenge_method) =
        match validate_authorize_params(&params) {
            Ok(v) => v,
            Err(r) => return r,
        };

    // Public server: require password gate when RAGMCP_AUTH_PASSWORD is configured.
    if std::env::var("RAGMCP_AUTH_PASSWORD").is_ok() {
        return authorize_password_form(&params);
    }

    // Local/dev: keep auto-redirect behavior when password env is unset.
    issue_authorize_redirect(
        &app_state,
        client_id,
        redirect_uri,
        oauth_state,
        code_challenge,
        code_challenge_method,
    )
}

/// POST /authorize — verify password (when set) then issue PKCE code.
async fn handle_authorize_post(
    State(app_state): State<AppState>,
    Form(params): Form<HashMap<String, String>>,
) -> Response {
    let (client_id, redirect_uri, oauth_state, code_challenge, code_challenge_method) =
        match validate_authorize_params(&params) {
            Ok(v) => v,
            Err(r) => return r,
        };

    if let Ok(expected) = std::env::var("RAGMCP_AUTH_PASSWORD") {
        let provided = params.get("password").map(|s| s.as_str()).unwrap_or("");
        // Constant-time-ish compare for equal-length strings; length mismatch fails closed.
        let ok = provided.len() == expected.len()
            && provided
                .as_bytes()
                .iter()
                .zip(expected.as_bytes().iter())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0;
        if !ok {
            return (
                StatusCode::UNAUTHORIZED,
                Html(
                    r#"<!DOCTYPE html><html><body>
<h1>Unauthorized</h1><p>Incorrect password.</p>
<p><a href="javascript:history.back()">Try again</a></p>
</body></html>"#
                        .to_string(),
                ),
            )
                .into_response();
        }
    }

    issue_authorize_redirect(
        &app_state,
        client_id,
        redirect_uri,
        oauth_state,
        code_challenge,
        code_challenge_method,
    )
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

    // Accept fixed Client ID from DCR or Claude Advanced settings (both grants).
    if client_id != OAUTH_CLIENT_ID {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_client",
                "error_description": "Invalid client_id"
            }))
        ).into_response();
    }

    // Public clients (DCR / token_endpoint_auth_method: none) send no secret — PKCE is the proof.
    // Confidential clients may send client_secret matching RAGMCP_API_KEY.
    let is_public_client = client_secret.is_empty();
    if !is_public_client && client_secret != app_state.api_key {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_client",
                "error_description": "Invalid client_secret"
            }))
        ).into_response();
    }

    // Claude may refresh with the same API-key refresh_token we issued.
    if grant_type == "refresh_token" {
        let refresh = params.get("refresh_token").cloned().unwrap_or_default();
        return match refresh_token_grant(&refresh, &app_state.api_key) {
            Ok(body) => (StatusCode::OK, Json(body)).into_response(),
            Err((error, description)) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": error,
                    "error_description": description
                })),
            )
                .into_response(),
        };
    }

    if grant_type != "authorization_code" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "unsupported_grant_type",
                "error_description": "Supported grant types: authorization_code, refresh_token"
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

    // Issue access + refresh tokens (both = API key) so Claude can refresh without re-auth.
    (StatusCode::OK, Json(bearer_token_json(&app_state.api_key))).into_response()
}

/// Build a 401 that points Claude at the path-specific PRM (RFC 9728).
/// `/mcp` challenges use `…/oauth-protected-resource/mcp`; `/sse` uses `…/sse`.
fn unauthorized_bearer(
    headers: &HeaderMap,
    request_path: &str,
    error: &str,
    message: &str,
) -> Response {
    let issuer = issuer_from_headers(headers);
    let metadata_url = unauthorized_prm_metadata_url(&issuer, request_path);
    let www = format!(
        "Bearer FAKESECRET_k1l2m3n4o5p6q7r8s9t0\"{}\", error=\"{}\"",
        metadata_url, error
    );

    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": error,
            "message": message
        })),
    )
        .into_response();

    if let Ok(value) = HeaderValue::from_str(&www) {
        response.headers_mut().insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

/// WWW-Authenticate metadata URL for a challenge path (testable without building a Response).
fn unauthorized_prm_metadata_url(issuer: &str, request_path: &str) -> String {
    format!(
        "{}/.well-known/oauth-protected-resource/{}",
        issuer,
        prm_resource_segment(request_path)
    )
}

/// Validate Authorization header; `request_path` selects which PRM URL goes in WWW-Authenticate.
fn validate_auth(
    headers: &HeaderMap,
    expected_key: &str,
    request_path: &str,
) -> std::result::Result<(), Response> {
    let auth_header = headers.get("authorization")
        .and_then(|h| h.to_str().ok());

    let auth_header = match auth_header {
        Some(h) => h,
        None => {
            return Err(unauthorized_bearer(
                headers,
                request_path,
                "invalid_token",
                "Missing Authorization header. Use 'Authorization: Bearer <token>' or complete OAuth.",
            ));
        }
    };

    if !auth_header.starts_with("Bearer ") {
        return Err(unauthorized_bearer(
            headers,
            request_path,
            "invalid_token",
            "Invalid Authorization header format. Use 'Authorization: Bearer <token>'.",
        ));
    }

    let provided_key = &auth_header[7..]; // Skip "Bearer "
    if provided_key != expected_key {
        return Err(unauthorized_bearer(
            headers,
            request_path,
            "invalid_token",
            "Invalid or expired access token.",
        ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn unready_without_auth_is_unauthorized_not_not_ready() {
        // Contract: cold-start must not return 503 before OAuth 401+PRM.
        assert_eq!(
            mcp_gate_decision(false, false, false),
            "unauthorized"
        );
        assert_eq!(mcp_gate_decision(false, true, false), "not_ready");
        assert_eq!(mcp_gate_decision(true, false, false), "not_ready");
        assert_eq!(mcp_gate_decision(false, true, true), "ok");
    }

    #[test]
    fn discovery_advertises_streamable_http_mcp() {
        let doc = discovery_document(false);
        assert_eq!(doc["transport"]["type"], "streamable-http");
        assert_eq!(doc["transport"]["endpoint"], "/mcp");
        assert_eq!(doc["transport"]["legacy_sse_endpoint"], "/sse");
        assert_eq!(doc["authentication"]["type"], "oauth2");
    }

    #[test]
    fn unauthorized_bearer_mcp_path_references_prm_mcp() {
        let url = unauthorized_prm_metadata_url("https://example.com", "/mcp");
        assert_eq!(
            url,
            "https://example.com/.well-known/oauth-protected-resource/mcp"
        );
        let sse_url = unauthorized_prm_metadata_url("https://example.com", "/sse");
        assert_eq!(
            sse_url,
            "https://example.com/.well-known/oauth-protected-resource/sse"
        );
    }

    #[test]
    fn prm_resource_selection_root_defaults_to_mcp() {
        assert_eq!(
            prm_resource_segment("/.well-known/oauth-protected-resource"),
            "mcp"
        );
        assert_eq!(
            prm_resource_segment("/.well-known/oauth-protected-resource/mcp"),
            "mcp"
        );
        assert_eq!(
            prm_resource_segment("/.well-known/oauth-protected-resource/sse"),
            "sse"
        );
    }

    #[test]
    fn issuer_prefers_x_forwarded_proto_and_host() {
        let headers = headers_with(&[
            ("x-forwarded-proto", "https"),
            ("x-forwarded-host", "ragmcp.example.com"),
            ("host", "localhost:8081"),
        ]);
        assert_eq!(issuer_from_headers(&headers), "https://ragmcp.example.com");
    }

    #[test]
    fn issuer_falls_back_to_host_when_no_forwarded() {
        let headers = headers_with(&[("host", "localhost:8081")]);
        assert_eq!(issuer_from_headers(&headers), "http://localhost:8081");
    }

    #[test]
    fn refresh_token_grant_returns_api_key_pair() {
        let api_key = "test-api-key-value";
        let body = refresh_token_grant(api_key, api_key).expect("refresh should succeed");
        assert_eq!(body["access_token"], api_key);
        assert_eq!(body["refresh_token"], api_key);
        assert_eq!(body["expires_in"], 3600);
        assert_eq!(body["token_type"], "Bearer");

        assert!(refresh_token_grant("wrong", api_key).is_err());
    }

    #[test]
    fn authorization_code_token_json_includes_refresh_token() {
        let body = bearer_token_json("k");
        assert_eq!(body["refresh_token"], "k");
        assert_eq!(body["access_token"], "k");
        assert_eq!(body["expires_in"], 3600);
    }
}
