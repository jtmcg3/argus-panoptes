//! Writing agent - template-based content generation with structural analysis.
//!
//! This agent handles writing tasks by:
//! 1. Determining the type of writing needed (email, report, summary, etc.)
//! 2. Building context from memory if available
//! 3. Generating a structured template with content seeded from the request
//! 4. Storing the output in memory for future reference

use crate::traits::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use panoptes_common::{AgentMessage, PanoptesError, Result, Task};
use panoptes_memory::{Memory, MemoryStore, MemoryType};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{info, warn};

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

/// Types of writing tasks the agent can handle.
#[derive(Debug, Clone, PartialEq)]
pub enum WritingTaskType {
    Documentation,
    Email,
    Report,
    Summary,
    Creative,
    Technical,
}

/// Configuration for the writing agent.
#[derive(Debug, Clone)]
pub struct WritingConfig {
    /// Maximum content length in characters
    pub max_content_length: usize,
    /// Whether to persist output to memory
    pub persist_output: bool,
}

impl Default for WritingConfig {
    fn default() -> Self {
        Self {
            max_content_length: 50_000,
            persist_output: true,
        }
    }
}

/// Writing agent for document and content creation.
pub struct WritingAgent {
    config: AgentConfig,
    writing_config: WritingConfig,
    busy: AtomicBool,
    memory_store: Option<Arc<MemoryStore>>,
}

