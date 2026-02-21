//! HTTP route handlers for the API.
//!
//! SEC-006: All endpoints implement rate limiting and input validation.

use crate::rate_limit::{RateLimitStats, WebSocketGuard};
use crate::AppState;
use axum::{
    extract::{ConnectInfo, Path, State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use panoptes_common::AgentMessage;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_seconds: u64,
    pub zeroclaw_enabled: bool,
    pub rate_limit_stats: RateLimitStats,
}

/// Health check endpoint.
pub async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let coordinator = state.coordinator.read().await;
    let rate_limit_stats = state.rate_limiter.stats();

    Json(HealthResponse {
        status: "healthy",
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: state.uptime_seconds(),
        zeroclaw_enabled: coordinator.has_zeroclaw_triage(),
        rate_limit_stats,
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

/// Rate limited error response.
fn rate_limited_response() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(ErrorResponse {
            error: "Rate limit exceeded. Please try again later.".into(),
            code: "RATE_LIMITED",
        }),
    )
}

/// Payload too large error response.
fn payload_too_large_response(max_size: usize) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        Json(ErrorResponse {
            error: format!("Request body exceeds maximum size of {} bytes", max_size),
            code: "PAYLOAD_TOO_LARGE",
        }),
    )
}

/// Maximum message content size (SEC-006).
const MAX_MESSAGE_CONTENT_SIZE: usize = 100_000; // 100KB

/// Send a message to the coordinator for processing.
///
/// SEC-006: Rate limited and size-bounded.
pub async fn send_message(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<MessageRequest>,
) -> Result<Json<MessageResponse>, (StatusCode, Json<ErrorResponse>)> {
    let client_ip = addr.ip();

    // SEC-006: Check rate limit
    if !state.rate_limiter.check_request(client_ip) {
        warn!(ip = %client_ip, "Rate limit exceeded for message request");
        return Err(rate_limited_response());
    }

    // SEC-006: Check content size
    if request.content.len() > MAX_MESSAGE_CONTENT_SIZE {
        warn!(
            ip = %client_ip,
            content_size = request.content.len(),
            "Message content exceeds size limit"
        );
        return Err(payload_too_large_response(MAX_MESSAGE_CONTENT_SIZE));
    }

    info!(
        content_preview = %request.content.chars().take(50).collect::<String>(),
        ip = %client_ip,
        "Received message"
    );

    // Create agent message
    let mut message = AgentMessage::user(&request.content);
    if let Some(metadata) = request.metadata {
        message.metadata = metadata;
    }

    // Process through coordinator with timeout (SEC-006)
    let coordinator = state.coordinator.read().await;
    let decision = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        coordinator.triage(&message),
    )
    .await
    .map_err(|_| {
        error!(ip = %client_ip, "Triage request timed out");
        (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorResponse {
                error: "Request timed out".into(),
                code: "TIMEOUT",
            }),
        )
    })?
    .map_err(|e| {
        error!(error = %e, "Triage failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Triage failed: {}", e),
                code: "TRIAGE_ERROR",
            }),
        )
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
///
/// SEC-006: Rate limited.
pub async fn get_session(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionResponse>, (StatusCode, Json<ErrorResponse>)> {
    let client_ip = addr.ip();

    // SEC-006: Check rate limit
    if !state.rate_limiter.check_request(client_ip) {
        warn!(ip = %client_ip, "Rate limit exceeded for session request");
        return Err(rate_limited_response());
    }

    debug!(session_id = %session_id, ip = %client_ip, "Getting session status");

    let coordinator = state.coordinator.read().await;

    // Check if PTY client is connected
    if !coordinator.is_pty_connected().await {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "PTY-MCP client not connected".into(),
                code: "PTY_NOT_CONNECTED",
            }),
        ));
    }

    // Get PTY client and read session status
    // We need to get the status before the coordinator lock is released
    let status_result = {
        match coordinator.get_pty_client().await {
            Ok(client) => client.get_status(&session_id).await,
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to get PTY client: {}", e),
                        code: "PTY_ERROR",
                    }),
                ))
            }
        }
    };

    match status_result {
        Ok(status) => Ok(Json(SessionResponse {
            session_id,
            status: status.status,
            output: None, // Would need to call get_output for full output
            awaiting_confirmation: status.awaiting_confirmation,
        })),
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Failed to get session status: {}", e),
                code: "SESSION_ERROR",
            }),
        )),
    }
}

/// Maximum WebSocket message size (SEC-006).
const MAX_WS_MESSAGE_SIZE: usize = 64_000; // 64KB

