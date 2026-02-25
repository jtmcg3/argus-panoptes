//! Concrete [`PtyTool`](crate::pty_tool::PtyTool) implementations.
//!
//! - [`ClaudeCodeTool`] -- wraps the PTY-MCP server running Claude Code.
//! - [`GenericCliTool`] -- wraps any CLI binary via `tokio::process::Command`.

pub mod claude;
pub mod generic;

pub use claude::ClaudeCodeTool;
pub use generic::{CliToolConfig, GenericCliTool};
