# Claude Custom Connector Setup Guide

This guide explains how to configure RAGMcp as a custom connector for Claude Web.

**Active codebase**: all server code lives under `release/` (this directory).

## Overview of Transport Implementation

1. **Streamable HTTP (primary)**: `POST /mcp` for JSON-RPC (sessionless `200 application/json` responses)
2. **Legacy SSE**: `GET /sse` / `POST /sse` kept for compatibility (deprecated for new Claude connectors)
3. **Discovery Endpoint**: `/.well-known/mcp-server` and `/.well-known/mcp.json` (`transport.type` = `streamable-http`, endpoint `/mcp`)
4. **OAuth 2.1 (recommended / default for secured access)** when `authless = false`:
   - `/.well-known/oauth-protected-resource` (+ `/mcp` and `/sse` path forms) — RFC 9728 PRM
   - `/.well-known/oauth-authorization-server` — AS metadata with `registration_endpoint`
   - `POST /register` — RFC 7591 Dynamic Client Registration (DCR)
   - `GET /authorize` — authorization code + PKCE (S256), auto-approves for fixed client
   - `POST /token` — authorization_code + refresh_token grants (tokens = `RAGMCP_API_KEY`)
   - Unauthenticated `/mcp` and `/sse` return **401** with `WWW-Authenticate: Bearer resource_metadata=…/oauth-protected-resource/mcp` (or `/sse`)
   - Auth runs **before** the ready gate: unauthenticated always gets 401+PRM even while restoring from R2
5. **Status codes**: requests → HTTP 200 JSON; `notifications/initialized` → HTTP 202; other notifications → 204

## Endpoints

Replace `your-server.example.com` with your hostname.

- **Claude connector URL**: `https://your-server.example.com/mcp`
- **Legacy SSE**: `https://your-server.example.com/sse`
- **MCP Discovery**: `https://your-server.example.com/.well-known/mcp-server`
- **Health**: `https://your-server.example.com/health`
- **PRM (default /mcp)**: `https://your-server.example.com/.well-known/oauth-protected-resource`
- **PRM (/mcp path)**: `https://your-server.example.com/.well-known/oauth-protected-resource/mcp`
- **OAuth AS**: `https://your-server.example.com/.well-known/oauth-authorization-server`
- **DCR**: `https://your-server.example.com/register`
- **Authorize / Token**: `/authorize`, `/token`

## Recommended: OAuth Mode (secured)

1. In `config.toml`:
   ```toml
   [http_server]
   authless = false
   allowed_origins = []   # Bearer is the gate; empty Origin allowlist avoids false 403s
   api_key_env = "RAGMCP_API_KEY"
   ```

2. In `.env`:
   ```bash
   RAGMCP_API_KEY=<openssl rand -base64 32>
   OPENAI_API_KEY=...
   ```

3. Start server + tunnel:
   ```bash
   cargo build --release
   ./target/release/ragmcp serve-http
   cloudflared tunnel --config cloudflared-config.yaml run
   ```

4. In Claude → **Settings → Connectors → Add custom connector**:
   - **URL**: `https://your-server.example.com/mcp`
   - **Name**: `ragmcp`
   - **Client ID / Secret**: leave empty (Claude uses DCR automatically)
   - Optional Advanced fallback: Client ID = `ragmcp-client`, Client Secret = your `RAGMCP_API_KEY`

5. Connect — Claude should complete OAuth (register → authorize redirect → token) then show tools.

### If you see registration errors

| Error | Likely cause | Fix |
|---|---|---|
| *Automatic client registration isn’t supported…* | AS metadata without `registration_endpoint` | Redeploy current build (includes `/register`) or set Client ID `ragmcp-client` |
| *Couldn't register with … sign-in service* | OAuth/DCR unreachable (authless 404, wrong URL, tunnel down) | Ensure `authless=false`, tunnel up, URL ends with `/mcp`, `/register` returns 201 |
| *Couldn't reach the MCP server* | Origin/tunnel/WAF | Check `/health`, Cloudflare allowlist `160.79.104.0/21` if needed |

## Authless Mode (public — not recommended for production)

```toml
[http_server]
authless = true
```

No OAuth routes; MCP is open. Prefer OAuth mode above for secured access.

## Important Notes

1. Use **`/mcp`**, not bare origin or `/sse`, as the Claude connector URL (Streamable HTTP)
2. Access token and refresh token issued by `/token` are your `RAGMCP_API_KEY` (Bearer on `/mcp` and `/sse`)
3. Redirect URIs allowed: `https://claude.ai/api/mcp/auth_callback` and `https://claude.com/api/mcp/auth_callback`
4. HTTPS required (Cloudflare Tunnel)

## Cloudflare Tunnel

```bash
cloudflared tunnel --config cloudflared-config.yaml run
```

Ingress should point `your-server.example.com` → `http://localhost:8081`.

## Testing

```bash
# Health
curl https://your-server.example.com/health

# Unauthenticated POST /mcp initialize should 401 with WWW-Authenticate → …/oauth-protected-resource/mcp
curl -i -X POST https://your-server.example.com/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'

# PRM for /mcp + AS
curl https://your-server.example.com/.well-known/oauth-protected-resource/mcp
curl https://your-server.example.com/.well-known/oauth-protected-resource
curl https://your-server.example.com/.well-known/oauth-authorization-server

# DCR
curl -s -X POST https://your-server.example.com/register \
  -H "Content-Type: application/json" \
  -d '{"redirect_uris":["https://claude.ai/api/mcp/auth_callback"],"token_endpoint_auth_method":"none"}'

# Authenticated tools/list
curl -X POST https://your-server.example.com/mcp \
  -H "Authorization: Bearer <RAGMCP_API_KEY>" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

## Credentials Summary

- **Client ID** (DCR + manual): `ragmcp-client`
- **Client Secret** (optional confidential mode): `RAGMCP_API_KEY`
- **Access token / refresh token**: same value as `RAGMCP_API_KEY`
