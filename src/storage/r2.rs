//! Minimal Cloudflare R2 (S3-compatible) client using SigV4 + reqwest.
//!
//! Kept dependency-light on purpose: no aws-sdk-s3. Supports list/get/put/delete
//! plus hydrate, pull-missing, and SQLite snapshot backup upload/download.

use crate::error::{RagmcpError, Result};
use crate::mcp::roots::PathValidator;
use hmac::{Hmac, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use quick_xml::events::Event;
use quick_xml::Reader;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

type HmacSha256 = Hmac<Sha256>;

/// Marker file that claims RAGMcp R2 ownership of cache_dir after the safety check.
/// Written before the download loop (not only after a fully successful hydrate) so a
/// mid-hydrate failure does not leave unmarked files that latch assert_cache_dir_safe.
/// Safety: refuse to use a non-empty cache_dir that lacks this marker.
pub const CACHE_MARKER_NAME: &str = ".ragmcp-r2-cache";

/// Marker payload — stable bytes so ownership is recognizable across retries.
const CACHE_MARKER_BYTES: &[u8] = b"ragmcp-r2-cache\n";

/// Default object key for the SQLite snapshot inside the bucket.
pub const DEFAULT_SNAPSHOT_KEY: &str = "_ragmcp/ragmcp.db";

/// Internal prefix — never treated as a user document during hydrate/pull.
pub const INTERNAL_PREFIX: &str = "_ragmcp/";

/// Encode URI path segments for SigV4 canonical requests (AWS rules).
const AWS_PATH_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~')
    .remove(b'/');

/// True when the object key is internal metadata (snapshot, etc.), not a doc.
pub fn is_internal_r2_key(key: &str) -> bool {
    key == INTERNAL_PREFIX.trim_end_matches('/') || key.starts_with(INTERNAL_PREFIX)
}

/// Refuse a non-empty cache directory that was not created by RAGMcp R2 hydrate.
/// Empty or missing dirs are OK (hydrate will create them).
pub fn assert_cache_dir_safe(cache_dir: &Path) -> Result<()> {
    if !cache_dir.exists() {
        return Ok(());
    }
    if !cache_dir.is_dir() {
        return Err(RagmcpError::Config(format!(
            "storage.cache_dir is not a directory: {}",
            cache_dir.display()
        )));
    }
    let marker = cache_dir.join(CACHE_MARKER_NAME);
    let mut entries = fs::read_dir(cache_dir).map_err(RagmcpError::Io)?;
    let has_any = entries.next().is_some();
    if has_any && !marker.exists() {
        return Err(RagmcpError::Config(format!(
            "storage.cache_dir {} is non-empty but missing {} marker. \
             Refusing to use a directory that may contain unrelated files. \
             Use an empty dir or remove contents first.",
            cache_dir.display(),
            CACHE_MARKER_NAME
        )));
    }
    Ok(())
}

/// Claim ownership of `cache_dir` so a mid-hydrate failure can still retry safely.
/// Idempotent: safe to call again after a successful hydrate (refreshes marker bytes).
pub fn write_cache_marker(cache_dir: &Path) -> Result<()> {
    fs::write(cache_dir.join(CACHE_MARKER_NAME), CACHE_MARKER_BYTES).map_err(RagmcpError::Io)
}

/// Counters for API-economy monitoring (list/get/put/delete).
#[derive(Debug, Default)]
pub struct R2CallCounters {
    pub list: AtomicU64,
    pub get: AtomicU64,
    pub put: AtomicU64,
    pub delete: AtomicU64,
}

impl R2CallCounters {
    pub fn snapshot(&self) -> (u64, u64, u64, u64) {
        (
            self.list.load(Ordering::Relaxed),
            self.get.load(Ordering::Relaxed),
            self.put.load(Ordering::Relaxed),
            self.delete.load(Ordering::Relaxed),
        )
    }

    pub fn log(&self, context: &str) {
        let (list, get, put, delete) = self.snapshot();
        log::info!(
            "R2 API counters [{}]: list={} get={} put={} delete={}",
            context, list, get, put, delete
        );
    }
}

/// S3-compatible R2 client (SigV4 signed).
#[derive(Clone)]
pub struct R2Client {
    bucket: String,
    endpoint: String,
    access_key: String,
    secret_key: String,
    snapshot_key: String,
    http: reqwest::Client,
    counters: Arc<R2CallCounters>,
}

impl R2Client {
    /// Build from explicit credentials (env already resolved by caller).
    pub fn new(
        bucket: String,
        endpoint: String,
        access_key: String,
        secret_key: String,
        snapshot_key: String,
    ) -> Result<Self> {
        let endpoint = endpoint.trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| RagmcpError::Config(format!("R2 HTTP client: {}", e)))?;
        Ok(Self {
            bucket,
            endpoint,
            access_key,
            secret_key,
            snapshot_key,
            http,
            counters: Arc::new(R2CallCounters::default()),
        })
    }

    pub fn counters(&self) -> Arc<R2CallCounters> {
        Arc::clone(&self.counters)
    }

    pub fn snapshot_key(&self) -> &str {
        &self.snapshot_key
    }

    /// List all object keys + sizes (paginated ListObjectsV2).
    pub async fn list_objects(&self) -> Result<Vec<(String, u64)>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            self.counters.list.fetch_add(1, Ordering::Relaxed);
            let mut query = vec![("list-type".to_string(), "2".to_string())];
            if let Some(ref t) = token {
                query.push(("continuation-token".to_string(), t.clone()));
            }
            let (status, body) = self
                .signed_request("GET", "", &query, None, b"")
                .await?;
            if !status.is_success() {
                return Err(RagmcpError::Config(format!(
                    "R2 ListObjectsV2 failed ({}): {}",
                    status.as_u16(),
                    String::from_utf8_lossy(&body)
                )));
            }
            let (keys, next) = parse_list_objects_v2(&body)?;
            out.extend(keys);
            match next {
                Some(t) if !t.is_empty() => token = Some(t),
                _ => break,
            }
        }
        self.counters.log("list_objects");
        Ok(out)
    }

    /// Download object bytes.
    pub async fn get_object(&self, key: &str) -> Result<Vec<u8>> {
        self.counters.get.fetch_add(1, Ordering::Relaxed);
        let (status, body) = self
            .signed_request("GET", key, &[], None, b"")
            .await?;
        if status.as_u16() == 404 {
            return Err(RagmcpError::DocumentNotFound(key.to_string()));
        }
        if !status.is_success() {
            return Err(RagmcpError::Config(format!(
                "R2 GetObject '{}' failed ({}): {}",
                key,
                status.as_u16(),
                String::from_utf8_lossy(&body)
            )));
        }
        Ok(body)
    }

    /// Upload object bytes (write-through / snapshot).
    pub async fn put_object(&self, key: &str, data: &[u8], content_type: &str) -> Result<()> {
        self.counters.put.fetch_add(1, Ordering::Relaxed);
        let (status, body) = self
            .signed_request("PUT", key, &[], Some(content_type), data)
            .await?;
        if !status.is_success() {
            return Err(RagmcpError::Config(format!(
                "R2 PutObject '{}' failed ({}): {}",
                key,
                status.as_u16(),
                String::from_utf8_lossy(&body)
            )));
        }
        Ok(())
    }

    /// Delete an object (write-through delete).
    pub async fn delete_object(&self, key: &str) -> Result<()> {
        self.counters.delete.fetch_add(1, Ordering::Relaxed);
        let (status, body) = self
            .signed_request("DELETE", key, &[], None, b"")
            .await?;
        // 204/200 success; 404 treated as already-gone success for idempotency
        if status.is_success() || status.as_u16() == 404 {
            return Ok(());
        }
        Err(RagmcpError::Config(format!(
            "R2 DeleteObject '{}' failed ({}): {}",
            key,
            status.as_u16(),
            String::from_utf8_lossy(&body)
        )))
    }

    /// Put a document at its relative path key.
    pub async fn put_document(&self, relative_path: &str, data: &[u8]) -> Result<()> {
        let key = normalize_object_key(relative_path);
        let ct = content_type_for_path(&key);
        self.put_object(&key, data, ct).await
    }

    /// Delete a document object by relative path.
    pub async fn delete_document(&self, relative_path: &str) -> Result<()> {
        let key = normalize_object_key(relative_path);
        self.delete_object(&key).await
    }

    /// Download SQLite snapshot to `db_path` if it exists. Returns true if downloaded.
    pub async fn download_snapshot(&self, db_path: &Path) -> Result<bool> {
        let key = self.snapshot_key.clone();
        match self.get_object(&key).await {
            Ok(bytes) => {
                if let Some(parent) = db_path.parent() {
                    fs::create_dir_all(parent).map_err(RagmcpError::Io)?;
                }
                // Write via temp then rename so a partial download cannot corrupt the live path.
                let tmp = db_path.with_extension("db.r2download");
                {
                    let mut f = fs::File::create(&tmp).map_err(RagmcpError::Io)?;
                    f.write_all(&bytes).map_err(RagmcpError::Io)?;
                    f.sync_all().map_err(RagmcpError::Io)?;
                }
                fs::rename(&tmp, db_path).map_err(RagmcpError::Io)?;
                log::info!(
                    "R2: downloaded snapshot {} ({} bytes) → {}",
                    key,
                    bytes.len(),
                    db_path.display()
                );
                self.counters.log("download_snapshot");
                Ok(true)
            }
            Err(RagmcpError::DocumentNotFound(_)) => {
                log::info!("R2: no snapshot at {} — will bootstrap from docs", key);
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// Upload SQLite snapshot via backup API to a temp file (never upload the live DB in place).
    pub async fn upload_snapshot(&self, db_path: &Path) -> Result<()> {
        let key = self.snapshot_key.clone();
        let db_path = db_path.to_path_buf();
        let tmp = db_path.with_extension("db.r2backup");

        // Blocking backup off the async runtime.
        let tmp_clone = tmp.clone();
        let src_path = db_path.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            use rusqlite::backup::Backup;
            use rusqlite::Connection;
            let src = Connection::open(&src_path).map_err(RagmcpError::Database)?;
            // Ensure parent exists for temp backup file
            if let Some(parent) = tmp_clone.parent() {
                fs::create_dir_all(parent).map_err(RagmcpError::Io)?;
            }
            let mut dst = Connection::open(&tmp_clone).map_err(RagmcpError::Database)?;
            {
                let backup = Backup::new(&src, &mut dst).map_err(RagmcpError::Database)?;
                backup
                    .run_to_completion(100, Duration::from_millis(10), None)
                    .map_err(RagmcpError::Database)?;
            }
            Ok(())
        })
        .await
        .map_err(|e| RagmcpError::Config(format!("R2 backup join: {}", e)))??;

        let bytes = fs::read(&tmp).map_err(RagmcpError::Io)?;
        let _ = fs::remove_file(&tmp);
        self.put_object(&key, &bytes, "application/octet-stream")
            .await?;
        log::info!(
            "R2: uploaded snapshot {} ({} bytes) from backup of {}",
            key,
            bytes.len(),
            db_path.display()
        );
        self.counters.log("upload_snapshot");
        Ok(())
    }

    /// Signed S3 request against path-style `{endpoint}/{bucket}/{key}`.
    async fn signed_request(
        &self,
        method: &str,
        key: &str,
        query: &[(String, String)],
        content_type: Option<&str>,
        payload: &[u8],
    ) -> Result<(reqwest::StatusCode, Vec<u8>)> {
        let now = chrono::Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date_stamp = now.format("%Y%m%d").to_string();
        let region = "auto";
        let service = "s3";

        let canonical_uri = if key.is_empty() {
            format!("/{}", self.bucket)
        } else {
            format!(
                "/{}/{}",
                self.bucket,
                utf8_percent_encode(key, AWS_PATH_ENCODE)
            )
        };

        // Sort query params for canonical query string
        let mut qpairs: Vec<(String, String)> = query
            .iter()
            .map(|(k, v)| {
                (
                    utf8_percent_encode(k, AWS_PATH_ENCODE).to_string(),
                    utf8_percent_encode(v, AWS_PATH_ENCODE).to_string(),
                )
            })
            .collect();
        qpairs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let canonical_query = qpairs
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");

        let payload_hash = hex::encode(Sha256::digest(payload));
        let host = self
            .endpoint
            .trim_start_matches("https://")
            .trim_start_matches("http://");

        let mut headers_to_sign: Vec<(String, String)> = vec![
            ("host".to_string(), host.to_string()),
            ("x-amz-content-sha256".to_string(), payload_hash.clone()),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];
        if let Some(ct) = content_type {
            headers_to_sign.push(("content-type".to_string(), ct.to_string()));
        }
        headers_to_sign.sort_by(|a, b| a.0.cmp(&b.0));

        let signed_headers = headers_to_sign
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(";");
        let canonical_headers = headers_to_sign
            .iter()
            .map(|(k, v)| format!("{}:{}\n", k, v.trim()))
            .collect::<String>();

        let canonical_request = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            method,
            canonical_uri,
            canonical_query,
            canonical_headers,
            signed_headers,
            payload_hash
        );
        let canonical_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
        let credential_scope = format!("{}/{}/{}/aws4_request", date_stamp, region, service);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date, credential_scope, canonical_hash
        );

        let signing_key = derive_signing_key(&self.secret_key, &date_stamp, region, service);
        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.access_key, credential_scope, signed_headers, signature
        );

        let url = if canonical_query.is_empty() {
            format!("{}{}", self.endpoint, canonical_uri)
        } else {
            format!("{}{}?{}", self.endpoint, canonical_uri, canonical_query)
        };

        let mut builder = match method {
            "GET" => self.http.get(&url),
            "PUT" => self.http.put(&url),
            "DELETE" => self.http.delete(&url),
            other => {
                return Err(RagmcpError::Config(format!(
                    "Unsupported R2 HTTP method: {}",
                    other
                )))
            }
        };

        builder = builder
            .header("host", host)
            .header("x-amz-content-sha256", &payload_hash)
            .header("x-amz-date", &amz_date)
            .header("authorization", authorization);

        if let Some(ct) = content_type {
            builder = builder.header("content-type", ct);
        }
        if method == "PUT" {
            builder = builder.body(payload.to_vec());
        }

        let resp = builder
            .send()
            .await
            .map_err(|e| RagmcpError::Config(format!("R2 request failed: {}", e)))?;
        let status = resp.status();
        let body = resp
            .bytes()
            .await
            .map_err(|e| RagmcpError::Config(format!("R2 body read: {}", e)))?
            .to_vec();
        Ok((status, body))
    }
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn derive_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{}", secret).as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

