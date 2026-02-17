# Claude Custom Connector Setup Guide

This guide explains how to configure RAGMcp as a custom connector for Claude Web.

## Overview of Transport Implementation

Based on the official Claude MCP documentation, the following are implemented:

1. **SSE Endpoint**: `/sse` for Server-Sent Events (required by Claude)
2. **Discovery Endpoint**: `/.well-known/mcp-server` and `/.well-known/mcp.json` for server discovery
3. **OAuth 2.0 Support**: OAuth 2.0 authorization code flow with PKCE:
   - `/.well-known/oauth-authorization-server` — OAuth discovery endpoint
   - `/authorize` — OAuth authorization endpoint
   - `/token` — OAuth token endpoint
4. **Status Code Fix**: `notifications/initialized` returns HTTP 202 Accepted (MCP spec requirement)
5. **Proper Transport**: Streamable HTTP transport with SSE support

## Endpoints

Replace `your-server.example.com` with your actual server hostname (local or via Cloudflare Tunnel).

- **SSE Endpoint**: `https://your-server.example.com/sse` ← use this in Claude Web
- **POST Endpoint**: `https://your-server.example.com/mcp`
- **OAuth Discovery**: `https://your-server.example.com/.well-known/oauth-authorization-server`
- **OAuth Authorize**: `https://your-server.example.com/authorize`
- **OAuth Token**: `https://your-server.example.com/token`
- **MCP Discovery**: `https://your-server.example.com/.well-known/mcp-server`
- **Health**: `https://your-server.example.com/health`

For local development, replace `https://your-server.example.com` with `http://localhost:8081`.

## Configuration in Claude Web

### Option 1: Authless Mode (Recommended — Simplest)

1. **Enable authless mode** in `config.toml`:
   ```toml
   [http_server]
   authless = true
   ```

2. **Start the HTTP server**:
   ```bash
   ./target/release/ragmcp serve-http
   ```

3. In Claude Web:
   - Go to **Settings → Connectors**
   - Click **"Add custom connector"**
   - Enter:
     - **URL**: `https://your-server.example.com/sse`
     - **Name**: `ragmcp` (or any name you prefer)
     - **Leave Client ID and Client Secret empty**
   - Click **"Connect"** and confirm
   - Enable the connector in your conversation

### Option 2: With OAuth Authentication

1. **Disable authless mode** in `config.toml`:
   ```toml
   [http_server]
   authless = false
   ```

2. **Set your API key** in `.env`:
   ```bash
   RAGMCP_API_KEY=<your-generated-api-key>
   ```
   Generate a key: `openssl rand -base64 32`

3. **Start the HTTP server**:
   ```bash
   ./target/release/ragmcp serve-http
   ```

4. In Claude Web:
   - Go to **Settings → Connectors**
   - Click **"Add custom connector"**
   - Enter:
     - **URL**: `https://your-server.example.com/sse`
     - **Name**: `ragmcp`
     - **Client ID**: `ragmcp-client` (fixed value)
     - **Client Secret**: Your `RAGMCP_API_KEY` value
   - Click **"Connect"** — Claude will redirect through OAuth flow automatically
   - Enable the connector in your conversation

### For Team/Enterprise

Follow the same steps, but the admin configures in **Admin Settings → Connectors**.

---

## Important Notes

1. **Use `/sse` endpoint**, not `/mcp` — Claude expects the SSE endpoint URL
2. **Authless Mode (Recommended)**: Simplest setup, no Client ID/Secret needed in Claude
3. **With Authentication**: If `authless = false`:
   - **Client ID**: `ragmcp-client` (fixed value)
   - **Client Secret**: Your `RAGMCP_API_KEY` value
4. **OAuth Flow** (when `authless = false`): Server implements OAuth 2.0 authorization code flow with PKCE:
   - Claude redirects to `/authorize` automatically
   - After authorization, Claude receives an authorization code
   - Code is exchanged for access token via `/token`
   - Access token = your `RAGMCP_API_KEY`, used as Bearer token
5. **HTTPS Required**: Claude requires HTTPS (use Cloudflare Tunnel or a reverse proxy for local setups)
6. **Status Codes**: Server returns 202 for `notifications/initialized` as required by MCP spec

---

## Cloudflare Tunnel Setup

For a public HTTPS endpoint without opening firewall ports, use Cloudflare Tunnel:

```bash
# Install cloudflared
# https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/get-started/

# Authenticate
cloudflared tunnel login

# Create tunnel
cloudflared tunnel create ragmcp-tunnel

# Copy the example config and fill in your tunnel UUID and hostname
cp cloudflared-config.yaml.example cloudflared-config.yaml
# Edit: set your tunnel UUID and your hostname

# Run the tunnel (server must already be running on port 8081)
cloudflared tunnel run ragmcp-tunnel
```

---

## Testing

After configuration, verify:

```bash
# Health check
curl https://your-server.example.com/health

# Discovery endpoint
curl https://your-server.example.com/.well-known/mcp-server

# Tools list (authenticated mode)
curl -X POST https://your-server.example.com/mcp \
  -H "Authorization: Bearer <your-api-key>" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

---

## Troubleshooting

| Symptom | Solution |
|---|---|
| Connection refused | Ensure `ragmcp serve-http` is running |
| 401 Unauthorized | Check `RAGMCP_API_KEY` in `.env` matches what you used in Claude |
| SSL errors | Ensure HTTPS is configured (Cloudflare Tunnel or reverse proxy) |
| Tools not showing | Verify server version; check `/health` response |
| Claude can't discover server | Test `/.well-known/mcp-server` with curl |

Check server logs for detailed error messages:
```bash
RUST_LOG=info ./target/release/ragmcp serve-http
```

---

## Credentials Summary

- **Client ID**: `ragmcp-client` (fixed value, same for all RAGMcp instances)
- **Client Secret**: Your `RAGMCP_API_KEY` value from the `.env` file

**Setting up your Client Secret:**
1. Generate a secure API key: `openssl rand -base64 32`
2. Add it to your `.env` file: `RAGMCP_API_KEY=<generated-key>`
3. Use that same value as the Client Secret when configuring Claude Web

---

## OAuth Flow Details

When configured with Client ID and Client Secret, Claude will:

1. **Discover OAuth endpoints** via `/.well-known/oauth-authorization-server`
2. **Redirect to `/authorize`** with OAuth parameters (client_id, redirect_uri, code_challenge, etc.)
3. **Receive authorization code** after successful authorization
4. **Exchange code for access token** via `/token` endpoint
5. **Use access token** as Bearer token for all subsequent MCP requests

The returned access token is your `RAGMCP_API_KEY`, used for bearer token authentication on all MCP endpoints.
