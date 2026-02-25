//! Review agent - static analysis via cargo clippy/fmt + pattern scanning.
//!
//! This agent handles review tasks by:
//! 1. Running cargo clippy for lint analysis
//! 2. Running cargo fmt --check for formatting issues
//! 3. Scanning for TODO/FIXME markers
//! 4. Aggregating findings into a severity-grouped report

use crate::command::{CommandOutput, CommandRunner, SystemCommandRunner};
use crate::traits::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use panoptes_common::{AgentMessage, PanoptesError, Result, Task};
use panoptes_llm::{ChatMessage, LlmClient, LlmRequest, Role};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{info, warn};

const REVIEW_SYSTEM_PROMPT: &str = r#"You are a senior code reviewer. Your role is to:

1. Review code for correctness, security, and maintainability
2. Identify bugs, vulnerabilities, and code smells
3. Suggest improvements and best practices
4. Ensure code follows project conventions
5. Provide constructive, actionable feedback

Focus on high-impact issues first.
Explain the "why" behind suggestions.
Be respectful and constructive in feedback.
"#;

/// Severity levels for review findings.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => write!(f, "Error"),
            Severity::Warning => write!(f, "Warning"),
            Severity::Info => write!(f, "Info"),
        }
    }
}

/// Category of a review finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindingCategory {
    Clippy,
    Formatting,
    Todo,
}

impl std::fmt::Display for FindingCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindingCategory::Clippy => write!(f, "Clippy"),
            FindingCategory::Formatting => write!(f, "Formatting"),
            FindingCategory::Todo => write!(f, "TODO/FIXME"),
        }
    }
}

/// A single finding from the review.
#[derive(Debug, Clone)]
pub struct ReviewFinding {
    pub severity: Severity,
    pub file: String,
    pub line: Option<u32>,
    pub message: String,
    pub category: FindingCategory,
}

/// Configuration for the review agent.
#[derive(Debug, Clone)]
pub struct ReviewConfig {
    /// Whether to run clippy
    pub run_clippy: bool,
    /// Whether to check formatting
    pub run_fmt_check: bool,
    /// Whether to scan for TODO/FIXME
    pub check_todos: bool,
    /// Timeout for each command in seconds
    pub timeout_secs: u64,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            run_clippy: true,
            run_fmt_check: true,
            check_todos: true,
            timeout_secs: 120,
        }
    }
}

/// Review agent for code review and quality analysis.
pub struct ReviewAgent {
    config: AgentConfig,
    review_config: ReviewConfig,
    busy: AtomicBool,
    command_runner: Arc<dyn CommandRunner>,
    /// Optional LLM client for enhanced output
    llm_client: Option<Arc<dyn LlmClient>>,
}

