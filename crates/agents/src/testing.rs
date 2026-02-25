//! Testing agent - test runner with output parsing and reporting.
//!
//! This agent handles testing tasks by:
//! 1. Constructing appropriate test commands based on the target
//! 2. Running tests via the command runner
//! 3. Parsing cargo test output into structured results
//! 4. Generating a markdown report with pass/fail details

use crate::command::{CommandOutput, CommandRunner, SystemCommandRunner};
use crate::traits::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use panoptes_common::{AgentMessage, PanoptesError, Result, Task};
use panoptes_llm::{ChatMessage, LlmClient, LlmRequest, Role};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{info, warn};

const TESTING_SYSTEM_PROMPT: &str = r#"You are a QA and testing specialist. Your role is to:

1. Run existing test suites and report results
2. Identify untested code paths
3. Generate new test cases for edge cases
4. Analyze test coverage reports
5. Suggest testing improvements

Prioritize tests that catch real bugs.
Focus on edge cases and error handling.
Ensure tests are deterministic and fast.
"#;

/// Status of an individual test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestStatus {
    Passed,
    Failed,
    Ignored,
}

impl std::fmt::Display for TestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TestStatus::Passed => write!(f, "PASS"),
            TestStatus::Failed => write!(f, "FAIL"),
            TestStatus::Ignored => write!(f, "SKIP"),
        }
    }
}

/// Result of a single test case.
#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub status: TestStatus,
    pub duration_ms: Option<u64>,
    pub failure_message: Option<String>,
}

/// Summary of a test run.
#[derive(Debug, Clone)]
pub struct TestSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub ignored: usize,
    pub duration_secs: Option<f64>,
    pub results: Vec<TestResult>,
}

/// Type of tests to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestType {
    Unit,
    Integration,
    All,
    Specific(String),
}

/// Configuration for the testing agent.
#[derive(Debug, Clone)]
pub struct TestingConfig {
    /// Base test command
    pub test_command: String,
    /// Default timeout in seconds
    pub timeout_secs: u64,
}

impl Default for TestingConfig {
    fn default() -> Self {
        Self {
            test_command: "cargo".into(),
            timeout_secs: 300,
        }
    }
}

/// Testing agent for test execution and coverage.
pub struct TestingAgent {
    config: AgentConfig,
    testing_config: TestingConfig,
    busy: AtomicBool,
    command_runner: Arc<dyn CommandRunner>,
    /// Optional LLM client for enhanced output
    llm_client: Option<Arc<dyn LlmClient>>,
}

