//! Core coordinator implementation using ZeroClaw.

use crate::config::CoordinatorConfig;
use crate::pty_client::PtyMcpClient;
use crate::routing::{AgentRoute, PermissionMode, RouteDecision};
use crate::zeroclaw_triage::ZeroClawTriageAgent;
use panoptes_common::{AgentMessage, PanoptesError, Result, Task};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// The main coordinator that orchestrates all agents.
///
/// Uses ZeroClaw for intelligent triage and routing decisions when configured
/// with an OpenAI provider. Falls back to keyword-based routing otherwise.
pub struct Coordinator {
    config: CoordinatorConfig,

    /// ZeroClaw triage agent (if OpenAI is configured)
    triage_agent: Option<RwLock<ZeroClawTriageAgent>>,

    // Active tasks being coordinated
    active_tasks: Arc<RwLock<Vec<Task>>>,

    // MCP client for PTY sessions
    pty_client: Arc<RwLock<Option<PtyMcpClient>>>,

    // Session counter for generating unique session IDs
    session_counter: Arc<std::sync::atomic::AtomicU64>,
}

impl Coordinator {
    /// Create a new coordinator with the given configuration.
    ///
    /// If the provider is configured as "openai" and an API key is available,
    /// ZeroClaw will be used for intelligent LLM-based triage. Otherwise,
    /// keyword-based fallback routing is used.
    pub fn new(config: CoordinatorConfig) -> Result<Self> {
        info!("Initializing Panoptes coordinator");

        // Try to create ZeroClaw triage agent if OpenAI is configured
        let triage_agent = if config.provider.provider_type == "openai" {
            match ZeroClawTriageAgent::new(&config.provider) {
                Ok(agent) => {
                    info!(
                        model = %config.provider.model,
                        "ZeroClaw triage agent initialized with OpenAI"
                    );
                    Some(RwLock::new(agent))
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        "Failed to initialize ZeroClaw triage, using keyword fallback"
                    );
                    None
                }
            }
        } else {
            info!(
                provider = %config.provider.provider_type,
                "Non-OpenAI provider configured, using keyword triage"
            );
            None
        };

