//! HTTP route handlers for the API.
//!
//! SEC-006: All endpoints implement rate limiting and input validation.
//!
//! # Agent Endpoints
//!
//! Each specialist agent has a dedicated endpoint:
//! - `POST /api/v1/coding` - Coding tasks via PTY-MCP
//! - `POST /api/v1/research` - Web search and research
//! - `POST /api/v1/writing` - Content and document creation
//! - `POST /api/v1/planning` - Task planning and breakdown
//! - `POST /api/v1/workflow` - Multi-agent workflow orchestration

use crate::AppState;
use crate::rate_limit::{RateLimitStats, WebSocketGuard};
use axum::{
    Json,
    extract::{ConnectInfo, Path, State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use panoptes_agents::Agent;
use panoptes_common::{AgentMessage, PathSecurityConfig, Task, validate_working_dir};
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

// ============================================================================
// Agent-specific request/response types
// ============================================================================

/// Request body for agent endpoints.
///
/// This unified request type works for all specialist agents.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentRequest {
    /// The instruction or task description for the agent.
    pub instruction: String,

    /// Working directory for code-related tasks.
    #[serde(default)]
    pub working_dir: Option<String>,

    /// Additional context to provide to the agent.
    #[serde(default)]
    pub context: Option<String>,

    /// Permission mode for coding tasks (SEC-012).
    /// Only "act" enables Act mode; everything else defaults to Plan.
    /// Requires API authentication to use.
    #[serde(default)]
    pub permission_mode: Option<String>,
}

/// Response from an agent endpoint.
#[derive(Debug, Serialize)]
pub struct AgentResponse {
    /// Unique ID for this request.
    pub id: String,

    /// Status of the request: "running", "completed", "failed".
    pub status: String,

    /// Output from the agent.
    pub output: String,

    /// Session ID for PTY sessions (coding agent only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// Execution time in milliseconds.
    pub duration_ms: u64,
}

/// Request body for workflow execution.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowRequest {
    /// The instruction or task description.
    pub instruction: String,

    /// Working directory for code-related tasks.
    #[serde(default)]
    pub working_dir: Option<String>,

    /// Additional context to provide to agents.
    #[serde(default)]
    pub context: Option<String>,

    /// Agent IDs to include in the workflow (in order for sequential).
    pub agents: Vec<String>,

    /// Workflow mode: "sequential" or "concurrent".
    #[serde(default = "default_workflow_mode")]
    pub mode: String,
}

fn default_workflow_mode() -> String {
    "sequential".into()
}

/// Response from a workflow execution.
#[derive(Debug, Serialize)]
pub struct WorkflowResponse {
    /// Unique workflow execution ID.
    pub id: String,

    /// Overall status: "completed", "partial", "failed".
    pub status: String,

    /// Combined output from all agents.
    pub output: String,

    /// Results from each step.
    pub steps: Vec<WorkflowStepResponse>,

    /// Total execution time in milliseconds.
    pub duration_ms: u64,
}

/// Response for a single workflow step.
#[derive(Debug, Serialize)]
pub struct WorkflowStepResponse {
    /// Agent ID.
    pub agent_id: String,

    /// Agent name.
    pub agent_name: String,

    /// Whether this step succeeded.
    pub success: bool,

    /// Output from this step.
    pub output: String,

    /// Execution time for this step.
    pub duration_ms: u64,
}

/// Invalid working directory error response (SEC-010).
fn invalid_working_dir_response(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: format!("Invalid working directory: {}", msg),
            code: "INVALID_WORKING_DIR",
        }),
    )
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
                ));
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

// ============================================================================
// Agent-specific endpoints
// ============================================================================

/// Maximum agent request size (SEC-006).
const MAX_AGENT_REQUEST_SIZE: usize = 500_000; // 500KB

/// Default timeout for agent requests.
const AGENT_REQUEST_TIMEOUT_SECS: u64 = 300; // 5 minutes

