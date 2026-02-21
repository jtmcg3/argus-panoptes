//! Common types and traits shared across Argus-Panoptes crates.
//!
//! This crate provides the foundational abstractions that all agents
//! and components use to communicate.

pub mod error;
pub mod message;
pub mod task;

pub use error::{PanoptesError, Result};
pub use message::{AgentMessage, MessageRole};
pub use task::{Task, TaskPriority, TaskStatus};
