# RAG MCP - Retrieval Language Model MCP Server

A high-performance Rust-based RAG Model Context Protocol (MCP) server that provides LLMs with intelligent access to your documentation through hybrid search combining BM25 full-text search and vector embeddings.

---

## ⚠️ Security Setup Required

Before running RAG MCP, configure the following **environment variables** in your `.env` file (copy from `.env.example`):

| Variable | Required | Description |
|---|---|---|
| `OPENAI_API_KEY` | Yes | Your OpenAI API key for embeddings |
| `RAGMCP_API_KEY` | Yes (HTTP mode) | Secret API key for HTTP transport authentication |
| `ADMIN_USERNAME` | Yes (dashboard) | Dashboard login username |
| `ADMIN_PASSWORD` | Yes (dashboard) | Dashboard login password |

**Generate strong secrets:**
```bash
# Generate a secure API key
openssl rand -base64 32

# Or on Windows PowerShell
[System.Convert]::ToBase64String([System.Security.Cryptography.RandomNumberGenerator]::GetBytes(32))
```

---

## Features

### Core Search Capabilities
- **Hybrid Search**: Combines BM25 (lexical) and vector (semantic) search using Reciprocal Rank Fusion (RRF)
- **RAG-Optimized**: Adaptive thresholding, comprehensive recall, namespace filtering, natural language query support
- **Local-First**: SQLite-based with zero external dependencies after setup
- **High Performance**: <1s P95 latency, optimized Rust implementation
- **Advanced RAG Support**: Optional `overfetch` parameter for fetching larger candidate sets

### Write Capabilities
- **Document Creation**: Create new documents via MCP with automatic indexing and embedding generation
- **Document Updates**: Update existing documents with automatic re-chunking and re-embedding
- **Security**: Path validation prevents traversal attacks, all operations confined to `rag_folder`
- **Audit Trail**: Complete logging of all write operations with timestamps and context

### Management Dashboard
- **Real-Time Monitoring**: Web dashboard with live statistics and query logs
- **Index Statistics**: Track document count, chunk count, and embedding coverage
- **Namespace Visualization**: View distribution across documentation namespaces
- **Query Analytics**: Monitor recent searches with latency tracking and performance metrics
- **Health Indicators**: Color-coded status showing system health and embedding coverage

### Transport Options
- **MCP Integration**: Native integration with Claude Desktop via stdio transport
- **HTTP Endpoint**: Expose MCP server via HTTP for custom connectors (Claude/ChatGPT)
- **Public Access**: Optional Cloudflare Tunnel integration for secure public endpoint

---

## Quick Start

### Prerequisites

- Rust 1.71 or higher (`rustup update stable`)
- OpenAI API key (for embedding generation)
- Node.js 18+ (optional, for the visual dashboard only)

### Installation

```bash
# Clone repository
git clone <repository-url>
cd RAGMcp

# Copy and configure environment file
cp .env.example .env
# Edit .env: set OPENAI_API_KEY and RAGMCP_API_KEY

# Copy and configure the server config
cp config.toml.example config.toml
# Edit config.toml: set rag_folder to the path of your docs folder
# See "Setting Up Your Docs Directory" section below for guidance

# Build all binaries
cargo build --release

# (Optional) Install dashboard dependencies
cd dashboard && npm install && cd ..
```

### Configuration

Edit `config.toml` (copied from `config.toml.example`):

```toml
[ragmcp]
# Path to your docs root directory (absolute or relative)
# Top-level subdirectories automatically become searchable namespaces
# See "Setting Up Your Docs Directory" section for full guidance
rag_folder = "/path/to/your/docs"        # Linux / macOS
# rag_folder = "C:/Users/you/my-docs"   # Windows (forward slashes work)
db_path = "./ragmcp.db"
log_level = "info"

[embeddings]
provider = "openai"
model = "text-embedding-3-small"
api_key_env = "OPENAI_API_KEY"
batch_size = 100
dimensions = 1536

[search]
default_k = 5
min_score = 0.25
hybrid_bm25_weight = 0.5
hybrid_vector_weight = 0.5

[performance]
max_latency_ms = 1000
chunk_size_tokens = 300
chunk_overlap_tokens = 50
```

