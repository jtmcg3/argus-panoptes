//! Task management types for agent coordination.

use serde::{Deserialize, Serialize};

/// Priority level for tasks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    Low,
    #[default]
    Normal,
    High,
    Critical,
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
            id: format!("task_{}", uuid::Uuid::new_v4()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_creation() {
        let task = Task::new("Test task");

        assert!(task.id.starts_with("task_"));
        assert_eq!(task.description, "Test task");
        assert_eq!(task.priority, TaskPriority::Normal);
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.assigned_agent.is_none());
        assert!(task.parent_task.is_none());
        assert!(task.working_dir.is_none());
        assert!(task.context.is_none());
        assert!(task.created_at > 0);
    }

    #[test]
    fn test_task_builder_methods() {
        let task = Task::new("Build task")
            .with_priority(TaskPriority::Critical)
            .with_context("Important context")
            .with_working_dir("/home/user/project");

        assert_eq!(task.priority, TaskPriority::Critical);
        assert_eq!(task.context, Some("Important context".to_string()));
        assert_eq!(task.working_dir, Some("/home/user/project".to_string()));
    }

    #[test]
    fn test_task_priority_ordering() {
        assert!(TaskPriority::Critical > TaskPriority::High);
        assert!(TaskPriority::High > TaskPriority::Normal);
        assert!(TaskPriority::Normal > TaskPriority::Low);
    }

    #[test]
    fn test_task_priority_default() {
        assert_eq!(TaskPriority::default(), TaskPriority::Normal);
    }

    #[test]
    fn test_task_unique_ids() {
        let task1 = Task::new("Task 1");
        let task2 = Task::new("Task 2");

        assert_ne!(task1.id, task2.id);
    }

    #[test]
    fn test_task_serialization() {
        let task = Task::new("Serialization test")
            .with_priority(TaskPriority::High)
            .with_context("Some context");

        let json = serde_json::to_string(&task).unwrap();
        let deserialized: Task = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.description, task.description);
        assert_eq!(deserialized.priority, task.priority);
        assert_eq!(deserialized.context, task.context);
    }

    #[test]
    fn test_task_status_variants() {
        let statuses = vec![
            TaskStatus::Pending,
            TaskStatus::Assigned,
            TaskStatus::InProgress,
            TaskStatus::AwaitingInput,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ];

        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: TaskStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, status);
        }
    }
}
