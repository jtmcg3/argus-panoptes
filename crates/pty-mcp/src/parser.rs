//! PTY output parser for detecting prompts and states.
//!
//! Adapted from argus shell/parser.rs

use regex::Regex;
use std::sync::LazyLock;

/// Type of prompt detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptType {
    YesNo,
    PressEnter,
    Permission,
    Input,
    Unknown,
}

/// Parsed state from PTY output.
#[derive(Debug, Clone)]
pub enum ParsedState {
    /// Normal output text
    Output(String),

    /// Needs user confirmation
    NeedsConfirmation {
        prompt: String,
        prompt_type: PromptType,
    },

    /// Tool execution marker
    ToolExecution { action: String, target: String },

    /// Error detected
    Error { message: String },

    /// Session exited
    Exited { exit_code: Option<i32> },
}

// Regex patterns for Claude CLI output
static CONFIRM_YN_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\[y/n\]|\(y/n\)|yes/no|\[yes/no\]").unwrap());

static PRESS_ENTER_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)press enter|hit enter|continue\?").unwrap());

static PERMISSION_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)allow|permit|proceed|approve|confirm").unwrap());

static TOOL_MARKER_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(Running|Executing|Creating|Reading|Writing|Editing|Deleting):\s*(.+)")
        .unwrap()
});

static ERROR_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(error|failed|exception|panic)").unwrap());

/// Parser for Claude CLI output.
pub struct ClaudeParser {
    buffer: String,
    _in_tool_block: bool,
}

impl ClaudeParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            _in_tool_block: false,
        }
    }

    /// Parse incoming PTY data.
    pub fn parse(&mut self, data: &[u8]) -> Vec<ParsedState> {
        let mut states = Vec::new();

        // Strip ANSI escape codes
        let clean_data = strip_ansi_escapes::strip(data);

        let text = String::from_utf8_lossy(&clean_data);
        self.buffer.push_str(&text);

        // Process complete lines
        while let Some(newline_pos) = self.buffer.find('\n') {
            let line = self.buffer[..newline_pos].trim().to_string();
            self.buffer = self.buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            // Check for confirmation prompts
            if CONFIRM_YN_PATTERN.is_match(&line) {
                states.push(ParsedState::NeedsConfirmation {
                    prompt: line,
                    prompt_type: PromptType::YesNo,
                });
                continue;
            }

            if PRESS_ENTER_PATTERN.is_match(&line) {
                states.push(ParsedState::NeedsConfirmation {
                    prompt: line,
                    prompt_type: PromptType::PressEnter,
                });
                continue;
            }

            if PERMISSION_PATTERN.is_match(&line) && line.contains('?') {
                states.push(ParsedState::NeedsConfirmation {
                    prompt: line,
                    prompt_type: PromptType::Permission,
                });
                continue;
            }

            // Check for tool markers
            if let Some(caps) = TOOL_MARKER_PATTERN.captures(&line) {
                let action = caps
                    .get(1)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();
                let target = caps
                    .get(2)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();
                states.push(ParsedState::ToolExecution { action, target });
                continue;
            }

            // Check for errors
            if ERROR_PATTERN.is_match(&line) {
                states.push(ParsedState::Error { message: line });
                continue;
            }

            // Regular output
            states.push(ParsedState::Output(line));
        }

        states
    }

    /// Flush any remaining buffered content.
    pub fn flush(&mut self) -> Option<ParsedState> {
        if self.buffer.is_empty() {
            return None;
        }

        let remaining = std::mem::take(&mut self.buffer);
        Some(ParsedState::Output(remaining))
    }
}

impl Default for ClaudeParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_confirmation() {
        let mut parser = ClaudeParser::new();
        let states = parser.parse(b"Do you want to proceed? [y/N]\n");

        assert_eq!(states.len(), 1);
        matches!(
            &states[0],
            ParsedState::NeedsConfirmation {
                prompt_type: PromptType::YesNo,
                ..
            }
        );
    }

    #[test]
    fn test_parse_tool_execution() {
        let mut parser = ClaudeParser::new();
        let states = parser.parse(b"Running: cargo build\n");

        assert_eq!(states.len(), 1);
        matches!(&states[0], ParsedState::ToolExecution { action, target } if action == "Running" && target == "cargo build");
    }
}
