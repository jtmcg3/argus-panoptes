//! Planning agent - day/project planning and task breakdown.

use crate::traits::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use panoptes_common::{AgentMessage, PanoptesError, Result, Task};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::info;

const PLANNING_SYSTEM_PROMPT: &str = r#"You are a productivity and planning assistant. Your role is to:

1. Help break down large goals into actionable tasks
2. Create realistic schedules and timelines
3. Identify dependencies and blockers
4. Prioritize tasks based on importance and urgency
5. Track progress and suggest adjustments

Use the Eisenhower matrix for prioritization.
Consider energy levels and context switching costs.
Build in buffer time for unexpected issues.
"#;

/// Planning agent for task management and scheduling.
pub struct PlanningAgent {
    config: AgentConfig,
    busy: AtomicBool,
}

impl PlanningAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            busy: AtomicBool::new(false),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(AgentConfig {
            id: "planning".into(),
            name: "Planning Agent".into(),
            ..Default::default()
        })
    }
}

#[async_trait]
impl Agent for PlanningAgent {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn capabilities(&self) -> &[AgentCapability] {
        &[AgentCapability::TaskPlanning, AgentCapability::MemoryAccess]
    }

    async fn process_task(&self, task: &Task) -> Result<AgentMessage> {
        info!(
            agent = %self.id(),
            task_id = %task.id,
            "Processing planning task"
        );

        if self.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err(PanoptesError::Agent(format!(
                "Agent {} is busy processing another task",
                self.id()
            )));
        }

        // TODO: Implement actual planning
        // 1. Parse planning request
        // 2. Retrieve calendar/existing tasks from memory
        // 3. Generate plan/breakdown
        // 4. Store plan in memory
        // 5. Return formatted plan

        let result = AgentMessage::from_agent(
            self.id(),
            format!(
                "[TODO: Generate plan]\nRequest: {}",
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
            .unwrap_or(PLANNING_SYSTEM_PROMPT)
    }

    fn is_available(&self) -> bool {
        !self.busy.load(Ordering::SeqCst)
    }
}
