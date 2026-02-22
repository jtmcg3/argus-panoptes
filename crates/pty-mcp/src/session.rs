//! PTY session management.
//!
//! This module provides the core session management logic, adapted from
//! the argus project's shell module.
//!
//! # Security Features
//!
//! - **SEC-001**: Command whitelist to prevent injection
//! - **SEC-004**: Bounded output buffer to prevent memory exhaustion
//! - **SEC-007**: Session limits to prevent resource exhaustion
//! - **SEC-007b**: Command argument validation to block inline code execution

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize, SlavePty};
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, LazyLock};
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, error, info, warn};

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
        "tree",
        "wc",
        "diff",
        "which",
        "whoami",
        "pwd",
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

/// Maximum number of concurrent sessions (SEC-007).
const MAX_SESSIONS: usize = 32;

/// Per-command denied argument patterns (SEC-007b).
/// These arguments allow inline code execution and must be blocked.
static DENIED_ARGS: LazyLock<HashMap<&'static str, Vec<&'static str>>> = LazyLock::new(|| {
    HashMap::from([
        ("python", vec!["-c", "-m"]),
        ("python3", vec!["-c", "-m"]),
        ("node", vec!["-e", "--eval"]),
        ("deno", vec!["eval", "-e"]),
        ("bun", vec!["-e", "--eval"]),
        ("cargo", vec!["--config"]),
        ("git", vec!["-c"]),
    ])
});

