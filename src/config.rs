use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Main configuration structure
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub ragmcp: RagmcpConfig,
    pub embeddings: EmbeddingsConfig,
    pub search: SearchConfig,
    pub performance: PerformanceConfig,
    #[serde(default)]
    pub http_server: HttpServerConfig,
    #[serde(default)]
    pub pageindex: PageIndexConfig,
    /// Optional remote storage (R2). Absent / backend!=r2 → local mode unchanged.
    #[serde(default)]
    pub storage: Option<StorageConfig>,
}

/// Optional `[storage]` section for portable R2 server mode.
#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    /// `"r2"` enables R2 hydrate/write-through; `"local"` or absent → local disk only.
    #[serde(default)]
    pub backend: String,
    /// Local cache directory used as effective rag_folder when backend=r2.
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
    #[serde(default)]
    pub r2: Option<R2StorageConfig>,
}

/// Cloudflare R2 / S3-compatible credentials + snapshot key.
/// Credentials may be present even when backend!=r2 (for `pull-r2` CLI).
#[derive(Debug, Clone, Deserialize)]
pub struct R2StorageConfig {
    pub bucket: String,
    pub endpoint: String,
    #[serde(default = "default_r2_access_key_env")]
    pub access_key_env: String,
    #[serde(default = "default_r2_secret_key_env")]
    pub secret_key_env: String,
    #[serde(default = "default_r2_snapshot_key")]
    pub snapshot_key: String,
}

fn default_r2_access_key_env() -> String {
    "R2_ACCESS_KEY_ID".to_string()
}
fn default_r2_secret_key_env() -> String {
    "R2_SECRET_ACCESS_KEY".to_string()
}
fn default_r2_snapshot_key() -> String {
    crate::storage::DEFAULT_SNAPSHOT_KEY.to_string()
}

/// RAGMcp-specific configuration
#[derive(Debug, Clone, Deserialize)]
pub struct RagmcpConfig {
    /// Path to the root directory containing documents to index.
    /// Top-level sub-directories become searchable namespaces automatically.
    pub rag_folder: PathBuf,
    pub db_path: PathBuf,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

/// Embeddings configuration
#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingsConfig {
    pub provider: String,
    pub model: String,
    pub api_key_env: String,
    pub batch_size: usize,
    pub dimensions: usize,
    #[serde(default = "default_cache_capacity")]
    pub cache_capacity: usize,
}

fn default_cache_capacity() -> usize {
    1000
}

/// Search configuration
#[derive(Debug, Clone, Deserialize)]
pub struct SearchConfig {
    pub default_k: usize,
    pub min_score: f32,
    pub hybrid_bm25_weight: f32,
    pub hybrid_vector_weight: f32,
}

/// Performance tuning configuration
#[derive(Debug, Clone, Deserialize)]
pub struct PerformanceConfig {
    pub max_latency_ms: u64,
    pub chunk_size_tokens: usize,
    pub chunk_overlap_tokens: usize,
}

/// HTTP server configuration
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HttpServerConfig {
    #[serde(default = "default_http_enabled")]
    pub enabled: bool,
    #[serde(default = "default_http_port")]
    pub port: u16,
    #[serde(default = "default_http_api_key_env")]
    pub api_key_env: String,
    #[serde(default = "default_allowed_origins")]
    pub allowed_origins: Vec<String>,
    #[serde(default = "default_authless")]
    pub authless: bool,
}

/// PageIndex configuration
#[derive(Debug, Clone, Deserialize)]
pub struct PageIndexConfig {
    #[serde(default = "default_pi_enabled")]
    pub enabled: bool,
    #[serde(default = "default_pi_port")]
    pub sidecar_port: u16,
    #[serde(default = "default_pi_timeout")]
    pub sidecar_startup_timeout_ms: u64,
    #[serde(default = "default_pi_model")]
    pub default_model: String,
    #[serde(default = "default_pi_max_iter")]
    pub max_iterations: u8,
    #[serde(default = "default_pi_cache_dir")]
    pub tree_cache_dir: String,
    #[serde(default = "default_pi_rebuild")]
    pub rebuild_on_update: bool,
    #[serde(default = "default_pi_index_startup")]
    pub index_on_startup: bool,
    #[serde(default = "default_pi_extensions")]
    pub eligible_extensions: Vec<String>,
}

