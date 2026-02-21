//! ZeroClaw-based triage coordinator for Argus-Panoptes.
//!
//! The coordinator is the central brain that:
//! 1. Receives user requests
//! 2. Uses ZeroClaw's triage capabilities to route to appropriate agents
//! 3. Manages agent lifecycle and task assignment
//! 4. Coordinates multi-agent workflows
//!
//! # Architecture
//!
//! ```text
//! User Request
//!      │
//!      ▼
//! ┌─────────────────┐
//! │   Coordinator   │  ◄── ZeroClaw triage
//! │   (this crate)  │
//! └────────┬────────┘
//!          │ MCP calls
//!    ┌─────┴─────┬─────────┬──────────┐
//!    ▼           ▼         ▼          ▼
//! [PTY-MCP]  [Research] [Writing] [Planning]
//!  Agent      Agent      Agent      Agent
//! ```

pub mod config;
pub mod pty_client;
pub mod routing;
pub mod triage;
pub mod zeroclaw_triage;

pub use config::CoordinatorConfig;
pub use zeroclaw_triage::ZeroClawTriageAgent;
pub use pty_client::{PtyMcpClient, PtyMcpClientBuilder, ReadResult, SpawnResult, StatusResult};
pub use routing::{AgentRoute, RouteDecision};
pub use triage::Coordinator;
