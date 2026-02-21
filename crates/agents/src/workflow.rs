//! Workflow orchestration for agent pipelines.
//!
//! This module provides workflow patterns for coordinating multiple agents:
//! - Sequential: Execute agents one after another, passing output as input
//! - Concurrent: Execute multiple agents in parallel
//!
//! # Example
//!
//! ```ignore
//! let workflow = SequentialWorkflow::new("code-review-pipeline")
//!     .add_agent(coding_agent)
//!     .add_agent(testing_agent)
//!     .add_agent(review_agent);
//!
//! let result = workflow.run(task).await?;
//! ```

use crate::traits::Agent;
use async_trait::async_trait;
use panoptes_common::{AgentMessage, Result, Task};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// A workflow that can execute a series of tasks.
#[async_trait]
pub trait Workflow: Send + Sync {
    /// Execute the workflow with the given initial task.
    async fn run(&self, task: Task) -> Result<WorkflowResult>;

    /// Get the workflow name.
    fn name(&self) -> &str;
}

/// Result of a workflow execution.
#[derive(Debug, Clone)]
pub struct WorkflowResult {
    /// Name of the workflow that was executed.
    pub workflow_name: String,
    /// Results from each step in the workflow.
    pub step_results: Vec<StepResult>,
    /// Final output message.
    pub final_output: AgentMessage,
    /// Whether the workflow completed successfully.
    pub success: bool,
    /// Total execution time in milliseconds.
    pub duration_ms: u64,
}

/// Result of a single workflow step.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// Name of the agent that executed this step.
    pub agent_name: String,
    /// Agent ID.
    pub agent_id: String,
    /// Output from this step.
    pub output: AgentMessage,
    /// Whether this step succeeded.
    pub success: bool,
    /// Execution time for this step in milliseconds.
    pub duration_ms: u64,
}

/// Sequential workflow that executes agents one after another.
///
/// Each agent receives the output from the previous agent as context.
/// The workflow stops if any agent fails.
pub struct SequentialWorkflow {
    name: String,
    description: Option<String>,
    agents: Vec<Arc<dyn Agent>>,
    /// Whether to continue on error (skip failed agent and continue).
    continue_on_error: bool,
}

impl SequentialWorkflow {
    /// Create a new sequential workflow.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            agents: Vec::new(),
            continue_on_error: false,
        }
    }

    /// Add a description to the workflow.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Add an agent to the workflow.
    pub fn add_agent(mut self, agent: Arc<dyn Agent>) -> Self {
        self.agents.push(agent);
        self
    }

    /// Set whether to continue on error.
    pub fn continue_on_error(mut self, value: bool) -> Self {
        self.continue_on_error = value;
        self
    }

    /// Build a modified task with context from previous step.
    fn build_next_task(&self, original_task: &Task, previous_output: &AgentMessage) -> Task {
        let mut next_task = original_task.clone();

        // Append previous output to context
        let new_context = match &next_task.context {
            Some(existing) => format!(
                "{}\n\n--- Previous Agent Output ---\n{}",
                existing, previous_output.content
            ),
            None => format!("--- Previous Agent Output ---\n{}", previous_output.content),
        };

        next_task.context = Some(new_context);
        next_task
    }
}