fn normalize_object_key(relative_path: &str) -> String {
    relative_path.replace('\\', "/").trim_start_matches('/').to_string()
}

fn content_type_for_path(key: &str) -> &'static str {
    match Path::new(key)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "md" | "markdown" | "txt" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

/// True when hydrate/pull should not write to `local` because it is already a directory.
/// Nested R2 docs (e.g. `Business/foo.md`) create `Business/` locally; a bare or
/// prefix key for the same name must not abort the whole hydrate with EISDIR.
pub(crate) fn should_skip_hydrate_path(local: &Path) -> bool {
    local.is_dir()
}

/// True when `fs::write` failed because the path is a directory (EISDIR).
/// Prefer raw_os_error 21 + message check — `ErrorKind::IsADirectory` needs rustc newer
/// than our MSRV (1.71).
fn is_eisdir_io_error(err: &std::io::Error) -> bool {
    if err.raw_os_error() == Some(21) {
        return true;
    }
    let msg = err.to_string().to_lowercase();
    msg.contains("is a directory") || msg.contains("eisdir")
}

/// Resolve a remote key to a local path under `root`, rejecting escapes / internal keys.
/// Returns Ok(None) when the key should be skipped (internal prefix or folder-marker key).
pub fn resolve_pull_path(root: &Path, key: &str) -> Result<Option<PathBuf>> {
    if is_internal_r2_key(key) {
        return Ok(None);
    }
    // S3/R2 "folder" markers are zero-byte keys ending in `/` — never download as files.
    // Writing them would collide with real dirs created by nested object keys.
    if key.ends_with('/') {
        return Ok(None);
    }
    if key.contains("..") || key.starts_with('/') || key.starts_with('\\') {
        return Err(RagmcpError::InvalidInput(format!(
            "Unsafe R2 object key rejected: {}",
            key
        )));
    }
    // PathValidator requires an existing root; caller must create/canonicalize first.
    let validator = PathValidator::new(root)?;
    let path = validator.validate_write_path(key)?;
    Ok(Some(path))
}

