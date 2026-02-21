//! Review agent - code review and quality analysis.

use crate::traits::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use panoptes_common::{AgentMessage, Result, Task};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::info;

const REVIEW_SYSTEM_PROMPT: &str = r#"You are a senior code reviewer. Your role is to:

1. Review code for correctness, security, and maintainability
2. Identify bugs, vulnerabilities, and code smells
3. Suggest improvements and best practices
4. Ensure code follows project conventions
5. Provide constructive, actionable feedback

Focus on high-impact issues first.
Explain the "why" behind suggestions.
Be respectful and constructive in feedback.
"#;

/// Review agent for code review and quality analysis.
pub struct ReviewAgent {
    config: AgentConfig,
    busy: AtomicBool,
}

impl ReviewAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            busy: AtomicBool::new(false),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(AgentConfig {
            id: "review".into(),
            name: "Review Agent".into(),
            mcp_servers: vec!["pty-mcp".into()],
            ..Default::default()
        })
    }
}

#[async_trait]
impl Agent for ReviewAgent {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn capabilities(&self) -> &[AgentCapability] {
        &[
            AgentCapability::CodeReview,
            AgentCapability::DocumentAnalysis,
            AgentCapability::MemoryAccess,
        ]
    }

    async fn process_task(&self, task: &Task) -> Result<AgentMessage> {
        info!(
            agent = %self.id(),
            task_id = %task.id,
            "Processing review task"
        );

        self.busy.store(true, Ordering::SeqCst);

        // TODO: Implement actual review
        // 1. Get files to review (from task context or git diff)
        // 2. Analyze code
        // 3. Generate review comments
        // 4. Return structured review

        let result = AgentMessage::from_agent(
            self.id(),
            format!(
                "[TODO: Perform code review]\nTarget: {}",
                task.description
            ),
        );

        self.busy.store(false, Ordering::SeqCst);
        Ok(result)
    }

    async fn handle_message(&self, message: &AgentMessage) -> Result<AgentMessage> {
        let task = Task::new(&message.content);
        self.process_task(&task).await
    }

    fn system_prompt(&self) -> &str {
        self.config
            .system_prompt
            .as_deref()
            .unwrap_or(REVIEW_SYSTEM_PROMPT)
    }

    fn is_available(&self) -> bool {
        !self.busy.load(Ordering::SeqCst)
    }
}
