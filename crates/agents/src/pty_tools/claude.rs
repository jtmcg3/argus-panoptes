//! Claude Code PTY tool implementation.
//!
//! Wraps [`PtyMcpClient`] behind the [`PtyTool`] trait, extracting the
//! PTY-MCP interaction logic from [`CodingAgent`](crate::CodingAgent) into
//! a standalone, reusable component.

use crate::coding::ConfirmationPolicy;
use crate::pty_tool::{PtyTool, PtyToolOutput};
use async_trait::async_trait;
use panoptes_common::{PanoptesError, Result};
use panoptes_coordinator::pty_client::PtyMcpClient;
use panoptes_coordinator::routing::PermissionMode;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// A [`PtyTool`] implementation backed by the PTY-MCP server running
/// Claude Code (the `claude` CLI).
pub struct ClaudeCodeTool {
    pty_client: Arc<RwLock<PtyMcpClient>>,
    connected: AtomicBool,
    server_binary: Option<PathBuf>,
    confirmation_policy: ConfirmationPolicy,
    poll_interval: Duration,
    max_session_duration: Duration,
}

impl ClaudeCodeTool {
    /// Create a new `ClaudeCodeTool` with default settings.
    pub fn new() -> Self {
        Self {
            pty_client: Arc::new(RwLock::new(PtyMcpClient::new(None))),
            connected: AtomicBool::new(false),
            server_binary: None,
            confirmation_policy: ConfirmationPolicy::default(),
            poll_interval: Duration::from_millis(500),
            max_session_duration: Duration::from_secs(600),
        }
    }

    /// Set the path to the PTY-MCP server binary.
    pub fn with_server_binary(mut self, path: PathBuf) -> Self {
        self.server_binary = Some(path.clone());
        self.pty_client = Arc::new(RwLock::new(PtyMcpClient::new(Some(path))));
        self
    }

    /// Set the confirmation policy.
    pub fn with_confirmation_policy(mut self, policy: ConfirmationPolicy) -> Self {
        self.confirmation_policy = policy;
        self
    }

    /// Set the poll interval for checking session status.
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Set the maximum session duration.
    pub fn with_max_duration(mut self, duration: Duration) -> Self {
        self.max_session_duration = duration;
        self
    }

    /// Ensure the PTY-MCP client is connected.
    async fn ensure_connected(&self) -> Result<()> {
        if self.connected.load(Ordering::SeqCst) {
            let client = self.pty_client.read().await;
            if client.is_connected().await {
                return Ok(());
            }
            drop(client);
        }

        info!("ClaudeCodeTool: connecting to PTY-MCP server");

        let client = self.pty_client.write().await;
        client.connect().await?;

        self.connected.store(true, Ordering::SeqCst);
        info!("ClaudeCodeTool: connected to PTY-MCP server");

        Ok(())
    }

    /// Check if a prompt requires manual confirmation based on policy.
    fn requires_manual_confirmation(&self, output: &str, confirm_count: usize) -> bool {
        if confirm_count >= self.confirmation_policy.max_auto_confirms {
            warn!(
                confirm_count,
                "Max auto-confirms reached, requiring manual confirmation"
            );
            return true;
        }

        let output_lower = output.to_lowercase();
        for pattern in &self.confirmation_policy.dangerous_patterns {
            if output_lower.contains(&pattern.to_lowercase()) {
                warn!(
                    pattern,
                    "Dangerous pattern detected, requiring manual confirmation"
                );
                return true;
            }
        }

        false
    }

