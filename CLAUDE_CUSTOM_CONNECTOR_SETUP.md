# Claude Custom Connector Setup Guide

This guide explains how to configure RAGMcp as a custom connector for Claude Web.

**Active codebase**: all server code lives under `release/` (this directory).

## Overview of Transport Implementation

1. **SSE Endpoint**: `/sse` for Server-Sent Events (required by Claude)
2. **Discovery Endpoint**: `/.well-known/mcp-server` and `/.well-known/mcp.json`
3. **OAuth 2.1 (recommended / default for secured access)** when `authless = false`:
   - `/.well-known/oauth-protected-resource` (+ `/sse` path form) — RFC 9728 PRM
   - `/.well-known/oauth-authorization-server` — AS metadata with `registration_endpoint`
   - `POST /register` — RFC 7591 Dynamic Client Registration (DCR)
   - `GET /authorize` — authorization code + PKCE (S256), auto-approves for fixed client
   - `POST /token` — token exchange (public client PKCE, or confidential secret)
   - Unauthenticated `/sse` and `/mcp` return **401** with `WWW-Authenticate: Bearer resource_metadata=…`
4. **Status codes**: `notifications/initialized` → HTTP 202 Accepted
5. **Transport**: Streamable HTTP + SSE

## Endpoints

Replace `your-server.example.com` with your hostname (e.g. `ragmcp.nlightened.club`).

- **SSE (Claude URL)**: `https://your-server.example.com/sse`
- **POST**: `https://your-server.example.com/mcp`
- **MCP Discovery**: `https://your-server.example.com/.well-known/mcp-server`
- **Health**: `https://your-server.example.com/health`
- **PRM**: `https://your-server.example.com/.well-known/oauth-protected-resource`
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
   - **URL**: `https://your-server.example.com/sse`
   - **Name**: `ragmcp`
   - **Client ID / Secret**: leave empty (Claude uses DCR automatically)
   - Optional Advanced fallback: Client ID = `ragmcp-client`, Client Secret = your `RAGMCP_API_KEY`

5. Connect — Claude should complete OAuth (register → authorize redirect → token) then show tools.

### If you see registration errors

| Error | Likely cause | Fix |
|---|---|---|
| *Automatic client registration isn’t supported…* | AS metadata without `registration_endpoint` | Redeploy current build (includes `/register`) or set Client ID `ragmcp-client` |
| *Couldn't register with … sign-in service* | OAuth/DCR unreachable (authless 404, wrong URL, tunnel down) | Ensure `authless=false`, tunnel up, URL ends with `/sse`, `/register` returns 201 |
| *Couldn't reach the MCP server* | Origin/tunnel/WAF | Check `/health`, Cloudflare allowlist `160.79.104.0/21` if needed |

## Authless Mode (public — not recommended for production)

```toml
[http_server]
authless = true
```

No OAuth routes; MCP is open. Prefer OAuth mode above for secured access.

## Important Notes

1. Use **`/sse`**, not bare origin or `/mcp`, as the Claude connector URL
2. Access token issued by `/token` is your `RAGMCP_API_KEY` (used as Bearer on `/sse` and `/mcp`)
3. Redirect URIs allowed: `https://claude.ai/api/mcp/auth_callback` and `https://claude.com/api/mcp/auth_callback`
4. HTTPS required (Cloudflare Tunnel)

## Cloudflare Tunnel

```bash
cloudflared tunnel --config cloudflared-config.yaml run
```

Ingress should point `ragmcp.nlightened.club` → `http://localhost:8081`.

## Testing

```bash
# Health
curl https://your-server.example.com/health

# Unauthenticated SSE should 401 with WWW-Authenticate
curl -i https://your-server.example.com/sse

# PRM + AS
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
- **Access token**: same value as `RAGMCP_API_KEY`