/// Handle coding agent requests.
///
/// POST /api/v1/coding
///
/// SEC-006: Rate limited and size-bounded.
pub async fn coding_handler(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<AgentRequest>,
) -> Result<Json<AgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    let client_ip = addr.ip();
    let start_time = std::time::Instant::now();

    // SEC-006: Check rate limit
    if !state.rate_limiter.check_request(client_ip) {
        warn!(ip = %client_ip, "Rate limit exceeded for coding request");
        return Err(rate_limited_response());
    }

    // SEC-006: Check content size
    if request.instruction.len() > MAX_AGENT_REQUEST_SIZE {
        return Err(payload_too_large_response(MAX_AGENT_REQUEST_SIZE));
    }

    info!(
        ip = %client_ip,
        instruction_preview = %request.instruction.chars().take(50).collect::<String>(),
        "Received coding request"
    );

    // SEC-010: Validate working directory
    let path_config = PathSecurityConfig::default();
    let validated_dir = validate_working_dir(request.working_dir.as_deref(), &path_config)
        .map_err(|e| invalid_working_dir_response(&e.to_string()))?;

    // Get the coding agent
    let agent = match state.agents.coding.as_ref() {
        Some(agent) => agent,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "Coding agent not available".into(),
                    code: "AGENT_UNAVAILABLE",
                }),
            ));
        }
    };

    // Build task from request
    let mut task = Task::new(&request.instruction);
    task = task.with_working_dir(&validated_dir);
    if let Some(ref ctx) = request.context {
        task = task.with_context(ctx);
    }

    // Execute with timeout
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(AGENT_REQUEST_TIMEOUT_SECS),
        agent.process_task(&task),
    )
    .await
    .map_err(|_| {
        error!(ip = %client_ip, "Coding request timed out");
        (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorResponse {
                error: "Request timed out".into(),
                code: "TIMEOUT",
            }),
        )
    })?
    .map_err(|e| {
        error!(error = %e, "Coding agent failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Agent failed: {}", e),
                code: "AGENT_ERROR",
            }),
        )
    })?;

    Ok(Json(AgentResponse {
        id: task.id,
        status: "completed".into(),
        output: result.content,
        session_id: result.task_id,
        duration_ms: start_time.elapsed().as_millis() as u64,
    }))
}

/// Handle research agent requests.
///
/// POST /api/v1/research
///
/// SEC-006: Rate limited and size-bounded.
pub async fn research_handler(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<AgentRequest>,
) -> Result<Json<AgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    let client_ip = addr.ip();
    let start_time = std::time::Instant::now();

    if !state.rate_limiter.check_request(client_ip) {
        warn!(ip = %client_ip, "Rate limit exceeded for research request");
        return Err(rate_limited_response());
    }

    if request.instruction.len() > MAX_AGENT_REQUEST_SIZE {
        return Err(payload_too_large_response(MAX_AGENT_REQUEST_SIZE));
    }

    info!(
        ip = %client_ip,
        instruction_preview = %request.instruction.chars().take(50).collect::<String>(),
        "Received research request"
    );

    let agent = match state.agents.research.as_ref() {
        Some(agent) => agent,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "Research agent not available".into(),
                    code: "AGENT_UNAVAILABLE",
                }),
            ));
        }
    };

    let mut task = Task::new(&request.instruction);
    if let Some(ref ctx) = request.context {
        task = task.with_context(ctx);
    }

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(AGENT_REQUEST_TIMEOUT_SECS),
        agent.process_task(&task),
    )
    .await
    .map_err(|_| {
        (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorResponse {
                error: "Request timed out".into(),
                code: "TIMEOUT",
            }),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Agent failed: {}", e),
                code: "AGENT_ERROR",
            }),
        )
    })?;

    Ok(Json(AgentResponse {
        id: task.id,
        status: "completed".into(),
        output: result.content,
        session_id: None,
        duration_ms: start_time.elapsed().as_millis() as u64,
    }))
}

