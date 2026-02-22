//! Coding agent - uses PTY-MCP to interact with Claude CLI.
//!
//! This agent spawns Claude CLI sessions via the PTY-MCP server and handles
//! the full lifecycle: spawning, streaming output, handling confirmation
//! prompts, and collecting results.

use crate::traits::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use panoptes_common::{AgentMessage, PanoptesError, Result, Task};
use panoptes_coordinator::pty_client::{PtyMcpClient, StatusResult};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

const CODING_SYSTEM_PROMPT: &str = r#"You are a senior software engineer assistant. Your role is to:

1. Understand coding tasks and break them into actionable steps
2. Write clean, well-documented code following best practices
3. Debug issues methodically
4. Suggest improvements when you see opportunities
5. Ask clarifying questions when requirements are ambiguous

You have access to PTY tools that let you run Claude CLI for actual code execution.
Always explain your reasoning before making changes.
Prefer small, incremental changes over large refactors.
"#;

/// Configuration for auto-confirmation behavior.
#[derive(Debug, Clone)]
pub struct ConfirmationPolicy {
    /// Whether to auto-confirm all prompts
    pub auto_confirm: bool,
    /// Maximum number of confirmations before requiring manual approval
    pub max_auto_confirms: usize,
    /// Patterns that should always require manual confirmation
    pub dangerous_patterns: Vec<String>,
}

impl Default for ConfirmationPolicy {
    fn default() -> Self {
        Self {
            auto_confirm: true, // Default to auto-confirm for autonomous operation
            max_auto_confirms: 10,
            dangerous_patterns: vec![
                "delete".into(),
                "remove".into(),
                "rm -rf".into(),
                "drop table".into(),
                "force push".into(),
            ],
        }
    }
}

/// Coding agent that uses PTY-MCP for code execution.
///
/// This agent connects to a PTY-MCP server to spawn Claude CLI sessions.
/// It handles the full interaction lifecycle including:
/// - Session spawning with proper working directory
/// - Output streaming and collection
/// - Automatic handling of confirmation prompts
/// - Session cleanup
pub struct CodingAgent {
    config: AgentConfig,
    busy: AtomicBool,
    /// MCP client for PTY tools
    pty_client: Arc<RwLock<PtyMcpClient>>,
    /// Whether the client has been connected
    connected: AtomicBool,
    /// Path to the PTY-MCP server binary
    server_binary: Option<PathBuf>,
    /// Confirmation handling policy
    confirmation_policy: ConfirmationPolicy,
    /// Poll interval for checking session status
    poll_interval: Duration,
    /// Maximum time to wait for a session to complete
    max_session_duration: Duration,
}

