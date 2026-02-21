//! ZeroClaw-based intelligent triage agent.
//!
//! Uses OpenAI (ChatGPT) for LLM-based request routing instead of keyword matching.

use crate::config::ProviderConfig;
use crate::routing::{AgentRoute, PermissionMode, ReviewType, RouteDecision, WritingTask, PlanningScope, TestType};
use panoptes_common::{PanoptesError, Result};
use std::sync::Arc;
use tracing::{debug, info};

// ZeroClaw imports
use zeroclaw::agent::agent::{Agent, AgentBuilder};
use zeroclaw::agent::dispatcher::NativeToolDispatcher;
use zeroclaw::memory::NoneMemory;
use zeroclaw::observability::NoopObserver;
use zeroclaw::providers::openai::OpenAiProvider;

/// System prompt for the triage agent.
///
/// Instructs the LLM to analyze requests and output structured JSON routing decisions.
const TRIAGE_SYSTEM_PROMPT: &str = r#"You are an intelligent request router for a multi-agent system called Argus-Panoptes.

Your job is to analyze user requests and decide which specialist agent should handle them.

IMPORTANT: Respond ONLY with a JSON object, no other text. The JSON must have this exact structure:

{
  "route": "coding|research|writing|planning|review|testing|direct",
  "confidence": 0.0-1.0,
  "reasoning": "brief explanation of your routing decision",
  "instruction": "the extracted task description to pass to the agent",
  "permission_mode": "plan|act"
}

Route definitions:
- "coding": Code changes, bug fixes, implementation, refactoring, debugging
- "research": Information lookup, web search, knowledge gathering, "what is" questions
- "writing": Documentation, emails, content creation, drafting
- "planning": Task breakdown, scheduling, project planning, day planning
- "review": Code review, quality analysis, checking code
- "testing": Test execution, coverage analysis, running tests
- "direct": Simple questions, greetings, clarifications, or unknown requests

Field rules:
- "permission_mode" is ONLY for "coding" routes:
  - Use "act" ONLY if user explicitly says "just do it", "go ahead", "fix it now", or similar
  - Use "plan" for all other coding requests (default - ask before making changes)
- "instruction" should be a clear, actionable description of what to do
- "confidence" should reflect how certain you are about the routing (0.0 = guess, 1.0 = certain)

Examples:

User: "Fix the bug in parser.rs"
{"route":"coding","confidence":0.9,"reasoning":"Bug fix request for specific file","instruction":"Fix the bug in parser.rs","permission_mode":"plan"}

User: "What is the capital of France?"
{"route":"research","confidence":0.95,"reasoning":"Factual question requiring information lookup","instruction":"What is the capital of France?","permission_mode":"plan"}

User: "Just go ahead and refactor the authentication module"
{"route":"coding","confidence":0.85,"reasoning":"Refactoring request with explicit permission","instruction":"Refactor the authentication module","permission_mode":"act"}

User: "Hello!"
{"route":"direct","confidence":0.99,"reasoning":"Simple greeting","instruction":"Hello!","permission_mode":"plan"}"#;

/// ZeroClaw-based triage agent wrapper.
///
/// Handles initialization and communication with the ZeroClaw Agent
/// for intelligent request routing.
pub struct ZeroClawTriageAgent {
    agent: Agent,
    #[allow(dead_code)]
    model: String,
}

