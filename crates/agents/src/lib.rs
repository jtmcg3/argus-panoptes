//! Specialist agents built on swarms-rs.
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
//! Each agent is a swarms-rs `Agent` with domain-specific:
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
pub mod writing;

pub use coding::CodingAgent;
pub use planning::PlanningAgent;
pub use research::ResearchAgent;
pub use review::ReviewAgent;
pub use testing::TestingAgent;
pub use traits::{Agent, AgentCapability};
pub use writing::WritingAgent;