fn default_pi_enabled() -> bool { false }
fn default_pi_port() -> u16 { 8181 }
fn default_pi_timeout() -> u64 { 10000 }
fn default_pi_model() -> String { "gpt-5.4-nano".to_string() }
fn default_pi_max_iter() -> u8 { 5 }
fn default_pi_cache_dir() -> String { "./ragmcp_data/pageindex".to_string() }
fn default_pi_rebuild() -> bool { true }
fn default_pi_index_startup() -> bool { true }
fn default_pi_extensions() -> Vec<String> {
    vec!["md".to_string(), "pdf".to_string(), "txt".to_string()]
}

impl Default for PageIndexConfig {
    fn default() -> Self {
        Self {
            enabled: default_pi_enabled(),
            sidecar_port: default_pi_port(),
            sidecar_startup_timeout_ms: default_pi_timeout(),
            default_model: default_pi_model(),
            max_iterations: default_pi_max_iter(),
            tree_cache_dir: default_pi_cache_dir(),
            rebuild_on_update: default_pi_rebuild(),
            index_on_startup: default_pi_index_startup(),
            eligible_extensions: default_pi_extensions(),
        }
    }
}

fn default_authless() -> bool {
    false
}

fn default_http_enabled() -> bool {
    false
}

fn default_http_port() -> u16 {
    8080
}

fn default_http_api_key_env() -> String {
    "RAGMCP_API_KEY".to_string()
}

fn default_allowed_origins() -> Vec<String> {
    // Default empty — set allowed_origins in config.toml for production
    vec![]
}

fn default_log_level() -> String {
    "info".to_string()
}

/// Prefer a non-empty env var over the TOML value.
/// Why: public repos / Render deploy leave placeholders in git; real values come from host env.
fn resolve_r2_env_or_toml(env_key: &str, toml_value: &str) -> String {
    match std::env::var(env_key) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => toml_value.to_string(),
    }
}

/// True when bucket is empty or still a documented placeholder (unsafe to use as-is).
fn r2_bucket_looks_unconfigured(bucket: &str) -> bool {
    let b = bucket.trim();
    b.is_empty() || b == "your-bucket-name" || b.contains("<accountid>")
}

/// True when endpoint is empty or still contains the example account-id placeholder.
fn r2_endpoint_looks_unconfigured(endpoint: &str) -> bool {
    let e = endpoint.trim();
    e.is_empty() || e.contains("<accountid>")
}

/// Host (or full URL fallback) for info logs — avoids dumping credentials.
fn r2_endpoint_host_for_log(endpoint: &str) -> &str {
    let trimmed = endpoint.trim().trim_end_matches('/');
    // Strip scheme so logs show host only when possible.
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    without_scheme.split('/').next().unwrap_or(without_scheme)
}

