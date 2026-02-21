//! Message types for inter-agent communication.

use serde::{Deserialize, Serialize};

/// Role of a message sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// A message that can be passed between agents or to/from the coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    /// Unique message ID
    pub id: String,

    /// Role of the sender
    pub role: MessageRole,

    /// Message content
    pub content: String,

    /// Source agent (if from an agent)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,

    /// Target agent (if routing to specific agent)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_agent: Option<String>,

    /// Associated task ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,

    /// Timestamp (Unix millis)
    pub timestamp: u64,

    /// Optional metadata
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
}

impl AgentMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: uuid_v4(),
            role: MessageRole::User,
            content: content.into(),
            source_agent: None,
            target_agent: None,
            task_id: None,
            timestamp: now_millis(),
            metadata: serde_json::Value::Null,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            id: uuid_v4(),
            role: MessageRole::System,
            content: content.into(),
            source_agent: None,
            target_agent: None,
            task_id: None,
            timestamp: now_millis(),
            metadata: serde_json::Value::Null,
        }
    }

    pub fn from_agent(agent: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: uuid_v4(),
            role: MessageRole::Assistant,
            content: content.into(),
            source_agent: Some(agent.into()),
            target_agent: None,
            task_id: None,
            timestamp: now_millis(),
            metadata: serde_json::Value::Null,
        }
    }
}

fn uuid_v4() -> String {
    // Simple UUID v4 generation without external crate
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:032x}", now)
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