/// Validate command arguments against deny list (SEC-007b).
///
/// Blocks dangerous flags that allow inline code execution.
pub fn validate_args(command: &str, args: &[&str]) -> anyhow::Result<()> {
    if let Some(denied) = DENIED_ARGS.get(command) {
        for arg in args {
            let arg_lower = arg.to_lowercase();
            for denied_flag in denied {
                if arg_lower == denied_flag.to_lowercase() {
                    warn!(
                        command = %command,
                        arg = %arg,
                        "Denied argument detected"
                    );
                    anyhow::bail!(
                        "Argument '{}' is not allowed for command '{}'. This flag enables arbitrary code execution.",
                        arg, command
                    );
                }
            }
        }
    }
    Ok(())
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

    // SEC-004: Bounded output buffer tests

    #[test]
    fn test_bounded_buffer_append() {
        let mut buffer = BoundedOutputBuffer::with_max_size(100);

        buffer.append(b"Hello ".to_vec());
        buffer.append(b"World".to_vec());

        assert_eq!(buffer.get_string().unwrap(), "Hello World");
        assert_eq!(buffer.size(), 11);
    }

    #[test]
    fn test_bounded_buffer_eviction() {
        let mut buffer = BoundedOutputBuffer::with_max_size(20);

        // Fill buffer
        buffer.append(b"12345".to_vec()); // 5 bytes
        buffer.append(b"67890".to_vec()); // 10 bytes total
        buffer.append(b"ABCDE".to_vec()); // 15 bytes total

        assert_eq!(buffer.size(), 15);

        // This should trigger eviction
        let evicted = buffer.append(b"FGHIJ".to_vec()); // Would be 20 bytes, exactly at limit
        assert_eq!(buffer.size(), 20);
        assert_eq!(evicted, 0); // No eviction needed, exactly at limit

        // This should evict the oldest chunk
        let evicted = buffer.append(b"KLMNO".to_vec()); // Would exceed 20 bytes
        assert!(evicted > 0);
        assert!(buffer.size() <= 20);
    }

    #[test]
    fn test_bounded_buffer_oversized_chunk() {
        let mut buffer = BoundedOutputBuffer::with_max_size(10);

        // Add a chunk larger than max size
        let oversized = vec![b'X'; 20];
        let evicted = buffer.append(oversized);

        // Should truncate to last 10 bytes
        assert_eq!(buffer.size(), 10);
        assert_eq!(evicted, 10); // 10 bytes were dropped
    }

    #[test]
    fn test_bounded_buffer_clear() {
        let mut buffer = BoundedOutputBuffer::with_max_size(100);

        buffer.append(b"test data".to_vec());
        assert!(buffer.size() > 0);

        buffer.clear();
        assert_eq!(buffer.size(), 0);
        assert_eq!(buffer.chunk_count(), 0);
    }

    #[test]
    fn test_bounded_buffer_utilization() {
        let mut buffer = BoundedOutputBuffer::with_max_size(100);

        buffer.append(vec![0u8; 50]);
        assert!((buffer.utilization() - 50.0).abs() < 0.1);

        buffer.append(vec![0u8; 25]);
        assert!((buffer.utilization() - 75.0).abs() < 0.1);
    }

    #[test]
    fn test_bounded_buffer_lossy_string() {
        let mut buffer = BoundedOutputBuffer::new();

        // Add some invalid UTF-8
        buffer.append(vec![0xFF, 0xFE, b'H', b'i']);

        let lossy = buffer.get_string_lossy();
        assert!(lossy.contains("Hi"));
        // Invalid bytes should be replaced with U+FFFD
        assert!(lossy.contains('\u{FFFD}'));
    }

    #[test]
    fn test_bounded_buffer_default_size() {
        let buffer = BoundedOutputBuffer::new();
        assert_eq!(buffer.max_size(), MAX_OUTPUT_BUFFER_SIZE);
    }

    // SEC-007: Session limit tests

    #[tokio::test]
    async fn test_session_limit_enforced() {
        let manager = SessionManager::with_ttl_and_limit(
            Duration::from_secs(3600),
            2,
        );

        // These should succeed (we can't actually create PtySessions in tests without PTY,
        // so we test the logic through the manager interface)
        // For unit test purposes, just verify the struct is created correctly
        assert_eq!(manager.max_sessions, 2);
    }

    #[tokio::test]
    async fn test_session_stats_include_max() {
        let manager = SessionManager::with_ttl_and_limit(
            Duration::from_secs(3600),
            16,
        );
        let stats = manager.stats().await;
        assert_eq!(stats.max_sessions, 16);
    }

    // SEC-007b: Argument validation tests

    #[test]
    fn test_validate_args_allows_normal() {
        assert!(validate_args("python", &["script.py"]).is_ok());
        assert!(validate_args("node", &["app.js"]).is_ok());
        assert!(validate_args("cargo", &["build"]).is_ok());
        assert!(validate_args("git", &["commit", "-m", "msg"]).is_ok());
    }

    #[test]
    fn test_validate_args_blocks_python_c() {
        assert!(validate_args("python", &["-c", "import os"]).is_err());
        assert!(validate_args("python3", &["-c", "print('hi')"]).is_err());
    }

    #[test]
    fn test_validate_args_blocks_python_m() {
        assert!(validate_args("python", &["-m", "http.server"]).is_err());
        assert!(validate_args("python3", &["-m", "pip"]).is_err());
    }

    #[test]
    fn test_validate_args_blocks_node_eval() {
        assert!(validate_args("node", &["-e", "process.exit()"]).is_err());
        assert!(validate_args("node", &["--eval", "require('child_process')"]).is_err());
    }

    #[test]
    fn test_validate_args_blocks_deno_eval() {
        assert!(validate_args("deno", &["eval", "Deno.exit()"]).is_err());
        assert!(validate_args("deno", &["-e", "code"]).is_err());
    }

    #[test]
    fn test_validate_args_blocks_cargo_config() {
        assert!(validate_args("cargo", &["--config", "build.rustflags=['-C','link-arg=-s']"]).is_err());
    }

    #[test]
    fn test_validate_args_blocks_git_c() {
        assert!(validate_args("git", &["-c", "core.sshCommand=evil"]).is_err());
    }

    #[test]
    fn test_validate_args_unknown_command_passes() {
        // Commands not in DENIED_ARGS have no restrictions
        assert!(validate_args("cargo", &["build", "--release"]).is_ok());
        assert!(validate_args("ls", &["-la"]).is_ok());
    }

    #[test]
    fn test_validate_command_rejects_removed_commands() {
        // env, find, and make should no longer be in the whitelist
        assert!(validate_command("env").is_err());
        assert!(validate_command("find").is_err());
        assert!(validate_command("make").is_err());
    }
}

/// Maximum buffer size in bytes (SEC-004).
/// Default: 10 MB - prevents memory exhaustion from large outputs.
pub const MAX_OUTPUT_BUFFER_SIZE: usize = 10 * 1024 * 1024;

/// Warning threshold as a percentage of max buffer size.
const BUFFER_WARNING_THRESHOLD: f32 = 0.8;

/// A bounded FIFO buffer for PTY output (SEC-004).
///
/// Prevents memory exhaustion by evicting oldest data when size limit is reached.
/// Uses a VecDeque of chunks to avoid expensive reallocation during eviction.
#[derive(Debug)]
pub struct BoundedOutputBuffer {
    /// Buffer chunks (newest at back).
    chunks: VecDeque<Vec<u8>>,
    /// Current total size in bytes.
    current_size: usize,
    /// Maximum allowed size.
    max_size: usize,
    /// Whether we've warned about approaching limit.
    warned: bool,
}

