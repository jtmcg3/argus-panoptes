//! Command runner abstraction for agents that invoke external tools.
//!
//! Provides a trait-based interface for running shell commands, with a real
//! implementation (`SystemCommandRunner`) and a mock for testing.

use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, warn};

/// Output from a command execution.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Exit code (None if the process was killed)
    pub exit_code: Option<i32>,
    /// Whether the command succeeded (exit code 0)
    pub success: bool,
}

/// Trait for running external commands. Mockable for testing.
#[async_trait::async_trait]
pub trait CommandRunner: Send + Sync {
    /// Run a command with the given arguments in the specified directory.
    async fn run(&self, program: &str, args: &[&str], working_dir: &str) -> CommandOutput;
}

/// Real command runner that spawns system processes.
pub struct SystemCommandRunner;

#[async_trait::async_trait]
impl CommandRunner for SystemCommandRunner {
    async fn run(&self, program: &str, args: &[&str], working_dir: &str) -> CommandOutput {
        debug!(program = %program, args = ?args, dir = %working_dir, "Running command");

        let result = Command::new(program)
            .args(args)
            .current_dir(working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match result {
            Ok(output) => {
                let exit_code = output.status.code();
                CommandOutput {
                    stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                    stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                    success: output.status.success(),
                    exit_code,
                }
            }
            Err(e) => {
                warn!(program = %program, error = %e, "Command failed to execute");
                CommandOutput {
                    stdout: String::new(),
                    stderr: format!("Failed to execute {}: {}", program, e),
                    exit_code: None,
                    success: false,
                }
            }
        }
    }
}

/// Mock command runner for testing.
#[cfg(test)]
pub struct MockCommandRunner {
    responses: std::sync::Mutex<Vec<CommandOutput>>,
}

#[cfg(test)]
impl MockCommandRunner {
    /// Create a mock that returns the given responses in order.
    pub fn new(responses: Vec<CommandOutput>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses),
        }
    }

    /// Create a mock that always returns success with the given stdout.
    pub fn success(stdout: &str) -> Self {
        Self::new(vec![CommandOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
        }])
    }

    /// Create a mock that always returns failure with the given stderr.
    pub fn failure(stderr: &str) -> Self {
        Self::new(vec![CommandOutput {
            stdout: String::new(),
            stderr: stderr.to_string(),
            exit_code: Some(1),
            success: false,
        }])
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl CommandRunner for MockCommandRunner {
    async fn run(&self, _program: &str, _args: &[&str], _working_dir: &str) -> CommandOutput {
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            CommandOutput {
                stdout: String::new(),
                stderr: "No more mock responses".to_string(),
                exit_code: Some(1),
                success: false,
            }
        } else {
            responses.remove(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_command_runner_success() {
        let runner = MockCommandRunner::success("hello world");
        let output = runner.run("echo", &["hello"], ".").await;
        assert!(output.success);
        assert_eq!(output.stdout, "hello world");
    }

    #[tokio::test]
    async fn test_mock_command_runner_failure() {
        let runner = MockCommandRunner::failure("not found");
        let output = runner.run("bad-cmd", &[], ".").await;
        assert!(!output.success);
        assert_eq!(output.stderr, "not found");
    }

    #[tokio::test]
    async fn test_mock_command_runner_multiple_responses() {
        let runner = MockCommandRunner::new(vec![
            CommandOutput {
                stdout: "first".into(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
            CommandOutput {
                stdout: "second".into(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
        ]);

        let out1 = runner.run("cmd", &[], ".").await;
        assert_eq!(out1.stdout, "first");

        let out2 = runner.run("cmd", &[], ".").await;
        assert_eq!(out2.stdout, "second");
    }
}
