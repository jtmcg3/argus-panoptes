//! A2A bridge implementations for all specialist agents.
//!
//! Each agent gets a thin A2aAgent impl that delegates to its existing
//! `Agent::process_task()` method.

use async_trait::async_trait;
use panoptes_a2a::{A2aAgent, AgentSkill, ProgressEvent};
use panoptes_common::{PanoptesError, Task};
use tokio::sync::broadcast;

use crate::coding::CodingAgent;
use crate::planning::PlanningAgent;
use crate::research::ResearchAgent;
use crate::review::ReviewAgent;
use crate::testing::TestingAgent;
use crate::traits::Agent;
use crate::writing::WritingAgent;

// =============================================================================
// Research Agent
// =============================================================================

#[async_trait]
impl A2aAgent for ResearchAgent {
    fn name(&self) -> &str {
        "panoptes-research"
    }

    fn description(&self) -> &str {
        "Web search, document analysis, and knowledge synthesis with persistent memory"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn skills(&self) -> Vec<AgentSkill> {
        vec![
            AgentSkill {
                id: "web-research".into(),
                name: "Web Research".into(),
                description: "Search the web, fetch and parse sources, synthesize findings".into(),
                tags: vec![
                    "research".into(),
                    "search".into(),
                    "web".into(),
                    "analysis".into(),
                ],
                examples: vec![
                    "Research the latest advances in CRISPR gene therapy".into(),
                    "Find information about Rust async patterns".into(),
                ],
            },
            AgentSkill {
                id: "knowledge-recall".into(),
                name: "Knowledge Recall".into(),
                description: "Retrieve and synthesize prior research from persistent memory".into(),
                tags: vec!["memory".into(), "recall".into(), "knowledge".into()],
                examples: vec!["What do we already know about quantum computing?".into()],
            },
        ]
    }

    async fn handle(
        &self,
        input: &str,
        progress_tx: Option<broadcast::Sender<ProgressEvent>>,
    ) -> Result<String, PanoptesError> {
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::Phase {
                name: "research".into(),
                detail: "Starting research task".into(),
            });
        }

        let task = Task::new(input);
        let result = self.process_task(&task).await?;

        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::Phase {
                name: "complete".into(),
                detail: "Research complete".into(),
            });
        }

        Ok(result.content)
    }
}

// =============================================================================
// Writing Agent
// =============================================================================

#[async_trait]
impl A2aAgent for WritingAgent {
    fn name(&self) -> &str {
        "panoptes-writing"
    }

    fn description(&self) -> &str {
        "Professional writing, editing, and content creation across multiple formats"
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    fn skills(&self) -> Vec<AgentSkill> {
        vec![AgentSkill {
            id: "content-creation".into(),
            name: "Content Creation".into(),
            description: "Create documentation, emails, reports, summaries, and technical writing"
                .into(),
            tags: vec!["writing".into(), "documentation".into(), "content".into()],
            examples: vec![
                "Write an email to the team about the quarterly results".into(),
                "Draft API documentation for the auth module".into(),
            ],
        }]
    }

    async fn handle(
        &self,
        input: &str,
        _progress_tx: Option<broadcast::Sender<ProgressEvent>>,
    ) -> Result<String, PanoptesError> {
        let task = Task::new(input);
        let result = self.process_task(&task).await?;
        Ok(result.content)
    }
}

// =============================================================================
// Planning Agent
// =============================================================================

#[async_trait]
impl A2aAgent for PlanningAgent {
    fn name(&self) -> &str {
        "panoptes-planning"
    }

