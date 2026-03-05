//! A2A triage coordinator for Argus-Panoptes.
//!
//! The coordinator is the central entry point that:
//! 1. Receives A2A requests from external callers
//! 2. Triages via direct LLM call (replacing ZeroClaw)
//! 3. Discovers agents via their A2A Agent Cards
//! 4. Delegates work via A2A `message/send` to specialist agents
//! 5. Forwards progress events back to the caller
//!
//! # Architecture
//!
//! ```text
//! External Caller
//!      │ A2A (JSON-RPC)
//!      ▼
//! ┌─────────────────┐
//! │   Coordinator   │  ◄── LLM triage (direct call)
//! │   (port 8080)   │
//! └────────┬────────┘
//!          │ A2A (JSON-RPC)
//!    ┌─────┴─────┬─────────┬──────────┐
//!    ▼           ▼         ▼          ▼
//! [Research] [Writing] [Planning]  [...]
//!  :9001      :9002     :9003     :9004-6
//! ```

pub mod config;
pub mod discovery;
pub mod orchestrator;
pub mod triage;

pub use config::CoordinatorConfig;
pub use discovery::{AgentRegistry, RegisteredAgent};
pub use orchestrator::Orchestrator;
pub use triage::{TriageDecision, TriageEngine};
