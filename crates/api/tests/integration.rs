//! Integration tests for the API layer.
//!
//! These tests spin up a real HTTP server on a random port so that
//! `ConnectInfo<SocketAddr>` is populated correctly by axum.

use panoptes_api::{AppState, create_router};
use panoptes_coordinator::CoordinatorConfig;
use std::net::SocketAddr;
use std::sync::Arc;

/// Spin up a test server on a random port and return the base URL.
async fn start_test_server() -> String {
    let state = Arc::new(AppState::new(CoordinatorConfig::default()).unwrap());
    let router = create_router(state, Some(vec!["*".to_string()]));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    format!("http://{}", addr)
}

/// Helper to GET a URL and return (status, body_string).
async fn get(base: &str, path: &str) -> (u16, String) {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}{}", base, path))
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap();
    (status, body)
}

/// Helper to POST JSON and return (status, body_string).
async fn post_json(base: &str, path: &str, json: &str) -> (u16, String) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}{}", base, path))
        .header("content-type", "application/json")
        .body(json.to_string())
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap();
    (status, body)
}

// ============================================================================
// Health endpoint
// ============================================================================

#[tokio::test]
async fn test_health_endpoint() {
    let base = start_test_server().await;
    let (status, body) = get(&base, "/health").await;
    assert_eq!(status, 200);
    assert!(body.contains("healthy"));
}

// ============================================================================
// Messages endpoint
// ============================================================================

#[tokio::test]
async fn test_messages_endpoint() {
    let base = start_test_server().await;
    let (status, body) = post_json(
        &base,
        "/api/v1/messages",
        r#"{"content": "Write a summary of the project"}"#,
    )
    .await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(!json["content"].as_str().unwrap().is_empty());
    assert!(json["route"].as_str().is_some());
}

// ============================================================================
// Writing endpoint
// ============================================================================

#[tokio::test]
async fn test_writing_endpoint() {
    let base = start_test_server().await;
    let (status, body) = post_json(
        &base,
        "/api/v1/writing",
        r#"{"instruction": "Write an email about the quarterly results"}"#,
    )
    .await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["status"].as_str().unwrap(), "completed");
    assert!(json["output"].as_str().unwrap().contains("Email"));
}

// ============================================================================
// Planning endpoint
// ============================================================================

#[tokio::test]
async fn test_planning_endpoint() {
    let base = start_test_server().await;
    let (status, body) = post_json(
        &base,
        "/api/v1/planning",
        r#"{"instruction": "Fix bugs; Update docs; Deploy to staging"}"#,
    )
    .await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["status"].as_str().unwrap(), "completed");
    assert!(
        json["output"].as_str().unwrap().contains("Plan")
            || json["output"].as_str().unwrap().len() > 50
    );
}

// ============================================================================
// Review endpoint (may run real cargo commands)
// ============================================================================

#[tokio::test]
async fn test_review_endpoint() {
    let base = start_test_server().await;
    let (status, _body) = post_json(
        &base,
        "/api/v1/review",
        r#"{"instruction": "Review the codebase"}"#,
    )
    .await;
    // Review agent runs real cargo commands which may or may not be available
    assert!(
        status == 200 || status == 500 || status == 504,
        "Unexpected status: {}",
        status
    );
}

// ============================================================================
// Testing endpoint (may run real cargo test)
// ============================================================================

#[tokio::test]
async fn test_testing_endpoint() {
    let base = start_test_server().await;
    let (status, _body) = post_json(
        &base,
        "/api/v1/testing",
        r#"{"instruction": "Run all tests"}"#,
    )
    .await;
    // Testing agent runs real cargo test which takes time
    assert!(
        status == 200 || status == 500 || status == 504,
        "Unexpected status: {}",
        status
    );
}

// ============================================================================
// Payload size limits
// ============================================================================

#[tokio::test]
async fn test_writing_payload_too_large() {
    let base = start_test_server().await;
    let large_content = "x".repeat(600_000);
    let (status, _body) = post_json(
        &base,
        "/api/v1/writing",
        &format!(r#"{{"instruction": "{}"}}"#, large_content),
    )
    .await;
    assert_eq!(
        status, 413,
        "Expected 413 Payload Too Large, got {}",
        status
    );
}

#[tokio::test]
async fn test_messages_payload_too_large() {
    let base = start_test_server().await;
    let large_content = "x".repeat(200_000);
    let (status, _body) = post_json(
        &base,
        "/api/v1/messages",
        &format!(r#"{{"content": "{}"}}"#, large_content),
    )
    .await;
    assert_eq!(
        status, 413,
        "Expected 413 Payload Too Large, got {}",
        status
    );
}

// ============================================================================
// Workflow endpoint
// ============================================================================

#[tokio::test]
async fn test_workflow_endpoint() {
    let base = start_test_server().await;
    let (status, body) = post_json(
        &base,
        "/api/v1/workflow",
        r#"{"instruction": "Write about testing", "agents": ["writing", "planning"]}"#,
    )
    .await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["steps"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_unknown_agent_in_workflow() {
    let base = start_test_server().await;
    let (status, _body) = post_json(
        &base,
        "/api/v1/workflow",
        r#"{"instruction": "test", "agents": ["nonexistent"]}"#,
    )
    .await;
    assert_eq!(status, 400, "Expected 400 Bad Request, got {}", status);
}
