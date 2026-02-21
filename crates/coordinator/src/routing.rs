//! Agent routing and decision types.

use panoptes_common::Task;
use serde::{Deserialize, Serialize};

/// Identifies which agent should handle a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentRoute {
    /// Route to the PTY/Claude CLI agent for coding tasks
    PtyCoding {
        instruction: String,
        working_dir: Option<String>,
        permission_mode: PermissionMode,
    },

    /// Route to a research agent
    Research {
        query: String,
        sources: Vec<String>,
    },

    /// Route to a writing agent
    Writing {
        task_type: WritingTask,
        context: String,
    },

    /// Route to day planning agent
    Planning {
        scope: PlanningScope,
        context: String,
    },

    /// Route to code review agent
    CodeReview {
        target: String,
        review_type: ReviewType,
    },

    /// Route to testing agent
    Testing {
        target: String,
        test_type: TestType,
    },

    /// Direct response (no agent needed)
    Direct {
        response: String,
    },

    /// Multi-agent workflow (decomposed task)
    Workflow {
        tasks: Vec<Task>,
    },
}

/// Permission mode for coding agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    /// Review actions before execution
    Plan,
    /// Execute immediately
    Act,
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::Plan
    }
}

/// Types of writing tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WritingTask {
    Documentation,
    Email,
    Report,
    Summary,
    Creative,
    Technical,
}

/// Scope for planning tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningScope {
    Day,
    Week,
    Project,
    Sprint,
}

/// Types of code review.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewType {
    Security,
    Performance,
    Style,
    Architecture,
    Full,
}

/// Types of testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestType {
    Unit,
    Integration,
    E2E,
    Coverage,
}

/// The result of triage analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    /// The chosen route
    pub route: AgentRoute,

    /// Reasoning for the decision
    pub reasoning: String,

    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,

    /// Extracted context from the request
    #[serde(default)]
    pub extracted_context: Option<String>,
}

impl RouteDecision {
    pub fn direct(response: impl Into<String>) -> Self {
        Self {
            route: AgentRoute::Direct {
                response: response.into(),
            },
            reasoning: "Direct response, no agent needed".into(),
            confidence: 1.0,
            extracted_context: None,
        }
    }

    pub fn coding(instruction: impl Into<String>) -> Self {
        Self {
            route: AgentRoute::PtyCoding {
                instruction: instruction.into(),
                working_dir: None,
                permission_mode: PermissionMode::Plan,
            },
            reasoning: "Routing to coding agent".into(),
            confidence: 0.8,
            extracted_context: None,
        }
    }
}
