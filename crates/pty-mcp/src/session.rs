//! PTY session management.
//!
//! This module provides the core session management logic, adapted from
//! the argus project's shell module.

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize, SlavePty};
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, LazyLock};
use tokio::sync::{broadcast, RwLock};
use tracing::{error, info, warn};

/// Whitelist of allowed commands for PTY sessions (SEC-001).
/// Commands not in this list will be rejected to prevent command injection.
static ALLOWED_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        // AI assistants
        "claude",
        // Build tools
        "cargo",
        "npm",
        "yarn",
        "pnpm",
        "make",
        "cmake",
        // Version control
        "git",
        // Languages/runtimes
        "python",
        "python3",
        "node",
        "deno",
        "bun",
        "rustc",
        // Testing
        "pytest",
        "jest",
        "vitest",
        // Utilities (read-only / safe)
        "ls",
        "cat",
        "head",
        "tail",
        "grep",
        "find",
        "tree",
        "wc",
        "diff",
        "which",
        "whoami",
        "pwd",
        "env",
        "echo",
    ])
});

/// Validates that a command is in the whitelist.
/// Returns an error if the command is not allowed.
///
/// Security: This function rejects any command containing a path separator ('/')
/// to prevent path spoofing attacks where an attacker could create a malicious
/// binary named like an allowed command (e.g., /tmp/git).
pub fn validate_command(command: &str) -> anyhow::Result<()> {
    // Security: Reject any command containing path separators
    // This prevents attacks like "/tmp/git" which would have basename "git"
    if command.contains('/') || command.contains('\\') {
        warn!(command = %command, "Command contains path separator - rejected");
        anyhow::bail!(
            "Command '{}' contains a path separator. Only bare command names are allowed (e.g., 'git' not '/usr/bin/git').",
            command
        )
    }

    // Get the command name (first token, no paths allowed at this point)
    let cmd_name = command.split_whitespace().next().unwrap_or(command);

    if ALLOWED_COMMANDS.contains(cmd_name) {
        Ok(())
    } else {
        warn!(command = %command, cmd_name = %cmd_name, "Command not in whitelist");
        anyhow::bail!("Command '{}' is not allowed. Only whitelisted commands can be executed.", cmd_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_command_allows_whitelisted() {
        assert!(validate_command("git").is_ok());
        assert!(validate_command("cargo").is_ok());
        assert!(validate_command("claude").is_ok());
    }

    #[test]
    fn test_validate_command_rejects_path_spoofing() {
        // These should all be rejected - path spoofing attempts
        assert!(validate_command("/tmp/git").is_err());
        assert!(validate_command("/usr/bin/git").is_err());
        assert!(validate_command("../git").is_err());
        assert!(validate_command("./git").is_err());
        assert!(validate_command("/home/attacker/malicious/claude").is_err());
    }

    #[test]
    fn test_validate_command_rejects_unknown() {
        assert!(validate_command("rm").is_err());
        assert!(validate_command("sudo").is_err());
        assert!(validate_command("sh").is_err());
        assert!(validate_command("bash").is_err());
    }
}

/// Status of a PTY session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    /// Session is idle, ready for commands
    Idle,
    /// Session is running a command
    Running,
    /// Session is awaiting user confirmation (y/N prompt)
    AwaitingConfirmation,
    /// Session has exited
    Exited(Option<u32>),
}

/// Events emitted by a PTY session.
#[derive(Debug, Clone)]
pub enum OutputEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit(Option<u32>),
}

/// A single PTY session.
pub struct PtySession {
    id: String,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    slave: Arc<Mutex<Option<Box<dyn SlavePty + Send>>>>,
    child: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    working_dir: PathBuf,
    status: Arc<AtomicU32>,
    running: Arc<AtomicBool>,
    output_tx: broadcast::Sender<OutputEvent>,
    output_buffer: Arc<RwLock<String>>,
}

impl PtySession {
    /// Create a new PTY session.
    pub fn new(id: impl Into<String>, working_dir: PathBuf) -> anyhow::Result<Self> {
        let id = id.into();
        info!(session_id = %id, working_dir = %working_dir.display(), "Creating new PTY session");

        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let (output_tx, _) = broadcast::channel(256);
        let output_buffer = Arc::new(RwLock::new(String::new()));

        let session = Self {
            id: id.clone(),
            master: Arc::new(Mutex::new(pair.master)),
            slave: Arc::new(Mutex::new(Some(pair.slave))),
            child: Arc::new(Mutex::new(None)),
            working_dir,
            status: Arc::new(AtomicU32::new(0)), // Idle
            running: Arc::new(AtomicBool::new(false)),
            output_tx,
            output_buffer,
        };

        // Spawn the reader task (reads from master PTY)
        session.spawn_reader();

        Ok(session)
    }