impl ReviewAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            review_config: ReviewConfig::default(),
            busy: AtomicBool::new(false),
            command_runner: Arc::new(SystemCommandRunner),
            llm_client: None,
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(AgentConfig {
            id: "review".into(),
            name: "Review Agent".into(),
            mcp_servers: vec!["pty-mcp".into()],
            ..Default::default()
        })
    }

    /// Use a custom command runner (for testing).
    pub fn with_command_runner(mut self, runner: Arc<dyn CommandRunner>) -> Self {
        self.command_runner = runner;
        self
    }

    /// Attach an LLM client for enhanced output.
    pub fn with_llm(mut self, client: Arc<dyn LlmClient>) -> Self {
        self.llm_client = Some(client);
        self
    }

    /// Enhance template output using the LLM, with graceful degradation.
    async fn enhance_with_llm(&self, template_output: &str, task: &Task) -> String {
        if let Some(ref llm) = self.llm_client {
            let system_prompt = self.system_prompt().to_string();
            let request = LlmRequest {
                system_prompt: Some(system_prompt),
                messages: vec![ChatMessage {
                    role: Role::User,
                    content: format!(
                        "Task: {}\n\nAnalyze these code review findings and provide actionable recommendations:\n{}",
                        task.description, template_output
                    ),
                }],
                temperature: Some(0.7),
                max_tokens: Some(4096),
            };

            match llm.complete(request).await {
                Ok(response) if !response.content.is_empty() => return response.content,
                Ok(_) => warn!(agent = %self.id(), "LLM returned empty response, using template"),
                Err(e) => {
                    warn!(agent = %self.id(), error = %e, "LLM failed, falling back to template")
                }
            }
        }
        template_output.to_string()
    }

    /// Parse cargo clippy output into review findings.
    fn parse_clippy_output(output: &CommandOutput) -> Vec<ReviewFinding> {
        let mut findings = Vec::new();
        let text = if output.stderr.is_empty() {
            &output.stdout
        } else {
            &output.stderr
        };

        for line in text.lines() {
            // Match patterns like: "warning: unused variable `x`" or "error[E0001]: ..."
            if let Some(finding) = Self::parse_clippy_line(line) {
                findings.push(finding);
            }
        }

        findings
    }

    /// Parse a single clippy output line.
    fn parse_clippy_line(line: &str) -> Option<ReviewFinding> {
        let trimmed = line.trim();

        // Match "warning: message" or "error: message" or "error[E0xxx]: message"
        let (severity, rest) = if let Some(rest) = trimmed.strip_prefix("warning: ") {
            (Severity::Warning, rest)
        } else if let Some(rest) = trimmed.strip_prefix("error: ") {
            (Severity::Error, rest)
        } else if trimmed.starts_with("error[") {
            if let Some(idx) = trimmed.find("]: ") {
                (Severity::Error, &trimmed[idx + 3..])
            } else {
                return None;
            }
        } else {
            return None;
        };

        // Skip meta-messages like "warning: 3 warnings emitted"
        if rest.contains("warning emitted")
            || rest.contains("warnings emitted")
            || rest.contains("aborting due to")
            || rest.contains("could not compile")
        {
            return None;
        }

        Some(ReviewFinding {
            severity,
            file: String::new(),
            line: None,
            message: rest.to_string(),
            category: FindingCategory::Clippy,
        })
    }

    /// Parse cargo fmt --check output into findings.
    fn parse_fmt_output(output: &CommandOutput) -> Vec<ReviewFinding> {
        let mut findings = Vec::new();

        // fmt --check outputs a diff; each "Diff in" line indicates a file
        let text = &output.stdout;

        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("Diff in ") {
                // "Diff in /path/to/file.rs at line 42:"
                let parts: Vec<&str> = rest.splitn(3, ' ').collect();
                let file = parts.first().unwrap_or(&"unknown").to_string();
                let line_num = if parts.len() >= 3 {
                    parts[2]
                        .trim_end_matches(':')
                        .trim_start_matches("line ")
                        .parse()
                        .ok()
                } else {
                    None
                };

                findings.push(ReviewFinding {
                    severity: Severity::Warning,
                    file,
                    line: line_num,
                    message: "Formatting differs from cargo fmt style".into(),
                    category: FindingCategory::Formatting,
                });
            }
        }

        // If fmt failed but no diff lines, the whole output is the issue
        if findings.is_empty() && !output.success && !text.trim().is_empty() {
            findings.push(ReviewFinding {
                severity: Severity::Warning,
                file: String::new(),
                line: None,
                message: "Code formatting does not match cargo fmt style".into(),
                category: FindingCategory::Formatting,
            });
        }

        findings
    }

    /// Scan for TODO/FIXME markers using grep.
    fn parse_todo_output(output: &CommandOutput) -> Vec<ReviewFinding> {
        let mut findings = Vec::new();

        for line in output.stdout.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // grep output: "file:line:content"
            let parts: Vec<&str> = trimmed.splitn(3, ':').collect();
            if parts.len() >= 3 {
                let file = parts[0].to_string();
                let line_num = parts[1].parse().ok();
                let content = parts[2].trim().to_string();

                findings.push(ReviewFinding {
                    severity: Severity::Info,
                    file,
                    line: line_num,
                    message: content,
                    category: FindingCategory::Todo,
                });
            } else if parts.len() >= 2 {
                findings.push(ReviewFinding {
                    severity: Severity::Info,
                    file: parts[0].to_string(),
                    line: None,
                    message: parts[1].trim().to_string(),
                    category: FindingCategory::Todo,
                });
            }
        }

        findings
    }

    /// Build a markdown review report grouped by severity.
    fn build_review_report(findings: &[ReviewFinding], target: &str) -> String {
        let mut report = String::new();

        report.push_str("# Code Review Report\n\n");
        report.push_str(&format!("**Target:** {}\n\n", target));

        let error_count = findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .count();
        let warning_count = findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .count();
        let info_count = findings
            .iter()
            .filter(|f| f.severity == Severity::Info)
            .count();

        report.push_str(&format!(
            "**Summary:** {} errors, {} warnings, {} info\n\n",
            error_count, warning_count, info_count
        ));

        if findings.is_empty() {
            report.push_str("No issues found. Code looks clean!\n");
            return report;
        }

        let severity_order = [Severity::Error, Severity::Warning, Severity::Info];

        for severity in &severity_order {
            let items: Vec<&ReviewFinding> = findings
                .iter()
                .filter(|f| f.severity == *severity)
                .collect();

            if items.is_empty() {
                continue;
            }

            report.push_str(&format!("## {} ({})\n\n", severity, items.len()));

            for (i, finding) in items.iter().enumerate() {
                let location = if finding.file.is_empty() {
                    String::new()
                } else if let Some(line) = finding.line {
                    format!(" `{}:{}`", finding.file, line)
                } else {
                    format!(" `{}`", finding.file)
                };

                report.push_str(&format!(
                    "{}. [{}]{} — {}\n",
                    i + 1,
                    finding.category,
                    location,
                    finding.message
                ));
            }

            report.push('\n');
        }

        report.push_str("---\n\n");
        report.push_str(&format!("**Total findings:** {}\n", findings.len()));

        report
    }

    /// Execute the review pipeline.
    async fn execute_review(&self, task: &Task) -> Result<AgentMessage> {
        let target = task.context.as_deref().unwrap_or(&task.description);

        info!(agent = %self.id(), task_id = %task.id, target = %target, "Starting review");

        let working_dir = ".";
        let mut findings = Vec::new();

        // Step 1: Run clippy
        if self.review_config.run_clippy {
            info!(agent = %self.id(), "Running cargo clippy");
            let output = self
                .command_runner
                .run(
                    "cargo",
                    &["clippy", "--all-targets", "--", "-D", "warnings"],
                    working_dir,
                )
                .await;
            let clippy_findings = Self::parse_clippy_output(&output);
            info!(agent = %self.id(), findings = clippy_findings.len(), "Clippy analysis complete");
            findings.extend(clippy_findings);
        }

        // Step 2: Check formatting
        if self.review_config.run_fmt_check {
            info!(agent = %self.id(), "Running cargo fmt --check");
            let output = self
                .command_runner
                .run("cargo", &["fmt", "--all", "--", "--check"], working_dir)
                .await;
            let fmt_findings = Self::parse_fmt_output(&output);
            info!(agent = %self.id(), findings = fmt_findings.len(), "Format check complete");
            findings.extend(fmt_findings);
        }

        // Step 3: Scan for TODOs
        if self.review_config.check_todos {
            info!(agent = %self.id(), "Scanning for TODO/FIXME");
            let output = self
                .command_runner
                .run(
                    "grep",
                    &["-rn", "--include=*.rs", "TODO\\|FIXME", "src/", "crates/"],
                    working_dir,
                )
                .await;
            let todo_findings = Self::parse_todo_output(&output);
            info!(agent = %self.id(), findings = todo_findings.len(), "TODO scan complete");
            findings.extend(todo_findings);
        }

        let report = Self::build_review_report(&findings, target);

        // Enhance with LLM if available
        let report = self.enhance_with_llm(&report, task).await;

        info!(
            agent = %self.id(),
            task_id = %task.id,
            total_findings = findings.len(),
            "Review complete"
        );

        let mut message = AgentMessage::from_agent(self.id(), report);
        message.task_id = Some(task.id.clone());
        Ok(message)
    }
}

