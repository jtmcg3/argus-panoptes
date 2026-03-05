//! Orchestrator: delegates tasks to agents via A2A HTTP calls.
//!
//! The orchestrator sends JSON-RPC `message/send` requests to the target
//! agent's A2A endpoint and returns the result. For streaming, it uses
//! `message/stream` and forwards SSE events.

use crate::discovery::AgentRegistry;
use crate::triage::{TriageDecision, TriageEngine};
use futures::StreamExt;
use panoptes_a2a::ProgressEvent;
use panoptes_common::{PanoptesError, Result};
use panoptes_llm::LlmClient;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// The orchestrator coordinates triage and delegation.
pub struct Orchestrator {
    triage: TriageEngine,
    registry: AgentRegistry,
    http_client: reqwest::Client,
}

impl Orchestrator {
    pub fn new(
        llm_client: Arc<dyn LlmClient>,
        registry: AgentRegistry,
        delegate_timeout: Option<Duration>,
    ) -> Self {
        let mut client_builder = reqwest::Client::builder();
        if let Some(timeout) = delegate_timeout {
            client_builder = client_builder.timeout(timeout);
        }

        Self {
            triage: TriageEngine::new(llm_client),
            registry,
            http_client: client_builder
                .build()
                .expect("Failed to create HTTP client"),
        }
    }