#[async_trait]
impl Workflow for SequentialWorkflow {
    async fn run(&self, task: Task) -> Result<WorkflowResult> {
        let start_time = std::time::Instant::now();

        info!(
            workflow = %self.name,
            agent_count = self.agents.len(),
            task_id = %task.id,
            "Starting sequential workflow"
        );

        if self.agents.is_empty() {
            warn!(workflow = %self.name, "Workflow has no agents");
            return Ok(WorkflowResult {
                workflow_name: self.name.clone(),
                step_results: Vec::new(),
                final_output: AgentMessage::system("Workflow has no agents"),
                success: false,
                duration_ms: start_time.elapsed().as_millis() as u64,
            });
        }

        let mut step_results = Vec::new();
        let mut current_task = task.clone();
        let mut last_output: Option<AgentMessage> = None;
        let mut all_success = true;

        for (i, agent) in self.agents.iter().enumerate() {
            let step_start = std::time::Instant::now();

            info!(
                workflow = %self.name,
                step = i + 1,
                agent = %agent.id(),
                "Executing workflow step"
            );

            // Check if agent is available
            if !agent.is_available() {
                let msg = format!("Agent {} is not available", agent.id());
                warn!(workflow = %self.name, agent = %agent.id(), "{}", msg);

                if self.continue_on_error {
                    step_results.push(StepResult {
                        agent_name: agent.name().to_string(),
                        agent_id: agent.id().to_string(),
                        output: AgentMessage::system(&msg),
                        success: false,
                        duration_ms: step_start.elapsed().as_millis() as u64,
                    });
                    all_success = false;
                    continue;
                } else {
                    return Ok(WorkflowResult {
                        workflow_name: self.name.clone(),
                        step_results,
                        final_output: AgentMessage::system(&msg),
                        success: false,
                        duration_ms: start_time.elapsed().as_millis() as u64,
                    });
                }
            }

            // Execute the agent
            match agent.process_task(&current_task).await {
                Ok(output) => {
                    debug!(
                        workflow = %self.name,
                        step = i + 1,
                        agent = %agent.id(),
                        output_len = output.content.len(),
                        "Step completed successfully"
                    );

                    step_results.push(StepResult {
                        agent_name: agent.name().to_string(),
                        agent_id: agent.id().to_string(),
                        output: output.clone(),
                        success: true,
                        duration_ms: step_start.elapsed().as_millis() as u64,
                    });

                    // Build next task with context from this output
                    current_task = self.build_next_task(&task, &output);
                    last_output = Some(output);
                }
                Err(e) => {
                    error!(
                        workflow = %self.name,
                        step = i + 1,
                        agent = %agent.id(),
                        error = %e,
                        "Step failed"
                    );

                    let error_msg = AgentMessage::system(&format!("Agent {} failed: {}", agent.id(), e));

                    step_results.push(StepResult {
                        agent_name: agent.name().to_string(),
                        agent_id: agent.id().to_string(),
                        output: error_msg.clone(),
                        success: false,
                        duration_ms: step_start.elapsed().as_millis() as u64,
                    });

                    all_success = false;

                    if !self.continue_on_error {
                        return Ok(WorkflowResult {
                            workflow_name: self.name.clone(),
                            step_results,
                            final_output: error_msg,
                            success: false,
                            duration_ms: start_time.elapsed().as_millis() as u64,
                        });
                    }
                }
            }
        }

        let final_output = last_output.unwrap_or_else(|| AgentMessage::system("No output produced"));

        info!(
            workflow = %self.name,
            steps = step_results.len(),
            success = all_success,
            duration_ms = start_time.elapsed().as_millis(),
            "Workflow completed"
        );

        Ok(WorkflowResult {
            workflow_name: self.name.clone(),
            step_results,
            final_output,
            success: all_success,
            duration_ms: start_time.elapsed().as_millis() as u64,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Concurrent workflow that executes agents in parallel.
///
/// All agents receive the same initial task and execute simultaneously.
/// Results are collected after all agents complete.
pub struct ConcurrentWorkflow {
    name: String,
    description: Option<String>,
    agents: Vec<Arc<dyn Agent>>,
    /// Whether to wait for all agents even if some fail.
    wait_for_all: bool,
}

impl ConcurrentWorkflow {
    /// Create a new concurrent workflow.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            agents: Vec::new(),
            wait_for_all: true,
        }
    }

    /// Add a description to the workflow.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Add an agent to the workflow.
    pub fn add_agent(mut self, agent: Arc<dyn Agent>) -> Self {
        self.agents.push(agent);
        self
    }

    /// Set whether to wait for all agents even if some fail.
    pub fn wait_for_all(mut self, value: bool) -> Self {
        self.wait_for_all = value;
        self
    }
}

#[async_trait]
impl Workflow for ConcurrentWorkflow {
    async fn run(&self, task: Task) -> Result<WorkflowResult> {
        let start_time = std::time::Instant::now();

        info!(
            workflow = %self.name,
            agent_count = self.agents.len(),
            task_id = %task.id,
            "Starting concurrent workflow"
        );

        if self.agents.is_empty() {
            warn!(workflow = %self.name, "Workflow has no agents");
            return Ok(WorkflowResult {
                workflow_name: self.name.clone(),
                step_results: Vec::new(),
                final_output: AgentMessage::system("Workflow has no agents"),
                success: false,
                duration_ms: start_time.elapsed().as_millis() as u64,
            });
        }

        // Spawn all agents concurrently
        let mut handles = Vec::new();

        for agent in &self.agents {
            let agent = agent.clone();
            let task = task.clone();

            let handle = tokio::spawn(async move {
                let step_start = std::time::Instant::now();
                let agent_id = agent.id().to_string();
                let agent_name = agent.name().to_string();

                if !agent.is_available() {
                    return StepResult {
                        agent_name,
                        agent_id,
                        output: AgentMessage::system("Agent not available"),
                        success: false,
                        duration_ms: step_start.elapsed().as_millis() as u64,
                    };
                }

                match agent.process_task(&task).await {
                    Ok(output) => StepResult {
                        agent_name,
                        agent_id,
                        output,
                        success: true,
                        duration_ms: step_start.elapsed().as_millis() as u64,
                    },
                    Err(e) => StepResult {
                        agent_name,
                        agent_id: agent_id.clone(),
                        output: AgentMessage::system(&format!("Agent {} failed: {}", agent_id, e)),
                        success: false,
                        duration_ms: step_start.elapsed().as_millis() as u64,
                    },
                }
            });

            handles.push(handle);
        }

        // Collect results
        let mut step_results = Vec::new();
        let mut all_success = true;

        for handle in handles {
            match handle.await {
                Ok(result) => {
                    if !result.success {
                        all_success = false;
                    }
                    step_results.push(result);
                }
                Err(e) => {
                    error!(workflow = %self.name, error = %e, "Task join error");
                    step_results.push(StepResult {
                        agent_name: "unknown".to_string(),
                        agent_id: "unknown".to_string(),
                        output: AgentMessage::system(&format!("Task join error: {}", e)),
                        success: false,
                        duration_ms: 0,
                    });
                    all_success = false;
                }
            }
        }

        // Combine outputs for final result
        let combined_output = step_results
            .iter()
            .filter(|r| r.success)
            .map(|r| format!("--- {} ---\n{}", r.agent_name, r.output.content))
            .collect::<Vec<_>>()
            .join("\n\n");

        let final_output = if combined_output.is_empty() {
            AgentMessage::system("No successful outputs")
        } else {
            AgentMessage::system(&combined_output)
        };

        info!(
            workflow = %self.name,
            steps = step_results.len(),
            success = all_success,
            duration_ms = start_time.elapsed().as_millis(),
            "Workflow completed"
        );

        Ok(WorkflowResult {
            workflow_name: self.name.clone(),
            step_results,
            final_output,
            success: all_success,
            duration_ms: start_time.elapsed().as_millis() as u64,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_common::PanoptesError;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Mock agent for testing workflows.
    struct MockAgent {
        id: String,
        name: String,
        response: String,
        should_fail: bool,
        available: AtomicBool,
        call_count: AtomicUsize,
    }

    impl MockAgent {
        fn new(id: &str, response: &str) -> Self {
            Self {
                id: id.to_string(),
                name: format!("Mock {}", id),
                response: response.to_string(),
                should_fail: false,
                available: AtomicBool::new(true),
                call_count: AtomicUsize::new(0),
            }
        }

        fn failing(id: &str) -> Self {
            Self {
                id: id.to_string(),
                name: format!("Mock {}", id),
                response: String::new(),
                should_fail: true,
                available: AtomicBool::new(true),
                call_count: AtomicUsize::new(0),
            }
        }

        fn unavailable(id: &str) -> Self {
            Self {
                id: id.to_string(),
                name: format!("Mock {}", id),
                response: String::new(),
                should_fail: false,
                available: AtomicBool::new(false),
                call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl Agent for MockAgent {
        fn id(&self) -> &str {
            &self.id
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn capabilities(&self) -> &[crate::traits::AgentCapability] {
            &[]
        }

        async fn process_task(&self, _task: &Task) -> Result<AgentMessage> {
            self.call_count.fetch_add(1, Ordering::SeqCst);

            if self.should_fail {
                Err(PanoptesError::Agent("Mock failure".into()))
            } else {
                Ok(AgentMessage::from_agent(&self.id, &self.response))
            }
        }

        async fn handle_message(&self, message: &AgentMessage) -> Result<AgentMessage> {
            let task = Task::new(&message.content);
            self.process_task(&task).await
        }

        fn system_prompt(&self) -> &str {
            "Mock agent"
        }

        fn is_available(&self) -> bool {
            self.available.load(Ordering::SeqCst)
        }
    }

    #[tokio::test]
    async fn test_sequential_workflow_single_agent() {
        let agent = Arc::new(MockAgent::new("agent1", "Hello from agent1"));
        let workflow = SequentialWorkflow::new("test").add_agent(agent.clone());

        let task = Task::new("Test task");
        let result = workflow.run(task).await.unwrap();

        assert!(result.success);
        assert_eq!(result.step_results.len(), 1);
        assert_eq!(result.step_results[0].agent_id, "agent1");
        assert!(result.final_output.content.contains("Hello from agent1"));
    }

    #[tokio::test]
    async fn test_sequential_workflow_multiple_agents() {
        let agent1 = Arc::new(MockAgent::new("agent1", "Step 1 done"));
        let agent2 = Arc::new(MockAgent::new("agent2", "Step 2 done"));
        let agent3 = Arc::new(MockAgent::new("agent3", "Step 3 done"));

        let workflow = SequentialWorkflow::new("test")
            .add_agent(agent1)
            .add_agent(agent2)
            .add_agent(agent3);

        let task = Task::new("Test task");
        let result = workflow.run(task).await.unwrap();

        assert!(result.success);
        assert_eq!(result.step_results.len(), 3);
        assert!(result.final_output.content.contains("Step 3 done"));
    }

    #[tokio::test]
    async fn test_sequential_workflow_fails_on_error() {
        let agent1 = Arc::new(MockAgent::new("agent1", "Step 1 done"));
        let agent2 = Arc::new(MockAgent::failing("agent2"));
        let agent3 = Arc::new(MockAgent::new("agent3", "Step 3 done"));

        let workflow = SequentialWorkflow::new("test")
            .add_agent(agent1)
            .add_agent(agent2)
            .add_agent(agent3);

        let task = Task::new("Test task");
        let result = workflow.run(task).await.unwrap();

        assert!(!result.success);
        assert_eq!(result.step_results.len(), 2); // Stopped at agent2
        assert!(!result.step_results[1].success);
    }

    #[tokio::test]
    async fn test_sequential_workflow_continue_on_error() {
        let agent1 = Arc::new(MockAgent::new("agent1", "Step 1 done"));
        let agent2 = Arc::new(MockAgent::failing("agent2"));
        let agent3 = Arc::new(MockAgent::new("agent3", "Step 3 done"));

        let workflow = SequentialWorkflow::new("test")
            .continue_on_error(true)
            .add_agent(agent1)
            .add_agent(agent2)
            .add_agent(agent3);

        let task = Task::new("Test task");
        let result = workflow.run(task).await.unwrap();

        assert!(!result.success); // Overall failed because one step failed
        assert_eq!(result.step_results.len(), 3); // All agents ran
        assert!(result.step_results[0].success);
        assert!(!result.step_results[1].success);
        assert!(result.step_results[2].success);
    }

    #[tokio::test]
    async fn test_concurrent_workflow() {
        let agent1 = Arc::new(MockAgent::new("agent1", "Concurrent 1"));
        let agent2 = Arc::new(MockAgent::new("agent2", "Concurrent 2"));
        let agent3 = Arc::new(MockAgent::new("agent3", "Concurrent 3"));

        let workflow = ConcurrentWorkflow::new("test")
            .add_agent(agent1)
            .add_agent(agent2)
            .add_agent(agent3);

        let task = Task::new("Test task");
        let result = workflow.run(task).await.unwrap();

        assert!(result.success);
        assert_eq!(result.step_results.len(), 3);

        // All agents should have been called
        for step in &result.step_results {
            assert!(step.success);
        }
    }

    #[tokio::test]
    async fn test_concurrent_workflow_partial_failure() {
        let agent1 = Arc::new(MockAgent::new("agent1", "Success 1"));
        let agent2 = Arc::new(MockAgent::failing("agent2"));
        let agent3 = Arc::new(MockAgent::new("agent3", "Success 3"));

        let workflow = ConcurrentWorkflow::new("test")
            .add_agent(agent1)
            .add_agent(agent2)
            .add_agent(agent3);

        let task = Task::new("Test task");
        let result = workflow.run(task).await.unwrap();

        assert!(!result.success); // One agent failed
        assert_eq!(result.step_results.len(), 3); // All ran

        // Final output should contain successful outputs only
        assert!(result.final_output.content.contains("Success 1"));
        assert!(result.final_output.content.contains("Success 3"));
    }

    #[tokio::test]
    async fn test_workflow_empty() {
        let workflow = SequentialWorkflow::new("empty");

        let task = Task::new("Test task");
        let result = workflow.run(task).await.unwrap();

        assert!(!result.success);
        assert!(result.step_results.is_empty());
    }
}
