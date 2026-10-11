# Multi-stage build for portable RAGMcp (R2 server mode on Railway/Render).
# Build context: release/ directory (or repo root with -f release/Dockerfile).

FROM rust:1.85-bookworm AS builder
WORKDIR /app

# Cache dependencies
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY migrations ./migrations

RUN cargo build --release --bin ragmcp

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/ragmcp /usr/local/bin/ragmcp
COPY migrations ./migrations
# Server config is mounted/copied at deploy time via RAGMCP_CONFIG
COPY config.server.toml.example ./config.server.toml.example

ENV RUST_LOG=info
# Railway/Render set PORT; binary binds 0.0.0.0:$PORT when present
EXPOSE 8080

CMD ["ragmcp", "serve-http"]