/// Hydrate documents from R2 into `cache_dir`. Never deletes local files.
/// Downloads missing or size-mismatched objects; claims ownership via marker before downloads.
pub async fn hydrate_docs(client: &R2Client, cache_dir: &Path) -> Result<usize> {
    // Refuse foreign/unmarked non-empty dirs before we claim or write anything.
    assert_cache_dir_safe(cache_dir)?;
    fs::create_dir_all(cache_dir).map_err(RagmcpError::Io)?;

    // Claim ownership BEFORE the download loop so a mid-hydrate failure leaves a marked
    // cache that assert_cache_dir_safe will accept on retry (avoids permanent latch).
    write_cache_marker(cache_dir)?;

    let objects = client.list_objects().await?;
    let mut downloaded = 0usize;

    for (key, size) in objects {
        if is_internal_r2_key(&key) {
            continue;
        }
        let local = match resolve_pull_path(cache_dir, &key)? {
            Some(p) => p,
            None => continue,
        };
        // Bare prefix keys (e.g. `Business`) resolve to a path that is already a dir
        // once nested docs created it — skip before size compare / get / write.
        if should_skip_hydrate_path(&local) {
            log::warn!(
                "R2 hydrate: skipping key '{}' — local path is a directory ({})",
                key,
                local.display()
            );
            continue;
        }
        let needs = if !local.exists() {
            true
        } else {
            match fs::metadata(&local) {
                Ok(m) => m.len() != size,
                Err(_) => true,
            }
        };
        if !needs {
            continue;
        }
        if let Some(parent) = local.parent() {
            fs::create_dir_all(parent).map_err(RagmcpError::Io)?;
        }
        // One missing/failed object must not brick entire hydrate (ready stays false).
        let bytes = match client.get_object(&key).await {
            Ok(b) => b,
            Err(e) => {
                log::warn!("R2 hydrate: get_object failed for '{}': {} — skipping", key, e);
                continue;
            }
        };
        // Guard again in case a concurrent mkdir raced between skip check and write.
        if should_skip_hydrate_path(&local) {
            log::warn!(
                "R2 hydrate: skipping write for '{}' — path became a directory ({})",
                key,
                local.display()
            );
            continue;
        }
        match fs::write(&local, &bytes) {
            Ok(()) => {
                downloaded += 1;
                log::debug!("R2 hydrate: wrote {} ({} bytes)", local.display(), bytes.len());
            }
            Err(e) if is_eisdir_io_error(&e) => {
                // EISDIR: do not abort hydrate — leave ready path open for remaining keys.
                log::warn!(
                    "R2 hydrate: write EISDIR for '{}' at {} — skipping: {}",
                    key,
                    local.display(),
                    e
                );
            }
            Err(e) => return Err(RagmcpError::Io(e)),
        }
    }

    // Idempotent refresh after successful hydrate — keeps end-of-path ownership explicit.
    write_cache_marker(cache_dir)?;

    log::info!(
        "R2 hydrate complete: {} object(s) downloaded into {}",
        downloaded,
        cache_dir.display()
    );
    client.counters().log("hydrate_docs");
    Ok(downloaded)
}