#[async_trait]
impl Agent for ReviewAgent {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn capabilities(&self) -> &[AgentCapability] {
        &[
            AgentCapability::CodeReview,
            AgentCapability::DocumentAnalysis,
            AgentCapability::MemoryAccess,
        ]
    }

    async fn process_task(&self, task: &Task) -> Result<AgentMessage> {
        info!(
            agent = %self.id(),
            task_id = %task.id,
            "Processing review task"
        );

        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(PanoptesError::Agent(format!(
                "Agent {} is busy processing another task",
                self.id()
            )));
        }

        let result = self.execute_review(task).await;
        self.busy.store(false, Ordering::SeqCst);
        result
    }

    async fn handle_message(&self, message: &AgentMessage) -> Result<AgentMessage> {
        let task = Task::new(&message.content);
        self.process_task(&task).await
    }

    fn system_prompt(&self) -> &str {
        self.config
            .system_prompt
            .as_deref()
            .unwrap_or(REVIEW_SYSTEM_PROMPT)
    }

    fn is_available(&self) -> bool {
        !self.busy.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::MockCommandRunner;

    #[test]
    fn test_review_agent_creation() {
        let agent = ReviewAgent::with_default_config();
        assert_eq!(agent.id(), "review");
        assert_eq!(agent.name(), "Review Agent");
        assert!(agent.is_available());
    }

    #[test]
    fn test_parse_clippy_warnings() {
        let output = CommandOutput {
            stdout: String::new(),
            stderr: "warning: unused variable `x`\n\
                     warning: unused import `std::io`\n\
                     warning: 2 warnings emitted\n"
                .into(),
            exit_code: Some(0),
            success: true,
        };

        let findings = ReviewAgent::parse_clippy_output(&output);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, Severity::Warning);
        assert!(findings[0].message.contains("unused variable"));
        assert_eq!(findings[0].category, FindingCategory::Clippy);
    }

    #[test]
    fn test_parse_clippy_errors() {
        let output = CommandOutput {
            stdout: String::new(),
            stderr: "error[E0308]: mismatched types\n\
                     error: aborting due to 1 previous error\n"
                .into(),
            exit_code: Some(1),
            success: false,
        };

        let findings = ReviewAgent::parse_clippy_output(&output);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(findings[0].message.contains("mismatched types"));
    }

    #[test]
    fn test_parse_clippy_clean() {
        let output = CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
        };

        let findings = ReviewAgent::parse_clippy_output(&output);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_parse_fmt_output_with_diffs() {
        let output = CommandOutput {
            stdout: "Diff in src/main.rs at line 42:\n\
                     Diff in src/lib.rs at line 10:\n"
                .into(),
            stderr: String::new(),
            exit_code: Some(1),
            success: false,
        };

        let findings = ReviewAgent::parse_fmt_output(&output);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, Severity::Warning);
        assert_eq!(findings[0].file, "src/main.rs");
        assert_eq!(findings[0].category, FindingCategory::Formatting);
    }

    #[test]
    fn test_parse_fmt_output_clean() {
        let output = CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
        };

        let findings = ReviewAgent::parse_fmt_output(&output);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_parse_todo_output() {
        let output = CommandOutput {
            stdout: "src/main.rs:10:    // TODO: implement this\n\
                     src/lib.rs:25:    // FIXME: broken logic\n"
                .into(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
        };

        let findings = ReviewAgent::parse_todo_output(&output);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, Severity::Info);
        assert_eq!(findings[0].file, "src/main.rs");
        assert_eq!(findings[0].line, Some(10));
        assert!(findings[0].message.contains("TODO"));
        assert_eq!(findings[0].category, FindingCategory::Todo);
    }

    #[test]
    fn test_build_review_report_with_findings() {
        let findings = vec![
            ReviewFinding {
                severity: Severity::Error,
                file: "src/main.rs".into(),
                line: Some(10),
                message: "mismatched types".into(),
                category: FindingCategory::Clippy,
            },
            ReviewFinding {
                severity: Severity::Warning,
                file: "src/lib.rs".into(),
                line: None,
                message: "unused import".into(),
                category: FindingCategory::Clippy,
            },
            ReviewFinding {
                severity: Severity::Info,
                file: "src/main.rs".into(),
                line: Some(42),
                message: "TODO: implement".into(),
                category: FindingCategory::Todo,
            },
        ];

        let report = ReviewAgent::build_review_report(&findings, ".");
        assert!(report.contains("Code Review Report"));
        assert!(report.contains("1 errors, 1 warnings, 1 info"));
        assert!(report.contains("mismatched types"));
        assert!(report.contains("unused import"));
        assert!(report.contains("TODO: implement"));
        assert!(report.contains("**Total findings:** 3"));
    }

    #[test]
    fn test_build_review_report_clean() {
        let report = ReviewAgent::build_review_report(&[], ".");
        assert!(report.contains("No issues found"));
    }

    #[tokio::test]
    async fn test_full_pipeline() {
        let runner = Arc::new(MockCommandRunner::new(vec![
            // clippy output
            CommandOutput {
                stdout: String::new(),
                stderr: "warning: unused variable `x`\nwarning: 1 warning emitted\n".into(),
                exit_code: Some(0),
                success: true,
            },
            // fmt output
            CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
            // grep output
            CommandOutput {
                stdout: "src/main.rs:5:    // TODO: fix this\n".into(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
        ]));

        let agent = ReviewAgent::with_default_config().with_command_runner(runner);
        let task = Task::new("Review the project codebase");

        let result = agent.process_task(&task).await.unwrap();
        assert!(result.content.contains("Code Review Report"));
        assert!(result.content.contains("unused variable"));
        assert!(result.content.contains("TODO: fix this"));
        assert!(!result.content.contains("[TODO:"));
    }
}