/// Handle writing agent requests.
///
/// POST /api/v1/writing
///
/// SEC-006: Rate limited and size-bounded.
pub async fn writing_handler(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<AgentRequest>,
) -> Result<Json<AgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    let client_ip = addr.ip();
    let start_time = std::time::Instant::now();

    if !state.rate_limiter.check_request(client_ip) {
        warn!(ip = %client_ip, "Rate limit exceeded for writing request");
        return Err(rate_limited_response());
    }

    if request.instruction.len() > MAX_AGENT_REQUEST_SIZE {
        return Err(payload_too_large_response(MAX_AGENT_REQUEST_SIZE));
    }

    info!(
        ip = %client_ip,
        instruction_preview = %request.instruction.chars().take(50).collect::<String>(),
        "Received writing request"
    );

    let agent = match state.agents.writing.as_ref() {
        Some(agent) => agent,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "Writing agent not available".into(),
                    code: "AGENT_UNAVAILABLE",
                }),
            ));
        }
    };

    let mut task = Task::new(&request.instruction);
    if let Some(ref ctx) = request.context {
        task = task.with_context(ctx);
    }

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(AGENT_REQUEST_TIMEOUT_SECS),
        agent.process_task(&task),
    )
    .await
    .map_err(|_| {
        (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorResponse {
                error: "Request timed out".into(),
                code: "TIMEOUT",
            }),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Agent failed: {}", e),
                code: "AGENT_ERROR",
            }),
        )
    })?;

    Ok(Json(AgentResponse {
        id: task.id,
        status: "completed".into(),
        output: result.content,
        session_id: None,
        duration_ms: start_time.elapsed().as_millis() as u64,
    }))
}

/// Handle planning agent requests.
///
/// POST /api/v1/planning
///
/// SEC-006: Rate limited and size-bounded.
pub async fn planning_handler(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<AgentRequest>,
) -> Result<Json<AgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    let client_ip = addr.ip();
    let start_time = std::time::Instant::now();

    if !state.rate_limiter.check_request(client_ip) {
        warn!(ip = %client_ip, "Rate limit exceeded for planning request");
        return Err(rate_limited_response());
    }

    if request.instruction.len() > MAX_AGENT_REQUEST_SIZE {
        return Err(payload_too_large_response(MAX_AGENT_REQUEST_SIZE));
    }

    info!(
        ip = %client_ip,
        instruction_preview = %request.instruction.chars().take(50).collect::<String>(),
        "Received planning request"
    );

    let agent = match state.agents.planning.as_ref() {
        Some(agent) => agent,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "Planning agent not available".into(),
                    code: "AGENT_UNAVAILABLE",
                }),
            ));
        }
    };

    let mut task = Task::new(&request.instruction);
    if let Some(ref ctx) = request.context {
        task = task.with_context(ctx);
    }

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(AGENT_REQUEST_TIMEOUT_SECS),
        agent.process_task(&task),
    )
    .await
    .map_err(|_| {
        (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorResponse {
                error: "Request timed out".into(),
                code: "TIMEOUT",
            }),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Agent failed: {}", e),
                code: "AGENT_ERROR",
            }),
        )
    })?;

    Ok(Json(AgentResponse {
        id: task.id,
        status: "completed".into(),
        output: result.content,
        session_id: None,
        duration_ms: start_time.elapsed().as_millis() as u64,
    }))
}