        Ok(Self {
            config,
            triage_agent,
            active_tasks: Arc::new(RwLock::new(Vec::new())),
            pty_client: Arc::new(RwLock::new(None)),
            session_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    /// Check if ZeroClaw triage is available.
    pub fn has_zeroclaw_triage(&self) -> bool {
        self.triage_agent.is_some()
    }

    /// Initialize and connect to the PTY-MCP server.
    ///
    /// This must be called before executing PtyCoding routes.
    /// The `server_binary` should be the path to the pty-mcp-server executable.
    pub async fn connect_pty_server(&self, server_binary: Option<PathBuf>) -> Result<()> {
        info!("Connecting to PTY-MCP server");

        let client = PtyMcpClient::new(server_binary);
        client.connect().await?;

        *self.pty_client.write().await = Some(client);

        info!("Successfully connected to PTY-MCP server");
        Ok(())
    }

    /// Check if the PTY-MCP client is connected.
    pub async fn is_pty_connected(&self) -> bool {
        if let Some(ref client) = *self.pty_client.read().await {
            client.is_connected().await
        } else {
            false
        }
    }

    /// Generate a unique session ID.
    fn generate_session_id(&self) -> String {
        let counter = self
            .session_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("session-{}", counter)
    }

    /// Analyze a user message and decide which agent(s) should handle it.
    ///
    /// Uses ZeroClaw LLM-based routing if OpenAI is configured,
    /// otherwise falls back to keyword-based routing.
    pub async fn triage(&self, message: &AgentMessage) -> Result<RouteDecision> {
        info!(
            message_id = %message.id,
            content_preview = %message.content.chars().take(50).collect::<String>(),
            using_zeroclaw = self.triage_agent.is_some(),
            "Triaging message"
        );

        let decision = if let Some(ref agent_lock) = self.triage_agent {
            // Use ZeroClaw for intelligent LLM-based routing
            let mut agent = agent_lock.write().await;
            match agent.triage(&message.content).await {
                Ok(decision) => {
                    debug!(
                        route = ?decision.route,
                        confidence = decision.confidence,
                        "ZeroClaw triage decision"
                    );
                    decision
                }
                Err(e) => {
                    // Fall back to keyword triage on ZeroClaw error
                    warn!(
                        error = %e,
                        "ZeroClaw triage failed, falling back to keyword triage"
                    );
                    self.keyword_triage(&message.content).await?
                }
            }
        } else {
            // Use keyword-based fallback
            self.keyword_triage(&message.content).await?
        };

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

            AgentRoute::PtyCoding {
                instruction,
                working_dir,
                permission_mode,
            } => {
                self.execute_pty_coding(&instruction, working_dir.as_deref(), permission_mode)
                    .await
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

    /// Execute a PtyCoding route by spawning a Claude CLI session.
    ///
    /// This method:
    /// 1. Ensures the PTY-MCP client is connected
    /// 2. Spawns a new PTY session with the claude CLI
    /// 3. Waits for initial output
    /// 4. Returns the session info and initial output
    async fn execute_pty_coding(
        &self,
        instruction: &str,
        working_dir: Option<&str>,
        permission_mode: PermissionMode,
    ) -> Result<AgentMessage> {
        // Check if PTY client is connected
        let client_guard = self.pty_client.read().await;
        let client = client_guard.as_ref().ok_or_else(|| {
            PanoptesError::Mcp(
                "PTY-MCP client not connected. Call connect_pty_server() first.".into(),
            )
        })?;

        // Determine working directory
        let work_dir = working_dir
            .map(String::from)
            .or_else(|| {
                self.config
                    .default_working_dir
                    .as_ref()
                    .map(|p| p.display().to_string())
            })
            .unwrap_or_else(|| ".".to_string());

        // Generate a unique session ID
        let session_id = self.generate_session_id();

        info!(
            session_id = %session_id,
            instruction = %instruction,
            working_dir = %work_dir,
            permission_mode = ?permission_mode,
            "Executing PtyCoding route"
        );

        // Build claude CLI arguments based on permission mode
        let mut args = vec!["-p".to_string(), instruction.to_string()];

        // Add permission mode flags
        match permission_mode {
            PermissionMode::Plan => {
                // Default plan mode - Claude will ask for confirmation
            }
            PermissionMode::Act => {
                // Act mode - dangerously allow all actions without confirmation
                args.push("--dangerously-skip-permissions".to_string());
            }
        }

        // Spawn the session
        let spawn_result = client
            .spawn_session(&session_id, "claude", &args, &work_dir)
            .await?;

        if !spawn_result.success {
            let error_msg = spawn_result
                .error
                .unwrap_or_else(|| "Unknown error spawning session".into());
            error!(session_id = %session_id, error = %error_msg, "Failed to spawn PTY session");
            return Err(PanoptesError::Pty(error_msg));
        }

        info!(session_id = %session_id, "PTY session spawned successfully");

        // Wait a bit for initial output
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Read initial output
        let read_result = client.get_output(&session_id, false).await?;

        // Build the response message with session info
        let response = serde_json::json!({
            "session_id": session_id,
            "status": read_result.status,
            "working_dir": work_dir,
            "permission_mode": format!("{:?}", permission_mode),
            "initial_output": read_result.output,
            "awaiting_confirmation": read_result.awaiting_confirmation,
            "instruction": instruction,
        });

        Ok(AgentMessage::from_agent("pty-mcp", response.to_string()))
    }

    /// Get the PTY-MCP client reference for direct session management.
    ///
    /// This allows callers to interact with active sessions (send input,
    /// read output, confirm prompts, etc.) after the initial spawn.
    pub async fn get_pty_client(&self) -> Result<impl std::ops::Deref<Target = PtyMcpClient> + '_> {
        let guard = self.pty_client.read().await;
        if guard.is_none() {
            return Err(PanoptesError::Mcp(
                "PTY-MCP client not connected. Call connect_pty_server() first.".into(),
            ));
        }
        // We need to map the guard to access the inner value
        // Using a wrapper that implements Deref
        Ok(PtyClientRef(guard))
    }
}

/// A reference wrapper that allows accessing the PtyMcpClient through a RwLockReadGuard.
pub struct PtyClientRef<'a>(tokio::sync::RwLockReadGuard<'a, Option<PtyMcpClient>>);

impl<'a> std::ops::Deref for PtyClientRef<'a> {
    type Target = PtyMcpClient;

    fn deref(&self) -> &Self::Target {
        // Safe because we checked is_some() before returning
        self.0.as_ref().unwrap()
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