---

## Setting Up Your Docs Directory

RAGMcp is **document-agnostic** — it can index any directory of text-based files (Markdown, plain text, XML, YAML, JSON, etc.). The only required step is pointing `rag_folder` in `config.toml` at a local folder you control.

### What to put in your docs directory

RAGMcp works well for any knowledge base you want to query via Claude:

| Use Case | Example structure |
|---|---|
| AI agent prompt library | `Agents/my-agent/prompt.xml` |
| Software documentation | `Docs/api/endpoints.md` |
| Internal wiki / runbooks | `Runbooks/deploy.md`, `Runbooks/rollback.md` |
| Research notes | `Research/topic/notes.md` |
| Project specs / PRDs | `Specs/feature-x.md` |
| Configuration docs | `Config/server.yaml` |

You can use any folder structure — top-level directories become **namespaces** automatically.

### Supported file types

By default, RAGMcp ingests all common text-based formats:

```
.md   .txt   .xml   .yaml   .yml   .json   .toml   .rs   .py   .ts   .js
```

Binary files (images, PDFs, etc.) are automatically skipped.

### Step-by-step: configure your directory

**1. Create (or choose) a local folder:**

```bash
# Example: create a new docs folder
mkdir ~/my-rag-docs
cd ~/my-rag-docs

# Example: use an existing project wiki
# Any local folder works
```

**2. Set `rag_folder` in `config.toml`:**

```toml
[ragmcp]
# Absolute or relative path to your docs root directory
# Top-level subdirectories automatically become namespaces
rag_folder = "/home/you/my-rag-docs"       # Linux / macOS
# rag_folder = "C:/Users/you/my-rag-docs"  # Windows
```

**3. Organize your files (optional but recommended):**

```
my-rag-docs/
├── Guides/
│   ├── getting-started.md
│   └── advanced-usage.md
├── API/
│   └── endpoints.md
├── Architecture/
│   └── overview.md
└── readme.md
```

This gives you namespaces `guides`, `api`, `architecture`, which you can filter on in Claude:
> *"Search my API namespace for authentication endpoints"*

**4. Ingest your files** (see Usage below — this populates the SQLite database and generates embeddings).

### Important constraints

- The path must exist and be readable before running `ingest`.
- RAGMcp creates the SQLite database (`ragmcp.db`) in the current working directory (configurable via `db_path`).
- All document paths stored in the DB are **relative to `rag_folder`**, so you can safely move the root folder as long as you update `config.toml`.
- **Never point `rag_folder` at a network drive or cloud-synced folder** while the watcher is running — use a local folder for reliable inotify/FSEvent-based watching.

---

## Usage

### Step 1: Ingest Documents

```bash
# Ingest documents (incremental: only new/modified files)
cargo run --bin ingest

# Force full re-ingestion of all files
cargo run --bin ingest -- --force

# Remove stale documents no longer on disk
cargo run --bin ingest -- --cleanup
```

### Step 2: Generate Embeddings

```bash
# Generate embeddings (incremental: only new chunks)
cargo run --bin embed

# Re-embed all chunks
cargo run --bin embed -- --force
```

### Step 3: Test Search

```bash
# Run a hybrid search query
cargo run --bin search "your search query"

# With namespace filter
cargo run --bin search "query" --namespace agents

# With agent filter
cargo run --bin search "query" --agent_filter myagent
```

### Step 4: Start MCP Server

RAGMcp supports two transport modes:

#### Mode 1: Stdio Transport (for Desktop Apps)

```bash
cargo run --bin ragmcp serve
# or with the release binary:
./target/release/ragmcp serve
```

Configure your app (Claude Desktop example) (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "ragmcp": {
      "command": "/absolute/path/to/ragmcp",
      "args": ["serve"],
      "env": {
        "OPENAI_API_KEY": "sk-...",
        "RAGMCP_CONFIG": "/absolute/path/to/config.toml"
      }
    }
  }
}
```

See `claude_desktop_config.json.example` for a template.

#### Mode 2: HTTP Transport (Custom Connectors)

```bash
# Set RAGMCP_API_KEY in .env, then:
cargo run --bin ragmcp serve-http
# Server starts on http://localhost:8081
```

Test with curl:
```bash
curl http://localhost:8081/health
# {"status":"ok","service":"ragmcp","version":"1.0.0"}
```

For your favorite LLM custom connectors, use the SSE endpoint:
- **SSE**: `http://localhost:8081/sse`
- **POST**: `http://localhost:8081/mcp`
- **Discovery**: `http://localhost:8081/.well-known/mcp-server`

