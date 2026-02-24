//! Planning agent - algorithmic task decomposition with Eisenhower-matrix prioritization.
//!
//! This agent handles planning tasks by:
//! 1. Parsing task descriptions into individual items
//! 2. Classifying priority using the Eisenhower matrix
//! 3. Building a structured plan grouped by priority quadrant
//! 4. Storing plans in memory for future reference

use crate::traits::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use panoptes_common::{AgentMessage, PanoptesError, Result, Task};
use panoptes_memory::{Memory, MemoryStore, MemoryType};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{info, warn};

const PLANNING_SYSTEM_PROMPT: &str = r#"You are a productivity and planning assistant. Your role is to:

1. Help break down large goals into actionable tasks
2. Create realistic schedules and timelines
3. Identify dependencies and blockers
4. Prioritize tasks based on importance and urgency
5. Track progress and suggest adjustments

Use the Eisenhower matrix for prioritization.
Consider energy levels and context switching costs.
Build in buffer time for unexpected issues.
"#;

/// Eisenhower matrix priority levels.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    UrgentImportant,
    NotUrgentImportant,
    UrgentNotImportant,
    Neither,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Priority::UrgentImportant => write!(f, "Urgent & Important (Do First)"),
            Priority::NotUrgentImportant => write!(f, "Important, Not Urgent (Schedule)"),
            Priority::UrgentNotImportant => write!(f, "Urgent, Not Important (Delegate)"),
            Priority::Neither => write!(f, "Neither (Consider Dropping)"),
        }
    }
}

/// A single item in a plan.
#[derive(Debug, Clone)]
pub struct PlanItem {
    pub title: String,
    pub priority: Priority,
    pub estimated_minutes: u32,
}

/// Configuration for the planning agent.
#[derive(Debug, Clone)]
pub struct PlanningConfig {
    /// Default estimated time per task in minutes
    pub default_task_duration: u32,
    /// Available work hours per day
    pub work_hours_per_day: f32,
    /// Whether to persist plans to memory
    pub persist_plans: bool,
}

impl Default for PlanningConfig {
    fn default() -> Self {
        Self {
            default_task_duration: 30,
            work_hours_per_day: 8.0,
            persist_plans: true,
        }
    }
}

/// Planning agent for task management and scheduling.
pub struct PlanningAgent {
    config: AgentConfig,
    planning_config: PlanningConfig,
    busy: AtomicBool,
    memory_store: Option<Arc<MemoryStore>>,
}