    fn description(&self) -> &str {
        "Task decomposition, scheduling, and prioritization using Eisenhower matrix"
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    fn skills(&self) -> Vec<AgentSkill> {
        vec![AgentSkill {
            id: "task-planning".into(),
            name: "Task Planning".into(),
            description: "Break down goals into prioritized, actionable tasks with time estimates"
                .into(),
            tags: vec![
                "planning".into(),
                "scheduling".into(),
                "prioritization".into(),
            ],
            examples: vec![
                "Plan the sprint for next week".into(),
                "Break down the authentication feature into tasks".into(),
            ],
        }]
    }

    async fn handle(
        &self,
        input: &str,
        _progress_tx: Option<broadcast::Sender<ProgressEvent>>,
    ) -> Result<String, PanoptesError> {
        let task = Task::new(input);
        let result = self.process_task(&task).await?;
        Ok(result.content)
    }
}

// =============================================================================
// Review Agent
// =============================================================================

#[async_trait]
impl A2aAgent for ReviewAgent {
    fn name(&self) -> &str {
        "panoptes-review"
    }

    fn description(&self) -> &str {
        "Code review via cargo clippy, formatting checks, and pattern scanning"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn skills(&self) -> Vec<AgentSkill> {
        vec![AgentSkill {
            id: "code-review".into(),
            name: "Code Review".into(),
            description:
                "Run clippy, check formatting, scan for TODOs, and generate review reports".into(),
            tags: vec![
                "review".into(),
                "clippy".into(),
                "quality".into(),
                "lint".into(),
            ],
            examples: vec![
                "Review the project codebase".into(),
                "Check for code quality issues".into(),
            ],
        }]
    }

    async fn handle(
        &self,
        input: &str,
        progress_tx: Option<broadcast::Sender<ProgressEvent>>,
    ) -> Result<String, PanoptesError> {
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::Phase {
                name: "review".into(),
                detail: "Running code analysis".into(),
            });
        }

        let task = Task::new(input);
        let result = self.process_task(&task).await?;
        Ok(result.content)
    }
}

// =============================================================================
// Testing Agent
// =============================================================================

#[async_trait]
impl A2aAgent for TestingAgent {
    fn name(&self) -> &str {
        "panoptes-testing"
    }

    fn description(&self) -> &str {
        "Test execution, output parsing, and coverage reporting"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn skills(&self) -> Vec<AgentSkill> {
        vec![AgentSkill {
            id: "test-execution".into(),
            name: "Test Execution".into(),
            description: "Run cargo tests, parse results, and generate reports".into(),
            tags: vec![
                "testing".into(),
                "cargo".into(),
                "test".into(),
                "coverage".into(),
            ],
            examples: vec![
                "Run all tests in the workspace".into(),
                "Run unit tests for the parser module".into(),
            ],
        }]
    }

    async fn handle(
        &self,
        input: &str,
        progress_tx: Option<broadcast::Sender<ProgressEvent>>,
    ) -> Result<String, PanoptesError> {
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::Phase {
                name: "testing".into(),
                detail: "Running tests".into(),
            });
        }

        let task = Task::new(input);
        let result = self.process_task(&task).await?;
        Ok(result.content)
    }
}

// =============================================================================
// Coding Agent
// =============================================================================

#[async_trait]
impl A2aAgent for CodingAgent {
    fn name(&self) -> &str {
        "panoptes-coding"
    }

    fn description(&self) -> &str {
        "Code generation and modification via Claude CLI in isolated PTY sessions"
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    fn skills(&self) -> Vec<AgentSkill> {
        vec![
            AgentSkill {
                id: "code-generation".into(),
                name: "Code Generation".into(),
                description: "Write new code, implement features, fix bugs".into(),
                tags: vec![
                    "coding".into(),
                    "generation".into(),
                    "implementation".into(),
                ],
                examples: vec!["Add a retry mechanism to the HTTP client".into()],
            },
            AgentSkill {
                id: "code-execution".into(),
                name: "Code Execution".into(),
                description: "Run code in a sandboxed PTY environment via Claude CLI".into(),
                tags: vec![
                    "coding".into(),
                    "execution".into(),
                    "pty".into(),
                    "cli".into(),
                ],
                examples: vec!["Run the test suite and fix any failures".into()],
            },
        ]
    }

    async fn handle(
        &self,
        input: &str,
        _progress_tx: Option<broadcast::Sender<ProgressEvent>>,
    ) -> Result<String, PanoptesError> {
        let task = Task::new(input);
        let result = self.process_task(&task).await?;
        Ok(result.content)
    }
}
