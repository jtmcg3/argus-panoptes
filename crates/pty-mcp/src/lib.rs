//! PTY-MCP: Model Context Protocol server for PTY session management.
//!
//! This crate provides an MCP server that exposes PTY (pseudo-terminal)
//! capabilities as tools, allowing any MCP client to spawn and interact
//! with terminal sessions (e.g., Claude CLI).
//!
//! # MCP Tools Exposed
//!
//! - `pty_spawn` - Spawn a new PTY session with a command
//! - `pty_write` - Write input to a running session
//! - `pty_read` - Read current output from a session
//! - `pty_status` - Get session status (running, awaiting input, exited)
//! - `pty_confirm` - Send y/n confirmation to a waiting session
//! - `pty_kill` - Terminate a session
//! - `pty_list` - List all active sessions
//!
//! # Architecture
//!
//! The server can run in two modes:
//! - **stdio**: For subprocess communication (Claude Code style)
//! - **http**: For remote MCP server access
//!
//! ```text
//! MCP Client (Coordinator)
//!        │
//!        │ MCP calls
//!        ▼
//! ┌─────────────────┐
//! │  PTY-MCP Server │
//! │                 │
//! │  ┌───────────┐  │
//! │  │ Session   │  │ ◄── Manages multiple PTY sessions
//! │  │ Manager   │  │
//! │  └─────┬─────┘  │
//! │        │        │
//! │  ┌─────▼─────┐  │
//! │  │ PTY Pool  │  │ ◄── portable-pty sessions
//! │  └───────────┘  │
//! └─────────────────┘
//! ```

pub mod parser;
pub mod server;
pub mod session;
pub mod tools;

pub use server::PtyMcpServer;
pub use session::{PtySession, SessionManager, SessionStatus};
