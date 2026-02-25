//! Pluggable PTY tool abstraction.
//!
//! This module defines the [`PtyTool`] trait, which allows different CLI-based
//! coding tools (Claude Code, Gemini CLI, Codex, etc.) to be used
//! interchangeably by the [`CodingAgent`](crate::CodingAgent).

use async_trait::async_trait;
use panoptes_common::Result;
use panoptes_coordinator::routing::PermissionMode;
use std::path::Path;

/// Output from a PTY tool execution.
#[derive(Debug, Clone)]
pub struct PtyToolOutput {
    /// The combined stdout/stderr content produced by the tool.
    pub content: String,
    /// The process exit code, if available.
    pub exit_code: Option<i32>,
    /// An optional session identifier for tracking purposes.
    pub session_id: Option<String>,
}

/// Trait for pluggable PTY-based coding tools.
///
/// Implementations wrap a specific CLI tool (e.g. Claude Code, Gemini CLI)
/// and provide a uniform interface for the [`CodingAgent`](crate::CodingAgent)
/// to execute coding instructions.
#[async_trait]
pub trait PtyTool: Send + Sync {
    /// Tool name (e.g., "claude-code", "gemini-cli", "codex").
    fn name(&self) -> &str;

    /// Whether this tool supports the given permission mode.
    fn supports_mode(&self, mode: &PermissionMode) -> bool;

    /// Execute an instruction using this tool.
    async fn execute(
        &self,
        instruction: &str,
        working_dir: &Path,
        mode: PermissionMode,
    ) -> Result<PtyToolOutput>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_coordinator::routing::PermissionMode;
    use std::path::PathBuf;

    /// A mock PtyTool for testing.
    struct MockPtyTool {
        name: String,
        response: String,
    }

    impl MockPtyTool {
        fn new(name: &str, response: &str) -> Self {
            Self {
                name: name.to_string(),
                response: response.to_string(),
            }
        }
    }

    #[async_trait]
    impl PtyTool for MockPtyTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn supports_mode(&self, _mode: &PermissionMode) -> bool {
            true
        }

        async fn execute(
            &self,
            instruction: &str,
            _working_dir: &Path,
            _mode: PermissionMode,
        ) -> Result<PtyToolOutput> {
            Ok(PtyToolOutput {
                content: format!("{}: {}", self.response, instruction),
                exit_code: Some(0),
                session_id: Some("mock-session-1".to_string()),
            })
        }
    }

    #[test]
    fn test_pty_tool_output() {
        let output = PtyToolOutput {
            content: "hello".into(),
            exit_code: Some(0),
            session_id: None,
        };
        assert_eq!(output.content, "hello");
        assert_eq!(output.exit_code, Some(0));
        assert!(output.session_id.is_none());
    }

    #[tokio::test]
    async fn test_mock_pty_tool() {
        let tool = MockPtyTool::new("mock-tool", "executed");
        assert_eq!(tool.name(), "mock-tool");
        assert!(tool.supports_mode(&PermissionMode::Plan));
        assert!(tool.supports_mode(&PermissionMode::Act));

        let output = tool
            .execute("fix the bug", &PathBuf::from("/tmp"), PermissionMode::Plan)
            .await
            .unwrap();

        assert_eq!(output.content, "executed: fix the bug");
        assert_eq!(output.exit_code, Some(0));
        assert_eq!(output.session_id, Some("mock-session-1".to_string()));
    }
}