impl PlanningAgent {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            planning_config: PlanningConfig::default(),
            busy: AtomicBool::new(false),
            memory_store: None,
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(AgentConfig {
            id: "planning".into(),
            name: "Planning Agent".into(),
            ..Default::default()
        })
    }

    /// Attach a memory store for plan persistence.
    pub fn with_memory(mut self, store: Arc<MemoryStore>) -> Self {
        self.memory_store = Some(store);
        self
    }

    /// Parse a description into individual plan items.
    fn parse_tasks(description: &str, default_duration: u32) -> Vec<PlanItem> {
        let mut items = Vec::new();

        let lines: Vec<&str> = description
            .lines()
            .flat_map(|line| line.split(';'))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        for line in lines {
            let cleaned = line
                .trim_start_matches(['-', '*', '\u{2022}'])
                .trim_start_matches(|c: char| c.is_ascii_digit())
                .trim_start_matches('.')
                .trim_start_matches(')')
                .trim();

            if cleaned.is_empty() || cleaned.len() < 3 {
                continue;
            }

            let priority = Self::classify_priority(cleaned);
            let estimated_minutes = Self::estimate_duration(cleaned, default_duration);

            items.push(PlanItem {
                title: cleaned.to_string(),
                priority,
                estimated_minutes,
            });
        }

        if items.is_empty() && !description.trim().is_empty() {
            items.push(PlanItem {
                title: description.trim().to_string(),
                priority: Self::classify_priority(description),
                estimated_minutes: default_duration,
            });
        }

        items
    }

    /// Classify priority based on urgency/importance keywords.
    fn classify_priority(text: &str) -> Priority {
        let lower = text.to_lowercase();

        let is_urgent = lower.contains("urgent")
            || lower.contains("asap")
            || lower.contains("immediately")
            || lower.contains("deadline")
            || lower.contains("today")
            || lower.contains("now")
            || lower.contains("emergency");

        let is_important = lower.contains("important")
            || lower.contains("critical")
            || lower.contains("must")
            || lower.contains("essential")
            || lower.contains("key")
            || lower.contains("blocker")
            || lower.contains("required");

        match (is_urgent, is_important) {
            (true, true) => Priority::UrgentImportant,
            (false, true) => Priority::NotUrgentImportant,
            (true, false) => Priority::UrgentNotImportant,
            (false, false) => Priority::NotUrgentImportant,
        }
    }

    /// Estimate duration based on task complexity keywords.
    fn estimate_duration(text: &str, default: u32) -> u32 {
        let lower = text.to_lowercase();

        if lower.contains("quick") || lower.contains("simple") || lower.contains("minor") {
            15
        } else if lower.contains("complex")
            || lower.contains("refactor")
            || lower.contains("redesign")
        {
            120
        } else if lower.contains("meeting") || lower.contains("review") {
            60
        } else {
            default
        }
    }

    /// Build a formatted plan grouped by Eisenhower quadrant.
    fn build_plan(items: &[PlanItem], context: &str) -> String {
        let mut plan = String::new();

        plan.push_str("# Plan\n\n");

        if !context.is_empty() {
            plan.push_str(&format!("**Context:** {}\n\n", context));
        }

        let quadrants = [
            Priority::UrgentImportant,
            Priority::NotUrgentImportant,
            Priority::UrgentNotImportant,
            Priority::Neither,
        ];

        let mut total_minutes: u32 = 0;

        for quadrant in &quadrants {
            let quadrant_items: Vec<&PlanItem> =
                items.iter().filter(|i| i.priority == *quadrant).collect();

            if quadrant_items.is_empty() {
                continue;
            }

            plan.push_str(&format!("## {}\n\n", quadrant));

            for (i, item) in quadrant_items.iter().enumerate() {
                let hours = item.estimated_minutes / 60;
                let mins = item.estimated_minutes % 60;
                let time_str = if hours > 0 {
                    format!("~{}h{}m", hours, mins)
                } else {
                    format!("~{}m", mins)
                };

                plan.push_str(&format!("{}. [ ] {} ({})\n", i + 1, item.title, time_str));

                total_minutes += item.estimated_minutes;
            }

            plan.push('\n');
        }

        let total_hours = total_minutes as f32 / 60.0;
        plan.push_str("---\n\n");
        plan.push_str(&format!(
            "**Total items:** {} | **Estimated time:** {:.1}h\n",
            items.len(),
            total_hours
        ));

        plan
    }

    /// Search memory for existing plans.
    async fn search_existing_plans(&self, description: &str) -> String {
        if let Some(ref store) = self.memory_store {
            match store
                .search(description, Some(MemoryType::Procedural), Some(3))
                .await
            {
                Ok(memories) if !memories.is_empty() => {
                    let mut context = String::from("### Existing Plans (from Memory)\n\n");
                    for mem in &memories {
                        context.push_str(&format!("- {}\n", mem.content));
                    }
                    context.push('\n');
                    context
                }
                _ => String::new(),
            }
        } else {
            String::new()
        }
    }

    /// Store a plan in memory.
    async fn store_plan(&self, plan: &str) {
        if !self.planning_config.persist_plans {
            return;
        }

        if let Some(ref store) = self.memory_store {
            let summary = if plan.len() > 300 {
                format!("{}...", &plan[..300])
            } else {
                plan.to_string()
            };

            let memory = Memory::semantic(&summary, "planning-agent")
                .with_agent("planning")
                .with_importance(0.7)
                .with_tags(vec!["plan".into(), "task-breakdown".into()]);

            if let Err(e) = store.add(memory, true).await {
                warn!(error = %e, "Failed to store plan in memory");
            }
        }
    }

    /// Execute the planning pipeline.
    async fn execute_planning(&self, task: &Task) -> Result<AgentMessage> {
        let description = &task.description;

        info!(agent = %self.id(), task_id = %task.id, "Starting planning task");

        let existing_context = self.search_existing_plans(description).await;

        let items = Self::parse_tasks(description, self.planning_config.default_task_duration);
        info!(agent = %self.id(), items = items.len(), "Parsed plan items");

        let context = task.context.as_deref().unwrap_or("");
        let mut plan = Self::build_plan(&items, context);

        if !existing_context.is_empty() {
            plan = format!("{}\n{}", existing_context, plan);
        }

        self.store_plan(&plan).await;

        info!(
            agent = %self.id(),
            task_id = %task.id,
            items = items.len(),
            "Planning task completed"
        );

        let mut message = AgentMessage::from_agent(self.id(), plan);
        message.task_id = Some(task.id.clone());
        Ok(message)
    }
}