impl BoundedOutputBuffer {
    /// Create a new bounded output buffer with default max size.
    pub fn new() -> Self {
        Self::with_max_size(MAX_OUTPUT_BUFFER_SIZE)
    }

    /// Create a new bounded output buffer with custom max size.
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            chunks: VecDeque::with_capacity(256),
            current_size: 0,
            max_size,
            warned: false,
        }
    }

    /// Append data to the buffer, evicting oldest chunks if necessary.
    ///
    /// Returns the number of bytes evicted (0 if no eviction needed).
    pub fn append(&mut self, data: Vec<u8>) -> usize {
        let data_len = data.len();

        // If single chunk exceeds max, truncate it
        if data_len > self.max_size {
            warn!(
                data_size = data_len,
                max_size = self.max_size,
                "Single output chunk exceeds buffer limit, truncating"
            );
            // Keep only the last max_size bytes
            let truncated = data[(data_len - self.max_size)..].to_vec();
            self.chunks.clear();
            self.current_size = truncated.len();
            self.chunks.push_back(truncated);
            return data_len - self.max_size;
        }

        let mut evicted = 0;

        // Evict oldest chunks until new data fits
        while self.current_size + data_len > self.max_size {
            if let Some(old) = self.chunks.pop_front() {
                evicted += old.len();
                self.current_size -= old.len();
            } else {
                break;
            }
        }

        if evicted > 0 {
            debug!(
                evicted_bytes = evicted,
                new_size = self.current_size + data_len,
                "Evicted old output data to make room"
            );
        }

        // Warn if approaching limit
        let utilization = (self.current_size + data_len) as f32 / self.max_size as f32;
        if utilization > BUFFER_WARNING_THRESHOLD && !self.warned {
            warn!(
                utilization_pct = format!("{:.1}%", utilization * 100.0),
                current_size = self.current_size + data_len,
                max_size = self.max_size,
                "Output buffer approaching size limit"
            );
            self.warned = true;
        }

        // Reset warning flag if we're back below threshold
        if utilization < BUFFER_WARNING_THRESHOLD * 0.9 {
            self.warned = false;
        }

        self.chunks.push_back(data);
        self.current_size += data_len;

        evicted
    }

    /// Get all data as a single byte vector.
    ///
    /// Note: This allocates a new vector. For large buffers, consider
    /// using `chunks()` for iteration instead.
    pub fn get_data(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(self.current_size);
        for chunk in &self.chunks {
            result.extend_from_slice(chunk);
        }
        result
    }

    /// Get data as a UTF-8 string if possible.
    pub fn get_string(&self) -> Result<String, std::string::FromUtf8Error> {
        String::from_utf8(self.get_data())
    }

    /// Get data as a lossy UTF-8 string (invalid bytes replaced with U+FFFD).
    pub fn get_string_lossy(&self) -> String {
        String::from_utf8_lossy(&self.get_data()).into_owned()
    }

    /// Clear all buffered data.
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.current_size = 0;
        self.warned = false;
    }

    /// Get current buffer size in bytes.
    pub fn size(&self) -> usize {
        self.current_size
    }

    /// Get maximum buffer size in bytes.
    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// Get utilization as a percentage.
    pub fn utilization(&self) -> f32 {
        self.current_size as f32 / self.max_size as f32 * 100.0
    }

    /// Get the number of chunks in the buffer.
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

impl Default for BoundedOutputBuffer {
    fn default() -> Self {
        Self::new()
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
    /// SEC-004: Bounded output buffer to prevent memory exhaustion.
    output_buffer: Arc<RwLock<BoundedOutputBuffer>>,
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
        // SEC-004: Use bounded buffer to prevent memory exhaustion
        let output_buffer = Arc::new(RwLock::new(BoundedOutputBuffer::new()));

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

                        // SEC-004: Update bounded buffer
                        if let Ok(mut b) = buffer.try_write() {
                            b.append(data.clone());
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

        // Validate arguments against deny list (SEC-007b)
        validate_args(command, args)?;

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

    /// Get accumulated output buffer as a string.
    ///
    /// SEC-004: Uses bounded buffer with maximum size limit.
    pub async fn get_output(&self) -> String {
        self.output_buffer.read().await.get_string_lossy()
    }

    /// Get raw output buffer as bytes.
    pub async fn get_output_bytes(&self) -> Vec<u8> {
        self.output_buffer.read().await.get_data()
    }

    /// Clear the output buffer.
    pub async fn clear_output(&self) {
        self.output_buffer.write().await.clear();
    }

    /// Get output buffer statistics.
    ///
    /// Returns (current_size, max_size, utilization_percent).
    pub async fn buffer_stats(&self) -> (usize, usize, f32) {
        let buffer = self.output_buffer.read().await;
        (buffer.size(), buffer.max_size(), buffer.utilization())
    }
}

use std::time::{Duration, Instant};

/// Default session TTL (SEC-007).
pub const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(3600); // 1 hour

/// Session entry with TTL metadata (SEC-007).
struct SessionEntry {
    session: Arc<PtySession>,
    created_at: Instant,
    last_accessed: Instant,
}

impl SessionEntry {
    fn new(session: Arc<PtySession>) -> Self {
        let now = Instant::now();
        Self {
            session,
            created_at: now,
            last_accessed: now,
        }
    }

    fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }

    fn is_expired(&self, ttl: Duration) -> bool {
        self.last_accessed.elapsed() > ttl
    }

    fn age(&self) -> Duration {
        self.created_at.elapsed()
    }
}

/// Manages multiple PTY sessions with TTL support (SEC-007).
pub struct SessionManager {
    sessions: RwLock<HashMap<String, SessionEntry>>,
    /// Session time-to-live after last access.
    ttl: Duration,
    /// Maximum number of concurrent sessions (SEC-007).
    pub max_sessions: usize,
}

impl SessionManager {
    /// Create a new session manager with default TTL.
    pub fn new() -> Self {
        Self::with_ttl(DEFAULT_SESSION_TTL)
    }

