//! Generic CLI tool implementation.
//!
//! Provides a configurable [`PtyTool`] that can wrap any CLI binary
//! (e.g. `gemini-cli`, `codex`, `aider`) by spawning it as a child
//! process via `tokio::process::Command`.

use crate::pty_tool::{PtyTool, PtyToolOutput};
use async_trait::async_trait;
use panoptes_common::{PanoptesError, Result};
use panoptes_coordinator::routing::PermissionMode;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Configuration for a generic CLI tool.
#[derive(Debug, Clone)]
pub struct CliToolConfig {
    /// Human-readable name for this tool (e.g. "gemini-cli").
    pub name: String,
    /// Path or name of the binary to invoke.
    pub binary: String,
    /// Argument template. Use `"{instruction}"` as a placeholder that
    /// will be replaced with the actual instruction text.
    pub args_template: Vec<String>,
    /// Extra environment variables to set when spawning the process.
    pub env_vars: HashMap<String, String>,
    /// Maximum time to wait for the process to complete.
    pub timeout: Duration,
    /// Per-mode extra flags. Keys are `"plan"` / `"act"`.
    /// Values are additional CLI arguments appended for that mode.
    pub mode_flags: HashMap<String, Vec<String>>,
}

impl Default for CliToolConfig {
    fn default() -> Self {
        Self {
            name: "generic-cli".into(),
            binary: "echo".into(),
            args_template: vec!["{instruction}".into()],
            env_vars: HashMap::new(),
            timeout: Duration::from_secs(300),
            mode_flags: HashMap::new(),
        }
    }
}

/// A [`PtyTool`] backed by an arbitrary CLI binary.
///
/// Uses `tokio::process::Command` to spawn the tool, pass the instruction
/// via templated arguments, and capture stdout + stderr.
pub struct GenericCliTool {
    pub config: CliToolConfig,
}

impl GenericCliTool {
    /// Create a new `GenericCliTool` from a [`CliToolConfig`].
    pub fn new(config: CliToolConfig) -> Self {
        Self { config }
    }

    /// Build the final argument list by expanding templates and adding mode flags.
    fn build_args(&self, instruction: &str, mode: &PermissionMode) -> Vec<String> {
        let mut args: Vec<String> = self
            .config
            .args_template
            .iter()
            .map(|arg| arg.replace("{instruction}", instruction))
            .collect();

        let mode_key = match mode {
            PermissionMode::Plan => "plan",
            PermissionMode::Act => "act",
        };

        if let Some(extra) = self.config.mode_flags.get(mode_key) {
            args.extend(extra.clone());
        }

        args
    }
}

