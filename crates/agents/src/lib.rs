//! Specialist agents for the Argus-Panoptes orchestration system.
//!
//! This crate provides specialized agents for different task domains:
//!
//! - **Coding Agent**: Uses PTY-MCP to run Claude CLI for code changes
//! - **Research Agent**: Web search and knowledge gathering
//! - **Writing Agent**: Document and content creation
//! - **Planning Agent**: Day/project planning and task breakdown
//! - **Review Agent**: Code review and quality analysis
//! - **Testing Agent**: Test execution and coverage analysis
//!
//! # Architecture
//!
//! Each agent implements the `Agent` trait with domain-specific:
//! - System prompts
//! - Tool bindings (via MCP)
//! - Memory access patterns
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    AGENT SWARM                              │
//! ├─────────────────────────────────────────────────────────────┤
//! │                                                             │
//! │  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐       │
//! │  │ Coding  │  │Research │  │ Writing │  │Planning │       │
//! │  │  Agent  │  │  Agent  │  │  Agent  │  │  Agent  │       │
//! │  └────┬────┘  └────┬────┘  └────┬────┘  └────┬────┘       │
//! │       │            │            │            │             │
//! │       ▼            ▼            ▼            ▼             │
//! │  ┌─────────────────────────────────────────────────────┐  │
//! │  │              Shared Memory (LanceDB)                │  │
//! │  └─────────────────────────────────────────────────────┘  │
//! │                                                             │
//! └─────────────────────────────────────────────────────────────┘
//! ```

pub mod coding;
pub mod planning;
pub mod research;
pub mod review;
pub mod testing;
pub mod traits;
pub mod workflow;
pub mod writing;

pub use coding::{CodingAgent, ConfirmationPolicy};
pub use planning::PlanningAgent;
pub use research::{ResearchAgent, ResearchConfig, SearchResult, WebContent};
pub use review::ReviewAgent;
pub use testing::TestingAgent;
pub use traits::{Agent, AgentCapability, AgentConfig};
pub use workflow::{ConcurrentWorkflow, SequentialWorkflow, StepResult, Workflow, WorkflowResult};
pub use writing::WritingAgent;