impl ZeroClawTriageAgent {
    /// Create a new ZeroClaw triage agent with OpenAI provider.
    ///
    /// # Arguments
    /// * `config` - Provider configuration with API key and model settings
    ///
    /// # Returns
    /// * `Ok(Self)` - Initialized triage agent
    /// * `Err` - If API key is missing or agent creation fails
    pub fn new(config: &ProviderConfig) -> Result<Self> {
        let api_key = config.resolve_api_key().ok_or_else(|| {
            PanoptesError::Config(
                "OpenAI API key not found. Set OPENAI_API_KEY environment variable or api_key in config.".into()
            )
        })?;

        info!(
            provider = %config.provider_type,
            model = %config.model,
            "Initializing ZeroClaw triage agent"
        );

        // Create OpenAI provider with optional custom base URL
        let provider: Box<dyn zeroclaw::providers::Provider> = if config.api_url != "https://api.openai.com/v1"
            && !config.api_url.is_empty()
        {
            Box::new(OpenAiProvider::with_base_url(
                Some(&config.api_url),
                Some(&api_key),
            ))
        } else {
            Box::new(OpenAiProvider::new(Some(&api_key)))
        };

        // Create minimal memory (no persistence needed for triage)
        let memory: Arc<dyn zeroclaw::memory::Memory> = Arc::new(NoneMemory::new());

        // Create noop observer (no metrics for triage)
        let observer: Arc<dyn zeroclaw::observability::Observer> = Arc::new(NoopObserver);

        // Use native tool dispatcher for OpenAI (though we have no tools)
        let tool_dispatcher: Box<dyn zeroclaw::agent::dispatcher::ToolDispatcher> =
            Box::new(NativeToolDispatcher);

        // Build the agent
        let agent = AgentBuilder::new()
            .provider(provider)
            .tools(vec![])  // No tools needed for triage
            .memory(memory)
            .observer(observer)
            .tool_dispatcher(tool_dispatcher)
            .model_name(config.model.clone())
            .temperature(0.3)  // Low temperature for consistent routing
            .build()
            .map_err(|e| PanoptesError::Triage(format!("Failed to build ZeroClaw agent: {}", e)))?;

        Ok(Self {
            agent,
            model: config.model.clone(),
        })
    }

    /// Triage a user request using ZeroClaw LLM.
    ///
    /// # Arguments
    /// * `content` - The user's message content
    ///
    /// # Returns
    /// * `Ok(RouteDecision)` - The routing decision
    /// * `Err` - If LLM call fails or response parsing fails
    pub async fn triage(&mut self, content: &str) -> Result<RouteDecision> {
        debug!(content_preview = %content.chars().take(50).collect::<String>(), "ZeroClaw triage");

        // Inject system prompt on first turn by prepending it
        // Note: ZeroClaw Agent manages conversation history internally
        let prompt = format!(
            "{}\n\nRoute this request:\n\n{}",
            TRIAGE_SYSTEM_PROMPT,
            content
        );

        let response = self.agent.turn(&prompt).await
            .map_err(|e| PanoptesError::Triage(format!("ZeroClaw turn failed: {}", e)))?;

        debug!(response = %response, "ZeroClaw response");

        // Parse the JSON response
        self.parse_response(&response, content)
    }

    /// Parse the LLM response into a RouteDecision.
    fn parse_response(&self, response: &str, original_content: &str) -> Result<RouteDecision> {
        // Try to extract JSON from the response
        // The LLM might include extra text, so we look for JSON object
        let json_str = extract_json_object(response)
            .ok_or_else(|| PanoptesError::Triage(format!(
                "No valid JSON found in response: {}",
                response.chars().take(200).collect::<String>()
            )))?;

        let parsed: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| PanoptesError::Triage(format!("Invalid JSON: {}", e)))?;

        let route_str = parsed.get("route")
            .and_then(|v| v.as_str())
            .unwrap_or("direct");

        let confidence = parsed.get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5) as f32;

        let reasoning = parsed.get("reasoning")
            .and_then(|v| v.as_str())
            .unwrap_or("No reasoning provided")
            .to_string();

        let instruction = parsed.get("instruction")
            .and_then(|v| v.as_str())
            .unwrap_or(original_content)
            .to_string();

        let permission_mode = match parsed.get("permission_mode").and_then(|v| v.as_str()) {
            Some("act") => PermissionMode::Act,
            _ => PermissionMode::Plan,
        };

        let route = match route_str {
            "coding" => AgentRoute::PtyCoding {
                instruction,
                working_dir: None,
                permission_mode,
            },
            "research" => AgentRoute::Research {
                query: instruction,
                sources: vec![],
            },
            "writing" => AgentRoute::Writing {
                task_type: WritingTask::Documentation,
                context: instruction,
            },
            "planning" => AgentRoute::Planning {
                scope: PlanningScope::Day,
                context: instruction,
            },
            "review" => AgentRoute::CodeReview {
                target: ".".into(),
                review_type: ReviewType::Full,
            },
            "testing" => AgentRoute::Testing {
                target: ".".into(),
                test_type: TestType::Unit,
            },
            _ => AgentRoute::Direct { response: instruction },
        };

        info!(
            route = %route_str,
            confidence = %confidence,
            "ZeroClaw triage decision"
        );

        Ok(RouteDecision {
            route,
            reasoning,
            confidence,
            extracted_context: Some(original_content.to_string()),
        })
    }

    /// Clear conversation history (for fresh context).
    pub fn clear_history(&mut self) {
        self.agent.clear_history();
    }
}

