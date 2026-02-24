//! Common types and traits shared across Argus-Panoptes crates.
//!
//! This crate provides the foundational abstractions that all agents
//! and components use to communicate.

pub mod error;
pub mod message;
pub mod security;
pub mod task;
pub mod traits;

pub use error::{PanoptesError, Result};
pub use message::{AgentMessage, MessageRole};
pub use security::{PathSecurityConfig, validate_working_dir};
pub use task::{Task, TaskPriority, TaskStatus};
pub use traits::{Agent, AgentCapability, AgentConfig};