    /// Spawn a reader thread to read output from the PTY master.
    /// This should be called once when the session is created.
    fn spawn_reader(&self) {
        let master = self.master.clone();
        let tx = self.output_tx.clone();
        let buffer = self.output_buffer.clone();
        let running = self.running.clone();
        let status = self.status.clone();
        let child_handle = self.child.clone();
        let session_id = self.id.clone();

        std::thread::spawn(move || {
            let mut reader = match master.lock() {
                Ok(m) => match m.try_clone_reader() {
                    Ok(r) => r,
                    Err(e) => {
                        error!(session_id = %session_id, error = %e, "Failed to clone PTY reader");
                        return;
                    }
                },
                Err(e) => {
                    error!(session_id = %session_id, error = %e, "Failed to lock master PTY");
                    return;
                }
            };

            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        // EOF - the child process has closed stdout
                        info!(session_id = %session_id, "PTY reader received EOF");

                        // Try to get exit status from child
                        let exit_code: Option<u32> = if let Ok(mut child_guard) = child_handle.lock() {
                            if let Some(ref mut child) = *child_guard {
                                match child.try_wait() {
                                    Ok(Some(status)) => {
                                        info!(session_id = %session_id, exit_status = ?status, "Child process exited");
                                        Some(status.exit_code())
                                    }
                                    Ok(None) => {
                                        // Child still running, wait for it
                                        match child.wait() {
                                            Ok(status) => Some(status.exit_code()),
                                            Err(_) => None,
                                        }
                                    }
                                    Err(_) => None,
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        running.store(false, Ordering::SeqCst);
                        status.store(3, Ordering::SeqCst); // Exited
                        let _ = tx.send(OutputEvent::Exit(exit_code));
                        break;
                    }
                    Ok(n) => {
                        let data = buf[..n].to_vec();

                        // Update buffer
                        if let Ok(text) = String::from_utf8(data.clone()) {
                            if let Ok(mut b) = buffer.try_write() {
                                b.push_str(&text);
                            }
                        }

                        let _ = tx.send(OutputEvent::Stdout(data));
                    }
                    Err(e) => {
                        error!(session_id = %session_id, error = %e, "PTY read error");
                        running.store(false, Ordering::SeqCst);
                        status.store(3, Ordering::SeqCst);
                        let _ = tx.send(OutputEvent::Exit(None));
                        break;
                    }
                }
            }
        });
    }

    /// Spawn a command in this session.
    ///
    /// The command must be in the whitelist (see `ALLOWED_COMMANDS`).
    /// This prevents command injection attacks (SEC-001).
    pub fn spawn(&self, command: &str, args: &[&str]) -> anyhow::Result<()> {
        // Validate command against whitelist (SEC-001)
        validate_command(command)?;

        // Check if a command is already running
        if self.running.load(Ordering::SeqCst) {
            anyhow::bail!("A command is already running in this session");
        }

        info!(
            session_id = %self.id,
            command = %command,
            args = ?args,
            "Spawning command in PTY"
        );

        // Build the command
        let mut cmd = CommandBuilder::new(command);
        for arg in args {
            cmd.arg(*arg);
        }
        cmd.cwd(&self.working_dir);

        // Set up environment
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        // Get the slave to spawn the command on
        let slave = {
            let mut slave_guard = self.slave.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            slave_guard.take().ok_or_else(|| anyhow::anyhow!("Slave PTY already consumed - session can only spawn one command"))?
        };

        // Spawn the command on the slave PTY
        let child = slave.spawn_command(cmd)?;

        info!(
            session_id = %self.id,
            "Command spawned successfully"
        );

        // Drop the slave after spawning (releases handles to the child)
        // This is important: the child now owns the PTY slave side
        drop(slave);

        // Store the child process handle
        {
            let mut child_guard = self.child.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            *child_guard = Some(child);
        }

        // Update status
        self.running.store(true, Ordering::SeqCst);
        self.status.store(1, Ordering::SeqCst); // Running

        Ok(())
    }

    /// Write data to the PTY.
    pub fn write(&self, data: &[u8]) -> anyhow::Result<()> {
        let master = self.master.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut writer = master.take_writer().map_err(|e| anyhow::anyhow!("Writer error: {}", e))?;
        use std::io::Write;
        writer.write_all(data)?;
        Ok(())
    }

    /// Subscribe to output events.
    pub fn subscribe(&self) -> broadcast::Receiver<OutputEvent> {
        self.output_tx.subscribe()
    }

    /// Get current status.
    pub fn status(&self) -> SessionStatus {
        match self.status.load(Ordering::SeqCst) {
            0 => SessionStatus::Idle,
            1 => SessionStatus::Running,
            2 => SessionStatus::AwaitingConfirmation,
            _ => SessionStatus::Exited(None),
        }
    }

    /// Check if session is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Get the session ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Get accumulated output buffer.
    pub async fn get_output(&self) -> String {
        self.output_buffer.read().await.clone()
    }

    /// Clear the output buffer.
    pub async fn clear_output(&self) {
        self.output_buffer.write().await.clear();
    }
}

/// Manages multiple PTY sessions.
pub struct SessionManager {
    sessions: RwLock<HashMap<String, Arc<PtySession>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Create a new session.
    pub async fn create(&self, id: &str, working_dir: PathBuf) -> anyhow::Result<Arc<PtySession>> {
        let session = Arc::new(PtySession::new(id, working_dir)?);
        self.sessions.write().await.insert(id.to_string(), session.clone());
        Ok(session)
    }

    /// Get an existing session.
    pub async fn get(&self, id: &str) -> Option<Arc<PtySession>> {
        self.sessions.read().await.get(id).cloned()
    }

    /// Get or create a session.
    pub async fn get_or_create(&self, id: &str, working_dir: PathBuf) -> anyhow::Result<Arc<PtySession>> {
        if let Some(session) = self.get(id).await {
            return Ok(session);
        }
        self.create(id, working_dir).await
    }

    /// Remove a session.
    pub async fn remove(&self, id: &str) -> Option<Arc<PtySession>> {
        self.sessions.write().await.remove(id)
    }

    /// List all session IDs.
    pub async fn list(&self) -> Vec<String> {
        self.sessions.read().await.keys().cloned().collect()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}
