//! Research agent - web search and knowledge gathering.

use crate::traits::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use panoptes_common::{AgentMessage, Result, Task};
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::info;

const RESEARCH_SYSTEM_PROMPT: &str = r#"You are a research assistant specialized in gathering and synthesizing information. Your role is to:

1. Understand research queries and identify key topics
2. Search for relevant, authoritative sources
3. Synthesize information into clear summaries
4. Cite sources properly
5. Identify gaps in knowledge and suggest follow-up research

Always prioritize accuracy over speed.
Distinguish between facts and opinions.
Note the recency and reliability of sources.
"#;

/// Research agent for web search and knowledge gathering.
pub struct ResearchAgent {
    config: AgentConfig,
    busy: AtomicBool,
}

impl ResearchAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            busy: AtomicBool::new(false),
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(AgentConfig {
            id: "research".into(),
            name: "Research Agent".into(),
            mcp_servers: vec!["web-search".into()],
            ..Default::default()
        })
    }
}

#[async_trait]
impl Agent for ResearchAgent {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn capabilities(&self) -> &[AgentCapability] {
        &[
            AgentCapability::WebSearch,
            AgentCapability::DocumentAnalysis,
            AgentCapability::MemoryAccess,
        ]
    }

    async fn process_task(&self, task: &Task) -> Result<AgentMessage> {
        info!(
            agent = %self.id(),
            task_id = %task.id,
            "Processing research task"
        );

        self.busy.store(true, Ordering::SeqCst);

        // TODO: Implement actual research
        // 1. Parse research query
        // 2. Use web search MCP tools
        // 3. Fetch and analyze sources
        // 4. Synthesize findings
        // 5. Store in memory for future reference

        let result = AgentMessage::from_agent(
            self.id(),
            format!(
                "[TODO: Execute research via web search tools]\nQuery: {}",
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
            .unwrap_or(RESEARCH_SYSTEM_PROMPT)
    }

    fn is_available(&self) -> bool {
        !self.busy.load(Ordering::SeqCst)
    }
}