impl Config {
    /// Load configuration from file
    /// 
    /// Loads environment variables from .env file (if present) before loading config.
    /// Looks for config file in this order:
    /// 1. Path specified in RAGMCP_CONFIG environment variable
    /// 2. ./config.toml in current directory
    pub fn load() -> Result<Self> {
        // Load .env file if it exists (ignore errors - file is optional)
        // This allows environment variables to be set from .env file
        let _ = dotenv::dotenv();
        
        let config_path = std::env::var("RAGMCP_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("config.toml"));
        
        let config_str = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
        
        let mut config: Config = toml::from_str(&config_str)
            .context("Failed to parse config.toml")?;
        
        // Validate configuration
        config.validate()?;
        // When backend=r2, effective docs root becomes cache_dir for all callers.
        config.apply_storage_overrides();
        
        Ok(config)
    }

    /// True when `[storage] backend = "r2"` (case-insensitive).
    pub fn is_r2_backend(&self) -> bool {
        self.storage
            .as_ref()
            .map(|s| s.backend.eq_ignore_ascii_case("r2"))
            .unwrap_or(false)
    }

    /// Override rag_folder → cache_dir when running in R2 server mode.
    pub fn apply_storage_overrides(&mut self) {
        if !self.is_r2_backend() {
            return;
        }
        if let Some(cache) = self
            .storage
            .as_ref()
            .and_then(|s| s.cache_dir.clone())
        {
            log::info!(
                "R2 mode: effective rag_folder → cache_dir {}",
                cache.display()
            );
            self.ragmcp.rag_folder = cache;
        }
    }

    /// Build an R2 client when `[storage.r2]` + env credentials are available.
    /// Works even if backend!=r2 (for `pull-r2`).
    ///
    /// Bucket / endpoint / snapshot key prefer env over TOML so a public repo can
    /// ship placeholders (e.g. `config.server.toml.example`) while Render sets
    /// `R2_BUCKET` / `R2_ENDPOINT` at deploy time.
    pub fn r2_client_from_config(&self) -> Result<crate::storage::R2Client> {
        let r2 = self
            .storage
            .as_ref()
            .and_then(|s| s.r2.as_ref())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Missing [storage.r2] in config. Required for R2 mode / pull-r2."
                )
            })?;

        // Env wins over TOML: public git must not require real bucket/endpoint values.
        let bucket = resolve_r2_env_or_toml("R2_BUCKET", &r2.bucket);
        let endpoint = resolve_r2_env_or_toml("R2_ENDPOINT", &r2.endpoint);
        let snapshot_key = resolve_r2_env_or_toml("R2_SNAPSHOT_KEY", &r2.snapshot_key);

        if r2_bucket_looks_unconfigured(&bucket) || r2_endpoint_looks_unconfigured(&endpoint) {
            anyhow::bail!(
                "R2 bucket/endpoint not configured. Set environment variables R2_BUCKET and \
                 R2_ENDPOINT (recommended on Render), or replace placeholders in [storage.r2]. \
                 Got bucket={:?} endpoint={:?}.",
                bucket,
                endpoint
            );
        }

        let access_key = std::env::var(&r2.access_key_env).with_context(|| {
            format!(
                "Environment variable {} not set (R2 access key)",
                r2.access_key_env
            )
        })?;
        let secret_key = std::env::var(&r2.secret_key_env).with_context(|| {
            format!(
                "Environment variable {} not set (R2 secret key)",
                r2.secret_key_env
            )
        })?;

        // Bucket name + endpoint host are not secrets; never log access/secret keys.
        log::info!(
            "R2 client using bucket={} endpoint={}",
            bucket,
            r2_endpoint_host_for_log(&endpoint)
        );

        crate::storage::R2Client::new(bucket, endpoint, access_key, secret_key, snapshot_key)
            .map_err(|e| anyhow::anyhow!("{}", e))
    }
    
    /// Validate configuration values
    fn validate(&self) -> Result<()> {
        if self.is_r2_backend() {
            // R2 mode: cache_dir may not exist yet (created during hydrate).
            let cache = self
                .storage
                .as_ref()
                .and_then(|s| s.cache_dir.as_ref())
                .ok_or_else(|| {
                    anyhow::anyhow!("storage.backend=r2 requires storage.cache_dir")
                })?;
            if let Some(s) = &self.storage {
                if s.r2.is_none() {
                    anyhow::bail!("storage.backend=r2 requires [storage.r2] credentials section");
                }
            }
            // Safety: refuse non-empty cache without marker (does not require dir to exist).
            crate::storage::assert_cache_dir_safe(cache)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
        } else {
            // Local mode: rag_folder must already exist (unchanged behavior).
            if !self.ragmcp.rag_folder.exists() {
                anyhow::bail!(
                    "rag_folder path does not exist: {}. Set rag_folder in config.toml to your docs directory.",
                    self.ragmcp.rag_folder.display()
                );
            }

            if !self.ragmcp.rag_folder.is_dir() {
                anyhow::bail!(
                    "rag_folder must be a directory, not a file: {}",
                    self.ragmcp.rag_folder.display()
                );
            }
        }
        
        // Validate environment variables
        // Check both environment variable and .env file (dotenv already loaded in Config::load)
        std::env::var(&self.embeddings.api_key_env)
            .with_context(|| {
                format!(
                    "Environment variable {} not set. Set it in your .env file or as an environment variable with your OpenAI API key.",
                    self.embeddings.api_key_env
                )
            })?;
        
        // Validate numeric ranges
        if self.search.default_k == 0 {
            anyhow::bail!("search.default_k must be greater than 0");
        }
        
        if self.search.min_score < 0.0 || self.search.min_score > 1.0 {
            anyhow::bail!("search.min_score must be between 0.0 and 1.0");
        }
        
        if self.performance.chunk_size_tokens == 0 {
            anyhow::bail!("performance.chunk_size_tokens must be greater than 0");
        }
        
        if self.performance.chunk_overlap_tokens >= self.performance.chunk_size_tokens {
            anyhow::bail!(
                "performance.chunk_overlap_tokens must be less than chunk_size_tokens"
            );
        }
        
        Ok(())
    }
    
    /// Get database path
    pub fn db_path(&self) -> &Path {
        &self.ragmcp.db_path
    }
    
    /// Get the docs root path (rag_folder; equals cache_dir after R2 override)
    pub fn rag_folder(&self) -> &Path {
        &self.ragmcp.rag_folder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Serialize config tests that mutate process-wide cwd and env so they don't race.
    static CONFIG_TEST_LOCK: Mutex<()> = Mutex::new(());
    
    fn create_test_config(temp_dir: &TempDir) -> String {
        let rag_folder = temp_dir.path().canonicalize().unwrap();
        let rag_folder_str = rag_folder.to_str().unwrap().replace('\\', "\\\\");
        format!(
            r#"
[ragmcp]
rag_folder = "{}"
db_path = "./test.db"
log_level = "debug"

[embeddings]
provider = "openai"
model = "text-embedding-3-small"
api_key_env = "OPENAI_API_KEY"
batch_size = 100
dimensions = 1536

[search]
default_k = 5
min_score = 0.65
hybrid_bm25_weight = 0.5
hybrid_vector_weight = 0.5

[performance]
max_latency_ms = 1000
chunk_size_tokens = 300
chunk_overlap_tokens = 50
"#,
            rag_folder_str
        )
    }

    /// Restores cwd when dropped (e.g. on panic).
    struct CwdGuard(std::path::PathBuf);
    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }

    fn with_config_env(config_path: &std::path::Path, api_key: Option<&str>, f: impl FnOnce()) {
        let original_config = std::env::var("RAGMCP_CONFIG").ok();
        let original_key = std::env::var("OPENAI_API_KEY").ok();
        std::env::set_var("RAGMCP_CONFIG", config_path.to_str().unwrap());
        match api_key {
            Some(k) => std::env::set_var("OPENAI_API_KEY", k),
            None => std::env::remove_var("OPENAI_API_KEY"),
        }
        f();
        std::env::remove_var("RAGMCP_CONFIG");
        std::env::remove_var("OPENAI_API_KEY");
        if let Some(val) = original_config {
            std::env::set_var("RAGMCP_CONFIG", val);
        }
        if let Some(val) = original_key {
            std::env::set_var("OPENAI_API_KEY", val);
        }
    }
    
    #[test]
    fn test_config_load_success() {
        let _lock = CONFIG_TEST_LOCK.lock().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let config_content = create_test_config(&temp_dir);
        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, config_content).unwrap();
        let config_path = config_path.canonicalize().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        let _cwd = CwdGuard(original_dir.clone());
        std::env::set_current_dir(temp_dir.path()).unwrap();
        with_config_env(&config_path, Some("test-key"), || {
            let config = Config::load();
            assert!(config.is_ok(), "Config::load() failed: {:?}", config.err());
            let config = config.unwrap();
            assert_eq!(config.ragmcp.log_level, "debug");
            assert_eq!(config.search.default_k, 5);
            assert_eq!(config.embeddings.batch_size, 100);
        });
    }
    
    #[test]
    fn test_config_missing_api_key() {
        let _lock = CONFIG_TEST_LOCK.lock().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let config_content = create_test_config(&temp_dir);
        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, config_content).unwrap();
        let config_path = config_path.canonicalize().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        let _cwd = CwdGuard(original_dir.clone());
        std::env::set_current_dir(temp_dir.path()).unwrap();
        with_config_env(&config_path, None, || {
            let config = Config::load();
            assert!(config.is_err(), "Expected missing API key error");
            assert!(config.unwrap_err().to_string().contains("OPENAI_API_KEY"));
        });
    }
    
    #[test]
    fn test_config_loads_from_env_file() {
        let _lock = CONFIG_TEST_LOCK.lock().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let config_content = create_test_config(&temp_dir);
        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, config_content).unwrap();
        
        // Create .env file in temp directory
        let env_file = temp_dir.path().join(".env");
        fs::write(&env_file, "OPENAI_API_KEY=test-key-from-env-file\n").unwrap();
        let config_path = config_path.canonicalize().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        let _cwd = CwdGuard(original_dir.clone());
        std::env::set_current_dir(temp_dir.path()).unwrap();
        with_config_env(&config_path, None, || {
            let config = Config::load();
            assert!(config.is_ok(), "Config should load with API key from .env file");
            let config = config.unwrap();
            assert_eq!(config.embeddings.api_key_env, "OPENAI_API_KEY");
        });
    }
    
    #[test]
    fn test_config_invalid_path() {
        let _lock = CONFIG_TEST_LOCK.lock().unwrap();
        let original = std::env::var("RAGMCP_CONFIG").ok();
        std::env::set_var("RAGMCP_CONFIG", "nonexistent.toml");
        let config = Config::load();
        assert!(config.is_err());
        std::env::remove_var("RAGMCP_CONFIG");
        if let Some(v) = original {
            std::env::set_var("RAGMCP_CONFIG", v);
        }
    }

    #[test]
    fn test_r2_env_overrides_and_placeholder_detection() {
        let _lock = CONFIG_TEST_LOCK.lock().unwrap();

        // Save / restore R2_* so we don't leak into other tests.
        let saved_bucket = std::env::var("R2_BUCKET").ok();
        let saved_endpoint = std::env::var("R2_ENDPOINT").ok();
        let saved_snap = std::env::var("R2_SNAPSHOT_KEY").ok();
        std::env::remove_var("R2_BUCKET");
        std::env::remove_var("R2_ENDPOINT");
        std::env::remove_var("R2_SNAPSHOT_KEY");

        assert_eq!(
            resolve_r2_env_or_toml("R2_BUCKET", "toml-bucket"),
            "toml-bucket"
        );
        std::env::set_var("R2_BUCKET", "env-bucket");
        assert_eq!(
            resolve_r2_env_or_toml("R2_BUCKET", "toml-bucket"),
            "env-bucket"
        );
        // Empty env must not override TOML.
        std::env::set_var("R2_BUCKET", "   ");
        assert_eq!(
            resolve_r2_env_or_toml("R2_BUCKET", "toml-bucket"),
            "toml-bucket"
        );

        assert!(r2_bucket_looks_unconfigured(""));
        assert!(r2_bucket_looks_unconfigured("your-bucket-name"));
        assert!(r2_bucket_looks_unconfigured("x-<accountid>-y"));
        assert!(!r2_bucket_looks_unconfigured("ragmcp-docs"));

        assert!(r2_endpoint_looks_unconfigured(""));
        assert!(r2_endpoint_looks_unconfigured(
            "https://<accountid>.r2.cloudflarestorage.com"
        ));
        assert!(!r2_endpoint_looks_unconfigured(
            "https://abc123.r2.cloudflarestorage.com"
        ));

        assert_eq!(
            r2_endpoint_host_for_log("https://abc123.r2.cloudflarestorage.com/path"),
            "abc123.r2.cloudflarestorage.com"
        );

        // Restore prior env.
        std::env::remove_var("R2_BUCKET");
        std::env::remove_var("R2_ENDPOINT");
        std::env::remove_var("R2_SNAPSHOT_KEY");
        if let Some(v) = saved_bucket {
            std::env::set_var("R2_BUCKET", v);
        }
        if let Some(v) = saved_endpoint {
            std::env::set_var("R2_ENDPOINT", v);
        }
        if let Some(v) = saved_snap {
            std::env::set_var("R2_SNAPSHOT_KEY", v);
        }
    }

    #[test]
    fn test_r2_client_rejects_placeholder_bucket_endpoint() {
        let _lock = CONFIG_TEST_LOCK.lock().unwrap();

        let saved_bucket = std::env::var("R2_BUCKET").ok();
        let saved_endpoint = std::env::var("R2_ENDPOINT").ok();
        let saved_ak = std::env::var("R2_ACCESS_KEY_ID").ok();
        let saved_sk = std::env::var("R2_SECRET_ACCESS_KEY").ok();
        std::env::remove_var("R2_BUCKET");
        std::env::remove_var("R2_ENDPOINT");
        std::env::set_var("R2_ACCESS_KEY_ID", "test-ak");
        std::env::set_var("R2_SECRET_ACCESS_KEY", "test-sk");

        let cfg = Config {
            ragmcp: RagmcpConfig {
                rag_folder: PathBuf::from("."),
                db_path: PathBuf::from("./t.db"),
                log_level: "info".into(),
            },
            embeddings: EmbeddingsConfig {
                provider: "openai".into(),
                model: "text-embedding-3-small".into(),
                api_key_env: "OPENAI_API_KEY".into(),
                batch_size: 100,
                dimensions: 1536,
                cache_capacity: 1000,
            },
            search: SearchConfig {
                default_k: 5,
                min_score: 0.65,
                hybrid_bm25_weight: 0.5,
                hybrid_vector_weight: 0.5,
            },
            performance: PerformanceConfig {
                max_latency_ms: 1000,
                chunk_size_tokens: 300,
                chunk_overlap_tokens: 50,
            },
            http_server: HttpServerConfig::default(),
            pageindex: PageIndexConfig::default(),
            storage: Some(StorageConfig {
                backend: "r2".into(),
                cache_dir: Some(PathBuf::from("/tmp/docs")),
                r2: Some(R2StorageConfig {
                    bucket: "your-bucket-name".into(),
                    endpoint: "https://<accountid>.r2.cloudflarestorage.com".into(),
                    access_key_env: default_r2_access_key_env(),
                    secret_key_env: default_r2_secret_key_env(),
                    snapshot_key: default_r2_snapshot_key(),
                }),
            }),
        };

        let err = match cfg.r2_client_from_config() {
            Ok(_) => panic!("expected placeholder rejection"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("R2_BUCKET") && err.contains("R2_ENDPOINT"),
            "unexpected error: {}",
            err
        );

        // Env overrides fix placeholders without editing TOML.
        std::env::set_var("R2_BUCKET", "real-bucket");
        std::env::set_var(
            "R2_ENDPOINT",
            "https://abc123.r2.cloudflarestorage.com",
        );
        assert!(cfg.r2_client_from_config().is_ok());

        std::env::remove_var("R2_BUCKET");
        std::env::remove_var("R2_ENDPOINT");
        std::env::remove_var("R2_ACCESS_KEY_ID");
        std::env::remove_var("R2_SECRET_ACCESS_KEY");
        if let Some(v) = saved_bucket {
            std::env::set_var("R2_BUCKET", v);
        }
        if let Some(v) = saved_endpoint {
            std::env::set_var("R2_ENDPOINT", v);
        }
        if let Some(v) = saved_ak {
            std::env::set_var("R2_ACCESS_KEY_ID", v);
        }
        if let Some(v) = saved_sk {
            std::env::set_var("R2_SECRET_ACCESS_KEY", v);
        }
    }
}