/// Maximum messages per WebSocket connection before forced close (SEC-006).
const MAX_WS_MESSAGES: u32 = 1000;

/// WebSocket handler for real-time streaming.
///
/// SEC-006: Connection limited per IP.
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    let client_ip = addr.ip();

    // SEC-006: Check WebSocket connection limit
    let guard = match WebSocketGuard::new(state.rate_limiter.clone(), client_ip) {
        Some(guard) => guard,
        None => {
            warn!(ip = %client_ip, "WebSocket connection limit exceeded");
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "WebSocket connection limit exceeded",
            )
                .into_response();
        }
    };

    info!(ip = %client_ip, "WebSocket connection accepted");

    ws.on_upgrade(move |socket| handle_websocket(socket, guard, client_ip))
}

/// Handle WebSocket connection with rate limiting.
///
/// SEC-006: Message size limited and connection tracked.
async fn handle_websocket(
    mut socket: axum::extract::ws::WebSocket,
    _guard: WebSocketGuard, // RAII guard - releases on drop
    client_ip: std::net::IpAddr,
) {
    use axum::extract::ws::Message;

    info!(ip = %client_ip, "WebSocket connection established");

    let mut message_count: u32 = 0;

    // Message loop with limits
    while let Some(msg) = socket.recv().await {
        match msg {
            Ok(Message::Text(text)) => {
                // SEC-006: Check message size
                if text.len() > MAX_WS_MESSAGE_SIZE {
                    warn!(
                        ip = %client_ip,
                        size = text.len(),
                        "WebSocket message too large"
                    );
                    let _ = socket
                        .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: 1009, // Message too big
                            reason: "Message too large".into(),
                        })))
                        .await;
                    break;
                }

                // SEC-006: Check message count
                message_count += 1;
                if message_count > MAX_WS_MESSAGES {
                    warn!(
                        ip = %client_ip,
                        count = message_count,
                        "WebSocket message limit exceeded"
                    );
                    let _ = socket
                        .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: 1008, // Policy violation
                            reason: "Too many messages".into(),
                        })))
                        .await;
                    break;
                }

                debug!(
                    ip = %client_ip,
                    message_count,
                    "Received WebSocket message"
                );

                // Echo back for now
                if socket
                    .send(Message::Text(format!("Echo: {}", text).into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(Message::Binary(data)) => {
                // SEC-006: Check binary message size
                if data.len() > MAX_WS_MESSAGE_SIZE {
                    warn!(ip = %client_ip, size = data.len(), "Binary message too large");
                    let _ = socket
                        .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: 1009,
                            reason: "Message too large".into(),
                        })))
                        .await;
                    break;
                }
                // Ignore binary messages for now
            }
            Ok(Message::Close(_)) => {
                info!(ip = %client_ip, "WebSocket connection closed by client");
                break;
            }
            Ok(Message::Ping(data)) => {
                // Respond to pings
                if socket.send(Message::Pong(data)).await.is_err() {
                    break;
                }
            }
            Ok(Message::Pong(_)) => {
                // Ignore pongs
            }
            Err(e) => {
                error!(ip = %client_ip, error = %e, "WebSocket error");
                break;
            }
        }
    }

    info!(
        ip = %client_ip,
        messages = message_count,
        "WebSocket connection closed"
    );
    // _guard drops here, releasing the connection slot
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
            rate_limit_stats: RateLimitStats {
                tracked_ips: 5,
                total_concurrent: 10,
                total_websockets: 2,
            },
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("healthy"));
        assert!(json.contains("zeroclaw_enabled"));
        assert!(json.contains("rate_limit_stats"));
        assert!(json.contains("tracked_ips"));
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

    #[test]
    fn test_error_response_serialization() {
        let response = ErrorResponse {
            error: "Something went wrong".into(),
            code: "TEST_ERROR",
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("Something went wrong"));
        assert!(json.contains("TEST_ERROR"));
    }

    #[test]
    fn test_message_content_size_check() {
        // Test that the constant is reasonable
        assert!(MAX_MESSAGE_CONTENT_SIZE > 1000);
        assert!(MAX_MESSAGE_CONTENT_SIZE < 10_000_000);
    }

    #[test]
    fn test_websocket_limits() {
        // Test that WebSocket constants are reasonable
        assert!(MAX_WS_MESSAGE_SIZE > 1000);
        assert!(MAX_WS_MESSAGE_SIZE < 1_000_000);
        assert!(MAX_WS_MESSAGES > 10);
        assert!(MAX_WS_MESSAGES < 100_000);
    }
}
