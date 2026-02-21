//! Core coordinator implementation using ZeroClaw.

use crate::config::CoordinatorConfig;
use crate::routing::{AgentRoute, RouteDecision};
use panoptes_common::{AgentMessage, PanoptesError, Result, Task};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// The main coordinator that orchestrates all agents.
///
/// Uses ZeroClaw for intelligent triage and routing decisions.
pub struct Coordinator {
    config: CoordinatorConfig,
    // TODO: ZeroClaw integration
    // zeroclaw: zeroclaw::Agent,

    // Active tasks being coordinated
    active_tasks: Arc<RwLock<Vec<Task>>>,

    // MCP client connections to agents
    // agent_clients: HashMap<String, McpClient>,
}

impl Coordinator {
    /// Create a new coordinator with the given configuration.
    pub fn new(config: CoordinatorConfig) -> Result<Self> {
        info!("Initializing Panoptes coordinator");

        // TODO: Initialize ZeroClaw with config
        // let zeroclaw = zeroclaw::Agent::builder()
        //     .provider(config.provider.provider_type)
        //     .model(config.provider.model)
        //     .build()?;

        Ok(Self {
            config,
            active_tasks: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Analyze a user message and decide which agent(s) should handle it.
    pub async fn triage(&self, message: &AgentMessage) -> Result<RouteDecision> {
        info!(
            message_id = %message.id,
            content_preview = %message.content.chars().take(50).collect::<String>(),
            "Triaging message"
        );

        // TODO: Use ZeroClaw for actual triage
        // For now, implement keyword-based fallback
        let decision = self.keyword_triage(&message.content).await?;

        debug!(
            route = ?decision.route,
            confidence = decision.confidence,
            "Triage decision made"
        );

        Ok(decision)
    }

    /// Simple keyword-based triage (fallback when ZeroClaw unavailable).
    async fn keyword_triage(&self, content: &str) -> Result<RouteDecision> {
        let lower = content.to_lowercase();

        // Coding/development keywords
        if lower.contains("code")
            || lower.contains("fix")
            || lower.contains("bug")
            || lower.contains("implement")
            || lower.contains("refactor")
            || lower.contains("debug")
            || lower.contains("write a function")
            || lower.contains("create a")
        {
            let permission = if lower.contains("just do")
                || lower.contains("go ahead")
                || lower.contains("act mode")
            {
                crate::routing::PermissionMode::Act
            } else {
                crate::routing::PermissionMode::Plan
            };

            return Ok(RouteDecision {
                route: AgentRoute::PtyCoding {
                    instruction: content.to_string(),
                    working_dir: self.config.default_working_dir.as_ref().map(|p| p.display().to_string()),
                    permission_mode: permission,
                },
                reasoning: "Detected coding-related request".into(),
                confidence: 0.75,
                extracted_context: None,
            });
        }

        // Research keywords
        if lower.contains("research")
            || lower.contains("find out")
            || lower.contains("look up")
            || lower.contains("what is")
            || lower.contains("how does")
        {
            return Ok(RouteDecision {
                route: AgentRoute::Research {
                    query: content.to_string(),
                    sources: vec![],
                },
                reasoning: "Detected research request".into(),
                confidence: 0.7,
                extracted_context: None,
            });
        }

        // Writing keywords
        if lower.contains("write")
            || lower.contains("draft")
            || lower.contains("email")
            || lower.contains("document")
        {
            return Ok(RouteDecision {
                route: AgentRoute::Writing {
                    task_type: crate::routing::WritingTask::Documentation,
                    context: content.to_string(),
                },
                reasoning: "Detected writing request".into(),
                confidence: 0.7,
                extracted_context: None,
            });
        }

        // Planning keywords
        if lower.contains("plan")
            || lower.contains("schedule")
            || lower.contains("today")
            || lower.contains("this week")
        {
            return Ok(RouteDecision {
                route: AgentRoute::Planning {
                    scope: crate::routing::PlanningScope::Day,
                    context: content.to_string(),
                },
                reasoning: "Detected planning request".into(),
                confidence: 0.7,
                extracted_context: None,
            });
        }

        // Review keywords
        if lower.contains("review") || lower.contains("check this") {
            return Ok(RouteDecision {
                route: AgentRoute::CodeReview {
                    target: ".".into(),
                    review_type: crate::routing::ReviewType::Full,
                },
                reasoning: "Detected review request".into(),
                confidence: 0.6,
                extracted_context: None,
            });
        }

        // Test keywords
        if lower.contains("test") || lower.contains("coverage") {
            return Ok(RouteDecision {
                route: AgentRoute::Testing {
                    target: ".".into(),
                    test_type: crate::routing::TestType::Unit,
                },
                reasoning: "Detected testing request".into(),
                confidence: 0.6,
                extracted_context: None,
            });
        }

        // Default: direct response
        Ok(RouteDecision::direct(
            "I'm not sure how to handle this request. Could you provide more context?"
        ))
    }

    /// Execute a routing decision by dispatching to the appropriate agent.
    pub async fn execute(&self, decision: RouteDecision) -> Result<AgentMessage> {
        info!(route = ?decision.route, "Executing route decision");

        match decision.route {
            AgentRoute::Direct { response } => {
                Ok(AgentMessage::from_agent("coordinator", response))
            }

            AgentRoute::PtyCoding { instruction, working_dir, permission_mode } => {
                // TODO: Call PTY-MCP agent via MCP
                warn!("PTY-MCP agent not yet connected");
                Ok(AgentMessage::from_agent(
                    "coordinator",
                    format!(
                        "[TODO: Route to PTY-MCP agent]\nInstruction: {}\nWorking dir: {:?}\nMode: {:?}",
                        instruction, working_dir, permission_mode
                    ),
                ))
            }

            AgentRoute::Research { query, .. } => {
                // TODO: Call research agent
                warn!("Research agent not yet connected");
                Ok(AgentMessage::from_agent(
                    "coordinator",
                    format!("[TODO: Route to Research agent]\nQuery: {}", query),
                ))
            }

            AgentRoute::Writing { task_type, context } => {
                // TODO: Call writing agent
                warn!("Writing agent not yet connected");
                Ok(AgentMessage::from_agent(
                    "coordinator",
                    format!("[TODO: Route to Writing agent]\nType: {:?}\nContext: {}", task_type, context),
                ))
            }

            AgentRoute::Planning { scope, context } => {
                // TODO: Call planning agent
                warn!("Planning agent not yet connected");
                Ok(AgentMessage::from_agent(
                    "coordinator",
                    format!("[TODO: Route to Planning agent]\nScope: {:?}\nContext: {}", scope, context),
                ))
            }

            AgentRoute::CodeReview { target, review_type } => {
                // TODO: Call review agent
                warn!("Review agent not yet connected");
                Ok(AgentMessage::from_agent(
                    "coordinator",
                    format!("[TODO: Route to Review agent]\nTarget: {}\nType: {:?}", target, review_type),
                ))
            }

            AgentRoute::Testing { target, test_type } => {
                // TODO: Call testing agent
                warn!("Testing agent not yet connected");
                Ok(AgentMessage::from_agent(
                    "coordinator",
                    format!("[TODO: Route to Testing agent]\nTarget: {}\nType: {:?}", target, test_type),
                ))
            }

            AgentRoute::Workflow { tasks } => {
                // TODO: Orchestrate multi-agent workflow
                warn!("Workflow orchestration not yet implemented");
                Ok(AgentMessage::from_agent(
                    "coordinator",
                    format!("[TODO: Execute workflow with {} tasks]", tasks.len()),
                ))
            }
        }
    }

    /// Process a user message end-to-end: triage then execute.
    pub async fn process(&self, message: AgentMessage) -> Result<AgentMessage> {
        let decision = self.triage(&message).await?;
        self.execute(decision).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_keyword_triage_coding() {
        let coordinator = Coordinator::new(CoordinatorConfig::default()).unwrap();
        let msg = AgentMessage::user("Please fix the bug in the parser");
        let decision = coordinator.triage(&msg).await.unwrap();

        matches!(decision.route, AgentRoute::PtyCoding { .. });
    }

    #[tokio::test]
    async fn test_keyword_triage_research() {
        let coordinator = Coordinator::new(CoordinatorConfig::default()).unwrap();
        let msg = AgentMessage::user("Research the best practices for async Rust");
        let decision = coordinator.triage(&msg).await.unwrap();

        matches!(decision.route, AgentRoute::Research { .. });
    }
}
