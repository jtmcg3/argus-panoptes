//! HTTP route handlers for the API.

use crate::AppState;
use axum::{
    extract::{Path, State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use panoptes_common::AgentMessage;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info};

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_seconds: u64,
    pub zeroclaw_enabled: bool,
}

/// Health check endpoint.
pub async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let coordinator = state.coordinator.read().await;

    Json(HealthResponse {
        status: "healthy",
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: state.uptime_seconds(),
        zeroclaw_enabled: coordinator.has_zeroclaw_triage(),
    })
}

/// Message request body.
#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    pub content: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Message response body.
#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub id: String,
    pub content: String,
    pub route: String,
    pub confidence: f32,
}

/// API error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: &'static str,
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
    }
}

/// Send a message to the coordinator for processing.
pub async fn send_message(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MessageRequest>,
) -> Result<Json<MessageResponse>, ErrorResponse> {
    info!(
        content_preview = %request.content.chars().take(50).collect::<String>(),
        "Received message"
    );

    // Create agent message
    let mut message = AgentMessage::user(&request.content);
    if let Some(metadata) = request.metadata {
        message.metadata = metadata;
    }

    // Process through coordinator
    let coordinator = state.coordinator.read().await;
    let decision = coordinator.triage(&message).await
        .map_err(|e| {
            error!(error = %e, "Triage failed");
            ErrorResponse {
                error: format!("Triage failed: {}", e),
                code: "TRIAGE_ERROR",
            }
        })?;

    // For now, return the triage decision without executing
    // Full execution would require the coordinator to not be borrowed
    let route_str = format!("{:?}", decision.route);

    Ok(Json(MessageResponse {
        id: message.id,
        content: decision.reasoning,
        route: route_str,
        confidence: decision.confidence,
    }))
}

/// Session status response.
#[derive(Debug, Serialize)]
pub struct SessionResponse {
    pub session_id: String,
    pub status: String,
    pub output: Option<String>,
    pub awaiting_confirmation: bool,
}

/// Get PTY session status.
pub async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionResponse>, ErrorResponse> {
    debug!(session_id = %session_id, "Getting session status");

    let coordinator = state.coordinator.read().await;

    // Check if PTY client is connected
    if !coordinator.is_pty_connected().await {
        return Err(ErrorResponse {
            error: "PTY-MCP client not connected".into(),
            code: "PTY_NOT_CONNECTED",
        });
    }

    // Get PTY client and read session status
    // We need to get the status before the coordinator lock is released
    let status_result = {
        match coordinator.get_pty_client().await {
            Ok(client) => client.get_status(&session_id).await,
            Err(e) => return Err(ErrorResponse {
                error: format!("Failed to get PTY client: {}", e),
                code: "PTY_ERROR",
            }),
        }
    };

    match status_result {
        Ok(status) => Ok(Json(SessionResponse {
            session_id,
            status: status.status,
            output: None,  // Would need to call get_output for full output
            awaiting_confirmation: status.awaiting_confirmation,
        })),
        Err(e) => Err(ErrorResponse {
            error: format!("Failed to get session status: {}", e),
            code: "SESSION_ERROR",
        }),
    }
}

/// WebSocket handler for real-time streaming.
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(_state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(handle_websocket)
}

/// Handle WebSocket connection.
async fn handle_websocket(mut socket: axum::extract::ws::WebSocket) {
    use axum::extract::ws::Message;

    info!("WebSocket connection established");

    // Simple echo for now - can be extended to stream PTY output
    while let Some(msg) = socket.recv().await {
        match msg {
            Ok(Message::Text(text)) => {
                debug!(message = %text, "Received WebSocket message");
                // Echo back for now
                if socket.send(Message::Text(format!("Echo: {}", text).into())).await.is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) => {
                info!("WebSocket connection closed");
                break;
            }
            Err(e) => {
                error!(error = %e, "WebSocket error");
                break;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_response_serialization() {
        let response = HealthResponse {
            status: "healthy",
            version: "0.1.0",
            uptime_seconds: 100,
            zeroclaw_enabled: true,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("healthy"));
        assert!(json.contains("zeroclaw_enabled"));
    }

    #[test]
    fn test_message_request_deserialization() {
        let json = r#"{"content": "Hello world"}"#;
        let request: MessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.content, "Hello world");
        assert!(request.metadata.is_none());
    }

    #[test]
    fn test_message_request_with_metadata() {
        let json = r#"{"content": "Hello", "metadata": {"key": "value"}}"#;
        let request: MessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.content, "Hello");
        assert!(request.metadata.is_some());
    }
}
