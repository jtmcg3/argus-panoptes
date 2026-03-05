//! Configuration for the coordinator.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::warn;

/// Main coordinator configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorConfig {
    /// Coordinator port.
    #[serde(default = "default_coordinator_port")]
    pub port: u16,

    /// Agent URLs for discovery (coordinator fetches Agent Cards from these).
    #[serde(default)]
    pub agent_urls: Vec<String>,

    /// Optional timeout for coordinator -> agent HTTP calls.
    /// `None` means no HTTP timeout (recommended for long-running streaming tasks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegate_timeout_ms: Option<u64>,

    /// Triage LLM configuration.
    #[serde(default)]
    pub triage: TriageConfig,

    /// LLM configuration for content generation.
    #[serde(default)]
    pub llm: LlmSection,

    /// Memory/LanceDB configuration.
    #[serde(default)]
    pub memory: MemoryConfig,
}

fn default_coordinator_port() -> u16 {
    8080
}

/// Triage LLM settings (replaces ZeroClaw provider config).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageConfig {
    /// Provider type: "openai" (also works for Ollama), "anthropic"
    #[serde(default = "default_triage_provider")]
    pub provider: String,

    /// Model name
    #[serde(default = "default_triage_model")]
    pub model: String,

    /// API endpoint
    #[serde(default = "default_triage_url")]
    pub api_url: String,

    /// API key (optional, read from env if not set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Temperature for triage (low for consistent routing)
    #[serde(default = "default_triage_temperature")]
    pub temperature: f32,

    /// Timeout in milliseconds
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_triage_provider() -> String {
    "openai".into()
}

fn default_triage_model() -> String {
    "lfm2:24b".into()
}

fn default_triage_url() -> String {
    "http://host.orb.internal:11434".into()
}

fn default_triage_temperature() -> f32 {
    0.3
}

fn default_timeout() -> u64 {
    30000
}

impl Default for TriageConfig {
    fn default() -> Self {
        Self {
            provider: default_triage_provider(),
            model: default_triage_model(),
            api_url: default_triage_url(),
            api_key: None,
            temperature: default_triage_temperature(),
            timeout_ms: default_timeout(),
        }
    }
}

/// LLM section for agent content generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSection {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_requests: usize,
}

fn default_max_concurrent() -> usize {
    2
}

impl Default for LlmSection {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            model: "lfm2:24b".into(),
            api_url: Some("http://host.orb.internal:11434".into()),
            api_key: None,
            max_concurrent_requests: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    #[serde(default = "default_max_context")]
    pub max_context_tokens: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
            embedding_model: default_embedding_model(),
            max_context_tokens: default_max_context(),
        }
    }
}

fn default_db_path() -> PathBuf {
    PathBuf::from("./data/memory")
}

fn default_embedding_model() -> String {
    "all-MiniLM-L6-v2".into()
}

fn default_max_context() -> usize {
    4096
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            port: default_coordinator_port(),
            agent_urls: vec![
                "http://localhost:9001".into(),
                "http://localhost:9002".into(),
                "http://localhost:9003".into(),
                "http://localhost:9004".into(),
                "http://localhost:9005".into(),
                "http://localhost:9006".into(),
            ],
            delegate_timeout_ms: None,
            triage: TriageConfig::default(),
            llm: LlmSection::default(),
            memory: MemoryConfig::default(),
        }
    }
}

impl CoordinatorConfig {
    /// Load configuration from a TOML file.
    pub fn from_file(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();

        #[cfg(unix)]
        validate_config_file_permissions(path)?;

        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;

        if config.triage.api_key.is_some() {
            warn!(
                "API key found in config file '{}'. Use environment variables instead.",
                path.display()
            );
        }

        Ok(config)
    }
}

/// Validate config file permissions on Unix systems (SEC-005).
#[cfg(unix)]
fn validate_config_file_permissions(path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("Failed to read config file '{}': {}", path.display(), e))?;

    if !metadata.is_file() {
        anyhow::bail!("Config path '{}' is not a regular file.", path.display());
    }

    let mode = metadata.permissions().mode();
    let permission_bits = mode & 0o777;

    if permission_bits & 0o002 != 0 {
        anyhow::bail!(
            "Config file '{}' is world-writable (mode {:04o}). Fix with: chmod o-w {}",
            path.display(),
            permission_bits,
            path.display()
        );
    }

    let content = std::fs::read_to_string(path).unwrap_or_default();
    let has_api_key =
        content.contains("api_key") && (content.contains("sk-") || content.contains("key ="));

    if has_api_key && permission_bits & 0o004 != 0 {
        anyhow::bail!(
            "Config file '{}' contains an API key but is world-readable (mode {:04o}). Fix with: chmod 600 {}",
            path.display(),
            permission_bits,
            path.display()
        );
    }

    if has_api_key && permission_bits & 0o040 != 0 {
        warn!(
            "Config file '{}' is group-readable with API key (mode {:04o}). Consider: chmod 600 {}",
            path.display(),
            permission_bits,
            path.display()
        );
    }

    Ok(())
}
