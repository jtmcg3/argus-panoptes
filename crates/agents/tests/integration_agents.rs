//! Integration tests for agents.
//!
//! These tests verify that agent capabilities work correctly.
//! They use mock agents to avoid requiring PTY-MCP server.

use async_trait::async_trait;
use panoptes_agents::{Agent, AgentCapability};
use panoptes_common::{AgentMessage, PanoptesError, Result, Task};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

/// A mock agent that simulates work with configurable behavior.
struct SimulatedAgent {
    id: String,
    name: String,
    capabilities: Vec<AgentCapability>,
    response_template: String,
    delay: Duration,
    should_fail: bool,
    available: AtomicBool,
    process_count: AtomicUsize,
}

impl SimulatedAgent {
    fn new(id: &str, response: &str) -> Self {
        Self {
            id: id.to_string(),
            name: format!("Simulated {}", id),
            capabilities: vec![AgentCapability::CodeGeneration],
            response_template: response.to_string(),
            delay: Duration::from_millis(10),
            should_fail: false,
            available: AtomicBool::new(true),
            process_count: AtomicUsize::new(0),
        }
    }

    fn with_capabilities(mut self, caps: Vec<AgentCapability>) -> Self {
        self.capabilities = caps;
        self
    }
}

#[async_trait]
impl Agent for SimulatedAgent {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> &[AgentCapability] {
        &self.capabilities
    }

    async fn process_task(&self, task: &Task) -> Result<AgentMessage> {
        self.process_count.fetch_add(1, Ordering::SeqCst);

        // Simulate work
        tokio::time::sleep(self.delay).await;

        if self.should_fail {
            return Err(PanoptesError::Agent(format!(
                "Simulated failure in {}",
                self.id
            )));
        }

        // Include task context in response if present
        let response = if let Some(ctx) = &task.context {
            format!(
                "{}\nReceived context: {} chars",
                self.response_template,
                ctx.len()
            )
        } else {
            self.response_template.clone()
        };

        Ok(AgentMessage::from_agent(&self.id, &response))
    }

    async fn handle_message(&self, message: &AgentMessage) -> Result<AgentMessage> {
        let task = Task::new(&message.content);
        self.process_task(&task).await
    }

    fn system_prompt(&self) -> &str {
        "Simulated agent for testing"
    }

    fn is_available(&self) -> bool {
        self.available.load(Ordering::SeqCst)
    }
}

// ============================================================================
// Agent Capability Tests
// ============================================================================

#[tokio::test]
async fn test_agent_capabilities() {
    let agent = SimulatedAgent::new("caps", "Done").with_capabilities(vec![
        AgentCapability::CodeGeneration,
        AgentCapability::CodeExecution,
        AgentCapability::MemoryAccess,
    ]);

    let caps = agent.capabilities();
    assert_eq!(caps.len(), 3);
    assert!(agent.has_capability(AgentCapability::CodeGeneration));
    assert!(agent.has_capability(AgentCapability::CodeExecution));
    assert!(!agent.has_capability(AgentCapability::WebSearch));
}
