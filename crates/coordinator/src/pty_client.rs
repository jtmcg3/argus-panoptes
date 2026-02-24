//! MCP client for PTY-MCP server.
//!
//! This module provides a client that spawns and communicates with
//! the PTY-MCP server as a subprocess, enabling the coordinator to
//! manage PTY sessions for coding tasks.

use panoptes_common::{PanoptesError, Result};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParam, CallToolResult, Content, RawContent};
use rmcp::service::{Peer, RoleClient};
use rmcp::transport::child_process::TokioChildProcess;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::borrow::Cow;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Result of spawning a PTY session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnResult {
    pub session_id: String,
    pub success: bool,
    pub error: Option<String>,
}

/// Result of reading from a PTY session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadResult {
    pub session_id: String,
    pub output: String,
    pub status: String,
    pub awaiting_confirmation: bool,
}

/// Result of getting PTY session status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResult {
    pub session_id: String,
    pub status: String,
    pub is_running: bool,
    pub awaiting_confirmation: bool,
    pub exit_code: Option<i32>,
}

/// MCP client for communicating with the PTY-MCP server.
///
/// This client spawns the pty-mcp-server as a subprocess and communicates
/// with it via MCP over stdio. It provides high-level methods for managing
/// PTY sessions.
pub struct PtyMcpClient {
    /// Peer for sending requests to the server
    peer: Arc<RwLock<Option<Peer<RoleClient>>>>,
    /// Task handle that keeps the MCP connection alive
    #[allow(dead_code)]
    connection_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
    /// Path to the pty-mcp-server binary
    server_binary: PathBuf,
    /// Whether the client is connected
    connected: Arc<RwLock<bool>>,
}

impl PtyMcpClient {
    /// Create a new PtyMcpClient.
    ///
    /// The `server_binary` should be the path to the pty-mcp-server executable.
    /// If `None`, it will try to find it via `cargo run --bin pty-mcp-server`.
    pub fn new(server_binary: Option<PathBuf>) -> Self {
        let server_binary = server_binary.unwrap_or_else(|| PathBuf::from("pty-mcp-server"));

        Self {
            peer: Arc::new(RwLock::new(None)),
            connection_handle: Arc::new(RwLock::new(None)),
            server_binary,
            connected: Arc::new(RwLock::new(false)),
        }
    }

    /// Connect to the PTY-MCP server by spawning it as a subprocess.
    ///
    /// This starts the pty-mcp-server binary and establishes an MCP connection
    /// over stdio.
    pub async fn connect(&self) -> Result<()> {
        info!(binary = %self.server_binary.display(), "Connecting to PTY-MCP server");

        // Build the command for spawning the server
        let mut cmd = Command::new(&self.server_binary);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit()); // Let server logs go to stderr

        // Spawn the child process transport
        let child_transport = TokioChildProcess::new(&mut cmd)
            .map_err(|e| PanoptesError::Mcp(format!("Failed to spawn PTY-MCP server: {}", e)))?;

        // Create the MCP client by serving on the transport
        // The () handler means we don't handle any server->client requests
        let running = ().serve(child_transport).await.map_err(|e| {
            PanoptesError::Mcp(format!("Failed to establish MCP connection: {}", e))
        })?;

        // Store the peer for making requests
        let peer = running.peer().clone();
        *self.peer.write().await = Some(peer);

        // Spawn a task to keep the connection alive
        // The running service needs to be held to keep the connection open
        let handle = tokio::spawn(async move {
            // Keep the running service alive until it completes
            let _ = running.waiting().await;
        });

        *self.connection_handle.write().await = Some(handle);
        *self.connected.write().await = true;

        info!("Successfully connected to PTY-MCP server");

        // List available tools for debugging
        if let Some(ref peer) = *self.peer.read().await {
            match peer.list_all_tools().await {
                Ok(tools) => {
                    debug!(
                        tools = ?tools.iter().map(|t| t.name.as_ref()).collect::<Vec<_>>(),
                        "Available PTY-MCP tools"
                    );
                }
                Err(e) => {
                    warn!(error = %e, "Failed to list tools from PTY-MCP server");
                }
            }
        }