impl CodingAgent {
    /// Create a new CodingAgent with the given configuration.
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            busy: AtomicBool::new(false),
            pty_client: Arc::new(RwLock::new(PtyMcpClient::new(None))),
            connected: AtomicBool::new(false),
            server_binary: None,
            confirmation_policy: ConfirmationPolicy::default(),
            poll_interval: Duration::from_millis(500),
            max_session_duration: Duration::from_secs(600), // 10 minutes default
        }
    }

    /// Create a CodingAgent with default configuration.
    pub fn with_default_config() -> Self {
        Self::new(AgentConfig {
            id: "coding".into(),
            name: "Coding Agent".into(),
            mcp_servers: vec!["pty-mcp".into()],
            ..Default::default()
        })
    }

    /// Set the path to the PTY-MCP server binary.
    pub fn with_server_binary(mut self, path: PathBuf) -> Self {
        self.server_binary = Some(path.clone());
        self.pty_client = Arc::new(RwLock::new(PtyMcpClient::new(Some(path))));
        self
    }

    /// Set the confirmation policy.
    pub fn with_confirmation_policy(mut self, policy: ConfirmationPolicy) -> Self {
        self.confirmation_policy = policy;
        self
    }

    /// Set the poll interval for checking session status.
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Set the maximum session duration.
    pub fn with_max_duration(mut self, duration: Duration) -> Self {
        self.max_session_duration = duration;
        self
    }

    /// Ensure the PTY-MCP client is connected.
    async fn ensure_connected(&self) -> Result<()> {
        if self.connected.load(Ordering::SeqCst) {
            // Already connected, check if still valid
            let client = self.pty_client.read().await;
            if client.is_connected().await {
                return Ok(());
            }
            // Connection lost, need to reconnect
            drop(client);
        }

        info!(agent = %self.id(), "Connecting to PTY-MCP server");

        let client = self.pty_client.write().await;
        client.connect().await?;

        self.connected.store(true, Ordering::SeqCst);
        info!(agent = %self.id(), "Connected to PTY-MCP server");

        Ok(())
    }

    /// Generate a unique session ID for this task.
    fn generate_session_id(&self, task: &Task) -> String {
        format!("{}-{}", task.id, Uuid::new_v4().to_string().split('-').next().unwrap_or("0000"))
    }

    /// Build Claude CLI arguments from the task.
    fn build_claude_args(&self, task: &Task) -> Vec<String> {
        let mut args = vec!["-p".to_string()];

        // Build the instruction from task description and context
        let mut instruction = task.description.clone();

        if let Some(context) = &task.context {
            instruction = format!("{}\n\nContext:\n{}", instruction, context);
        }

        args.push(instruction);

        args
    }

    /// Check if a prompt requires manual confirmation based on policy.
    fn requires_manual_confirmation(&self, output: &str, confirm_count: usize) -> bool {
        // Check if we've exceeded auto-confirm limit
        if confirm_count >= self.confirmation_policy.max_auto_confirms {
            warn!(
                agent = %self.id(),
                confirm_count = confirm_count,
                "Max auto-confirms reached, requiring manual confirmation"
            );
            return true;
        }

        // Check for dangerous patterns
        let output_lower = output.to_lowercase();
        for pattern in &self.confirmation_policy.dangerous_patterns {
            if output_lower.contains(&pattern.to_lowercase()) {
                warn!(
                    agent = %self.id(),
                    pattern = %pattern,
                    "Dangerous pattern detected, requiring manual confirmation"
                );
                return true;
            }
        }

        false
    }

    /// Run a session and collect output.
    async fn run_session(
        &self,
        session_id: &str,
        task: &Task,
    ) -> Result<(String, Option<i32>)> {
        let client = self.pty_client.read().await;

        // Get working directory from task or use current dir
        let working_dir = task.working_dir
            .as_ref()
            .cloned()
            .unwrap_or_else(|| ".".to_string());

        // Build Claude CLI arguments
        let args = self.build_claude_args(task);

        info!(
            agent = %self.id(),
            session_id = %session_id,
            working_dir = %working_dir,
            "Spawning Claude CLI session"
        );

        // Spawn the session
        let spawn_result = client
            .spawn_session(session_id, "claude", &args, &working_dir)
            .await?;

        if !spawn_result.success {
            return Err(PanoptesError::Pty(
                spawn_result.error.unwrap_or_else(|| "Failed to spawn session".into())
            ));
        }

        info!(
            agent = %self.id(),
            session_id = %session_id,
            "Session spawned successfully"
        );

        // Poll for output and handle confirmations
        let mut accumulated_output = String::new();
        let mut confirm_count = 0;
        let start_time = std::time::Instant::now();

        loop {
            // Check timeout
            if start_time.elapsed() > self.max_session_duration {
                warn!(
                    agent = %self.id(),
                    session_id = %session_id,
                    "Session timed out"
                );
                let _ = client.kill_session(session_id).await;
                return Err(PanoptesError::Pty("Session timed out".into()));
            }

            // Get session status
            let status: StatusResult = client.get_status(session_id).await?;

            // Get new output
            let read_result = client.get_output(session_id, true).await?;
            if !read_result.output.is_empty() {
                debug!(
                    agent = %self.id(),
                    session_id = %session_id,
                    output_len = read_result.output.len(),
                    "Received output"
                );
                accumulated_output.push_str(&read_result.output);
            }

            // Handle confirmation prompts
            if status.awaiting_confirmation || read_result.awaiting_confirmation {
                if self.confirmation_policy.auto_confirm
                    && !self.requires_manual_confirmation(&accumulated_output, confirm_count)
                {
                    info!(
                        agent = %self.id(),
                        session_id = %session_id,
                        confirm_count = confirm_count,
                        "Auto-confirming prompt"
                    );
                    client.send_confirmation(session_id, true).await?;
                    confirm_count += 1;
                } else {
                    // SEC-013: Deny actions that require manual confirmation
                    // When manual confirmation is needed but no human is available,
                    // we must deny the action and terminate the session to prevent
                    // unreviewed operations.
                    error!(
                        agent = %self.id(),
                        session_id = %session_id,
                        confirm_count = confirm_count,
                        "Manual confirmation required - denying action and terminating session"
                    );
                    client.send_confirmation(session_id, false).await?;
                    let _ = client.kill_session(session_id).await;
                    return Err(PanoptesError::Agent(format!(
                        "Session {} terminated: manual confirmation required but no human operator available. \
                         The action was denied for safety. Reduce dangerous operations or increase max_auto_confirms.",
                        session_id
                    )));
                }
            }

            // Check if session completed
            if !status.is_running {
                info!(
                    agent = %self.id(),
                    session_id = %session_id,
                    exit_code = ?status.exit_code,
                    "Session completed"
                );

                // Get any final output
                let final_read = client.get_output(session_id, true).await?;
                if !final_read.output.is_empty() {
                    accumulated_output.push_str(&final_read.output);
                }

                return Ok((accumulated_output, status.exit_code));
            }

            // Wait before next poll
            tokio::time::sleep(self.poll_interval).await;
        }
    }
}

#[async_trait]
impl Agent for CodingAgent {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn capabilities(&self) -> &[AgentCapability] {
        &[
            AgentCapability::CodeGeneration,
            AgentCapability::CodeExecution,
            AgentCapability::MemoryAccess,
        ]
    }

