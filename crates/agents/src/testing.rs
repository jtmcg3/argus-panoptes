//! Testing agent - test execution and coverage analysis.

use crate::traits::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use panoptes_common::{AgentMessage, Result, Task};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::info;

const TESTING_SYSTEM_PROMPT: &str = r#"You are a QA and testing specialist. Your role is to:

1. Run existing test suites and report results
2. Identify untested code paths
3. Generate new test cases for edge cases
4. Analyze test coverage reports
5. Suggest testing improvements

Prioritize tests that catch real bugs.
Focus on edge cases and error handling.
Ensure tests are deterministic and fast.
"#;

/// Testing agent for test execution and coverage.
pub struct TestingAgent {
    config: AgentConfig,
    busy: AtomicBool,
}

impl TestingAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            busy: AtomicBool::new(false),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(AgentConfig {
            id: "testing".into(),
            name: "Testing Agent".into(),
            mcp_servers: vec!["pty-mcp".into()],
            ..Default::default()
        })
    }
}

#[async_trait]
impl Agent for TestingAgent {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn capabilities(&self) -> &[AgentCapability] {
        &[
            AgentCapability::TestExecution,
            AgentCapability::CodeExecution,
            AgentCapability::MemoryAccess,
        ]
    }

    async fn process_task(&self, task: &Task) -> Result<AgentMessage> {
        info!(
            agent = %self.id(),
            task_id = %task.id,
            "Processing testing task"
        );

        self.busy.store(true, Ordering::SeqCst);

        // TODO: Implement actual testing
        // 1. Identify test command (cargo test, pytest, etc.)
        // 2. Run tests via PTY-MCP
        // 3. Parse results
        // 4. Generate report
        // 5. Optionally run coverage

        let result = AgentMessage::from_agent(
            self.id(),
            format!(
                "[TODO: Run tests]\nTarget: {}",
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
            .unwrap_or(TESTING_SYSTEM_PROMPT)
    }

    fn is_available(&self) -> bool {
        !self.busy.load(Ordering::SeqCst)
    }
}