        Ok(())
    }

    /// Check if the client is connected.
    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }

    /// Spawn a new PTY session with the given command.
    ///
    /// # Arguments
    /// * `session_id` - Unique identifier for the session
    /// * `command` - Command to run (e.g., "claude")
    /// * `args` - Arguments to pass to the command
    /// * `working_dir` - Working directory for the session
    pub async fn spawn_session(
        &self,
        session_id: &str,
        command: &str,
        args: &[String],
        working_dir: &str,
    ) -> Result<SpawnResult> {
        let peer = self.get_peer().await?;

        let arguments = json!({
            "session_id": session_id,
            "command": command,
            "args": args,
            "working_dir": working_dir,
        });

        let result = self.call_tool(&peer, "pty_spawn", arguments).await?;
        self.parse_result(&result)
    }

    /// Send input to a PTY session.
    ///
    /// # Arguments
    /// * `session_id` - The session to write to
    /// * `data` - The data to write (e.g., user input)
    pub async fn send_input(&self, session_id: &str, data: &str) -> Result<()> {
        let peer = self.get_peer().await?;

        let arguments = json!({
            "session_id": session_id,
            "data": data,
        });

        let result = self.call_tool(&peer, "pty_write", arguments).await?;

        // Check for errors in the result
        if result.is_error.unwrap_or(false) {
            let error_msg = self.extract_text(&result.content);
            return Err(PanoptesError::Pty(error_msg));
        }

        Ok(())
    }

    /// Read output from a PTY session.
    ///
    /// # Arguments
    /// * `session_id` - The session to read from
    /// * `clear` - Whether to clear the buffer after reading
    pub async fn get_output(&self, session_id: &str, clear: bool) -> Result<ReadResult> {
        let peer = self.get_peer().await?;

        let arguments = json!({
            "session_id": session_id,
            "clear": clear,
        });

        let result = self.call_tool(&peer, "pty_read", arguments).await?;
        self.parse_result(&result)
    }

    /// Get the status of a PTY session.
    ///
    /// # Arguments
    /// * `session_id` - The session to check
    pub async fn get_status(&self, session_id: &str) -> Result<StatusResult> {
        let peer = self.get_peer().await?;

        let arguments = json!({
            "session_id": session_id,
        });

        let result = self.call_tool(&peer, "pty_status", arguments).await?;
        self.parse_result(&result)
    }

    /// Send a confirmation response (y/n) to a waiting session.
    ///
    /// # Arguments
    /// * `session_id` - The session waiting for confirmation
    /// * `confirmed` - `true` for "y", `false` for "n"
    pub async fn send_confirmation(&self, session_id: &str, confirmed: bool) -> Result<()> {
        let peer = self.get_peer().await?;

        let arguments = json!({
            "session_id": session_id,
            "confirmed": confirmed,
        });

        let result = self.call_tool(&peer, "pty_confirm", arguments).await?;

        if result.is_error.unwrap_or(false) {
            let error_msg = self.extract_text(&result.content);
            return Err(PanoptesError::Pty(error_msg));
        }

        Ok(())
    }

    /// Kill a PTY session.
    ///
    /// # Arguments
    /// * `session_id` - The session to kill
    pub async fn kill_session(&self, session_id: &str) -> Result<()> {
        let peer = self.get_peer().await?;

        let arguments = json!({
            "session_id": session_id,
        });

        let result = self.call_tool(&peer, "pty_kill", arguments).await?;

        if result.is_error.unwrap_or(false) {
            let error_msg = self.extract_text(&result.content);
            return Err(PanoptesError::Pty(error_msg));
        }

        Ok(())
    }

    /// List all active PTY sessions.
    pub async fn list_sessions(&self) -> Result<Vec<StatusResult>> {
        let peer = self.get_peer().await?;

        let arguments = json!({});

        let result = self.call_tool(&peer, "pty_list", arguments).await?;

        #[derive(Deserialize)]
        struct ListOutput {
            sessions: Vec<StatusResult>,
        }

        let output: ListOutput = self.parse_result(&result)?;
        Ok(output.sessions)
    }

    /// Get the peer, returning an error if not connected.
    async fn get_peer(&self) -> Result<Peer<RoleClient>> {
        self.peer
            .read()
            .await
            .clone()
            .ok_or_else(|| PanoptesError::Mcp("Not connected to PTY-MCP server".into()))
    }

    /// Call an MCP tool.
    async fn call_tool(
        &self,
        peer: &Peer<RoleClient>,
        name: &'static str,
        arguments: serde_json::Value,
    ) -> Result<CallToolResult> {
        debug!(tool = %name, "Calling PTY-MCP tool");

        let arguments = match arguments {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        };

        let request = CallToolRequestParam {
            name: Cow::Borrowed(name),
            arguments,
        };

        peer.call_tool(request)
            .await
            .map_err(|e| PanoptesError::Mcp(format!("Tool call failed: {}", e)))
    }

    /// Parse a tool result into the expected type.
    fn parse_result<T: for<'de> Deserialize<'de>>(&self, result: &CallToolResult) -> Result<T> {
        if result.is_error.unwrap_or(false) {
            let error_msg = self.extract_text(&result.content);
            return Err(PanoptesError::Pty(error_msg));
        }

        let text = self.extract_text(&result.content);
        serde_json::from_str(&text).map_err(|e| {
            PanoptesError::Mcp(format!("Failed to parse result: {} (text: {})", e, text))
        })
    }

    /// Extract text content from MCP content array.
    fn extract_text(&self, content: &[Content]) -> String {
        content
            .iter()
            .filter_map(|c| match &c.raw {
                RawContent::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

/// Builder for PtyMcpClient with additional configuration options.
pub struct PtyMcpClientBuilder {
    server_binary: Option<PathBuf>,
    use_cargo_run: bool,
    cargo_manifest_path: Option<PathBuf>,
}

impl PtyMcpClientBuilder {
    pub fn new() -> Self {
        Self {
            server_binary: None,
            use_cargo_run: false,
            cargo_manifest_path: None,
        }
    }

    /// Set the path to the pty-mcp-server binary.
    pub fn server_binary(mut self, path: PathBuf) -> Self {
        self.server_binary = Some(path);
        self
    }

    /// Use `cargo run --bin pty-mcp-server` to run the server.
    /// This is useful during development.
    pub fn use_cargo_run(mut self, manifest_path: Option<PathBuf>) -> Self {
        self.use_cargo_run = true;
        self.cargo_manifest_path = manifest_path;
        self
    }

    /// Build the client.
    pub fn build(self) -> PtyMcpClient {
        PtyMcpClient::new(self.server_binary)
    }
}

impl Default for PtyMcpClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = PtyMcpClient::new(Some(PathBuf::from("/usr/bin/pty-mcp-server")));
        assert_eq!(
            client.server_binary,
            PathBuf::from("/usr/bin/pty-mcp-server")
        );
    }

    #[test]
    fn test_builder() {
        let client = PtyMcpClientBuilder::new()
            .server_binary(PathBuf::from("/custom/path"))
            .build();
        assert_eq!(client.server_binary, PathBuf::from("/custom/path"));
    }
}
