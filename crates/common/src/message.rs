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
    uuid::Uuid::new_v4().to_string()
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_message() {
        let msg = AgentMessage::user("Hello from user");

        assert!(!msg.id.is_empty());
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.content, "Hello from user");
        assert!(msg.source_agent.is_none());
        assert!(msg.target_agent.is_none());
        assert!(msg.task_id.is_none());
        assert!(msg.timestamp > 0);
    }

    #[test]
    fn test_system_message() {
        let msg = AgentMessage::system("System notification");

        assert_eq!(msg.role, MessageRole::System);
        assert_eq!(msg.content, "System notification");
    }

    #[test]
    fn test_agent_message() {
        let msg = AgentMessage::from_agent("coding-agent", "Code generated");

        assert_eq!(msg.role, MessageRole::Assistant);
        assert_eq!(msg.content, "Code generated");
        assert_eq!(msg.source_agent, Some("coding-agent".to_string()));
    }

    #[test]
    fn test_message_unique_ids() {
        let msg1 = AgentMessage::user("Message 1");
        let msg2 = AgentMessage::user("Message 2");

        assert_ne!(msg1.id, msg2.id);
    }

    #[test]
    fn test_message_serialization() {
        let msg = AgentMessage::from_agent("test-agent", "Test content");

        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, msg.id);
        assert_eq!(deserialized.role, msg.role);
        assert_eq!(deserialized.content, msg.content);
        assert_eq!(deserialized.source_agent, msg.source_agent);
    }

    #[test]
    fn test_message_role_variants() {
        let roles = vec![
            MessageRole::User,
            MessageRole::Assistant,
            MessageRole::System,
            MessageRole::Tool,
        ];

        for role in roles {
            let json = serde_json::to_string(&role).unwrap();
            let deserialized: MessageRole = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, role);
        }
    }

    #[test]
    fn test_message_with_metadata() {
        let mut msg = AgentMessage::user("With metadata");
        msg.metadata = serde_json::json!({
            "key": "value",
            "count": 42
        });

        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.metadata["key"], "value");
        assert_eq!(deserialized.metadata["count"], 42);
    }
}