/// Handle workflow execution requests.
///
/// POST /api/v1/workflow
///
/// SEC-006: Rate limited and size-bounded.
pub async fn workflow_handler(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<WorkflowRequest>,
) -> Result<Json<WorkflowResponse>, (StatusCode, Json<ErrorResponse>)> {
    use panoptes_agents::{ConcurrentWorkflow, SequentialWorkflow, Workflow};

    let client_ip = addr.ip();
    let start_time = std::time::Instant::now();

    if !state.rate_limiter.check_request(client_ip) {
        warn!(ip = %client_ip, "Rate limit exceeded for workflow request");
        return Err(rate_limited_response());
    }

    if request.instruction.len() > MAX_AGENT_REQUEST_SIZE {
        return Err(payload_too_large_response(MAX_AGENT_REQUEST_SIZE));
    }

    if request.agents.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Workflow must specify at least one agent".into(),
                code: "INVALID_REQUEST",
            }),
        ));
    }

    info!(
        ip = %client_ip,
        agents = ?request.agents,
        mode = %request.mode,
        "Received workflow request"
    );

    // SEC-010: Validate working directory
    let path_config = PathSecurityConfig::default();
    let validated_dir = validate_working_dir(request.working_dir.as_deref(), &path_config)
        .map_err(|e| invalid_working_dir_response(&e.to_string()))?;

    // Collect requested agents
    let mut agents_list: Vec<Arc<dyn panoptes_agents::Agent>> = Vec::new();
    for agent_id in &request.agents {
        match agent_id.as_str() {
            "coding" => {
                if let Some(ref agent) = state.agents.coding {
                    agents_list.push(agent.clone() as Arc<dyn panoptes_agents::Agent>);
                }
            }
            "research" => {
                if let Some(ref agent) = state.agents.research {
                    agents_list.push(agent.clone() as Arc<dyn panoptes_agents::Agent>);
                }
            }
            "writing" => {
                if let Some(ref agent) = state.agents.writing {
                    agents_list.push(agent.clone() as Arc<dyn panoptes_agents::Agent>);
                }
            }
            "planning" => {
                if let Some(ref agent) = state.agents.planning {
                    agents_list.push(agent.clone() as Arc<dyn panoptes_agents::Agent>);
                }
            }
            "review" => {
                if let Some(ref agent) = state.agents.review {
                    agents_list.push(agent.clone() as Arc<dyn panoptes_agents::Agent>);
                }
            }
            "testing" => {
                if let Some(ref agent) = state.agents.testing {
                    agents_list.push(agent.clone() as Arc<dyn panoptes_agents::Agent>);
                }
            }
            _ => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Unknown agent: {}", agent_id),
                        code: "INVALID_AGENT",
                    }),
                ));
            }
        }
    }

    if agents_list.len() != request.agents.len() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "One or more requested agents are not available".into(),
                code: "AGENT_UNAVAILABLE",
            }),
        ));
    }

    // Build task with validated working directory
    let mut task = Task::new(&request.instruction);
    task = task.with_working_dir(&validated_dir);
    if let Some(ref ctx) = request.context {
        task = task.with_context(ctx);
    }

    // Execute workflow based on mode
    let workflow_result = match request.mode.as_str() {
        "concurrent" => {
            let mut workflow = ConcurrentWorkflow::new("api-workflow");
            for agent in agents_list {
                workflow = workflow.add_agent(agent);
            }
            tokio::time::timeout(
                std::time::Duration::from_secs(AGENT_REQUEST_TIMEOUT_SECS * 2),
                workflow.run(task.clone()),
            )
            .await
            .map_err(|_| {
                (
                    StatusCode::GATEWAY_TIMEOUT,
                    Json(ErrorResponse {
                        error: "Workflow timed out".into(),
                        code: "TIMEOUT",
                    }),
                )
            })?
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Workflow failed: {}", e),
                        code: "WORKFLOW_ERROR",
                    }),
                )
            })?
        }
        _ => {
            // Default to sequential
            let mut workflow = SequentialWorkflow::new("api-workflow");
            for agent in agents_list {
                workflow = workflow.add_agent(agent);
            }
            tokio::time::timeout(
                std::time::Duration::from_secs(AGENT_REQUEST_TIMEOUT_SECS * 2),
                workflow.run(task.clone()),
            )
            .await
            .map_err(|_| {
                (
                    StatusCode::GATEWAY_TIMEOUT,
                    Json(ErrorResponse {
                        error: "Workflow timed out".into(),
                        code: "TIMEOUT",
                    }),
                )
            })?
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Workflow failed: {}", e),
                        code: "WORKFLOW_ERROR",
                    }),
                )
            })?
        }
    };

    let steps = workflow_result
        .step_results
        .iter()
        .map(|s| WorkflowStepResponse {
            agent_id: s.agent_id.clone(),
            agent_name: s.agent_name.clone(),
            success: s.success,
            output: s.output.content.clone(),
            duration_ms: s.duration_ms,
        })
        .collect();

    let status = if workflow_result.success {
        "completed"
    } else if workflow_result.step_results.iter().any(|s| s.success) {
        "partial"
    } else {
        "failed"
    };

    Ok(Json(WorkflowResponse {
        id: task.id,
        status: status.into(),
        output: workflow_result.final_output.content,
        steps,
        duration_ms: start_time.elapsed().as_millis() as u64,
    }))
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
        let size = MAX_MESSAGE_CONTENT_SIZE;
        assert!(size > 1000);
        assert!(size < 10_000_000);
    }

    #[test]
    fn test_websocket_limits() {
        // Test that WebSocket constants are reasonable
        let msg_size = MAX_WS_MESSAGE_SIZE;
        let msg_count = MAX_WS_MESSAGES;
        assert!(msg_size > 1000);
        assert!(msg_size < 1_000_000);
        assert!(msg_count > 10);
        assert!(msg_count < 100_000);
    }
}
