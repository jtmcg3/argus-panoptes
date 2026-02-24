//! Integration tests for agent workflows.
//!
//! These tests verify that agents and workflows work together correctly.
//! They use mock agents to avoid requiring PTY-MCP server.

use async_trait::async_trait;
use panoptes_agents::{Agent, AgentCapability, ConcurrentWorkflow, SequentialWorkflow, Workflow};
use panoptes_common::{AgentMessage, PanoptesError, Result, Task};
use std::sync::Arc;
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

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    fn failing(mut self) -> Self {
        self.should_fail = true;
        self
    }

    fn process_count(&self) -> usize {
        self.process_count.load(Ordering::SeqCst)
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
// Sequential Workflow Tests
// ============================================================================

#[tokio::test]
async fn test_sequential_workflow_context_passing() {
    // Create agents that verify they receive context from previous agent
    let agent1 = Arc::new(SimulatedAgent::new("step1", "Step 1 completed with data A"));
    let agent2 = Arc::new(SimulatedAgent::new("step2", "Step 2 completed with data B"));
    let agent3 = Arc::new(SimulatedAgent::new("step3", "Step 3 completed with data C"));

    let workflow = SequentialWorkflow::new("context-test")
        .add_agent(agent1.clone())
        .add_agent(agent2.clone())
        .add_agent(agent3.clone());

    let task = Task::new("Initial task");
    let result = workflow.run(task).await.unwrap();

    assert!(result.success);
    assert_eq!(result.step_results.len(), 3);

    // Each step should have been called once
    assert_eq!(agent1.process_count(), 1);
    assert_eq!(agent2.process_count(), 1);
    assert_eq!(agent3.process_count(), 1);

    // Later steps should receive context (indicated by "Received context")
    assert!(
        result.step_results[1]
            .output
            .content
            .contains("Received context")
    );
    assert!(
        result.step_results[2]
            .output
            .content
            .contains("Received context")
    );
}

#[tokio::test]
async fn test_sequential_workflow_timing() {
    let agent1 =
        Arc::new(SimulatedAgent::new("fast", "Fast").with_delay(Duration::from_millis(10)));
    let agent2 =
        Arc::new(SimulatedAgent::new("slow", "Slow").with_delay(Duration::from_millis(50)));

    let workflow = SequentialWorkflow::new("timing-test")
        .add_agent(agent1)
        .add_agent(agent2);

    let task = Task::new("Timing test");
    let result = workflow.run(task).await.unwrap();

    assert!(result.success);
    assert!(result.duration_ms >= 60); // Should take at least 60ms total

    // Each step should record its own duration
    assert!(result.step_results[0].duration_ms >= 10);
    assert!(result.step_results[1].duration_ms >= 50);
}

// ============================================================================
// Concurrent Workflow Tests
// ============================================================================

#[tokio::test]
async fn test_concurrent_workflow_parallel_execution() {
    // Create agents with longer delays
    let agent1 =
        Arc::new(SimulatedAgent::new("parallel1", "P1").with_delay(Duration::from_millis(50)));
    let agent2 =
        Arc::new(SimulatedAgent::new("parallel2", "P2").with_delay(Duration::from_millis(50)));
    let agent3 =
        Arc::new(SimulatedAgent::new("parallel3", "P3").with_delay(Duration::from_millis(50)));

    let workflow = ConcurrentWorkflow::new("parallel-test")
        .add_agent(agent1)
        .add_agent(agent2)
        .add_agent(agent3);

    let task = Task::new("Parallel test");

    let start = std::time::Instant::now();
    let result = workflow.run(task).await.unwrap();
    let elapsed = start.elapsed();

    assert!(result.success);
    assert_eq!(result.step_results.len(), 3);

    // Should complete in roughly 50ms (parallel), not 150ms (sequential)
    assert!(elapsed.as_millis() < 120, "Took too long: {:?}", elapsed);
}

#[tokio::test]
async fn test_concurrent_workflow_result_aggregation() {
    let agent1 = Arc::new(
        SimulatedAgent::new("agg1", "Result from agent 1")
            .with_capabilities(vec![AgentCapability::CodeGeneration]),
    );
    let agent2 = Arc::new(
        SimulatedAgent::new("agg2", "Result from agent 2")
            .with_capabilities(vec![AgentCapability::WebSearch]),
    );

    let workflow = ConcurrentWorkflow::new("aggregation-test")
        .add_agent(agent1)
        .add_agent(agent2);

    let task = Task::new("Aggregation test");
    let result = workflow.run(task).await.unwrap();

    assert!(result.success);

    // Final output should contain both results
    assert!(result.final_output.content.contains("Result from agent 1"));
    assert!(result.final_output.content.contains("Result from agent 2"));
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[tokio::test]
async fn test_mixed_success_failure() {
    let agent1 = Arc::new(SimulatedAgent::new("success1", "OK 1"));
    let agent2 = Arc::new(SimulatedAgent::new("failure", "").failing());
    let agent3 = Arc::new(SimulatedAgent::new("success2", "OK 2"));

    // Sequential - should stop at failure
    let seq_workflow = SequentialWorkflow::new("seq-fail")
        .add_agent(agent1.clone())
        .add_agent(agent2.clone())
        .add_agent(agent3.clone());

    let result = seq_workflow.run(Task::new("Test")).await.unwrap();
    assert!(!result.success);
    assert_eq!(result.step_results.len(), 2); // Stopped at agent2

    // Concurrent - all should run
    let conc_workflow = ConcurrentWorkflow::new("conc-fail")
        .add_agent(Arc::new(SimulatedAgent::new("s1", "OK")))
        .add_agent(Arc::new(SimulatedAgent::new("f1", "").failing()))
        .add_agent(Arc::new(SimulatedAgent::new("s2", "OK")));

    let result = conc_workflow.run(Task::new("Test")).await.unwrap();
    assert!(!result.success);
    assert_eq!(result.step_results.len(), 3); // All ran

    // Should still have successful outputs
    let successes: Vec<_> = result.step_results.iter().filter(|r| r.success).collect();
    assert_eq!(successes.len(), 2);
}

// ============================================================================
// Task Configuration Tests
// ============================================================================

#[tokio::test]
async fn test_task_with_working_dir() {
    let agent = Arc::new(SimulatedAgent::new("dir-agent", "Working in directory"));

    let workflow = SequentialWorkflow::new("dir-test").add_agent(agent);

    let task = Task::new("Do work")
        .with_working_dir("/home/user/project")
        .with_context("Important context");

    let result = workflow.run(task).await.unwrap();

    assert!(result.success);
}

#[tokio::test]
async fn test_workflow_metadata() {
    let agent = Arc::new(SimulatedAgent::new("meta", "Done"));

    let workflow = SequentialWorkflow::new("my-workflow")
        .with_description("A test workflow")
        .add_agent(agent);

    assert_eq!(workflow.name(), "my-workflow");

    let result = workflow.run(Task::new("Test")).await.unwrap();
    assert_eq!(result.workflow_name, "my-workflow");
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