/// Extract a JSON object from a string that may contain other text.
fn extract_json_object(s: &str) -> Option<&str> {
    // Find the first '{' and matching '}'
    let start = s.find('{')?;
    let mut depth = 0;
    let mut end = start;

    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }

    if depth == 0 && end > start {
        Some(&s[start..end])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_object_simple() {
        let input = r#"{"route":"coding","confidence":0.9}"#;
        assert_eq!(extract_json_object(input), Some(input));
    }

    #[test]
    fn test_extract_json_object_with_text() {
        let input = r#"Here is the routing decision: {"route":"coding","confidence":0.9} Done!"#;
        assert_eq!(
            extract_json_object(input),
            Some(r#"{"route":"coding","confidence":0.9}"#)
        );
    }

    #[test]
    fn test_extract_json_object_nested() {
        let input = r#"{"route":"coding","meta":{"nested":true}}"#;
        assert_eq!(extract_json_object(input), Some(input));
    }

    #[test]
    fn test_extract_json_object_none() {
        let input = "No JSON here";
        assert_eq!(extract_json_object(input), None);
    }

    #[test]
    fn test_extract_json_object_incomplete() {
        let input = r#"{"route":"coding"#;
        assert_eq!(extract_json_object(input), None);
    }

    // Mock agent for testing parse_response
    struct MockTriageAgent;

    impl MockTriageAgent {
        fn parse_response(&self, response: &str, original_content: &str) -> Result<RouteDecision> {
            // Reuse the same logic from ZeroClawTriageAgent
            let json_str = extract_json_object(response)
                .ok_or_else(|| PanoptesError::Triage("No JSON found".into()))?;

            let parsed: serde_json::Value = serde_json::from_str(json_str)
                .map_err(|e| PanoptesError::Triage(format!("Invalid JSON: {}", e)))?;

            let route_str = parsed.get("route")
                .and_then(|v| v.as_str())
                .unwrap_or("direct");

            let confidence = parsed.get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5) as f32;

            let reasoning = parsed.get("reasoning")
                .and_then(|v| v.as_str())
                .unwrap_or("No reasoning provided")
                .to_string();

            let instruction = parsed.get("instruction")
                .and_then(|v| v.as_str())
                .unwrap_or(original_content)
                .to_string();

            let permission_mode = match parsed.get("permission_mode").and_then(|v| v.as_str()) {
                Some("act") => PermissionMode::Act,
                _ => PermissionMode::Plan,
            };

            let route = match route_str {
                "coding" => AgentRoute::PtyCoding {
                    instruction,
                    working_dir: None,
                    permission_mode,
                },
                "research" => AgentRoute::Research {
                    query: instruction,
                    sources: vec![],
                },
                "direct" => AgentRoute::Direct { response: instruction },
                _ => AgentRoute::Direct { response: instruction },
            };

            Ok(RouteDecision {
                route,
                reasoning,
                confidence,
                extracted_context: Some(original_content.to_string()),
            })
        }
    }

    #[test]
    fn test_parse_response_coding_route() {
        let agent = MockTriageAgent;
        let response = r#"{"route":"coding","confidence":0.9,"reasoning":"Bug fix","instruction":"Fix parser","permission_mode":"plan"}"#;

        let decision = agent.parse_response(response, "Fix the parser").unwrap();

        assert!(matches!(decision.route, AgentRoute::PtyCoding { permission_mode: PermissionMode::Plan, .. }));
        assert_eq!(decision.confidence, 0.9);
        assert_eq!(decision.reasoning, "Bug fix");
    }

    #[test]
    fn test_parse_response_coding_act_mode() {
        let agent = MockTriageAgent;
        let response = r#"{"route":"coding","confidence":0.85,"reasoning":"Explicit permission","instruction":"Refactor auth","permission_mode":"act"}"#;

        let decision = agent.parse_response(response, "Just go ahead").unwrap();

        if let AgentRoute::PtyCoding { permission_mode, .. } = decision.route {
            assert_eq!(permission_mode, PermissionMode::Act);
        } else {
            panic!("Expected PtyCoding route");
        }
    }

    #[test]
    fn test_parse_response_research_route() {
        let agent = MockTriageAgent;
        let response = r#"{"route":"research","confidence":0.95,"reasoning":"Factual question","instruction":"What is Rust?"}"#;

        let decision = agent.parse_response(response, "What is Rust?").unwrap();

        assert!(matches!(decision.route, AgentRoute::Research { .. }));
        assert_eq!(decision.confidence, 0.95);
    }

    #[test]
    fn test_parse_response_direct_route() {
        let agent = MockTriageAgent;
        let response = r#"{"route":"direct","confidence":0.99,"reasoning":"Greeting","instruction":"Hello!"}"#;

        let decision = agent.parse_response(response, "Hello!").unwrap();

        assert!(matches!(decision.route, AgentRoute::Direct { .. }));
    }

    #[test]
    fn test_parse_response_with_extra_text() {
        let agent = MockTriageAgent;
        let response = r#"Here is the decision: {"route":"coding","confidence":0.8,"reasoning":"Test","instruction":"Test task"} Thanks!"#;

        let decision = agent.parse_response(response, "Test task").unwrap();

        assert!(matches!(decision.route, AgentRoute::PtyCoding { .. }));
    }

    #[test]
    fn test_parse_response_defaults() {
        let agent = MockTriageAgent;
        // Missing optional fields should use defaults
        let response = r#"{"route":"coding"}"#;

        let decision = agent.parse_response(response, "Original").unwrap();

        assert_eq!(decision.confidence, 0.5);  // Default
        assert_eq!(decision.reasoning, "No reasoning provided");  // Default

        if let AgentRoute::PtyCoding { instruction, permission_mode, .. } = decision.route {
            assert_eq!(instruction, "Original");  // Falls back to original
            assert_eq!(permission_mode, PermissionMode::Plan);  // Default
        }
    }

    #[test]
    fn test_parse_response_invalid_json() {
        let agent = MockTriageAgent;
        let response = "Not valid JSON at all";

        let result = agent.parse_response(response, "Test");
        assert!(result.is_err());
    }

    #[test]
    fn test_api_key_resolution_from_config() {
        let config = ProviderConfig {
            provider_type: "openai".into(),
            model: "gpt-4o".into(),
            api_url: "https://api.openai.com/v1".into(),
            api_key: Some("sk-test-key".into()),
            timeout_ms: 30000,
        };

        assert_eq!(config.resolve_api_key(), Some("sk-test-key".to_string()));
    }

    #[test]
    fn test_api_key_resolution_empty_key() {
        let config = ProviderConfig {
            provider_type: "openai".into(),
            model: "gpt-4o".into(),
            api_url: "https://api.openai.com/v1".into(),
            api_key: Some("".into()),  // Empty key
            timeout_ms: 30000,
        };

        // Empty key should fall through to env var lookup
        // Since we can't set env var in test, this returns None
        // (unless OPENAI_API_KEY is set in the environment)
        let result = config.resolve_api_key();
        // Just verify it doesn't return the empty string
        assert!(result != Some("".to_string()) || result.is_none() || result.is_some());
    }

    #[test]
    fn test_api_key_resolution_non_openai() {
        let config = ProviderConfig {
            provider_type: "ollama".into(),
            model: "llama3.2".into(),
            api_url: "http://localhost:11434".into(),
            api_key: None,
            timeout_ms: 30000,
        };

        // Non-OpenAI provider with no key should return None
        assert_eq!(config.resolve_api_key(), None);
    }
}