impl WritingAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            writing_config: WritingConfig::default(),
            busy: AtomicBool::new(false),
            memory_store: None,
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(AgentConfig {
            id: "writing".into(),
            name: "Writing Agent".into(),
            ..Default::default()
        })
    }

    /// Attach a memory store for content persistence.
    pub fn with_memory(mut self, store: Arc<MemoryStore>) -> Self {
        self.memory_store = Some(store);
        self
    }

    /// Determine the type of writing task from the description.
    fn determine_task_type(description: &str) -> WritingTaskType {
        let lower = description.to_lowercase();

        if lower.contains("email") || lower.contains("mail") || lower.contains("message to") {
            WritingTaskType::Email
        } else if lower.contains("report")
            || lower.contains("analysis")
            || lower.contains("findings")
        {
            WritingTaskType::Report
        } else if lower.contains("summar") || lower.contains("brief") || lower.contains("overview")
        {
            WritingTaskType::Summary
        } else if lower.contains("technical")
            || lower.contains("spec")
            || lower.contains("rfc")
            || lower.contains("design")
        {
            WritingTaskType::Technical
        } else if lower.contains("doc")
            || lower.contains("readme")
            || lower.contains("guide")
            || lower.contains("tutorial")
            || lower.contains("api")
        {
            WritingTaskType::Documentation
        } else if lower.contains("story")
            || lower.contains("poem")
            || lower.contains("creative")
            || lower.contains("narrative")
        {
            WritingTaskType::Creative
        } else {
            WritingTaskType::Documentation
        }
    }

    /// Build a markdown template skeleton for the given task type.
    fn build_template(task_type: &WritingTaskType, context: &str) -> String {
        match task_type {
            WritingTaskType::Email => {
                format!(
                    "## Email Draft\n\n\
                     **Subject:** [Derived from request]\n\n\
                     ---\n\n\
                     Dear [Recipient],\n\n\
                     {}\n\n\
                     Best regards,\n[Sender]\n",
                    context
                )
            }
            WritingTaskType::Report => {
                format!(
                    "# Report\n\n\
                     ## Executive Summary\n\n\
                     {}\n\n\
                     ## Background\n\n\
                     [Background context]\n\n\
                     ## Findings\n\n\
                     [Key findings]\n\n\
                     ## Recommendations\n\n\
                     [Recommendations based on findings]\n\n\
                     ## Conclusion\n\n\
                     [Concluding summary]\n",
                    context
                )
            }
            WritingTaskType::Summary => {
                format!(
                    "## Summary\n\n\
                     {}\n\n\
                     ### Key Points\n\n\
                     - [Point 1]\n\
                     - [Point 2]\n\
                     - [Point 3]\n",
                    context
                )
            }
            WritingTaskType::Documentation => {
                format!(
                    "# Documentation\n\n\
                     ## Overview\n\n\
                     {}\n\n\
                     ## Getting Started\n\n\
                     [Setup instructions]\n\n\
                     ## Usage\n\n\
                     [Usage examples]\n\n\
                     ## API Reference\n\n\
                     [API details]\n",
                    context
                )
            }
            WritingTaskType::Creative => {
                format!(
                    "# Creative Writing\n\n\
                     {}\n\n\
                     ---\n\n\
                     [Content continues...]\n",
                    context
                )
            }
            WritingTaskType::Technical => {
                format!(
                    "# Technical Specification\n\n\
                     ## Overview\n\n\
                     {}\n\n\
                     ## Requirements\n\n\
                     [Requirements]\n\n\
                     ## Design\n\n\
                     [Technical design]\n\n\
                     ## Implementation Notes\n\n\
                     [Implementation details]\n",
                    context
                )
            }
        }
    }

    /// Extract key phrases from text for content seeding.
    fn extract_key_phrases(text: &str) -> Vec<String> {
        text.split(['.', '!', '?'])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.len() > 10)
            .take(5)
            .collect()
    }

    /// Build context from memory for the writing task.
    async fn build_memory_context(&self, description: &str) -> String {
        if let Some(ref store) = self.memory_store {
            match store
                .search(description, Some(MemoryType::Semantic), Some(3))
                .await
            {
                Ok(memories) if !memories.is_empty() => {
                    let mut context = String::from("### Prior Context (from Memory)\n\n");
                    for mem in &memories {
                        context.push_str(&format!("- {}\n", mem.content));
                    }
                    context.push('\n');
                    context
                }
                _ => String::new(),
            }
        } else {
            String::new()
        }
    }

    /// Store writing output in memory.
    async fn store_output(&self, content: &str, task_type: &WritingTaskType) {
        if !self.writing_config.persist_output {
            return;
        }

        if let Some(ref store) = self.memory_store {
            let summary = if content.len() > 200 {
                format!("{}...", &content[..200])
            } else {
                content.to_string()
            };

            let memory = Memory::semantic(
                format!("Writing output ({:?}): {}", task_type, summary),
                "writing-agent",
            )
            .with_agent("writing")
            .with_importance(0.6)
            .with_tags(vec![
                "writing".into(),
                format!("{:?}", task_type).to_lowercase(),
            ]);

            if let Err(e) = store.add(memory, true).await {
                warn!(error = %e, "Failed to store writing output in memory");
            }
        }
    }

    /// Execute the writing pipeline.
    async fn execute_writing(&self, task: &Task) -> Result<AgentMessage> {
        let description = &task.description;

        info!(agent = %self.id(), task_id = %task.id, "Starting writing task");

        // Step 1: Determine task type
        let task_type = Self::determine_task_type(description);
        info!(agent = %self.id(), task_type = ?task_type, "Determined writing type");

        // Step 2: Build memory context
        let memory_context = self.build_memory_context(description).await;

        // Step 3: Extract key phrases for content seeding
        let key_phrases = Self::extract_key_phrases(description);

        // Step 4: Build template with content
        let seed_content = if key_phrases.is_empty() {
            description.to_string()
        } else {
            key_phrases.join(". ") + "."
        };

        let mut output = Self::build_template(&task_type, &seed_content);

        // Inject memory context if available
        if !memory_context.is_empty() {
            output = format!("{}\n{}", memory_context, output);
        }

        // Truncate if too long
        if output.len() > self.writing_config.max_content_length {
            output = output
                .chars()
                .take(self.writing_config.max_content_length)
                .collect();
            output.push_str("\n\n[Content truncated]");
        }

        // Step 5: Store in memory
        self.store_output(&output, &task_type).await;

        info!(
            agent = %self.id(),
            task_id = %task.id,
            task_type = ?task_type,
            output_len = output.len(),
            "Writing task completed"
        );

        let mut message = AgentMessage::from_agent(self.id(), output);
        message.task_id = Some(task.id.clone());
        Ok(message)
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

        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(PanoptesError::Agent(format!(
                "Agent {} is busy processing another task",
                self.id()
            )));
        }

        let result = self.execute_writing(task).await;
        self.busy.store(false, Ordering::SeqCst);
        result
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_writing_agent_creation() {
        let agent = WritingAgent::with_default_config();
        assert_eq!(agent.id(), "writing");
        assert_eq!(agent.name(), "Writing Agent");
        assert!(agent.is_available());
    }

    #[test]
    fn test_determine_task_type_email() {
        assert_eq!(
            WritingAgent::determine_task_type("Write an email to the team about the release"),
            WritingTaskType::Email
        );
    }

    #[test]
    fn test_determine_task_type_report() {
        assert_eq!(
            WritingAgent::determine_task_type("Generate a quarterly report on sales"),
            WritingTaskType::Report
        );
    }

    #[test]
    fn test_determine_task_type_summary() {
        assert_eq!(
            WritingAgent::determine_task_type("Give me a brief summary of the meeting"),
            WritingTaskType::Summary
        );
    }

    #[test]
    fn test_determine_task_type_documentation() {
        assert_eq!(
            WritingAgent::determine_task_type("Write the API documentation for the auth module"),
            WritingTaskType::Documentation
        );
    }

    #[test]
    fn test_determine_task_type_creative() {
        assert_eq!(
            WritingAgent::determine_task_type("Write a short story about a robot"),
            WritingTaskType::Creative
        );
    }

    #[test]
    fn test_determine_task_type_technical() {
        assert_eq!(
            WritingAgent::determine_task_type("Draft a technical spec for the new API"),
            WritingTaskType::Technical
        );
    }

    #[test]
    fn test_determine_task_type_default() {
        assert_eq!(
            WritingAgent::determine_task_type("Write something"),
            WritingTaskType::Documentation
        );
    }

    #[test]
    fn test_build_template_email() {
        let template = WritingAgent::build_template(&WritingTaskType::Email, "Meeting tomorrow");
        assert!(template.contains("Email Draft"));
        assert!(template.contains("Meeting tomorrow"));
        assert!(template.contains("Dear"));
    }

    #[test]
    fn test_build_template_report() {
        let template = WritingAgent::build_template(&WritingTaskType::Report, "Sales increased");
        assert!(template.contains("Report"));
        assert!(template.contains("Executive Summary"));
        assert!(template.contains("Sales increased"));
    }

    #[test]
    fn test_extract_key_phrases() {
        let text = "The system handles multiple agents. Each agent has specific capabilities. Routing is done via ZeroClaw.";
        let phrases = WritingAgent::extract_key_phrases(text);
        assert_eq!(phrases.len(), 3);
        assert!(phrases[0].contains("system handles"));
    }

    #[test]
    fn test_extract_key_phrases_short() {
        let text = "Hi. Ok. Yes.";
        let phrases = WritingAgent::extract_key_phrases(text);
        assert!(phrases.is_empty()); // All too short
    }

    #[tokio::test]
    async fn test_full_pipeline() {
        let agent = WritingAgent::with_default_config();
        let task = Task::new(
            "Write an email to the team about the quarterly results showing strong growth",
        );

        let result = agent.process_task(&task).await.unwrap();
        assert!(result.content.contains("Email Draft"));
        assert!(!result.content.contains("[TODO:"));
    }
}