impl TestingAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            testing_config: TestingConfig::default(),
            busy: AtomicBool::new(false),
            command_runner: Arc::new(SystemCommandRunner),
            llm_client: None,
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(AgentConfig {
            id: "testing".into(),
            name: "Testing Agent".into(),
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
                        "Task: {}\n\nAnalyze these test results and provide insights on failures and improvements:\n{}",
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

    /// Build command arguments for the given test type.
    fn build_test_args(test_type: &TestType, target: &str) -> Vec<String> {
        let mut args = vec!["test".to_string()];

        match test_type {
            TestType::Unit => {
                if target != "." && !target.is_empty() {
                    args.push("-p".into());
                    args.push(target.into());
                }
                args.push("--lib".into());
            }
            TestType::Integration => {
                if target != "." && !target.is_empty() {
                    args.push("-p".into());
                    args.push(target.into());
                }
                args.push("--test".into());
                args.push("*".into());
            }
            TestType::All => {
                if target != "." && !target.is_empty() {
                    args.push("-p".into());
                    args.push(target.into());
                } else {
                    args.push("--workspace".into());
                }
            }
            TestType::Specific(name) => {
                args.push(name.clone());
            }
        }

        args
    }

    /// Parse cargo test output into a test summary.
    fn parse_cargo_test_output(output: &CommandOutput) -> TestSummary {
        let mut results = Vec::new();
        let mut total = 0;
        let mut passed = 0;
        let mut failed = 0;
        let mut ignored = 0;
        let mut duration_secs = None;

        let text = format!("{}\n{}", output.stdout, output.stderr);

        for line in text.lines() {
            let trimmed = line.trim();

            // Match individual test results: "test module::test_name ... ok"
            if trimmed.starts_with("test ")
                && (trimmed.ends_with(" ok")
                    || trimmed.ends_with(" FAILED")
                    || trimmed.ends_with(" ignored"))
            {
                let rest = &trimmed[5..]; // skip "test "

                let (name, status) = if let Some(name) = rest.strip_suffix(" ... ok") {
                    (name.to_string(), TestStatus::Passed)
                } else if let Some(name) = rest.strip_suffix(" ... FAILED") {
                    (name.to_string(), TestStatus::Failed)
                } else if let Some(name) = rest.strip_suffix(" ... ignored") {
                    (name.to_string(), TestStatus::Ignored)
                } else {
                    continue;
                };

                results.push(TestResult {
                    name,
                    status: status.clone(),
                    duration_ms: None,
                    failure_message: None,
                });

                match status {
                    TestStatus::Passed => passed += 1,
                    TestStatus::Failed => failed += 1,
                    TestStatus::Ignored => ignored += 1,
                }
            }

            // Match summary line: "test result: ok. 10 passed; 0 failed; 2 ignored; ..."
            if trimmed.starts_with("test result:")
                && let Some(summary_info) = Self::parse_summary_line(trimmed)
            {
                total = summary_info.0;
                passed = summary_info.1;
                failed = summary_info.2;
                ignored = summary_info.3;
                duration_secs = summary_info.4;
            }
        }

        if total == 0 {
            total = passed + failed + ignored;
        }

        TestSummary {
            total,
            passed,
            failed,
            ignored,
            duration_secs,
            results,
        }
    }

    /// Parse the "test result:" summary line.
    fn parse_summary_line(line: &str) -> Option<(usize, usize, usize, usize, Option<f64>)> {
        // "test result: ok. 10 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 1.23s"
        let mut passed = 0;
        let mut failed = 0;
        let mut ignored = 0;
        let mut duration = None;

        // Helper: extract the number right before a keyword like "passed", "failed", "ignored"
        fn extract_count(part: &str) -> Option<usize> {
            let tokens: Vec<&str> = part.split_whitespace().collect();
            if tokens.len() >= 2 {
                tokens[tokens.len() - 2].parse().ok()
            } else {
                None
            }
        }

        for part in line.split(';') {
            let part = part.trim();
            if part.ends_with("passed") {
                if let Some(n) = extract_count(part) {
                    passed = n;
                }
            } else if part.ends_with("failed") {
                if let Some(n) = extract_count(part) {
                    failed = n;
                }
            } else if part.ends_with("ignored") {
                if let Some(n) = extract_count(part) {
                    ignored = n;
                }
            } else if part.contains("finished in")
                && let Some(time_str) = part.strip_suffix('s')
                && let Some(time_part) = time_str.split("finished in ").last()
            {
                duration = time_part.trim().parse().ok();
            }
        }

        let total = passed + failed + ignored;
        Some((total, passed, failed, ignored, duration))
    }

    /// Build a markdown test report.
    fn build_test_report(summary: &TestSummary, target: &str) -> String {
        let mut report = String::new();

        report.push_str("# Test Report\n\n");
        report.push_str(&format!("**Target:** {}\n", target));

        if let Some(duration) = summary.duration_secs {
            report.push_str(&format!("**Duration:** {:.2}s\n", duration));
        }

        let status_emoji = if summary.failed == 0 { "PASS" } else { "FAIL" };
        report.push_str(&format!("**Result:** {}\n\n", status_emoji));

        report.push_str(&format!(
            "| Metric | Count |\n|--------|-------|\n\
             | Total  | {} |\n\
             | Passed | {} |\n\
             | Failed | {} |\n\
             | Ignored | {} |\n\n",
            summary.total, summary.passed, summary.failed, summary.ignored
        ));

        // Show failures in detail
        let failures: Vec<&TestResult> = summary
            .results
            .iter()
            .filter(|r| r.status == TestStatus::Failed)
            .collect();

        if !failures.is_empty() {
            report.push_str("## Failures\n\n");
            for (i, failure) in failures.iter().enumerate() {
                report.push_str(&format!("{}. `{}`\n", i + 1, failure.name));
                if let Some(ref msg) = failure.failure_message {
                    report.push_str(&format!("   ```\n   {}\n   ```\n", msg));
                }
            }
            report.push('\n');
        }

        // Show all results if not too many
        if !summary.results.is_empty() && summary.results.len() <= 50 {
            report.push_str("## All Tests\n\n");
            for result in &summary.results {
                let marker = match result.status {
                    TestStatus::Passed => "[x]",
                    TestStatus::Failed => "[ ]",
                    TestStatus::Ignored => "[-]",
                };
                report.push_str(&format!("- {} {}\n", marker, result.name));
            }
            report.push('\n');
        }

        report.push_str("---\n\n");
        report.push_str(&format!(
            "**Pass rate:** {:.1}%\n",
            if summary.total > 0 {
                (summary.passed as f64 / summary.total as f64) * 100.0
            } else {
                0.0
            }
        ));

        report
    }

    /// Execute the testing pipeline.
    async fn execute_testing(&self, task: &Task) -> Result<AgentMessage> {
        let description = &task.description;
        let target = task.context.as_deref().unwrap_or(".");

        info!(agent = %self.id(), task_id = %task.id, target = %target, "Starting test run");

        // Determine test type from description
        let test_type = Self::determine_test_type(description);
        info!(agent = %self.id(), test_type = ?test_type, "Determined test type");

        // Build args
        let args = Self::build_test_args(&test_type, target);
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        info!(agent = %self.id(), args = ?args, "Running tests");

        // Execute
        let output = self
            .command_runner
            .run(&self.testing_config.test_command, &args_refs, ".")
            .await;

        // Parse output
        let summary = Self::parse_cargo_test_output(&output);
        info!(
            agent = %self.id(),
            total = summary.total,
            passed = summary.passed,
            failed = summary.failed,
            "Test run complete"
        );

        let report = Self::build_test_report(&summary, target);

        // Enhance with LLM if available
        let report = self.enhance_with_llm(&report, task).await;

        let mut message = AgentMessage::from_agent(self.id(), report);
        message.task_id = Some(task.id.clone());
        Ok(message)
    }

    /// Determine test type from task description keywords.
    fn determine_test_type(description: &str) -> TestType {
        let lower = description.to_lowercase();

        if lower.contains("integration") {
            TestType::Integration
        } else if lower.contains("unit") {
            TestType::Unit
        } else {
            TestType::All
        }
    }
}

#[async_trait]
impl Agent for TestingAgent {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn capabilities(&self) -> &[AgentCapability] {
        &[
            AgentCapability::TestExecution,
            AgentCapability::CodeExecution,
            AgentCapability::MemoryAccess,
        ]
    }

    async fn process_task(&self, task: &Task) -> Result<AgentMessage> {
        info!(
            agent = %self.id(),
            task_id = %task.id,
            "Processing testing task"
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

        let result = self.execute_testing(task).await;
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
            .unwrap_or(TESTING_SYSTEM_PROMPT)
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
    fn test_testing_agent_creation() {
        let agent = TestingAgent::with_default_config();
        assert_eq!(agent.id(), "testing");
        assert_eq!(agent.name(), "Testing Agent");
        assert!(agent.is_available());
    }

    #[test]
    fn test_build_test_args_unit() {
        let args = TestingAgent::build_test_args(&TestType::Unit, "my-crate");
        assert!(args.contains(&"test".to_string()));
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"my-crate".to_string()));
        assert!(args.contains(&"--lib".to_string()));
    }

    #[test]
    fn test_build_test_args_all_workspace() {
        let args = TestingAgent::build_test_args(&TestType::All, ".");
        assert!(args.contains(&"test".to_string()));
        assert!(args.contains(&"--workspace".to_string()));
    }

    #[test]
    fn test_build_test_args_specific() {
        let args = TestingAgent::build_test_args(&TestType::Specific("test_foo".into()), ".");
        assert!(args.contains(&"test".to_string()));
        assert!(args.contains(&"test_foo".to_string()));
    }

    #[test]
    fn test_parse_all_passing() {
        let output = CommandOutput {
            stdout: "\
running 3 tests
test tests::test_one ... ok
test tests::test_two ... ok
test tests::test_three ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
"
            .into(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
        };

        let summary = TestingAgent::parse_cargo_test_output(&output);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.passed, 3);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.ignored, 0);
        assert_eq!(summary.results.len(), 3);
        assert!(
            summary
                .results
                .iter()
                .all(|r| r.status == TestStatus::Passed)
        );
    }

    #[test]
    fn test_parse_with_failures() {
        let output = CommandOutput {
            stdout: "\
running 3 tests
test tests::test_one ... ok
test tests::test_two ... FAILED
test tests::test_three ... ignored

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.45s
"
            .into(),
            stderr: String::new(),
            exit_code: Some(101),
            success: false,
        };

        let summary = TestingAgent::parse_cargo_test_output(&output);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.ignored, 1);
        assert_eq!(summary.results.len(), 3);
        assert_eq!(summary.results[1].status, TestStatus::Failed);
        assert_eq!(summary.results[1].name, "tests::test_two");
    }

    #[test]
    fn test_parse_empty_output() {
        let output = CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
        };

        let summary = TestingAgent::parse_cargo_test_output(&output);
        assert_eq!(summary.total, 0);
        assert_eq!(summary.passed, 0);
        assert!(summary.results.is_empty());
    }

    #[test]
    fn test_build_report_passing() {
        let summary = TestSummary {
            total: 5,
            passed: 5,
            failed: 0,
            ignored: 0,
            duration_secs: Some(1.23),
            results: vec![TestResult {
                name: "test_one".into(),
                status: TestStatus::Passed,
                duration_ms: None,
                failure_message: None,
            }],
        };

        let report = TestingAgent::build_test_report(&summary, "my-crate");
        assert!(report.contains("Test Report"));
        assert!(report.contains("my-crate"));
        assert!(report.contains("1.23s"));
        assert!(report.contains("PASS"));
        assert!(report.contains("**Pass rate:** 100.0%"));
    }

    #[test]
    fn test_build_report_with_failures() {
        let summary = TestSummary {
            total: 3,
            passed: 2,
            failed: 1,
            ignored: 0,
            duration_secs: Some(2.50),
            results: vec![
                TestResult {
                    name: "test_one".into(),
                    status: TestStatus::Passed,
                    duration_ms: None,
                    failure_message: None,
                },
                TestResult {
                    name: "test_broken".into(),
                    status: TestStatus::Failed,
                    duration_ms: None,
                    failure_message: None,
                },
            ],
        };

        let report = TestingAgent::build_test_report(&summary, ".");
        assert!(report.contains("FAIL"));
        assert!(report.contains("Failures"));
        assert!(report.contains("test_broken"));
        assert!(report.contains("**Pass rate:** 66.7%"));
    }

    #[test]
    fn test_determine_test_type() {
        assert_eq!(
            TestingAgent::determine_test_type("run unit tests"),
            TestType::Unit
        );
        assert_eq!(
            TestingAgent::determine_test_type("run integration tests"),
            TestType::Integration
        );
        assert_eq!(
            TestingAgent::determine_test_type("run all tests"),
            TestType::All
        );
        assert_eq!(
            TestingAgent::determine_test_type("test the project"),
            TestType::All
        );
    }

    #[tokio::test]
    async fn test_full_pipeline() {
        let runner = Arc::new(MockCommandRunner::new(vec![CommandOutput {
            stdout: "\
running 2 tests
test tests::test_one ... ok
test tests::test_two ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
"
            .into(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
        }]));

        let agent = TestingAgent::with_default_config().with_command_runner(runner);
        let task = Task::new("Run all tests in the workspace");

        let result = agent.process_task(&task).await.unwrap();
        assert!(result.content.contains("Test Report"));
        assert!(result.content.contains("PASS"));
        assert!(result.content.contains("**Pass rate:** 100.0%"));
        assert!(!result.content.contains("[TODO:"));
    }
}
