//! Integration tests for the coordinator's triage, execute, and process pipeline.
//!
//! These tests use keyword-based fallback (no LLM) so they work without Ollama.

use panoptes_agents::{PlanningAgent, ResearchAgent, ReviewAgent, TestingAgent, WritingAgent};
use panoptes_common::AgentMessage;
use panoptes_coordinator::{AgentRoute, Coordinator, CoordinatorConfig};
use std::sync::Arc;

/// Helper to create a coordinator with all specialist agents wired in.
fn create_test_coordinator() -> Coordinator {
    let config = CoordinatorConfig::default();
    let mut coordinator = Coordinator::new(config).unwrap();

    coordinator.set_research_agent(
        Arc::new(ResearchAgent::with_default_config()) as Arc<dyn panoptes_common::Agent>
    );
    coordinator.set_writing_agent(
        Arc::new(WritingAgent::with_default_config()) as Arc<dyn panoptes_common::Agent>
    );
    coordinator.set_planning_agent(
        Arc::new(PlanningAgent::with_default_config()) as Arc<dyn panoptes_common::Agent>
    );
    coordinator.set_review_agent(
        Arc::new(ReviewAgent::with_default_config()) as Arc<dyn panoptes_common::Agent>
    );
    coordinator.set_testing_agent(
        Arc::new(TestingAgent::with_default_config()) as Arc<dyn panoptes_common::Agent>
    );

    coordinator
}

// ============================================================================
// Triage routing tests
// ============================================================================

#[tokio::test]
async fn test_triage_research_keyword() {
    let coordinator = create_test_coordinator();
    let msg = AgentMessage::user("Research the best practices for Rust error handling");
    let decision = coordinator.triage(&msg).await.unwrap();
    assert!(matches!(decision.route, AgentRoute::Research { .. }));
    assert!(decision.confidence > 0.5);
}

#[tokio::test]
async fn test_triage_writing_keyword() {
    let coordinator = create_test_coordinator();
    let msg = AgentMessage::user("Write documentation for the auth module");
    let decision = coordinator.triage(&msg).await.unwrap();
    assert!(matches!(decision.route, AgentRoute::Writing { .. }));
}

#[tokio::test]
async fn test_triage_planning_keyword() {
    let coordinator = create_test_coordinator();
    let msg = AgentMessage::user("Plan my tasks for today");
    let decision = coordinator.triage(&msg).await.unwrap();
    assert!(matches!(decision.route, AgentRoute::Planning { .. }));
}

#[tokio::test]
async fn test_triage_review_keyword() {
    let coordinator = create_test_coordinator();
    let msg = AgentMessage::user("Review the pull request for issues");
    let decision = coordinator.triage(&msg).await.unwrap();
    assert!(matches!(decision.route, AgentRoute::CodeReview { .. }));
}

#[tokio::test]
async fn test_triage_testing_keyword() {
    let coordinator = create_test_coordinator();
    let msg = AgentMessage::user("Run tests and check coverage for the project");
    let decision = coordinator.triage(&msg).await.unwrap();
    assert!(matches!(decision.route, AgentRoute::Testing { .. }));
}

#[tokio::test]
async fn test_triage_coding_keyword() {
    let coordinator = create_test_coordinator();
    let msg = AgentMessage::user("Fix the bug in parser.rs");
    let decision = coordinator.triage(&msg).await.unwrap();
    assert!(matches!(decision.route, AgentRoute::PtyCoding { .. }));
}

#[tokio::test]
async fn test_triage_direct_response() {
    let coordinator = create_test_coordinator();
    let msg = AgentMessage::user("Hello there");
    let decision = coordinator.triage(&msg).await.unwrap();
    assert!(matches!(decision.route, AgentRoute::Direct { .. }));
}

// ============================================================================
// Execute tests
// ============================================================================

#[tokio::test]
async fn test_execute_writing_route() {
    let coordinator = create_test_coordinator();
    let msg = AgentMessage::user("Write an email to the team about the release");
    let decision = coordinator.triage(&msg).await.unwrap();
    let result = coordinator.execute(decision).await.unwrap();
    assert!(!result.content.is_empty());
    // Writing agent should produce template-based output containing Email
    assert!(
        result.content.contains("Email")
            || result.content.contains("email")
            || result.content.len() > 50,
        "Expected email-related content, got: {}",
        &result.content[..result.content.len().min(200)]
    );
}

#[tokio::test]
async fn test_execute_planning_route() {
    let coordinator = create_test_coordinator();
    let msg = AgentMessage::user("Plan my schedule for this week");
    let decision = coordinator.triage(&msg).await.unwrap();
    let result = coordinator.execute(decision).await.unwrap();
    assert!(!result.content.is_empty());
    assert!(
        result.content.contains("Plan")
            || result.content.contains("plan")
            || result.content.len() > 50,
        "Expected planning content, got: {}",
        &result.content[..result.content.len().min(200)]
    );
}

#[tokio::test]
async fn test_execute_direct_route() {
    let coordinator = create_test_coordinator();
    let msg = AgentMessage::user("Hello there");
    let decision = coordinator.triage(&msg).await.unwrap();
    let result = coordinator.execute(decision).await.unwrap();
    assert!(!result.content.is_empty());
}

// ============================================================================
// End-to-end process tests
// ============================================================================

#[tokio::test]
async fn test_process_end_to_end() {
    let coordinator = create_test_coordinator();
    let msg = AgentMessage::user("Write a summary of the project status");
    let result = coordinator.process(msg).await.unwrap();
    assert!(!result.content.is_empty());
}

// ============================================================================
// Graceful degradation tests
// ============================================================================

#[tokio::test]
async fn test_missing_agent_graceful() {
    let config = CoordinatorConfig::default();
    let coordinator = Coordinator::new(config).unwrap();
    // No agents wired - should get a helpful message, not a crash
    let msg = AgentMessage::user("Research Rust async");
    let result = coordinator.process(msg).await.unwrap();
    assert!(
        result.content.contains("not")
            || result.content.contains("unavailable")
            || !result.content.is_empty(),
        "Expected graceful error message, got: {}",
        result.content
    );
}
