//! Core agent traits and capabilities.
//!
//! These traits are defined in `panoptes-common` so that both the coordinator
//! and agent crates can reference them without circular dependencies.

use crate::{AgentMessage, Result, Task};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Capabilities that an agent can have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCapability {
    /// Can write and modify code
    CodeGeneration,
    /// Can execute code in a PTY
    CodeExecution,
    /// Can perform web searches
    WebSearch,
    /// Can read and analyze documents
    DocumentAnalysis,
    /// Can create written content
    ContentCreation,
    /// Can manage tasks and schedules
    TaskPlanning,
    /// Can review code for issues
    CodeReview,
    /// Can run tests
    TestExecution,
    /// Can access long-term memory
    MemoryAccess,
}

/// The core agent trait that all specialist agents implement.
#[async_trait]
pub trait Agent: Send + Sync {
    /// Get the agent's unique identifier.
    fn id(&self) -> &str;

    /// Get the agent's human-readable name.
    fn name(&self) -> &str;

    /// Get the agent's capabilities.
    fn capabilities(&self) -> &[AgentCapability];

    /// Check if the agent has a specific capability.
    fn has_capability(&self, cap: AgentCapability) -> bool {
        self.capabilities().contains(&cap)
    }

    /// Process a task assigned to this agent.
    async fn process_task(&self, task: &Task) -> Result<AgentMessage>;

    /// Handle a direct message to this agent.
    async fn handle_message(&self, message: &AgentMessage) -> Result<AgentMessage>;

    /// Get the agent's system prompt.
    fn system_prompt(&self) -> &str;

    /// Check if the agent is available (not busy with another task).
    fn is_available(&self) -> bool;
}

/// Configuration for agent creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent ID
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// LLM model to use
    pub model: String,

    /// Custom system prompt (optional, uses default if not set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    /// MCP servers this agent can access
    #[serde(default)]
    pub mcp_servers: Vec<String>,

    /// Temperature for LLM responses
    #[serde(default = "default_temperature")]
    pub temperature: f32,

    /// Max tokens for responses
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
}

fn default_temperature() -> f32 {
    0.7
}

fn default_max_tokens() -> usize {
    4096
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            id: "agent".into(),
            name: "Agent".into(),
            model: "llama3.2".into(),
            system_prompt: None,
            mcp_servers: vec![],
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
        }
    }
}