    /// Create a new session manager with custom TTL.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            ttl,
            max_sessions: MAX_SESSIONS,
        }
    }

    /// Create a new session manager with custom TTL and session limit.
    pub fn with_ttl_and_limit(ttl: Duration, max_sessions: usize) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            ttl,
            max_sessions,
        }
    }

    /// Create a new session.
    pub async fn create(&self, id: &str, working_dir: PathBuf) -> anyhow::Result<Arc<PtySession>> {
        // Clean up expired sessions first
        self.cleanup_expired().await;

        // Check session limit
        let sessions = self.sessions.read().await;
        if sessions.len() >= self.max_sessions {
            anyhow::bail!(
                "Session limit reached ({}/{}). Remove existing sessions or wait for expiration.",
                sessions.len(),
                self.max_sessions
            );
        }
        drop(sessions);

        let session = Arc::new(PtySession::new(id, working_dir)?);
        let entry = SessionEntry::new(session.clone());
        self.sessions.write().await.insert(id.to_string(), entry);
        info!(session_id = %id, ttl_secs = self.ttl.as_secs(), max_sessions = self.max_sessions, "Created session with TTL");
        Ok(session)
    }

    /// Get an existing session, updating last_accessed time.
    pub async fn get(&self, id: &str) -> Option<Arc<PtySession>> {
        let mut sessions = self.sessions.write().await;
        if let Some(entry) = sessions.get_mut(id) {
            // Check if expired
            if entry.is_expired(self.ttl) {
                info!(session_id = %id, "Session expired, removing");
                sessions.remove(id);
                return None;
            }
            entry.touch();
            return Some(entry.session.clone());
        }
        None
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
        self.sessions.write().await.remove(id).map(|e| e.session)
    }

    /// List all session IDs.
    pub async fn list(&self) -> Vec<String> {
        self.sessions.read().await.keys().cloned().collect()
    }

    /// Clean up expired sessions (SEC-007).
    ///
    /// Returns the number of sessions removed.
    pub async fn cleanup_expired(&self) -> usize {
        let mut sessions = self.sessions.write().await;
        let expired: Vec<String> = sessions
            .iter()
            .filter(|(_, entry)| entry.is_expired(self.ttl))
            .map(|(id, _)| id.clone())
            .collect();

        let count = expired.len();
        for id in &expired {
            sessions.remove(id);
            info!(session_id = %id, "Cleaned up expired session");
        }

        if count > 0 {
            info!(cleaned = count, "Session cleanup complete");
        }

        count
    }

    /// Get session statistics.
    pub async fn stats(&self) -> SessionStats {
        let sessions = self.sessions.read().await;
        let total = sessions.len();
        let expired = sessions.values().filter(|e| e.is_expired(self.ttl)).count();
        let active = total - expired;
        let oldest_age = sessions.values().map(|e| e.age()).max();

        SessionStats {
            total_sessions: total,
            active_sessions: active,
            expired_sessions: expired,
            ttl: self.ttl,
            oldest_session_age: oldest_age,
            max_sessions: self.max_sessions,
        }
    }
}

/// Session manager statistics (SEC-007).
#[derive(Debug, Clone)]
pub struct SessionStats {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub expired_sessions: usize,
    pub ttl: Duration,
    pub oldest_session_age: Option<Duration>,
    pub max_sessions: usize,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}
