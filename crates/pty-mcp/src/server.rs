//! MCP server implementation for PTY tools.

use crate::session::{SessionManager, SessionStatus};
use crate::tools::*;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error, info};

/// The PTY MCP server.
///
/// Exposes PTY session management as MCP tools.
pub struct PtyMcpServer {
    sessions: Arc<SessionManager>,
}

impl PtyMcpServer {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(SessionManager::new()),
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

        match self.sessions.get_or_create(&input.session_id, working_dir).await {
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
    pub async fn handle_write(&self, input: PtyWriteInput) -> Result<(), String> {
        debug!(session_id = %input.session_id, "Handling pty_write");

        let session = self.sessions.get(&input.session_id).await
            .ok_or_else(|| format!("Session not found: {}", input.session_id))?;

        session.write(input.data.as_bytes())
            .map_err(|e| e.to_string())
    }

    /// Handle pty_read tool call.
    pub async fn handle_read(&self, input: PtyReadInput) -> Result<PtyReadOutput, String> {
        debug!(session_id = %input.session_id, "Handling pty_read");

        let session = self.sessions.get(&input.session_id).await
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
    pub async fn handle_confirm(&self, input: PtyConfirmInput) -> Result<(), String> {
        info!(
            session_id = %input.session_id,
            confirmed = input.confirmed,
            "Handling pty_confirm"
        );

        let session = self.sessions.get(&input.session_id).await
            .ok_or_else(|| format!("Session not found: {}", input.session_id))?;

        let response = if input.confirmed { "y\n" } else { "n\n" };
        session.write(response.as_bytes())
            .map_err(|e| e.to_string())
    }

    /// Handle pty_status tool call.
    pub async fn handle_status(&self, input: PtyStatusInput) -> Result<PtyStatusOutput, String> {
        debug!(session_id = %input.session_id, "Handling pty_status");

        let session = self.sessions.get(&input.session_id).await
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
    pub async fn handle_kill(&self, input: PtyKillInput) -> Result<(), String> {
        info!(session_id = %input.session_id, "Handling pty_kill");

        self.sessions.remove(&input.session_id).await
            .ok_or_else(|| format!("Session not found: {}", input.session_id))?;

        Ok(())
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
}

impl Default for PtyMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

// TODO: Implement actual MCP ServerHandler trait when rmcp crate is available
// This will involve:
// 1. Implementing ServerHandler trait
// 2. Registering tools with their schemas
// 3. Handling tool calls and routing to the appropriate handler
// 4. Setting up stdio or HTTP transport