    async fn process_task(&self, task: &Task) -> Result<AgentMessage> {
        info!(
            agent = %self.id(),
            task_id = %task.id,
            task_description = %task.description,
            "Processing coding task"
        );

        // Atomically claim the agent
        if self.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err(PanoptesError::Agent(format!(
                "Agent {} is busy processing another task",
                self.id()
            )));
        }

        // Ensure we're connected to PTY-MCP
        if let Err(e) = self.ensure_connected().await {
            self.busy.store(false, Ordering::SeqCst);
            error!(agent = %self.id(), error = %e, "Failed to connect to PTY-MCP");
            return Err(e);
        }

        // Generate session ID
        let session_id = self.generate_session_id(task);

        // Run the session
        let result = self.run_session(&session_id, task).await;

        self.busy.store(false, Ordering::SeqCst);

        match result {
            Ok((output, exit_code)) => {
                let mut message = AgentMessage::from_agent(self.id(), output);
                message.task_id = Some(task.id.clone());

                // Add exit code to metadata if available
                if let Some(code) = exit_code {
                    if code != 0 {
                        warn!(
                            agent = %self.id(),
                            task_id = %task.id,
                            exit_code = code,
                            "Session exited with non-zero code"
                        );
                    }
                }

                info!(
                    agent = %self.id(),
                    task_id = %task.id,
                    "Task completed successfully"
                );

                Ok(message)
            }
            Err(e) => {
                error!(
                    agent = %self.id(),
                    task_id = %task.id,
                    error = %e,
                    "Task failed"
                );
                Err(e)
            }
        }
    }

    async fn handle_message(&self, message: &AgentMessage) -> Result<AgentMessage> {
        info!(
            agent = %self.id(),
            message_id = %message.id,
            "Handling message"
        );

        // Convert message to task and process
        let mut task = Task::new(&message.content);

        // Inherit working directory from message if specified
        if let Some(ref msg_task_id) = message.task_id {
            task.id = msg_task_id.clone();
        }

        self.process_task(&task).await
    }

    fn system_prompt(&self) -> &str {
        self.config
            .system_prompt
            .as_deref()
            .unwrap_or(CODING_SYSTEM_PROMPT)
    }

    fn is_available(&self) -> bool {
        !self.busy.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coding_agent_creation() {
        let agent = CodingAgent::with_default_config();
        assert_eq!(agent.id(), "coding");
        assert_eq!(agent.name(), "Coding Agent");
        assert!(agent.is_available());
    }

    #[test]
    fn test_capabilities() {
        let agent = CodingAgent::with_default_config();
        let caps = agent.capabilities();

        assert!(caps.contains(&AgentCapability::CodeGeneration));
        assert!(caps.contains(&AgentCapability::CodeExecution));
        assert!(caps.contains(&AgentCapability::MemoryAccess));
    }

    #[test]
    fn test_build_claude_args() {
        let agent = CodingAgent::with_default_config();
        let task = Task::new("Fix the bug in parser.rs");

        let args = agent.build_claude_args(&task);

        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-p");
        assert!(args[1].contains("Fix the bug"));
    }

    #[test]
    fn test_build_claude_args_with_context() {
        let agent = CodingAgent::with_default_config();
        let mut task = Task::new("Fix the bug");
        task.context = Some("The parser fails on line 42".into());

        let args = agent.build_claude_args(&task);

        assert!(args[1].contains("Fix the bug"));
        assert!(args[1].contains("Context:"));
        assert!(args[1].contains("line 42"));
    }

    #[test]
    fn test_confirmation_policy_default() {
        let policy = ConfirmationPolicy::default();

        assert!(policy.auto_confirm);
        assert_eq!(policy.max_auto_confirms, 10);
        assert!(policy.dangerous_patterns.contains(&"delete".into()));
    }

    #[test]
    fn test_requires_manual_confirmation_dangerous_pattern() {
        let agent = CodingAgent::with_default_config();

        // Should require manual confirmation for dangerous patterns
        assert!(agent.requires_manual_confirmation("rm -rf /important", 0));
        assert!(agent.requires_manual_confirmation("DROP TABLE users", 0));

        // Should not require for normal output
        assert!(!agent.requires_manual_confirmation("Fixing bug in parser.rs", 0));
    }

    #[test]
    fn test_requires_manual_confirmation_max_reached() {
        let agent = CodingAgent::with_default_config();

        // Should require manual after max confirms
        assert!(agent.requires_manual_confirmation("Normal prompt", 10));
        assert!(!agent.requires_manual_confirmation("Normal prompt", 9));
    }

    #[test]
    fn test_session_id_generation() {
        let agent = CodingAgent::with_default_config();
        let task = Task::new("Test task");

        let id1 = agent.generate_session_id(&task);
        let id2 = agent.generate_session_id(&task);

        // Should contain task ID
        assert!(id1.starts_with(&task.id));
        // Should be unique
        assert_ne!(id1, id2);
    }
}