    /// Process a user message: triage, then delegate to the appropriate agent.
    pub async fn process(
        &self,
        input: &str,
        progress_tx: Option<broadcast::Sender<ProgressEvent>>,
    ) -> Result<String> {
        // Phase 1: Triage
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::Phase {
                name: "triage".into(),
                detail: "Analyzing request".into(),
            });
        }

        let decision = match self.triage.triage(input).await {
            Ok(d) => d,
            Err(e) => {
                warn!(error = %e, "LLM triage failed, falling back to keyword triage");
                self.triage.keyword_triage(input)?
            }
        };

        info!(
            agent = %decision.agent_name,
            confidence = %decision.confidence,
            reasoning = %decision.reasoning,
            "Triage decision"
        );

        // Handle direct responses (no agent delegation needed)
        if decision.agent_name == "direct" {
            return Ok(format!(
                "I'm not sure how to handle this request. Could you provide more context?\n\n(Triage reasoning: {})",
                decision.reasoning
            ));
        }

        // Phase 2: Discover agent
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::Phase {
                name: "routing".into(),
                detail: format!("Routing to {}", decision.agent_name),
            });
        }

        let agent = self.registry.get(&decision.agent_name).ok_or_else(|| {
            PanoptesError::Agent(format!(
                "Agent '{}' not found in registry. Is it running?",
                decision.agent_name
            ))
        })?;

        // Phase 3: Delegate via A2A
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::Phase {
                name: "delegating".into(),
                detail: format!("Sending task to {}", decision.agent_name),
            });
        }

        if let Some(tx) = progress_tx {
            self.delegate_streaming(&agent.base_url, &decision, tx)
                .await
        } else {
            self.delegate(&agent.base_url, &decision).await
        }
    }

    /// Send a JSON-RPC `message/send` request to an agent.
    async fn delegate(&self, base_url: &str, decision: &TriageDecision) -> Result<String> {
        let request_body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": "message/send",
            "params": {
                "message": {
                    "role": "user",
                    "parts": [
                        {
                            "type": "text",
                            "text": decision.instruction
                        }
                    ]
                }
            }
        });

        let response = self
            .http_client
            .post(base_url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| {
                PanoptesError::Agent(format!("Failed to reach agent at {}: {}", base_url, e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(PanoptesError::Agent(format!(
                "Agent returned error {}: {}",
                status, body
            )));
        }

        let json_response: serde_json::Value = response
            .json()
            .await
            .map_err(|e| PanoptesError::Agent(format!("Failed to parse agent response: {}", e)))?;

        // Extract result from JSON-RPC response
        if let Some(error) = json_response.get("error") {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            return Err(PanoptesError::Agent(format!("Agent error: {}", message)));
        }

        // Extract text from task artifacts
        let result = json_response
            .get("result")
            .and_then(|r| r.get("artifacts"))
            .and_then(|a| a.as_array())
            .and_then(|artifacts| {
                artifacts.first().and_then(|a| {
                    a.get("parts").and_then(|p| p.as_array()).and_then(|parts| {
                        parts
                            .iter()
                            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                            .next()
                    })
                })
            })
            .unwrap_or("No response from agent");

        Ok(result.to_string())
    }

    /// Send a JSON-RPC `message/stream` request to an agent and parse SSE events.
    async fn delegate_streaming(
        &self,
        base_url: &str,
        decision: &TriageDecision,
        progress_tx: broadcast::Sender<ProgressEvent>,
    ) -> Result<String> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let request_body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "message/stream",
            "params": {
                "message": {
                    "role": "user",
                    "parts": [
                        {
                            "type": "text",
                            "text": decision.instruction
                        }
                    ]
                }
            }
        });

        let response = self
            .http_client
            .post(base_url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| {
                PanoptesError::Agent(format!("Stream request failed to {}: {}", base_url, e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(PanoptesError::Agent(format!(
                "Agent returned error {}: {}",
                status, body
            )));
        }

        let mut stream = response.bytes_stream();
        let mut result_text = String::new();
        let mut buffer = String::new();
        let mut task_done = false;

        while !task_done {
            let Some(chunk) = stream.next().await else {
                break;
            };

            let chunk =
                chunk.map_err(|e| PanoptesError::Agent(format!("Stream read error: {}", e)))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Parse SSE events from buffer (format: "event: <type>\ndata: <json>\n\n")
            while let Some(event_end) = buffer.find("\n\n") {
                let event_block = buffer[..event_end].to_string();
                buffer = buffer[event_end + 2..].to_string();

                let (event_type, data) = parse_sse_event_block(&event_block);
                debug!(event_type = %event_type, "SSE event received");

                match event_type.as_str() {
                    "status" => {
                        let (state, detail) = parse_status_event(&data);
                        let _ = progress_tx.send(ProgressEvent::Phase {
                            name: "agent-status".into(),
                            detail,
                        });

                        match state.as_deref() {
                            Some("completed") | Some("canceled") => {
                                task_done = true;
                                break;
                            }
                            Some("failed") => {
                                return Err(PanoptesError::Agent(format!(
                                    "Agent task failed: {}",
                                    data
                                )));
                            }
                            _ => {}
                        }
                    }
                    "artifact" => {
                        extract_artifact_text(&data, &mut result_text);
                    }
                    "error" => {
                        return Err(PanoptesError::Agent(format!(
                            "Agent stream error: {}",
                            data
                        )));
                    }
                    _ => {}
                }
            }
        }

        if result_text.is_empty() {
            result_text = "Task completed (no artifact returned)".to_string();
        }
        Ok(result_text)
    }

    /// Get a reference to the agent registry.
    pub fn registry(&self) -> &AgentRegistry {
        &self.registry
    }
}

fn parse_sse_event_block(block: &str) -> (String, String) {
    let mut event_type = String::new();
    let mut data_lines = Vec::new();

    for line in block.lines() {
        if let Some(t) = line.strip_prefix("event: ") {
            event_type = t.to_string();
        } else if let Some(d) = line.strip_prefix("data: ") {
            data_lines.push(d.to_string());
        }
    }

    (event_type, data_lines.join("\n"))
}

fn parse_status_event(data: &str) -> (Option<String>, String) {
    let value = match serde_json::from_str::<serde_json::Value>(data) {
        Ok(v) => v,
        Err(_) => return (None, data.to_string()),
    };

    let status = value.get("status");
    let state = status
        .and_then(|s| s.get("state"))
        .and_then(|s| s.as_str())
        .map(str::to_string);

    let detail = status
        .and_then(|s| s.get("message"))
        .and_then(|m| m.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| data.to_string());

    (state, detail)
}

fn extract_artifact_text(data: &str, result_text: &mut String) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
        return;
    };

    let artifact = value.get("artifact").unwrap_or(&value);
    if let Some(parts) = artifact.get("parts").and_then(|p| p.as_array()) {
        for part in parts {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                result_text.push_str(text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_a2a::{AgentCapabilities, AgentCard, AgentSkill};

    fn mock_registry() -> AgentRegistry {
        let mut registry = AgentRegistry::new();
        registry.register(
            "http://localhost:9001".into(),
            AgentCard {
                name: "panoptes-research".into(),
                description: "Research agent".into(),
                url: Some("http://localhost:9001/".into()),
                version: "0.1.0".into(),
                capabilities: AgentCapabilities {
                    streaming: true,
                    push_notifications: false,
                },
                skills: vec![AgentSkill {
                    id: "web-research".into(),
                    name: "Web Research".into(),
                    description: "Search the web".into(),
                    tags: vec!["research".into()],
                    examples: vec![],
                }],
                default_input_modes: vec!["text/plain".into()],
                default_output_modes: vec!["text/plain".into()],
            },
        );
        registry
    }

    #[test]
    fn test_registry_has_agent() {
        let registry = mock_registry();
        assert!(registry.get("panoptes-research").is_some());
        assert!(registry.get("panoptes-writing").is_none());
    }
}
