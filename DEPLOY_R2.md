# Deploy RAGMcp on Render + Cloudflare R2 (beginner guide)

This guide deploys the **MCP server** so it stays available without your laptop.
Documents and a SQLite backup live in **Cloudflare R2**. The process runs on **Render**.

Your local laptop setup (local `config.toml`, local docs folder, Cloudflare Tunnel) is separate and can keep running.

---

## What you will have at the end

| Piece | Purpose |
|--------|---------|
| Cloudflare R2 bucket | Stores your documents + `_ragmcp/ragmcp.db` backup |
| Render Web Service | Runs `ragmcp serve-http` in Docker |
| Custom hostname (optional) | e.g. `ragmcp.example.com` → Render |
| Claude / Cursor | Connect to `https://YOUR-HOST/sse` after waking |

### Dashboard note (important)

The **dashboard** (`dashboard/`) is a Next.js app that reads a **local SQLite file** with `better-sqlite3`.

**Cloudflare Pages cannot host this dashboard as-is.** Pages is for static / edge sites. It cannot open your R2 SQLite backup the way the laptop dashboard does.

| Option | Works today? |
|--------|----------------|
| Run dashboard on your laptop against local `ragmcp.db` | Yes |
| Deploy MCP server on Render + R2 | Yes (this guide) |
| Host dashboard on Cloudflare Pages | No — needs a rewrite to call MCP HTTP APIs instead of SQLite |

Keep using the dashboard locally for now.

---

## Before you start — collect these values

Write them in a private notes file (not in git):

1. **R2 bucket name** — e.g. `ragmcp-docs`
2. **Cloudflare Account ID** — Cloudflare dashboard → R2 → overview (or any R2 bucket page)
3. **R2 Access Key ID** — from the API token you created
4. **R2 Secret Access Key** — from the same token (shown once)
5. **R2 endpoint** — build it like this (replace `ACCOUNT_ID`):

   ```text
   https://ACCOUNT_ID.r2.cloudflarestorage.com
   ```

6. **OpenAI API key** — from https://platform.openai.com/api-keys
7. **RAGMCP_API_KEY** — make a long random secret. On Windows PowerShell:

   ```powershell
   [Convert]::ToBase64String((1..32 | ForEach-Object { Get-Random -Maximum 256 }) -as [byte[]])
   ```

8. **RAGMCP_AUTH_PASSWORD** — a password you will type when Claude opens the authorize page
9. **Path to your docs folder** — the folder you already use for RAG (same layout you want in the bucket)
10. **GitHub** — this repo pushed so Render can build it (the folder that contains `Dockerfile`, usually this `release/` tree)

---

## Part A — Put your documents into R2 (one time)

Upload **documents only**. Do **not** upload `ragmcp.db`.

### Option A1 — Cloudflare dashboard (simplest, no CLI)

