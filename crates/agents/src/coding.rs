//! Coding agent - uses PTY-MCP to interact with Claude CLI.

use crate::traits::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use panoptes_common::{AgentMessage, Result, Task};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::info;

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

/// Coding agent that uses PTY-MCP for code execution.
pub struct CodingAgent {
    config: AgentConfig,
    busy: AtomicBool,
    // TODO: MCP client for PTY tools
    // pty_client: McpClient,
}

impl CodingAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            busy: AtomicBool::new(false),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(AgentConfig {
            id: "coding".into(),
            name: "Coding Agent".into(),
            mcp_servers: vec!["pty-mcp".into()],
            ..Default::default()
        })
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
            "Processing coding task"
        );

        self.busy.store(true, Ordering::SeqCst);

        // TODO: Implement actual task processing
        // 1. Connect to PTY-MCP server
        // 2. Spawn Claude CLI session
        // 3. Send task instruction
        // 4. Stream output and handle confirmations
        // 5. Return result

        let result = AgentMessage::from_agent(
            self.id(),
            format!(
                "[TODO: Execute coding task via PTY-MCP]\nTask: {}",
                task.description
            ),
        );

        self.busy.store(false, Ordering::SeqCst);
        Ok(result)
    }

    async fn handle_message(&self, message: &AgentMessage) -> Result<AgentMessage> {
        info!(
            agent = %self.id(),
            message_id = %message.id,
            "Handling message"
        );

        // For now, convert message to task and process
        let task = Task::new(&message.content);
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
