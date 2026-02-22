//! Writing agent - document and content creation.

use crate::traits::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use panoptes_common::{AgentMessage, PanoptesError, Result, Task};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::info;

const WRITING_SYSTEM_PROMPT: &str = r#"You are a professional writer and editor. Your role is to:

1. Create clear, well-structured written content
2. Adapt tone and style to the audience and purpose
3. Edit and improve existing text
4. Ensure proper grammar and formatting
5. Create documentation, emails, reports, and other business writing

Focus on clarity and conciseness.
Structure content with appropriate headings and formatting.
Tailor the reading level to the intended audience.
"#;

/// Writing agent for document and content creation.
pub struct WritingAgent {
    config: AgentConfig,
    busy: AtomicBool,
}

impl WritingAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            busy: AtomicBool::new(false),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(AgentConfig {
            id: "writing".into(),
            name: "Writing Agent".into(),
            ..Default::default()
        })
    }
}

#[async_trait]
impl Agent for WritingAgent {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn capabilities(&self) -> &[AgentCapability] {
        &[
            AgentCapability::ContentCreation,
            AgentCapability::DocumentAnalysis,
            AgentCapability::MemoryAccess,
        ]
    }

    async fn process_task(&self, task: &Task) -> Result<AgentMessage> {
        info!(
            agent = %self.id(),
            task_id = %task.id,
            "Processing writing task"
        );

        if self.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err(PanoptesError::Agent(format!(
                "Agent {} is busy processing another task",
                self.id()
            )));
        }

        // TODO: Implement actual writing
        // 1. Understand the writing request
        // 2. Retrieve relevant context from memory
        // 3. Generate content
        // 4. Edit and refine
        // 5. Return formatted output

        let result = AgentMessage::from_agent(
            self.id(),
            format!(
                "[TODO: Generate content]\nRequest: {}",
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
            .unwrap_or(WRITING_SYSTEM_PROMPT)
    }

    fn is_available(&self) -> bool {
        !self.busy.load(Ordering::SeqCst)
    }
}