1. Open [Cloudflare Dashboard](https://dash.cloudflare.com) → **R2** → open your bucket.
2. Click **Upload**.
3. Upload files/folders so the **relative layout matches your docs root**  
   (top-level folders like `Agents/`, `System/`, etc. should appear at the bucket root).
4. Do **not** create or upload a folder named `_ragmcp` yet. The server creates that.

For many files, use Option A2.

### Option A2 — AWS CLI (recommended for a full folder)

1. Install [AWS CLI v2](https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html).
2. Open PowerShell and run (paste your real values):

```powershell
$env:AWS_ACCESS_KEY_ID = "YOUR_R2_ACCESS_KEY_ID"
$env:AWS_SECRET_ACCESS_KEY = "YOUR_R2_SECRET_ACCESS_KEY"
$env:AWS_DEFAULT_REGION = "auto"

aws s3 sync "D:\path\to\your\docs" s3://YOUR_BUCKET_NAME/ `
  --endpoint-url "https://YOUR_ACCOUNT_ID.r2.cloudflarestorage.com"
```

3. In the R2 bucket UI, confirm files appear. There should be **no** `ragmcp.db` at the root unless you put one there by mistake (delete it if so).

---

## Part B — Server config in the repo (placeholders only)

Render builds from git. **Leave** `config.server.toml.example` with placeholder `bucket` / `endpoint` — do **not** commit real R2 values.

At runtime the server reads:

| Source | What |
|--------|------|
| Env `R2_BUCKET` / `R2_ENDPOINT` | Real bucket + endpoint (**set these on Render**) |
| Env `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY` | R2 API credentials |
| TOML `[storage.r2]` | Placeholders + key env names + optional `snapshot_key` |

1. Keep `config.server.toml.example` as-is in git (placeholders are OK).
2. Leave `access_key_env` / `secret_key_env` as `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY`.
3. Keep `authless = false`.
4. Set real bucket/endpoint (and keys) as **Render Environment Variables** (Part C2). Optional: `R2_SNAPSHOT_KEY` if you do not use the default snapshot path.

**Optional:** a Render Secret File with a custom TOML is possible, but the primary path is: placeholders in git + env vars on the host.

Do **not** put Access Key, Secret Key, OpenAI key, passwords, or real bucket/endpoint in committed TOML.

---

## Part C — Deploy on Render

### C1 — Create the Web Service

1. Go to [https://dashboard.render.com](https://dashboard.render.com) and sign in.
2. **New +** → **Web Service**.
3. Connect the GitHub repository that contains this project’s `Dockerfile`.
4. Settings:

   | Field | Value |
   |--------|--------|
   | Name | e.g. `ragmcp` |
   | Region | Closest to you |
   | Branch | `main` (or your default) |
   | Root Directory | folder that contains `Dockerfile` (often `release` if the monorepo root is above this) |
   | Runtime | **Docker** |
   | Instance type | Free (or paid if you want fewer cold starts) |

5. If the repo root **is** this directory (Dockerfile at repo root), leave Root Directory empty.
6. Click **Advanced** → add Environment Variables (next section).

### C2 — Environment variables (Render → Environment)

Add each of these (no quotes):

| Key | Value |
|-----|--------|
| `OPENAI_API_KEY` | your OpenAI key |
| `RAGMCP_API_KEY` | the random key you generated |
| `RAGMCP_AUTH_PASSWORD` | your authorize password |
| `R2_BUCKET` | your R2 bucket name |
| `R2_ENDPOINT` | `https://YOUR_ACCOUNT_ID.r2.cloudflarestorage.com` |
| `R2_ACCESS_KEY_ID` | R2 token access key |
| `R2_SECRET_ACCESS_KEY` | R2 token secret |
| `RAGMCP_CONFIG` | `/app/config.server.toml.example` |
| `RUST_LOG` | `info` |

Optional: `R2_SNAPSHOT_KEY` (defaults to `_ragmcp/ragmcp.db` from the example TOML).

Render sets `PORT` automatically. Do not override it unless you know why.

### C3 — First deploy

1. Click **Create Web Service**.
2. Wait for the Docker build (Rust compile can take several minutes the first time).
3. When live, open the Render URL, e.g. `https://ragmcp-xxxx.onrender.com/health`.
4. You should see JSON including `"status":"ok"` and `"ready": true` or `false`.

### C4 — Wake the service

Free instances sleep when idle.

1. Open: `https://YOUR-RENDER-HOST/wake`
2. If you get **503**, wait 5–15 seconds and refresh until **200**.
3. First boot ever (no `_ragmcp/ragmcp.db` in R2 yet):
   - Downloads all docs from R2
   - Builds the search index (calls OpenAI once)
   - Uploads `_ragmcp/ragmcp.db` into the bucket  
   This can take several minutes. Later wakes are faster.

---

## Part D — Custom domain (optional)

Example: point `ragmcp.example.com` at Render.

1. In Render → your service → **Settings** → **Custom Domains** → add `ragmcp.example.com`.
2. In Cloudflare DNS for your domain:

   | Type | Name | Target | Proxy |
   |------|------|--------|--------|
   | CNAME | `ragmcp` (or as Render instructs) | `YOUR-SERVICE.onrender.com` | **DNS only** (grey cloud) |

   **Important:** leave the record **DNS only** (not proxied / orange cloud). Orange proxy can break MCP Server-Sent Events.

3. Wait for TLS to become active in Render.
4. Wake URL becomes: `https://ragmcp.example.com/wake`  
   MCP URL: `https://ragmcp.example.com/sse`

If you already use a Cloudflare Tunnel hostname for your laptop, use a **different** hostname for Render so both can run at once.

---

## Part E — Connect Claude

1. Wake the server (`/wake` → HTTP 200).
2. Claude → **Settings** → **Connectors** → **Add custom connector**.
3. URL: `https://YOUR-HOST/sse` (must end with `/sse`).
4. Leave Client ID / Secret empty (Claude registers automatically).
5. When the browser asks for a password, enter `RAGMCP_AUTH_PASSWORD`.
6. After connect, tools like `ragmcp_search` should appear.

If Claude says it cannot reach the server: wake again, wait for ready, retry.

### Cursor (or any client with a bearer token)

Send header:

```http
Authorization: Bearer YOUR_RAGMCP_API_KEY
```

Point the MCP HTTP URL at `https://YOUR-HOST/sse` or `/mcp` as your client expects.

---

## Part F — Day-to-day use

| Situation | What to do |
|-----------|------------|
| Server slept (free tier) | Open `/wake` until 200, then use Claude/Cursor |
| New docs created via MCP on the server | Stored in R2 automatically |
| Want those new files on your laptop | On the laptop: set R2 env vars + `[storage.r2]` in config (backend need **not** be `r2`), run `ragmcp pull-r2` |
| New docs only on laptop | Upload/sync them to R2 again (Part A), then restart/wake server so it hydrates |
| Redeploy on Render | Normal — next boot restores from R2 snapshot |

---

## Part G — Pull new server docs to your laptop

On the machine where your local docs folder lives:

1. Ensure `.env` (or the environment) has `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, plus `R2_BUCKET` and `R2_ENDPOINT` (or real values in `[storage.r2]`).
2. Ensure config has an `[storage.r2]` section (placeholders OK if env overrides are set). You can keep `backend` unset / not `r2` so normal local serve stays local-only.
3. From this project directory:

```bash
ragmcp pull-r2
```

This copies **only files that do not already exist** locally. It never overwrites or deletes. It skips `_ragmcp/`.

---

## Part H — Checklist

- [ ] Docs uploaded to R2 (no `ragmcp.db` seed)
- [ ] `config.server.toml.example` keeps placeholders in git (no real bucket/endpoint committed)
- [ ] Render Web Service (Docker) created
- [ ] Env vars set on Render including `R2_BUCKET`, `R2_ENDPOINT`, and R2 keys
- [ ] `/health` returns ok
- [ ] `/wake` returns 200
- [ ] R2 bucket now contains `_ragmcp/ragmcp.db` after first successful boot
- [ ] Custom domain DNS-only (if used)
- [ ] Claude connector URL ends with `/sse`
- [ ] Dashboard still used **locally** (not on Cloudflare Pages)

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| Build fails on Render | Confirm Root Directory points at the folder with `Dockerfile`; check build logs |
| `/health` connection refused | Service still deploying or crashed — open Render **Logs** |
| `/wake` stuck on 503 | Check logs for R2 auth errors; verify `R2_BUCKET`, `R2_ENDPOINT`, and Access Key |
| Claude OAuth with no password prompt | Set `RAGMCP_AUTH_PASSWORD` on Render and redeploy |
| 401 on `/sse` | Expected without Bearer / before OAuth completes |
| SSE hangs / never connects via custom domain | Cloudflare proxy must be **off** (DNS only) |
| First boot very slow | Normal while embedding; watch logs for progress |
| Dashboard on Pages blank/errors | Not supported with current architecture — use local dashboard |

---

## Docker reference (local test)

```bash
docker build -t ragmcp-server .
docker run --rm -e PORT=8080 -p 8080:8080 \
  -e OPENAI_API_KEY=... \
  -e RAGMCP_API_KEY=... \
  -e RAGMCP_AUTH_PASSWORD=... \
  -e R2_BUCKET=... \
  -e R2_ENDPOINT=https://YOUR_ACCOUNT_ID.r2.cloudflarestorage.com \
  -e R2_ACCESS_KEY_ID=... \
  -e R2_SECRET_ACCESS_KEY=... \
  -e RAGMCP_CONFIG=/app/config.server.toml.example \
  ragmcp-server
```

Then open `http://localhost:8080/wake`.