    /// Run the PTY session and collect output.
    async fn run_session(
        &self,
        instruction: &str,
        working_dir: &Path,
        mode: PermissionMode,
    ) -> Result<PtyToolOutput> {
        let session_id = format!("claude-{}", &Uuid::new_v4().to_string()[..8]);
        let client = self.pty_client.read().await;

        let wd = working_dir.to_string_lossy().to_string();

        // Build args: -p <instruction> with optional mode flags
        let mut args = vec!["-p".to_string(), instruction.to_string()];
        match mode {
            PermissionMode::Plan => {
                // Plan mode is the default for Claude CLI
            }
            PermissionMode::Act => {
                args.push("--dangerously-skip-permissions".to_string());
            }
        }

        info!(session_id, "Spawning Claude CLI session");

        let spawn_result = client
            .spawn_session(&session_id, "claude", &args, &wd)
            .await?;

        if !spawn_result.success {
            return Err(PanoptesError::Pty(
                spawn_result
                    .error
                    .unwrap_or_else(|| "Failed to spawn session".into()),
            ));
        }

        // Poll for output and handle confirmations
        let mut accumulated_output = String::new();
        let mut confirm_count = 0;
        let start_time = std::time::Instant::now();

        loop {
            if start_time.elapsed() > self.max_session_duration {
                warn!(session_id, "Session timed out");
                let _ = client.kill_session(&session_id).await;
                return Err(PanoptesError::Pty("Session timed out".into()));
            }

            let status = client.get_status(&session_id).await?;
            let read_result = client.get_output(&session_id, true).await?;

            if !read_result.output.is_empty() {
                debug!(
                    session_id,
                    output_len = read_result.output.len(),
                    "Received output"
                );
                accumulated_output.push_str(&read_result.output);
            }

            if status.awaiting_confirmation || read_result.awaiting_confirmation {
                if self.confirmation_policy.auto_confirm
                    && !self.requires_manual_confirmation(&accumulated_output, confirm_count)
                {
                    info!(session_id, confirm_count, "Auto-confirming prompt");
                    client.send_confirmation(&session_id, true).await?;
                    confirm_count += 1;
                } else {
                    error!(
                        session_id,
                        confirm_count, "Manual confirmation required - denying and terminating"
                    );
                    client.send_confirmation(&session_id, false).await?;
                    let _ = client.kill_session(&session_id).await;
                    return Err(PanoptesError::Agent(format!(
                        "Session {} terminated: manual confirmation required but no human \
                         operator available. The action was denied for safety.",
                        session_id
                    )));
                }
            }

            if !status.is_running {
                info!(session_id, exit_code = ?status.exit_code, "Session completed");

                let final_read = client.get_output(&session_id, true).await?;
                if !final_read.output.is_empty() {
                    accumulated_output.push_str(&final_read.output);
                }

                return Ok(PtyToolOutput {
                    content: accumulated_output,
                    exit_code: status.exit_code,
                    session_id: Some(session_id),
                });
            }

            tokio::time::sleep(self.poll_interval).await;
        }
    }
}

impl Default for ClaudeCodeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PtyTool for ClaudeCodeTool {
    fn name(&self) -> &str {
        "claude-code"
    }

    fn supports_mode(&self, _mode: &PermissionMode) -> bool {
        true // Claude Code supports both Plan and Act modes
    }

    async fn execute(
        &self,
        instruction: &str,
        working_dir: &Path,
        mode: PermissionMode,
    ) -> Result<PtyToolOutput> {
        self.ensure_connected().await?;
        self.run_session(instruction, working_dir, mode).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_code_tool_creation() {
        let tool = ClaudeCodeTool::new();
        assert_eq!(tool.name(), "claude-code");
        assert!(tool.supports_mode(&PermissionMode::Plan));
        assert!(tool.supports_mode(&PermissionMode::Act));
    }

    #[test]
    fn test_claude_code_tool_default() {
        let tool = ClaudeCodeTool::default();
        assert_eq!(tool.name(), "claude-code");
    }

    #[test]
    fn test_builder_methods() {
        let tool = ClaudeCodeTool::new()
            .with_poll_interval(Duration::from_secs(1))
            .with_max_duration(Duration::from_secs(300))
            .with_confirmation_policy(ConfirmationPolicy {
                auto_confirm: false,
                max_auto_confirms: 5,
                dangerous_patterns: vec!["rm".into()],
            })
            .with_server_binary(PathBuf::from("/usr/bin/pty-mcp-server"));

        assert_eq!(tool.poll_interval, Duration::from_secs(1));
        assert_eq!(tool.max_session_duration, Duration::from_secs(300));
        assert!(!tool.confirmation_policy.auto_confirm);
        assert_eq!(
            tool.server_binary,
            Some(PathBuf::from("/usr/bin/pty-mcp-server"))
        );
    }

    #[test]
    fn test_requires_manual_confirmation_dangerous() {
        let tool = ClaudeCodeTool::new();
        assert!(tool.requires_manual_confirmation("rm -rf /important", 0));
        assert!(tool.requires_manual_confirmation("DROP TABLE users", 0));
        assert!(!tool.requires_manual_confirmation("Fixing bug in parser.rs", 0));
    }

    #[test]
    fn test_requires_manual_confirmation_max_reached() {
        let tool = ClaudeCodeTool::new();
        assert!(tool.requires_manual_confirmation("Normal prompt", 10));
        assert!(!tool.requires_manual_confirmation("Normal prompt", 9));
    }
}
