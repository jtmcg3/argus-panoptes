use async_trait::async_trait;
use panoptes_common::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmRequest {
    pub system_prompt: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub model: String,
    pub usage: Option<TokenUsage>,
    pub finish_reason: Option<String>,
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse>;
    fn model_name(&self) -> &str;
}

#[async_trait]
impl LlmClient for Box<dyn LlmClient> {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        (**self).complete(request).await
    }
    fn model_name(&self) -> &str {
        (**self).model_name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_serialization_roundtrip() {
        let msg = ChatMessage {
            role: Role::User,
            content: "Hello".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.role, Role::User);
        assert_eq!(deserialized.content, "Hello");
    }

    #[test]
    fn llm_request_serialization_roundtrip() {
        let request = LlmRequest {
            system_prompt: Some("You are helpful.".to_string()),
            messages: vec![ChatMessage {
                role: Role::User,
                content: "Hi".to_string(),
            }],
            temperature: Some(0.7),
            max_tokens: Some(1024),
        };
        let json = serde_json::to_string(&request).unwrap();
        let deserialized: LlmRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(
            deserialized.system_prompt.as_deref(),
            Some("You are helpful.")
        );
        assert_eq!(deserialized.messages.len(), 1);
        assert_eq!(deserialized.temperature, Some(0.7));
        assert_eq!(deserialized.max_tokens, Some(1024));
    }

    #[test]
    fn llm_response_serialization_roundtrip() {
        let response = LlmResponse {
            content: "Hello there!".to_string(),
            model: "gpt-4".to_string(),
            usage: Some(TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
            }),
            finish_reason: Some("stop".to_string()),
        };
        let json = serde_json::to_string(&response).unwrap();
        let deserialized: LlmResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.content, "Hello there!");
        assert_eq!(deserialized.model, "gpt-4");
        let usage = deserialized.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(deserialized.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn role_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Role::System).unwrap(), "\"system\"");
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
        assert_eq!(
            serde_json::to_string(&Role::Assistant).unwrap(),
            "\"assistant\""
        );
    }
}