/// Pull missing objects into `rag_folder` (never overwrite / delete).
/// Prints copied / skipped / failed counts. Ignores `_ragmcp/` keys.
pub async fn pull_missing_to_folder(client: &R2Client, rag_folder: &Path) -> Result<()> {
    if !rag_folder.exists() {
        fs::create_dir_all(rag_folder).map_err(RagmcpError::Io)?;
    }
    // Ensure PathValidator can canonicalize
    let rag_folder = rag_folder
        .canonicalize()
        .map_err(|e| RagmcpError::Config(format!("Cannot canonicalize rag_folder: {}", e)))?;

    let objects = client.list_objects().await?;
    let mut copied = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for (key, _size) in objects {
        if is_internal_r2_key(&key) {
            skipped += 1;
            continue;
        }
        let local = match resolve_pull_path(&rag_folder, &key) {
            Ok(Some(p)) => p,
            Ok(None) => {
                skipped += 1;
                continue;
            }
            Err(e) => {
                eprintln!("FAIL  {} ({})", key, e);
                failed += 1;
                continue;
            }
        };
        // Directory collision (nested docs created the path) — same EISDIR class as hydrate.
        if should_skip_hydrate_path(&local) {
            println!("SKIP  {} (is directory)", key);
            skipped += 1;
            continue;
        }
        if local.exists() {
            println!("SKIP  {} (exists)", key);
            skipped += 1;
            continue;
        }
        match client.get_object(&key).await {
            Ok(bytes) => {
                if let Some(parent) = local.parent() {
                    if let Err(e) = fs::create_dir_all(parent) {
                        eprintln!("FAIL  {} (mkdir: {})", key, e);
                        failed += 1;
                        continue;
                    }
                }
                if should_skip_hydrate_path(&local) {
                    println!("SKIP  {} (is directory)", key);
                    skipped += 1;
                    continue;
                }
                match fs::write(&local, &bytes) {
                    Ok(()) => {
                        println!("COPY  {} → {}", key, local.display());
                        copied += 1;
                    }
                    Err(e) if is_eisdir_io_error(&e) => {
                        eprintln!("SKIP  {} (write EISDIR: {})", key, e);
                        skipped += 1;
                    }
                    Err(e) => {
                        eprintln!("FAIL  {} (write: {})", key, e);
                        failed += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("FAIL  {} ({})", key, e);
                failed += 1;
            }
        }
    }

    println!(
        "pull-r2 complete: copied={}, skipped={}, failed={}",
        copied, skipped, failed
    );
    client.counters().log("pull_missing_to_folder");
    if failed > 0 {
        return Err(RagmcpError::Config(format!(
            "pull-r2 finished with {} failure(s)",
            failed
        )));
    }
    Ok(())
}

fn parse_list_objects_v2(xml: &[u8]) -> Result<(Vec<(String, u64)>, Option<String>)> {
    let text = String::from_utf8_lossy(xml);
    let mut reader = Reader::from_str(&text);
    reader.config_mut().trim_text(true);

    let mut objects = Vec::new();
    let mut next_token: Option<String> = None;
    let mut in_contents = false;
    let mut in_key = false;
    let mut in_size = false;
    let mut in_next = false;
    let mut cur_key = String::new();
    let mut cur_size: u64 = 0;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                // Strip namespace prefix if present (ns:Contents → Contents)
                let local = name.rsplit(':').next().unwrap_or(&name);
                match local {
                    "Contents" => {
                        in_contents = true;
                        cur_key.clear();
                        cur_size = 0;
                    }
                    "Key" if in_contents => in_key = true,
                    "Size" if in_contents => in_size = true,
                    "NextContinuationToken" => in_next = true,
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                // quick-xml 0.39: decode text bytes (entities already expanded for S3 XML keys/sizes)
                let s = String::from_utf8_lossy(t.as_ref()).into_owned();
                if in_key {
                    cur_key = s;
                } else if in_size {
                    cur_size = s.parse().unwrap_or(0);
                } else if in_next {
                    next_token = Some(s);
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = name.rsplit(':').next().unwrap_or(&name);
                match local {
                    "Contents" => {
                        if !cur_key.is_empty() {
                            objects.push((cur_key.clone(), cur_size));
                        }
                        in_contents = false;
                    }
                    "Key" => in_key = false,
                    "Size" => in_size = false,
                    "NextContinuationToken" => in_next = false,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(RagmcpError::Parse(format!(
                    "ListObjectsV2 XML parse error: {}",
                    e
                )))
            }
            _ => {}
        }
        buf.clear();
    }

    Ok((objects, next_token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_ignore_internal_prefix() {
        assert!(is_internal_r2_key("_ragmcp/ragmcp.db"));
        assert!(is_internal_r2_key("_ragmcp/"));
        assert!(is_internal_r2_key("_ragmcp"));
        assert!(!is_internal_r2_key("Guides/readme.md"));
        assert!(!is_internal_r2_key("ragmcp.db"));
    }

    #[test]
    fn test_marker_safety_refuse_nonempty_without_marker() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("stray.txt"), b"x").unwrap();
        let err = assert_cache_dir_safe(tmp.path()).unwrap_err();
        assert!(err.to_string().contains(CACHE_MARKER_NAME));
    }

    #[test]
    fn test_marker_safety_ok_with_marker() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(CACHE_MARKER_NAME), b"ok").unwrap();
        fs::write(tmp.path().join("doc.md"), b"hi").unwrap();
        assert!(assert_cache_dir_safe(tmp.path()).is_ok());
    }

    #[test]
    fn test_marker_safety_ok_after_write_cache_marker_helper() {
        // Early ownership claim (same path hydrate_docs uses before downloads).
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("partial.md"), b"mid-hydrate").unwrap();
        write_cache_marker(tmp.path()).unwrap();
        assert!(assert_cache_dir_safe(tmp.path()).is_ok());
        let written = fs::read(tmp.path().join(CACHE_MARKER_NAME)).unwrap();
        assert_eq!(written, CACHE_MARKER_BYTES);
    }

    #[test]
    fn test_marker_latch_retry_after_ownership_claimed() {
        // Documents the pre-fix latch: unmarked non-empty dir fails forever until
        // ownership is claimed; once the marker exists, assert passes and hydrate can retry.
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("orphaned.md"), b"from failed hydrate").unwrap();
        let err = assert_cache_dir_safe(tmp.path()).unwrap_err();
        assert!(err.to_string().contains(CACHE_MARKER_NAME));
        write_cache_marker(tmp.path()).unwrap();
        assert!(assert_cache_dir_safe(tmp.path()).is_ok());
    }

    #[test]
    fn test_marker_safety_ok_empty_or_missing() {
        let tmp = TempDir::new().unwrap();
        assert!(assert_cache_dir_safe(tmp.path()).is_ok());
        let missing = tmp.path().join("does-not-exist-yet");
        assert!(assert_cache_dir_safe(&missing).is_ok());
    }

    #[test]
    fn test_pull_path_rejects_escape() {
        let tmp = TempDir::new().unwrap();
        let err = resolve_pull_path(tmp.path(), "../etc/passwd").unwrap_err();
        assert!(err.to_string().contains("Unsafe") || err.to_string().contains("traversal") || err.to_string().contains(".."));
    }

    #[test]
    fn test_pull_path_skips_internal() {
        let tmp = TempDir::new().unwrap();
        assert!(resolve_pull_path(tmp.path(), "_ragmcp/ragmcp.db")
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_pull_path_skips_trailing_slash_folder_marker() {
        // R2/S3 zero-byte prefix keys like `Guides/` must not map to a writable file path.
        let tmp = TempDir::new().unwrap();
        assert!(resolve_pull_path(tmp.path(), "Guides/")
            .unwrap()
            .is_none());
        assert!(resolve_pull_path(tmp.path(), "Business/")
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_should_skip_hydrate_path_when_local_is_dir() {
        // Documents the bare-prefix collision: nested `Business/doc.md` creates a dir;
        // a later key `Business` must be skipped or fs::write aborts hydrate (EISDIR).
        let tmp = TempDir::new().unwrap();
        let business = tmp.path().join("Business");
        fs::create_dir_all(&business).unwrap();
        assert!(should_skip_hydrate_path(&business));
        let file = tmp.path().join("readme.md");
        fs::write(&file, b"hi").unwrap();
        assert!(!should_skip_hydrate_path(&file));
        assert!(!should_skip_hydrate_path(&tmp.path().join("missing.md")));
    }

    #[test]
    fn test_is_eisdir_io_error_detects_os_21_and_message() {
        let from_os = std::io::Error::from_raw_os_error(21);
        assert!(is_eisdir_io_error(&from_os));
        let from_msg = std::io::Error::new(std::io::ErrorKind::Other, "Is a directory (os error 21)");
        assert!(is_eisdir_io_error(&from_msg));
        let other = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert!(!is_eisdir_io_error(&other));
    }

    #[test]
    fn test_pull_path_ok_relative() {
        let tmp = TempDir::new().unwrap();
        let p = resolve_pull_path(tmp.path(), "Guides/api.md").unwrap().unwrap();
        assert!(p.to_string_lossy().replace('\\', "/").ends_with("Guides/api.md"));
    }

    #[test]
    fn test_parse_list_objects_v2() {
        let xml = br#"<?xml version="1.0"?>
        <ListBucketResult>
          <Contents><Key>a.md</Key><Size>12</Size></Contents>
          <Contents><Key>_ragmcp/ragmcp.db</Key><Size>99</Size></Contents>
          <NextContinuationToken>tok123</NextContinuationToken>
        </ListBucketResult>"#;
        let (objs, next) = parse_list_objects_v2(xml).unwrap();
        assert_eq!(objs.len(), 2);
        assert_eq!(objs[0], ("a.md".to_string(), 12));
        assert_eq!(next.as_deref(), Some("tok123"));
    }
}