#[async_trait]
impl PtyTool for GenericCliTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supports_mode(&self, _mode: &PermissionMode) -> bool {
        // Generic tools support all modes; unsupported flags simply won't
        // be present in mode_flags and no extras will be appended.
        true
    }

    async fn execute(
        &self,
        instruction: &str,
        working_dir: &Path,
        mode: PermissionMode,
    ) -> Result<PtyToolOutput> {
        let args = self.build_args(instruction, &mode);

        info!(
            tool = self.config.name,
            binary = self.config.binary,
            "Spawning generic CLI tool"
        );
        debug!(tool = self.config.name, ?args, "CLI arguments");

        let mut cmd = tokio::process::Command::new(&self.config.binary);
        cmd.args(&args).current_dir(working_dir);

        for (k, v) in &self.config.env_vars {
            cmd.env(k, v);
        }

        let output = tokio::time::timeout(self.config.timeout, cmd.output())
            .await
            .map_err(|_| {
                warn!(tool = self.config.name, "CLI tool timed out");
                PanoptesError::Pty(format!(
                    "Tool '{}' timed out after {:?}",
                    self.config.name, self.config.timeout
                ))
            })?
            .map_err(|e| {
                PanoptesError::Pty(format!("Failed to spawn '{}': {}", self.config.binary, e))
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut content = stdout.into_owned();
        if !stderr.is_empty() {
            if !content.is_empty() {
                content.push('\n');
            }
            content.push_str(&stderr);
        }

        let exit_code = output.status.code();

        info!(
            tool = self.config.name,
            exit_code = ?exit_code,
            "CLI tool completed"
        );

        Ok(PtyToolOutput {
            content,
            exit_code,
            session_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_default_config() {
        let config = CliToolConfig::default();
        assert_eq!(config.name, "generic-cli");
        assert_eq!(config.binary, "echo");
        assert_eq!(config.args_template, vec!["{instruction}"]);
        assert!(config.env_vars.is_empty());
        assert!(config.mode_flags.is_empty());
    }

    #[test]
    fn test_build_args_basic() {
        let tool = GenericCliTool::new(CliToolConfig {
            args_template: vec!["-p".into(), "{instruction}".into()],
            ..Default::default()
        });

        let args = tool.build_args("fix the bug", &PermissionMode::Plan);
        assert_eq!(args, vec!["-p", "fix the bug"]);
    }

    #[test]
    fn test_build_args_with_mode_flags() {
        let mut mode_flags = HashMap::new();
        mode_flags.insert("plan".into(), vec!["--mode".into(), "suggest".into()]);
        mode_flags.insert("act".into(), vec!["--mode".into(), "apply".into()]);

        let tool = GenericCliTool::new(CliToolConfig {
            args_template: vec!["{instruction}".into()],
            mode_flags,
            ..Default::default()
        });

        let plan_args = tool.build_args("hello", &PermissionMode::Plan);
        assert_eq!(plan_args, vec!["hello", "--mode", "suggest"]);

        let act_args = tool.build_args("hello", &PermissionMode::Act);
        assert_eq!(act_args, vec!["hello", "--mode", "apply"]);
    }

    #[test]
    fn test_generic_tool_name() {
        let tool = GenericCliTool::new(CliToolConfig {
            name: "my-tool".into(),
            ..Default::default()
        });
        assert_eq!(tool.name(), "my-tool");
    }

    #[test]
    fn test_supports_mode() {
        let tool = GenericCliTool::new(CliToolConfig::default());
        assert!(tool.supports_mode(&PermissionMode::Plan));
        assert!(tool.supports_mode(&PermissionMode::Act));
    }

    #[tokio::test]
    async fn test_execute_with_echo() {
        let tool = GenericCliTool::new(CliToolConfig {
            name: "echo-tool".into(),
            binary: "echo".into(),
            args_template: vec!["{instruction}".into()],
            timeout: Duration::from_secs(5),
            ..Default::default()
        });

        let output = tool
            .execute("hello world", &PathBuf::from("/tmp"), PermissionMode::Plan)
            .await
            .unwrap();

        assert_eq!(output.content.trim(), "hello world");
        assert_eq!(output.exit_code, Some(0));
        assert!(output.session_id.is_none());
    }

    #[tokio::test]
    async fn test_execute_with_env_vars() {
        let mut env_vars = HashMap::new();
        env_vars.insert("MY_VAR".into(), "test_value".into());

        let tool = GenericCliTool::new(CliToolConfig {
            name: "env-test".into(),
            binary: "sh".into(),
            args_template: vec!["-c".into(), "echo $MY_VAR".into()],
            env_vars,
            timeout: Duration::from_secs(5),
            ..Default::default()
        });

        let output = tool
            .execute("ignored", &PathBuf::from("/tmp"), PermissionMode::Plan)
            .await
            .unwrap();

        assert_eq!(output.content.trim(), "test_value");
    }

    #[tokio::test]
    async fn test_execute_nonexistent_binary() {
        let tool = GenericCliTool::new(CliToolConfig {
            name: "bad-tool".into(),
            binary: "this-binary-does-not-exist-12345".into(),
            timeout: Duration::from_secs(5),
            ..Default::default()
        });

        let result = tool
            .execute("hello", &PathBuf::from("/tmp"), PermissionMode::Plan)
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_captures_stderr() {
        let tool = GenericCliTool::new(CliToolConfig {
            name: "stderr-test".into(),
            binary: "sh".into(),
            args_template: vec!["-c".into(), "echo err >&2".into()],
            timeout: Duration::from_secs(5),
            ..Default::default()
        });

        let output = tool
            .execute("ignored", &PathBuf::from("/tmp"), PermissionMode::Plan)
            .await
            .unwrap();

        assert!(output.content.contains("err"));
    }
}
