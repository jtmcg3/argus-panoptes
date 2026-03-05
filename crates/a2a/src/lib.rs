//! A2A protocol infrastructure for Argus-Panoptes.
//!
//! This crate provides shared A2A infrastructure that bridges existing agent
//! logic to the A2A (Agent-to-Agent) protocol. Each specialist agent uses this
//! crate to run as a standalone A2A server.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │              AgentServer<A>                   │
//! │                                              │
//! │  GET /.well-known/agent.json  → Agent Card   │
//! │  POST /  (JSON-RPC 2.0)      → Dispatcher   │
//! │    message/send  → sync execution            │
//! │    message/stream → SSE streaming            │
//! │    tasks/get     → task lookup               │
//! │    tasks/cancel  → task cancellation          │
//! │  GET /health                 → "ok"           │
//! └──────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use panoptes_a2a::{A2aAgent, AgentServer};
//!
//! let agent = MyAgent::new();
//! AgentServer::new(agent, 9001).run().await?;
//! ```

pub mod agent;
pub mod server;
pub mod types;

pub use agent::A2aAgent;
pub use server::AgentServer;
pub use types::*;
