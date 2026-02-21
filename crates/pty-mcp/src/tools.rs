//! MCP tool definitions for PTY operations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Input for pty_create tool.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PtyCreateInput {
    /// Session ID (unique identifier for this session)
    pub session_id: String,

    /// Working directory for the session
    pub working_dir: String,
}

/// Input for pty_spawn tool.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PtySpawnInput {
    /// Session ID (will be created if doesn't exist)
    pub session_id: String,

    /// Command to run (e.g., "claude")
    pub command: String,

    /// Command arguments
    #[serde(default)]
    pub args: Vec<String>,

    /// Working directory
    pub working_dir: String,
}

/// Input for pty_write tool.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PtyWriteInput {
    /// Session ID
    pub session_id: String,

    /// Data to write
    pub data: String,
}

/// Input for pty_read tool.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PtyReadInput {
    /// Session ID
    pub session_id: String,

    /// Whether to clear buffer after reading
    #[serde(default)]
    pub clear: bool,
}

/// Input for pty_confirm tool.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PtyConfirmInput {
    /// Session ID
    pub session_id: String,

    /// Whether to confirm (y) or deny (n)
    pub confirmed: bool,
}

/// Input for pty_status tool.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PtyStatusInput {
    /// Session ID
    pub session_id: String,
}

/// Input for pty_kill tool.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PtyKillInput {
    /// Session ID
    pub session_id: String,
}

/// Input for pty_list tool (empty - lists all sessions).
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct PtyListInput {}

/// Output from pty_spawn tool.
#[derive(Debug, Serialize, Deserialize)]
pub struct PtySpawnOutput {
    pub session_id: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Output from pty_read tool.
#[derive(Debug, Serialize, Deserialize)]
pub struct PtyReadOutput {
    pub session_id: String,
    pub output: String,
    pub status: String,
    pub awaiting_confirmation: bool,
}

/// Output from pty_status tool.
#[derive(Debug, Serialize, Deserialize)]
pub struct PtyStatusOutput {
    pub session_id: String,
    pub status: String,
    pub is_running: bool,
    pub awaiting_confirmation: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// Output from pty_list tool.
#[derive(Debug, Serialize, Deserialize)]
pub struct PtyListOutput {
    pub sessions: Vec<PtyStatusOutput>,
}

/// MCP tool descriptions for registration.
pub mod descriptions {
    pub const PTY_SPAWN: &str = r#"
Spawn a new PTY session with a command.

Creates a persistent terminal session that can run interactive commands
like the Claude CLI. The session persists until explicitly killed or
the command exits.

Example:
{
    "session_id": "coding-session-1",
    "command": "claude",
    "args": ["-p", "Fix the bug in parser.rs"],
    "working_dir": "/home/user/project"
}
"#;

    pub const PTY_WRITE: &str = r#"
Write data to a running PTY session.

Sends input to the terminal, including responses to prompts.
Use this for interactive input that isn't a simple y/n confirmation.

Example:
{
    "session_id": "coding-session-1",
    "data": "hello world\n"
}
"#;

    pub const PTY_READ: &str = r#"
Read current output from a PTY session.

Returns accumulated output since the last read (or session start).
Optionally clears the buffer after reading.

Example:
{
    "session_id": "coding-session-1",
    "clear": true
}
"#;

    pub const PTY_CONFIRM: &str = r#"
Send a y/n confirmation to a waiting PTY session.

Use this when the session is awaiting_confirmation (detected a [y/N] prompt).

Example:
{
    "session_id": "coding-session-1",
    "confirmed": true
}
"#;

    pub const PTY_STATUS: &str = r#"
Get the current status of a PTY session.

Returns whether the session is running, awaiting confirmation, or exited.

Example:
{
    "session_id": "coding-session-1"
}
"#;

    pub const PTY_KILL: &str = r#"
Terminate a PTY session.

Forcefully kills the session and its child processes.

Example:
{
    "session_id": "coding-session-1"
}
"#;

    pub const PTY_LIST: &str = r#"
List all active PTY sessions.

Returns status information for all managed sessions.
"#;
}