See `CLAUDE_CUSTOM_CONNECTOR_SETUP.md` for detailed connector configuration.

For public access, see `cloudflared-config.yaml.example` for Cloudflare Tunnel setup.

### Step 5: Watch Mode (auto-reindex on save)

```bash
# Watch docs directory and auto-reindex on file changes
cargo run --bin watch

# Custom debounce delay
cargo run --bin watch -- --debounce-ms 1000
```

### Step 6: Visual Dashboard (optional)

```bash
cd dashboard
cp .env.example .env.local
# Edit .env.local: set ADMIN_USERNAME, ADMIN_PASSWORD, DB_PATH

npm install
npm run dev
# Dashboard available at http://localhost:3001
```

---

## MCP Tools

### Read Tools

#### `ragmcp_search`
Hybrid search (BM25 + vector) across your documentation.

**Parameters**:
- `query` (required): Search query text (min 3 characters)
- `k` (optional, default: 5): Number of results (1-20)
- `namespace` (optional, default: "all"): Filter by namespace. Use `ragmcp_list` with `list_type=namespaces` to discover available values.
- `agent_filter` (optional): Filter by specific agent name
- `min_score` (optional, default: 0.25): Minimum relevance score (0-1)
- `overfetch` (optional, 1-100): Fetch raw fused results before score thresholding (advanced RAG use)

**Example**:
```json
{ "query": "how does authentication work", "namespace": "all", "k": 5 }
```

#### `ragmcp_get`
Retrieve a specific document by path.

**Parameters**:
- `doc_path` (required): Relative path from docs root
- `return_full_doc` (optional, default: false): Return full content or metadata only
- `sections` (optional): Specific sections to retrieve

#### `ragmcp_list`
Browse documentation structure.

**Parameters**:
- `list_type` (required): `"agents"` | `"system_docs"` | `"namespaces"` | `"doc_types"`
- `agent_name` (optional): Filter by agent name

#### `ragmcp_related`
Graph traversal over knowledge relationships. Relations are extracted during ingestion from content using arrow patterns (e.g. `"Agent-A → Agent-B"`).

**Parameters**:
- `entity` (required): Entity identifier (e.g. `agent:example`)
- `relation_types` (optional): Relation types to traverse (e.g. `["routes_to"]`); omit for all
- `max_depth` (optional, default: 1, max: 3): Traversal depth (hops)

#### `ragmcp_explain`
Meta-information and diagnostics.

**Parameters**:
- `explain_what` (required): `"index_stats"` | `"doc_info"` | `"freshness"`
- `doc_path` (optional): Required for `"doc_info"`

---

### Write Tools

#### `ragmcp_create_doc`
Create a new document with automatic indexing and embedding generation.

**Parameters**:
- `doc_path` (required): Relative path from docs root (e.g. `"System/new-doc.md"`)
- `content` (required): Full document content (Markdown, XML, YAML, or JSON)
- `doc_type` (optional): Document type string

**Behavior**: Validates path, creates file, parses/chunks, inserts into DB, generates embeddings, logs to audit table.

#### `ragmcp_update_doc`
Update existing document and refresh index.

**Parameters**:
- `doc_path` (required): Relative path of document to update
- `content` (required): New content (full replacement)

#### `ragmcp_delete_doc`
Delete document from filesystem and database.

**Parameters**:
- `doc_path` (required): Relative path of document to delete
- `confirm` (required): Must be `true` to confirm deletion

---

## Architecture

