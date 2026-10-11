# Deploy RAGMcp with Cloudflare R2 (portable server)

Optional R2 backend for free Railway/Render instances. Local `config.toml` stays local-only.

## Seed the bucket (docs only)

Upload a **read-only copy of documents** into the R2 bucket root (same relative layout as `rag_folder`).

**Do not** seed the SQLite database — the server builds/uploads `_ragmcp/ragmcp.db` itself.

```bash
# rclone example (configure an R2 remote first)
rclone copy ./docs r2:your-bucket/ --exclude "_ragmcp/**"

# or AWS CLI against the R2 S3 endpoint
aws s3 sync ./docs s3://your-bucket/ \
  --endpoint-url https://<accountid>.r2.cloudflarestorage.com
```

## Environment variables

| Variable | Required | Purpose |
|---|---|---|
| `OPENAI_API_KEY` | Yes | Embeddings |
| `RAGMCP_API_KEY` | Yes | OAuth access token / HTTP bearer |
| `RAGMCP_AUTH_PASSWORD` | Yes (public) | Password gate on `GET/POST /authorize` |
| `R2_ACCESS_KEY_ID` | Yes | R2 API token access key |
| `R2_SECRET_ACCESS_KEY` | Yes | R2 API token secret |
| `RAGMCP_CONFIG` | Optional | Path to server config (defaults `config.toml`) |
| `PORT` | Platform | Bind `0.0.0.0:$PORT` when set |
| `RUST_LOG` | Optional | Default `info` |

Use `config.server.toml.example` as the base server config (`backend = "r2"`, `cache_dir = "/tmp/docs"`, `db_path = "/tmp/ragmcp.db"`, `authless = false`).

## DNS

Create a **DNS-only** CNAME (orange cloud off) from `ragmcp.example.com` to the Railway/Render hostname. TLS terminates at the platform.

## Boot sequence

1. Process binds and serves `/health` immediately (`ready: false` while restoring).
2. Background: download `_ragmcp/ragmcp.db` if present → hydrate docs into `cache_dir` → if no snapshot, one ingest+embed then upload snapshot → `ready: true` → start watcher.
3. Clients should `GET /wake` (retry on 503 + `Retry-After: 5`) then connect to `/sse`.

## Local pull from R2

On a machine with `[storage.r2]` credentials (backend need not be `r2`):

```bash
ragmcp pull-r2
```

Copies **missing** objects into `rag_folder` only — never overwrites or deletes. Skips `_ragmcp/` keys.

## Docker

From `release/`:

```bash
docker build -t ragmcp-server .
docker run --env-file .env -e PORT=8080 -p 8080:8080 \
  -v "$(pwd)/config.server.toml:/app/config.toml" \
  -e RAGMCP_CONFIG=/app/config.toml \
  ragmcp-server
```
