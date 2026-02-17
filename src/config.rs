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
        
        let config: Config = toml::from_str(&config_str)
            .context("Failed to parse config.toml")?;
        
        // Validate configuration
        config.validate()?;
        
        Ok(config)
    }
    
    /// Validate configuration values
    fn validate(&self) -> Result<()> {
        // Validate that rag_folder exists and is a directory
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
    
    /// Get the docs root path (rag_folder from config.toml)
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
}
