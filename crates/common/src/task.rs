//! Task management types for agent coordination.

use serde::{Deserialize, Serialize};

/// Priority level for tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    Low,
    Normal,
    High,
    Critical,
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::Normal
    }
}

/// Current status of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Assigned,
    InProgress,
    AwaitingInput,
    Completed,
    Failed,
    Cancelled,
}

/// A task that can be assigned to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique task ID
    pub id: String,

    /// Human-readable task description
    pub description: String,

    /// Task priority
    pub priority: TaskPriority,

    /// Current status
    pub status: TaskStatus,

    /// Assigned agent (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned_agent: Option<String>,

    /// Parent task ID (for subtasks)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task: Option<String>,

    /// Working directory context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,

    /// Task-specific context/instructions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,

    /// Creation timestamp
    pub created_at: u64,

    /// Last update timestamp
    pub updated_at: u64,
}

impl Task {
    pub fn new(description: impl Into<String>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        Self {
            id: format!("task_{:016x}", now),
            description: description.into(),
            priority: TaskPriority::Normal,
            status: TaskStatus::Pending,
            assigned_agent: None,
            parent_task: None,
            working_dir: None,
            context: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    pub fn with_working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }
}
