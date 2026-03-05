//! Integration tests for the coordinator's triage engine.
//!
//! These tests use keyword-based fallback (no LLM) so they work without Ollama.

use panoptes_coordinator::triage::TriageEngine;
use panoptes_llm::{LlmClient, LlmRequest, LlmResponse};
use std::sync::Arc;

/// Mock LLM client that returns a triage failure (to exercise keyword fallback).
struct MockLlm;

#[async_trait::async_trait]
impl LlmClient for MockLlm {
    async fn complete(&self, _request: LlmRequest) -> panoptes_common::Result<LlmResponse> {
        Err(panoptes_common::PanoptesError::Agent(
            "Mock: no LLM available".into(),
        ))
    }
    fn model_name(&self) -> &str {
        "mock"
    }
}

fn create_triage_engine() -> TriageEngine {
    TriageEngine::new(Arc::new(MockLlm))
}

// ============================================================================
// Keyword triage routing tests
// ============================================================================

#[test]
fn test_keyword_triage_research() {
    let engine = create_triage_engine();
    let decision = engine
        .keyword_triage("Research the best practices for Rust error handling")
        .unwrap();
    assert_eq!(decision.agent_name, "panoptes-research");
    assert!(decision.confidence > 0.5);
}

#[test]
fn test_keyword_triage_writing() {
    let engine = create_triage_engine();
    let decision = engine
        .keyword_triage("Write documentation for the auth module")
        .unwrap();
    assert_eq!(decision.agent_name, "panoptes-writing");
}

#[test]
fn test_keyword_triage_planning() {
    let engine = create_triage_engine();
    let decision = engine.keyword_triage("Plan my tasks for today").unwrap();
    assert_eq!(decision.agent_name, "panoptes-planning");
}

#[test]
fn test_keyword_triage_review() {
    let engine = create_triage_engine();
    let decision = engine
        .keyword_triage("Review the pull request for issues")
        .unwrap();
    assert_eq!(decision.agent_name, "panoptes-review");
}

#[test]
fn test_keyword_triage_testing() {
    let engine = create_triage_engine();
    let decision = engine
        .keyword_triage("Run tests and check coverage for the project")
        .unwrap();
    assert_eq!(decision.agent_name, "panoptes-testing");
}

#[test]
fn test_keyword_triage_coding() {
    let engine = create_triage_engine();
    let decision = engine.keyword_triage("Fix the bug in parser.rs").unwrap();
    assert_eq!(decision.agent_name, "panoptes-coding");
}

#[test]
fn test_keyword_triage_direct() {
    let engine = create_triage_engine();
    let decision = engine.keyword_triage("Hello there").unwrap();
    assert_eq!(decision.agent_name, "direct");
}

// ============================================================================
// Compound pattern regression tests
// ============================================================================

#[test]
fn test_keyword_triage_code_review_routes_to_review() {
    let engine = create_triage_engine();
    let decision = engine
        .keyword_triage("Please code review the auth module")
        .unwrap();
    assert_eq!(
        decision.agent_name, "panoptes-review",
        "'code review' must route to review, not coding"
    );
}

#[test]
fn test_keyword_triage_review_code_routes_to_review() {
    let engine = create_triage_engine();
    let decision = engine
        .keyword_triage("Review my code for security issues")
        .unwrap();
    assert_eq!(
        decision.agent_name, "panoptes-review",
        "'review my code' must route to review, not coding"
    );
}

#[test]
fn test_keyword_triage_write_test_routes_to_testing() {
    let engine = create_triage_engine();
    let decision = engine
        .keyword_triage("Write a test for the parser module")
        .unwrap();
    assert_eq!(
        decision.agent_name, "panoptes-testing",
        "'write a test' must route to testing, not writing"
    );
}

#[test]
fn test_keyword_triage_write_tests_routes_to_testing() {
    let engine = create_triage_engine();
    let decision = engine
        .keyword_triage("Write tests for the API endpoints")
        .unwrap();
    assert_eq!(
        decision.agent_name, "panoptes-testing",
        "'write tests' must route to testing, not writing"
    );
}

// ============================================================================
// LLM triage fallback test
// ============================================================================

#[tokio::test]
async fn test_triage_falls_back_to_keyword_on_llm_failure() {
    let engine = create_triage_engine();
    // LLM will fail (MockLlm returns error), so triage should fail
    let result = engine.triage("Research Rust async").await;
    // The triage method itself returns an error when LLM fails;
    // the orchestrator handles the fallback to keyword triage
    assert!(result.is_err());
}