```
RAGMcp/
├── src/
│   ├── main.rs              # Entry point (serve / serve-http commands)
│   ├── lib.rs               # Library root
│   ├── config.rs            # Configuration loading (.env + config.toml)
│   ├── error.rs             # Error types (thiserror)
│   ├── db/                  # SQLite connection + migrations
│   ├── ingest/              # Document ingestion pipeline
│   │   ├── walker.rs        # File discovery
│   │   ├── metadata.rs      # Hash, namespace, agent extraction
│   │   ├── parsers/         # XML, YAML, JSON, Markdown parsers
│   │   ├── chunker.rs       # Semantic chunking with overlap
│   │   └── db_writer.rs     # Database insertion
│   ├── search/              # Search implementations
│   │   ├── bm25.rs          # FTS5 BM25 full-text search
│   │   ├── vector.rs        # Vector cosine similarity search
│   │   └── hybrid.rs        # Hybrid RRF fusion
│   ├── embeddings/          # OpenAI embedding client + storage
│   ├── mcp/                 # MCP server (stdio + HTTP transports)
│   │   ├── server.rs        # JSON-RPC stdio server
│   │   ├── http.rs          # axum HTTP+SSE transport
│   │   ├── tools.rs         # All 7 tool handlers
│   │   ├── roots.rs         # PathValidator (security)
│   │   └── audit.rs         # Write operation audit log
│   ├── graph/               # Knowledge graph extraction + traversal
│   ├── cache/               # In-memory embedding cache
│   ├── eval/                # Evaluation metrics (Precision@K, MRR)
│   └── watch/               # File watcher for automatic re-indexing
├── migrations/              # SQLite schema migrations (001–006)
├── dashboard/               # Next.js 15 visual dashboard
├── tests/                   # Integration tests
├── config.toml.example      # Configuration template
├── .env.example             # Environment variables template
└── eval_queries.json        # (Optional) Evaluation query dataset — create your own for your docs
```

**Technology Stack**:
- **Language**: Rust (edition 2021, min 1.71)
- **Database**: SQLite with FTS5 (BM25) — bundled via rusqlite
- **Embeddings**: OpenAI `text-embedding-3-small` (1536-dim)
- **Search**: Hybrid BM25 + vector with Reciprocal Rank Fusion (RRF K=60)
- **MCP Protocol**: Manual JSON-RPC 2.0 (stdio + HTTP+SSE transports)
- **HTTP Server**: axum 0.7 with tower middleware
- **Dashboard**: Next.js 15, React 19, TypeScript, Tailwind CSS, better-sqlite3

---

## Architecture Decisions

1. **rusqlite over sqlx**: 17-100% faster for SQLite workloads, synchronous API fits the use case
2. **log over tracing**: Lighter weight, no structured logging overhead required
3. **Manual MCP implementation**: Full control over protocol, no framework dependencies
4. **SQLite-based storage**: Single portable file, zero-config, ACID-compliant
5. **Brute-force vector search**: Acceptable for typical corpora (<50K chunks); abstracted for future migration to sqlite-vec

See `ADR.md` for full Architecture Decision Records.

---

## Development

```bash
# Run all tests
cargo test -- --test-threads=1

# Check code
cargo clippy

# Format code
cargo fmt

# Run with debug logging
RUST_LOG=debug cargo run --bin ragmcp serve

# Run evaluation suite
cargo run --bin eval
```

---

## Namespace System

Namespaces are automatically derived from the **top-level directory** of your `rag_folder`. Examples:

```
your-docs/
├── Guides/       → namespace: "guides"
├── API/          → namespace: "api"
├── Architecture/ → namespace: "architecture"
├── Agents/       → namespace: "agents"   (if you use AI agent prompt files)
└── readme.md     → namespace: "all"      (root-level files have no namespace)
```

Use `ragmcp_list` with `list_type=namespaces` to discover all available namespaces in your index.

---

## Performance Targets

| Metric | Target |
|---|---|
| Search latency P50 | < 500ms |
| Search latency P95 | < 1s |
| Cold start (full index) | < 30s |
| Incremental update | < 5s per document |
| Precision@5 | > 85% |
| Recall@10 | > 90% |

---

## License

MIT

---

## Contributing

Contributions welcome. Please:
1. Run `cargo fmt` and `cargo clippy` before submitting
2. Add tests for new functionality
3. Update `config.toml.example` if new config options are added
