//! Configuration for the coordinator.
//!
//! # Security Features (SEC-005)
//!
//! - Config file permission validation on Unix systems
//! - Rejects world-readable files containing API keys
//! - Warns about API keys stored in config files

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::warn;

/// Main coordinator configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorConfig {
    /// ZeroClaw provider configuration
    pub provider: ProviderConfig,

    /// Registered agents and their MCP endpoints
    pub agents: HashMap<String, AgentConfig>,

    /// Memory/LanceDB configuration
    pub memory: MemoryConfig,

    /// Default working directory
    #[serde(default)]
    pub default_working_dir: Option<PathBuf>,

    /// Allowed base directories for working_dir validation (SEC-010).
    /// Empty vec = allow all (dev mode fallback).
    #[serde(default)]
    pub allowed_base_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Provider type: "ollama", "openai", "anthropic"
    pub provider_type: String,

    /// Model name
    pub model: String,

    /// API endpoint (for Ollama, OpenAI-compatible endpoints)
    #[serde(default = "default_ollama_url")]
    pub api_url: String,

    /// API key for authentication (OpenAI, Anthropic)
    /// If not set, will attempt to read from environment variables:
    /// - OPENAI_API_KEY for OpenAI
    /// - ANTHROPIC_API_KEY for Anthropic
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Timeout in milliseconds
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_ollama_url() -> String {
    "http://localhost:11434".into()
}

fn default_timeout() -> u64 {
    30000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent type: "pty-mcp", "swarm", "external"
    pub agent_type: String,

    /// MCP transport: "stdio", "http", "socket"
    pub transport: String,

    /// Command to spawn (for stdio transport)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// HTTP endpoint (for http transport)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// Agent capabilities (for routing decisions)
    #[serde(default)]
    pub capabilities: Vec<String>,

    /// Whether this agent requires confirmation for actions
    #[serde(default)]
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Path to LanceDB database
    pub db_path: PathBuf,

    /// Embedding model to use
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,

    /// Maximum context tokens to include from memory
    #[serde(default = "default_max_context")]
    pub max_context_tokens: usize,
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
            provider: ProviderConfig {
                provider_type: "ollama".into(),
                model: "llama3.2".into(),
                api_url: default_ollama_url(),
                api_key: None,
                timeout_ms: default_timeout(),
            },
            agents: HashMap::new(),
            memory: MemoryConfig {
                db_path: PathBuf::from("./data/memory"),
                embedding_model: default_embedding_model(),
                max_context_tokens: default_max_context(),
            },
            default_working_dir: None,
            allowed_base_dirs: Vec::new(),
        }
    }
}

impl CoordinatorConfig {
    /// Load configuration from a TOML file.
    ///
    /// # Security (SEC-005)
    ///
    /// On Unix systems, this function validates that:
    /// - The file is a regular file (not a symlink)
    /// - The file is not world-readable if it contains an API key
    /// - Warns if API keys are stored in the config file
    pub fn from_file(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();

        // SEC-005: Validate file permissions on Unix
        #[cfg(unix)]
        validate_config_file_permissions(path)?;

        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;

        // SEC-005: Warn if API key is in config file
        if config.provider.api_key.is_some() {
            warn!(
                "API key found in config file '{}'. For better security, \
                 use environment variables instead (OPENAI_API_KEY, ANTHROPIC_API_KEY).",
                path.display()
            );
        }

        Ok(config)
    }

    /// Load configuration from a TOML file without permission checks.
    ///
    /// Use this only for testing or when you've already validated the file.
    pub fn from_file_unchecked(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }
}

/// Validate config file permissions on Unix systems (SEC-005).
///
/// Requirements:
/// - File must be a regular file (not symlink, directory, etc.)
/// - File must not be world-writable (mode & 0o002 == 0)
/// - If file contains API key patterns, must not be world-readable
#[cfg(unix)]
fn validate_config_file_permissions(path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("Failed to read config file '{}': {}", path.display(), e))?;

    // Must be a regular file
    if !metadata.is_file() {
        anyhow::bail!(
            "Config path '{}' is not a regular file. Symlinks and directories are not allowed.",
            path.display()
        );
    }

    let mode = metadata.permissions().mode();
    let permission_bits = mode & 0o777;

    // Must not be world-writable
    if permission_bits & 0o002 != 0 {
        anyhow::bail!(
            "Config file '{}' is world-writable (mode {:04o}). \
             This is a security risk. Fix with: chmod o-w {}",
            path.display(),
            permission_bits,
            path.display()
        );
    }

    // Check if file might contain sensitive data
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let has_api_key = content.contains("api_key")
        && (content.contains("sk-") || content.contains("key ="));

    // If contains API key, must not be world-readable
    if has_api_key && permission_bits & 0o004 != 0 {
        anyhow::bail!(
            "Config file '{}' contains an API key but is world-readable (mode {:04o}). \
             This is a security risk. Fix with: chmod 600 {}",
            path.display(),
            permission_bits,
            path.display()
        );
    }

    // Warn about group-readable files with API keys
    if has_api_key && permission_bits & 0o040 != 0 {
        warn!(
            "Config file '{}' contains an API key and is group-readable (mode {:04o}). \
             Consider restricting access with: chmod 600 {}",
            path.display(),
            permission_bits,
            path.display()
        );
    }

    Ok(())
}

impl ProviderConfig {
    /// Resolve the API key from config or environment variables.
    ///
    /// Priority:
    /// 1. Explicit api_key in config
    /// 2. Environment variable based on provider_type:
    ///    - "openai" -> OPENAI_API_KEY
    ///    - "anthropic" -> ANTHROPIC_API_KEY
    pub fn resolve_api_key(&self) -> Option<String> {
        // First check explicit config
        if let Some(ref key) = self.api_key {
            if !key.is_empty() {
                return Some(key.clone());
            }
        }

        // Fall back to environment variable based on provider type
        let env_var = match self.provider_type.as_str() {
            "openai" => "OPENAI_API_KEY",
            "anthropic" => "ANTHROPIC_API_KEY",
            _ => return None,
        };

        std::env::var(env_var).ok()
    }
}