#[async_trait]
impl Agent for PlanningAgent {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn capabilities(&self) -> &[AgentCapability] {
        &[AgentCapability::TaskPlanning, AgentCapability::MemoryAccess]
    }

    async fn process_task(&self, task: &Task) -> Result<AgentMessage> {
        info!(
            agent = %self.id(),
            task_id = %task.id,
            "Processing planning task"
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

        let result = self.execute_planning(task).await;
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
            .unwrap_or(PLANNING_SYSTEM_PROMPT)
    }

    fn is_available(&self) -> bool {
        !self.busy.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_planning_agent_creation() {
        let agent = PlanningAgent::with_default_config();
        assert_eq!(agent.id(), "planning");
        assert_eq!(agent.name(), "Planning Agent");
        assert!(agent.is_available());
    }

    #[test]
    fn test_parse_tasks_bullet_list() {
        let desc = "- Fix the login bug\n- Update the API docs\n- Review PR #42";
        let items = PlanningAgent::parse_tasks(desc, 30);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].title, "Fix the login bug");
        assert_eq!(items[1].title, "Update the API docs");
        assert_eq!(items[2].title, "Review PR #42");
    }

    #[test]
    fn test_parse_tasks_numbered_list() {
        let desc = "1. First task\n2. Second task\n3. Third task";
        let items = PlanningAgent::parse_tasks(desc, 30);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].title, "First task");
    }

    #[test]
    fn test_parse_tasks_semicolons() {
        let desc = "Fix bugs; Update docs; Deploy to staging";
        let items = PlanningAgent::parse_tasks(desc, 30);
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn test_parse_tasks_single_item() {
        let desc = "Plan the architecture for the new microservice";
        let items = PlanningAgent::parse_tasks(desc, 30);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, desc);
    }

    #[test]
    fn test_classify_priority_urgent_important() {
        assert_eq!(
            PlanningAgent::classify_priority("This is urgent and critical to fix today"),
            Priority::UrgentImportant
        );
    }

    #[test]
    fn test_classify_priority_important_not_urgent() {
        assert_eq!(
            PlanningAgent::classify_priority("Important refactoring of the auth module"),
            Priority::NotUrgentImportant
        );
    }

    #[test]
    fn test_classify_priority_urgent_not_important() {
        assert_eq!(
            PlanningAgent::classify_priority("Need to reply to this email ASAP"),
            Priority::UrgentNotImportant
        );
    }

    #[test]
    fn test_classify_priority_default() {
        assert_eq!(
            PlanningAgent::classify_priority("Refactor the helper functions"),
            Priority::NotUrgentImportant
        );
    }

    #[test]
    fn test_estimate_duration() {
        assert_eq!(PlanningAgent::estimate_duration("quick fix", 30), 15);
        assert_eq!(
            PlanningAgent::estimate_duration("complex refactoring", 30),
            120
        );
        assert_eq!(PlanningAgent::estimate_duration("team meeting", 30), 60);
        assert_eq!(PlanningAgent::estimate_duration("normal task", 30), 30);
    }

    #[test]
    fn test_build_plan_grouped() {
        let items = vec![
            PlanItem {
                title: "Fix critical bug".into(),
                priority: Priority::UrgentImportant,
                estimated_minutes: 60,
            },
            PlanItem {
                title: "Write docs".into(),
                priority: Priority::NotUrgentImportant,
                estimated_minutes: 30,
            },
        ];

        let plan = PlanningAgent::build_plan(&items, "Sprint planning");
        assert!(plan.contains("Urgent & Important"));
        assert!(plan.contains("Fix critical bug"));
        assert!(plan.contains("Important, Not Urgent"));
        assert!(plan.contains("Write docs"));
        assert!(plan.contains("Sprint planning"));
        assert!(plan.contains("**Total items:** 2"));
    }

    #[tokio::test]
    async fn test_full_pipeline() {
        let agent = PlanningAgent::with_default_config();
        let task = Task::new(
            "- Fix the urgent login bug\n- Update important API docs\n- Quick review of PR",
        );

        let result = agent.process_task(&task).await.unwrap();
        assert!(result.content.contains("Plan"));
        assert!(result.content.contains("Fix the urgent login bug"));
        assert!(!result.content.contains("[TODO:"));
    }
}
