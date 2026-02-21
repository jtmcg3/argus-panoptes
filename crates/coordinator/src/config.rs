//! Configuration for the coordinator.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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
        }
    }
}

impl CoordinatorConfig {
    /// Load configuration from a TOML file.
    pub fn from_file(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }
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
