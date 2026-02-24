//! MCP server implementation for PTY tools.

use crate::session::{SessionManager, SessionStatus};
use crate::tools::*;
use rmcp::Error as McpError;
use rmcp::ServerHandler;
use rmcp::handler::server::tool::cached_schema_for_type;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParam, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{Peer, RequestContext, RoleServer};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// The PTY MCP server.
///
/// Exposes PTY session management as MCP tools.
#[derive(Clone)]
pub struct PtyMcpServer {
    sessions: Arc<SessionManager>,
    peer: Arc<RwLock<Option<Peer<RoleServer>>>>,
}

impl PtyMcpServer {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(SessionManager::new()),
            peer: Arc::new(RwLock::new(None)),
        }
    }

    /// Handle pty_create tool call.
    pub async fn handle_create(&self, input: PtyCreateInput) -> Result<String, String> {
        info!(
            session_id = %input.session_id,
            working_dir = %input.working_dir,
            "Handling pty_create"
        );

        let working_dir = PathBuf::from(&input.working_dir);

        match self.sessions.create(&input.session_id, working_dir).await {
            Ok(_session) => Ok(serde_json::json!({
                "session_id": input.session_id,
                "success": true
            })
            .to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Handle pty_spawn tool call.
    pub async fn handle_spawn(&self, input: PtySpawnInput) -> PtySpawnOutput {
        info!(
            session_id = %input.session_id,
            command = %input.command,
            working_dir = %input.working_dir,
            "Handling pty_spawn"
        );

        let working_dir = PathBuf::from(&input.working_dir);

        match self
            .sessions
            .get_or_create(&input.session_id, working_dir)
            .await
        {
            Ok(session) => {
                let args: Vec<&str> = input.args.iter().map(|s| s.as_str()).collect();
                match session.spawn(&input.command, &args) {
                    Ok(()) => PtySpawnOutput {
                        session_id: input.session_id,
                        success: true,
                        error: None,
                    },
                    Err(e) => PtySpawnOutput {
                        session_id: input.session_id,
                        success: false,
                        error: Some(e.to_string()),
                    },
                }
            }
            Err(e) => PtySpawnOutput {
                session_id: input.session_id,
                success: false,
                error: Some(e.to_string()),
            },
        }
    }

    /// Handle pty_write tool call.
    pub async fn handle_write(&self, input: PtyWriteInput) -> Result<String, String> {
        debug!(session_id = %input.session_id, "Handling pty_write");

        let session = self
            .sessions
            .get(&input.session_id)
            .await
            .ok_or_else(|| format!("Session not found: {}", input.session_id))?;

        session
            .write(input.data.as_bytes())
            .map_err(|e| e.to_string())?;

        Ok(serde_json::json!({
            "session_id": input.session_id,
            "success": true,
            "bytes_written": input.data.len()
        })
        .to_string())
    }

    /// Handle pty_read tool call.
    pub async fn handle_read(&self, input: PtyReadInput) -> Result<PtyReadOutput, String> {
        debug!(session_id = %input.session_id, "Handling pty_read");

        let session = self
            .sessions
            .get(&input.session_id)
            .await
            .ok_or_else(|| format!("Session not found: {}", input.session_id))?;

        let output = session.get_output().await;
        let status = session.status();

        if input.clear {
            session.clear_output().await;
        }

        Ok(PtyReadOutput {
            session_id: input.session_id,
            output,
            status: format!("{:?}", status),
            awaiting_confirmation: matches!(status, SessionStatus::AwaitingConfirmation),
        })
    }

    /// Handle pty_confirm tool call.
    pub async fn handle_confirm(&self, input: PtyConfirmInput) -> Result<String, String> {
        info!(
            session_id = %input.session_id,
            confirmed = input.confirmed,
            "Handling pty_confirm"
        );

        let session = self
            .sessions
            .get(&input.session_id)
            .await
            .ok_or_else(|| format!("Session not found: {}", input.session_id))?;

        let response = if input.confirmed { "y\n" } else { "n\n" };
        session
            .write(response.as_bytes())
            .map_err(|e| e.to_string())?;

        Ok(serde_json::json!({
            "session_id": input.session_id,
            "confirmed": input.confirmed
        })
        .to_string())
    }

    /// Handle pty_status tool call.
    pub async fn handle_status(&self, input: PtyStatusInput) -> Result<PtyStatusOutput, String> {
        debug!(session_id = %input.session_id, "Handling pty_status");

        let session = self
            .sessions
            .get(&input.session_id)
            .await
            .ok_or_else(|| format!("Session not found: {}", input.session_id))?;

        let status = session.status();

        Ok(PtyStatusOutput {
            session_id: input.session_id,
            status: format!("{:?}", status),
            is_running: session.is_running(),
            awaiting_confirmation: matches!(status, SessionStatus::AwaitingConfirmation),
            exit_code: match status {
                SessionStatus::Exited(code) => code.map(|c| c as i32),
                _ => None,
            },
        })
    }

    /// Handle pty_kill tool call.
    pub async fn handle_kill(&self, input: PtyKillInput) -> Result<String, String> {
        info!(session_id = %input.session_id, "Handling pty_kill");

        self.sessions
            .remove(&input.session_id)
            .await
            .ok_or_else(|| format!("Session not found: {}", input.session_id))?;

        Ok(serde_json::json!({
            "session_id": input.session_id,
            "killed": true
        })
        .to_string())
    }

    /// Handle pty_list tool call.
    pub async fn handle_list(&self) -> PtyListOutput {
        debug!("Handling pty_list");

        let session_ids = self.sessions.list().await;
        let mut sessions = Vec::new();

        for id in session_ids {
            if let Some(session) = self.sessions.get(&id).await {
                let status = session.status();
                sessions.push(PtyStatusOutput {
                    session_id: id,
                    status: format!("{:?}", status),
                    is_running: session.is_running(),
                    awaiting_confirmation: matches!(status, SessionStatus::AwaitingConfirmation),
                    exit_code: match status {
                        SessionStatus::Exited(code) => code.map(|c| c as i32),
                        _ => None,
                    },
                });
            }
        }

        PtyListOutput { sessions }
    }

    /// Get the list of available tools.
    fn get_tools() -> Vec<Tool> {
        vec![
            Tool::new(
                "pty_create",
                "Create a new PTY session without spawning a command. Use this to prepare a session before spawning.",
                cached_schema_for_type::<PtyCreateInput>(),
            ),
            Tool::new(
                "pty_spawn",
                "Spawn a command in a PTY session. Creates a persistent terminal session that can run interactive commands like the Claude CLI.",
                cached_schema_for_type::<PtySpawnInput>(),
            ),
            Tool::new(
                "pty_write",
                "Write data to a running PTY session. Sends input to the terminal, including responses to prompts.",
                cached_schema_for_type::<PtyWriteInput>(),
            ),
            Tool::new(
                "pty_read",
                "Read current output from a PTY session. Returns accumulated output since the last read.",
                cached_schema_for_type::<PtyReadInput>(),
            ),
            Tool::new(
                "pty_status",
                "Get the current status of a PTY session. Returns whether the session is running, awaiting confirmation, or exited.",
                cached_schema_for_type::<PtyStatusInput>(),
            ),
            Tool::new(
                "pty_list",
                "List all active PTY sessions. Returns status information for all managed sessions.",
                cached_schema_for_type::<PtyListInput>(),
            ),
            Tool::new(
                "pty_kill",
                "Terminate a PTY session. Forcefully kills the session and its child processes.",
                cached_schema_for_type::<PtyKillInput>(),
            ),
        ]
    }
}

impl Default for PtyMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerHandler for PtyMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "pty-mcp-server".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(
                "PTY-MCP server provides pseudo-terminal session management for running interactive CLI tools like Claude. \
                Use pty_spawn to start sessions, pty_write/pty_read for I/O, and pty_status to check state."
                    .to_string(),
            ),
        }
    }

    fn get_peer(&self) -> Option<Peer<RoleServer>> {
        // We need to block here since this is not async
        // Using try_read to avoid blocking
        self.peer.try_read().ok().and_then(|guard| guard.clone())
    }

    fn set_peer(&mut self, peer: Peer<RoleServer>) {
        // Use blocking_write for simplicity in set_peer
        if let Ok(mut guard) = self.peer.try_write() {
            *guard = Some(peer);
        }
    }

    async fn list_tools(
        &self,
        _request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            next_cursor: None,
            tools: Self::get_tools(),
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool_name = request.name.as_ref();
        let arguments = request.arguments.unwrap_or_default();

        info!(tool = %tool_name, "Calling tool");

        // Helper macro to parse input and handle errors
        macro_rules! parse_input {
            ($t:ty) => {
                match serde_json::from_value::<$t>(serde_json::Value::Object(arguments.clone())) {
                    Ok(input) => input,
                    Err(e) => {
                        let error = format!("Invalid arguments: {}", e);
                        error!(tool = %tool_name, %error, "Tool call failed");
                        return Ok(CallToolResult::error(vec![Content::text(error)]));
                    }
                }
            };
        }

        let result: Result<String, String> = match tool_name {
            "pty_create" => {
                let input = parse_input!(PtyCreateInput);
                self.handle_create(input).await
            }
            "pty_spawn" => {
                let input = parse_input!(PtySpawnInput);
                let output = self.handle_spawn(input).await;
                serde_json::to_string(&output).map_err(|e| e.to_string())
            }
            "pty_write" => {
                let input = parse_input!(PtyWriteInput);
                self.handle_write(input).await
            }
            "pty_read" => {
                let input = parse_input!(PtyReadInput);
                match self.handle_read(input).await {
                    Ok(output) => serde_json::to_string(&output).map_err(|e| e.to_string()),
                    Err(e) => Err(e),
                }
            }
            "pty_status" => {
                let input = parse_input!(PtyStatusInput);
                match self.handle_status(input).await {
                    Ok(output) => serde_json::to_string(&output).map_err(|e| e.to_string()),
                    Err(e) => Err(e),
                }
            }
            "pty_list" => {
                let output = self.handle_list().await;
                serde_json::to_string(&output).map_err(|e| e.to_string())
            }
            "pty_kill" => {
                let input = parse_input!(PtyKillInput);
                self.handle_kill(input).await
            }
            _ => Err(format!("Unknown tool: {}", tool_name)),
        };

        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(error) => {
                error!(tool = %tool_name, %error, "Tool call failed");
                Ok(CallToolResult::error(vec![Content::text(error)]))
            }
        }
    }
}
