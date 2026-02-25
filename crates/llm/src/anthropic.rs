use async_trait::async_trait;
use panoptes_common::PanoptesError;
use panoptes_common::Result;
use serde::{Deserialize, Serialize};

use crate::client::{LlmClient, LlmRequest, LlmResponse, Role, TokenUsage};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    max_tokens: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContent>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct AnthropicContent {
    #[serde(rename = "type")]
    content_type: String,
    text: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    model: String,
    usage: Option<AnthropicUsage>,
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

pub struct AnthropicClient {
    model: String,
    api_key: String,
    http_client: reqwest::Client,
}

impl AnthropicClient {
    pub fn new(model: String, api_key: String) -> Self {
        Self {
            model,
            api_key,
            http_client: reqwest::Client::new(),
        }
    }

    fn role_to_string(role: &Role) -> &'static str {
        match role {
            Role::System => "user", // system messages go in the top-level system field
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    fn build_messages(request: &LlmRequest) -> Vec<AnthropicMessage> {
        request
            .messages
            .iter()
            .filter(|msg| msg.role != Role::System)
            .map(|msg| AnthropicMessage {
                role: Self::role_to_string(&msg.role).to_string(),
                content: vec![AnthropicContent {
                    content_type: "text".to_string(),
                    text: msg.content.clone(),
                }],
            })
            .collect()
    }

    /// Build the request body for testing purposes.
    #[cfg(test)]
    fn build_request_body(&self, request: &LlmRequest) -> AnthropicRequest {
        AnthropicRequest {
            model: self.model.clone(),
            messages: Self::build_messages(request),
            system: request.system_prompt.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens.unwrap_or(4096),
        }
    }
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let body = AnthropicRequest {
            model: self.model.clone(),
            messages: Self::build_messages(&request),
            system: request.system_prompt.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens.unwrap_or(4096),
        };

        let response = self
            .http_client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| PanoptesError::Agent(format!("Anthropic request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(PanoptesError::Agent(format!(
                "Anthropic API error {status}: {body_text}"
            )));
        }

        let anthropic_response: AnthropicResponse = response.json().await.map_err(|e| {
            PanoptesError::Agent(format!("Failed to parse Anthropic response: {e}"))
        })?;

        let content = anthropic_response
            .content
            .into_iter()
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join("");

        Ok(LlmResponse {
            content,
            model: anthropic_response.model,
            usage: anthropic_response.usage.map(|u| TokenUsage {
                prompt_tokens: u.input_tokens,
                completion_tokens: u.output_tokens,
            }),
            finish_reason: anthropic_response.stop_reason,
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ChatMessage;

    #[test]
    fn request_body_matches_anthropic_format() {
        let client = AnthropicClient::new(
            "claude-sonnet-4-20250514".to_string(),
            "sk-ant-test".to_string(),
        );
        let request = LlmRequest {
            system_prompt: Some("Be helpful.".to_string()),
            messages: vec![
                ChatMessage {
                    role: Role::User,
                    content: "Hello".to_string(),
                },
                ChatMessage {
                    role: Role::Assistant,
                    content: "Hi there!".to_string(),
                },
                ChatMessage {
                    role: Role::User,
                    content: "How are you?".to_string(),
                },
            ],
            temperature: Some(0.7),
            max_tokens: Some(1024),
        };

        let body = client.build_request_body(&request);
        let json = serde_json::to_value(&body).unwrap();

        assert_eq!(json["model"], "claude-sonnet-4-20250514");
        assert_eq!(json["system"], "Be helpful.");
        let temp = json["temperature"].as_f64().unwrap();
        assert!((temp - 0.7).abs() < 0.001);
        assert_eq!(json["max_tokens"], 1024);

        let messages = json["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["type"], "text");
        assert_eq!(messages[0]["content"][0]["text"], "Hello");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["role"], "user");
    }

    #[test]
    fn system_prompt_is_top_level_not_in_messages() {
        let client =
            AnthropicClient::new("claude-sonnet-4-20250514".to_string(), "key".to_string());
        let request = LlmRequest {
            system_prompt: Some("System instruction".to_string()),
            messages: vec![ChatMessage {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            temperature: None,
            max_tokens: None,
        };

        let body = client.build_request_body(&request);
        let json = serde_json::to_value(&body).unwrap();

        // System should be top-level
        assert_eq!(json["system"], "System instruction");

        // No message should have role "system"
        let messages = json["messages"].as_array().unwrap();
        for msg in messages {
            assert_ne!(msg["role"], "system");
        }
    }

    #[test]
    fn default_max_tokens_when_none() {
        let client =
            AnthropicClient::new("claude-sonnet-4-20250514".to_string(), "key".to_string());
        let request = LlmRequest {
            system_prompt: None,
            messages: vec![ChatMessage {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            temperature: None,
            max_tokens: None,
        };

        let body = client.build_request_body(&request);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["max_tokens"], 4096);
    }
}
