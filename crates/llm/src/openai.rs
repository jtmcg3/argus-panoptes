use async_trait::async_trait;
use panoptes_common::PanoptesError;
use panoptes_common::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::{LlmClient, LlmRequest, LlmResponse, Role, TokenUsage};

const DEFAULT_BASE_URL: &str = "http://host.orb.internal:11434";

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct OpenAiMessage {
    role: String,
    #[serde(default, deserialize_with = "deserialize_content")]
    content: String,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    model: String,
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

fn content_from_value(value: Value) -> String {
    match value {
        Value::String(s) => s,
        Value::Null => String::new(),
        Value::Array(items) => {
            let mut chunks = Vec::new();
            for item in items {
                match item {
                    Value::String(s) => {
                        if !s.trim().is_empty() {
                            chunks.push(s);
                        }
                    }
                    Value::Object(map) => {
                        if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                            if !text.trim().is_empty() {
                                chunks.push(text.to_string());
                            }
                        } else if let Some(text) = map.get("content").and_then(|v| v.as_str()) {
                            if !text.trim().is_empty() {
                                chunks.push(text.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            chunks.join("\n")
        }
        _ => String::new(),
    }
}

fn deserialize_content<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(content_from_value(value))
}

fn resolve_message_content(message: &OpenAiMessage) -> String {
    if !message.content.trim().is_empty() {
        return message.content.clone();
    }

    [
        message.reasoning_content.as_deref(),
        message.reasoning.as_deref(),
        message.thinking.as_deref(),
        message.text.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find(|s| !s.trim().is_empty())
    .map(str::to_string)
    .unwrap_or_default()
}

pub struct OpenAiClient {
    base_url: String,
    model: String,
    api_key: Option<String>,
    http_client: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(base_url: Option<String>, model: String, api_key: Option<String>) -> Self {
        Self {
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            model,
            api_key,
            http_client: reqwest::Client::new(),
        }
    }

    fn role_to_string(role: &Role) -> &'static str {
        match role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    fn build_messages(request: &LlmRequest) -> Vec<OpenAiMessage> {
        let mut messages = Vec::new();
        if let Some(ref system) = request.system_prompt {
            messages.push(OpenAiMessage {
                role: "system".to_string(),
                content: system.clone(),
                reasoning_content: None,
                reasoning: None,
                thinking: None,
                text: None,
            });
        }
        for msg in &request.messages {
            messages.push(OpenAiMessage {
                role: Self::role_to_string(&msg.role).to_string(),
                content: msg.content.clone(),
                reasoning_content: None,
                reasoning: None,
                thinking: None,
                text: None,
            });
        }
        messages
    }

    /// Build the request body for testing purposes.
    #[cfg(test)]
    fn build_request_body(&self, request: &LlmRequest) -> OpenAiRequest {
        OpenAiRequest {
            model: self.model.clone(),
            messages: Self::build_messages(request),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        }
    }
}

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let body = OpenAiRequest {
            model: self.model.clone(),
            messages: Self::build_messages(&request),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        };

        let mut http_req = self.http_client.post(&url).json(&body);
        if let Some(ref key) = self.api_key {
            http_req = http_req.bearer_auth(key);
        }

        let response = http_req
            .send()
            .await
            .map_err(|e| PanoptesError::Agent(format!("OpenAI request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(PanoptesError::Agent(format!(
                "OpenAI API error {status}: {body_text}"
            )));
        }

        let oai_response: OpenAiResponse = response
            .json()
            .await
            .map_err(|e| PanoptesError::Agent(format!("Failed to parse OpenAI response: {e}")))?;

        let choice = oai_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| PanoptesError::Agent("No choices in OpenAI response".to_string()))?;

        let content = resolve_message_content(&choice.message);
        if content.trim().is_empty() {
            let finish_reason = choice.finish_reason.unwrap_or_else(|| "unknown".to_string());
            return Err(PanoptesError::Agent(format!(
                "OpenAI response contained empty assistant content (finish_reason={finish_reason})"
            )));
        }

        Ok(LlmResponse {
            content,
            model: oai_response.model,
            usage: oai_response.usage.map(|u| TokenUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
            }),
            finish_reason: choice.finish_reason,
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
    fn request_body_matches_openai_format() {
        let client = OpenAiClient::new(None, "gpt-4".to_string(), Some("sk-test".to_string()));
        let request = LlmRequest {
            system_prompt: Some("Be helpful.".to_string()),
            messages: vec![ChatMessage {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            temperature: Some(0.5),
            max_tokens: Some(512),
        };

        let body = client.build_request_body(&request);
        let json = serde_json::to_value(&body).unwrap();

        assert_eq!(json["model"], "gpt-4");
        assert_eq!(json["temperature"], 0.5);
        assert_eq!(json["max_tokens"], 512);

        let messages = json["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "Be helpful.");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "Hello");
    }

    #[test]
    fn request_body_omits_system_when_none() {
        let client = OpenAiClient::new(None, "gpt-4".to_string(), None);
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

        let messages = json["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");

        // temperature and max_tokens should be absent (skip_serializing_if)
        assert!(json.get("temperature").is_none());
        assert!(json.get("max_tokens").is_none());
    }

    #[test]
    fn default_base_url_is_ollama() {
        let client = OpenAiClient::new(None, "llama3".to_string(), None);
        assert_eq!(client.base_url, "http://host.orb.internal:11434");
    }

    #[test]
    fn deserialize_content_from_array_parts() {
        let value = serde_json::json!([
            {"type":"text","text":"line1"},
            {"type":"output_text","text":"line2"}
        ]);
        assert_eq!(content_from_value(value), "line1\nline2");
    }

    #[test]
    fn resolve_message_content_falls_back_to_reasoning() {
        let message = OpenAiMessage {
            role: "assistant".to_string(),
            content: String::new(),
            reasoning_content: Some("{\"route\":\"planning\"}".to_string()),
            reasoning: None,
            thinking: None,
            text: None,
        };
        assert_eq!(resolve_message_content(&message), "{\"route\":\"planning\"}");
    }
}
