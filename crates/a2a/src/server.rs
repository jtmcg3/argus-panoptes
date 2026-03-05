//! Generic A2A server that wraps any `A2aAgent`.
//!
//! Serves:
//! - `GET /.well-known/agent.json` — Agent Card (discovery)
//! - `POST /` — JSON-RPC 2.0 endpoint (message/send, message/stream, tasks/get, tasks/cancel)
//! - `GET /health` — Health check

use crate::agent::A2aAgent;
use crate::types::*;

use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use chrono::Utc;
use dashmap::DashMap;
use futures::stream::Stream;
use panoptes_common::PanoptesError;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Maximum number of tasks held in memory.
const MAX_TASKS: usize = 10_000;

/// Shared state for the A2A server.
struct ServerState<A: A2aAgent> {
    agent: A,
    tasks: DashMap<String, Task>,
    agent_card: AgentCard,
}

/// Generic A2A server that wraps any A2aAgent.
pub struct AgentServer<A: A2aAgent> {
    agent: A,
    port: u16,
}

impl<A: A2aAgent> AgentServer<A> {
    pub fn new(agent: A, port: u16) -> Self {
        Self { agent, port }
    }

    /// Build the Agent Card from the agent's metadata.
    fn build_agent_card(agent: &A, port: u16) -> AgentCard {
        AgentCard {
            name: agent.name().into(),
            description: agent.description().into(),
            url: Some(format!("http://localhost:{}/", port)),
            version: agent.version().into(),
            capabilities: AgentCapabilities {
                streaming: agent.supports_streaming(),
                push_notifications: false,
            },
            skills: agent.skills(),
            default_input_modes: vec!["text/plain".into()],
            default_output_modes: vec!["text/plain".into(), "text/markdown".into()],
        }
    }

    /// Run the server.
    pub async fn run(self) -> anyhow::Result<()> {
        let card = Self::build_agent_card(&self.agent, self.port);

        info!(
            agent = %card.name,
            port = self.port,
            skills = card.skills.len(),
            streaming = card.capabilities.streaming,
            "Starting A2A agent server"
        );

        let state = Arc::new(ServerState {
            agent: self.agent,
            tasks: DashMap::new(),
            agent_card: card,
        });

        let app = Router::new()
            .route("/.well-known/agent.json", get(handle_agent_card::<A>))
            .route("/", post(handle_jsonrpc::<A>))
            .route("/health", get(handle_health))
            .layer(axum::extract::DefaultBodyLimit::max(512_000))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", self.port)).await?;
        info!(port = self.port, "A2A server listening");
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await?;

        Ok(())
    }
}

/// Wait for Ctrl+C to trigger graceful shutdown.
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
    info!("Shutdown signal received, draining connections");
}

// =============================================================================
// Route Handlers
// =============================================================================

/// Serve the Agent Card.
async fn handle_agent_card<A: A2aAgent>(
    State(state): State<Arc<ServerState<A>>>,
) -> Json<AgentCard> {
    Json(state.agent_card.clone())
}

/// Health check.
async fn handle_health() -> &'static str {
    "ok"
}

/// JSON-RPC 2.0 dispatcher.
async fn handle_jsonrpc<A: A2aAgent>(
    State(state): State<Arc<ServerState<A>>>,
    Json(request): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    if request.jsonrpc != "2.0" {
        return JsonRpcResponse::error(request.id, -32600, "Invalid JSON-RPC version")
            .into_response();
    }

    match request.method.as_str() {
        "message/send" => handle_message_send(state, request).await.into_response(),
        "message/stream" => handle_message_stream(state, request).await.into_response(),
        "tasks/get" => handle_tasks_get(state, request).await.into_response(),
        "tasks/cancel" => handle_tasks_cancel(state, request).await.into_response(),
        _ => JsonRpcResponse::error(
            request.id,
            -32601,
            format!("Method not found: {}", request.method),
        )
        .into_response(),
    }
}

/// Handle `message/send` — synchronous task execution.
async fn handle_message_send<A: A2aAgent>(
    state: Arc<ServerState<A>>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let params: SendMessageParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                -32602,
                format!("Invalid params: {}", e),
            ));
        }
    };

    // Check task capacity
    if state.tasks.len() >= MAX_TASKS {
        return Json(JsonRpcResponse::error(
            request.id,
            -32003,
            "Server overloaded",
        ));
    }

    // Extract text from message parts
    let input = extract_text_from_message(&params.message);

    // Create task
    let task_id = params
        .task_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Check for task ID collision
    if state.tasks.contains_key(&task_id) {
        return Json(JsonRpcResponse::error(
            request.id,
            -32004,
            "Task already exists",
        ));
    }

    let task = Task {
        id: task_id.clone(),
        status: TaskStatus {
            state: TaskState::Working,
            message: None,
            timestamp: Utc::now(),
        },
        messages: vec![params.message.clone()],
        artifacts: vec![],
    };
    state.tasks.insert(task_id.clone(), task);

    // Execute
    match state.agent.handle(&input, None).await {
        Ok(result) => {
            let artifact = Artifact {
                id: Some(uuid::Uuid::new_v4().to_string()),
                parts: vec![Part::text(result)],
                metadata: serde_json::Value::Null,
            };

            let completed_task = Task {
                id: task_id.clone(),
                status: TaskStatus {
                    state: TaskState::Completed,
                    message: None,
                    timestamp: Utc::now(),
                },
                messages: vec![params.message],
                artifacts: vec![artifact],
            };
            state.tasks.insert(task_id, completed_task.clone());

            Json(JsonRpcResponse::success(
                request.id,
                serde_json::to_value(completed_task).unwrap(),
            ))
        }
        Err(e) => {
            let failed_task = Task {
                id: task_id.clone(),
                status: TaskStatus {
                    state: TaskState::Failed,
                    message: Some(e.to_string()),
                    timestamp: Utc::now(),
                },
                messages: vec![params.message],
                artifacts: vec![],
            };
            state.tasks.insert(task_id, failed_task);

            error!(error = %e, "Agent task failed");
            Json(JsonRpcResponse::error(
                request.id,
                -32000,
                format!("Agent error: {}", e),
            ))
        }
    }
}

/// Handle `message/stream` — SSE streaming task execution.
async fn handle_message_stream<A: A2aAgent>(
    state: Arc<ServerState<A>>,
    request: JsonRpcRequest,
) -> impl IntoResponse {
    let params: SendMessageParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            let err_response = JsonRpcResponse::error(
                request.id.clone(),
                -32602,
                format!("Invalid params: {}", e),
            );
            return Sse::new(error_sse_stream(err_response)).into_response();
        }
    };

    // Check task capacity
    if state.tasks.len() >= MAX_TASKS {
        let err_response = JsonRpcResponse::error(request.id.clone(), -32003, "Server overloaded");
        return Sse::new(error_sse_stream(err_response)).into_response();
    }

    let input = extract_text_from_message(&params.message);
    let task_id = params
        .task_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Check for task ID collision
    if state.tasks.contains_key(&task_id) {
        let err_response =
            JsonRpcResponse::error(request.id.clone(), -32004, "Task already exists");
        return Sse::new(error_sse_stream(err_response)).into_response();
    }

    // Store initial task state
    let task = Task {
        id: task_id.clone(),
        status: TaskStatus {
            state: TaskState::Working,
            message: None,
            timestamp: Utc::now(),
        },
        messages: vec![params.message.clone()],
        artifacts: vec![],
    };
    state.tasks.insert(task_id.clone(), task);

    // Create broadcast channel for progress
    let (progress_tx, progress_rx) = broadcast::channel::<ProgressEvent>(64);

    let state_clone = state.clone();
    let input_clone = input.clone();

    // Spawn the agent work in background
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let result = state_clone
            .agent
            .handle(&input_clone, Some(progress_tx))
            .await;
        let _ = result_tx.send(result);
    });

    // Build SSE stream
    let stream = build_sse_stream(task_id, state, params.message, progress_rx, result_rx);

    Sse::new(stream).into_response()
}

/// Build an SSE stream that emits a single JSON-RPC error event.
fn error_sse_stream(
    response: JsonRpcResponse,
) -> impl Stream<Item = Result<Event, std::convert::Infallible>> {
    async_stream::stream! {
        yield Ok(Event::default()
            .event("error")
            .json_data(&response)
            .unwrap_or_else(|_| Event::default()));
    }
}

/// Build an SSE stream from progress events and the final result.
fn build_sse_stream<A: A2aAgent>(
    task_id: String,
    state: Arc<ServerState<A>>,
    original_message: Message,
    mut progress_rx: broadcast::Receiver<ProgressEvent>,
    result_rx: tokio::sync::oneshot::Receiver<Result<String, PanoptesError>>,
) -> impl Stream<Item = Result<Event, std::convert::Infallible>> {
    async_stream::stream! {
        // Send initial working status
        let status_event = TaskStatusUpdateEvent {
            task_id: task_id.clone(),
            status: TaskStatus {
                state: TaskState::Working,
                message: Some("Starting...".into()),
                timestamp: Utc::now(),
            },
        };
        yield Ok(Event::default()
            .event("status")
            .json_data(&status_event)
            .unwrap_or_else(|_| Event::default()));

        // Forward progress events until the channel closes
        let mut result_rx = Some(result_rx);
        loop {
            tokio::select! {
                progress = progress_rx.recv() => {
                    match progress {
                        Ok(event) => {
                            let (event_name, status_msg) = match &event {
                                ProgressEvent::Phase { name, detail } => {
                                    ("status", format!("{}: {}", name, detail))
                                }
                                ProgressEvent::PartialResult { content } => {
                                    ("status", format!("partial: {}", &content[..content.len().min(100)]))
                                }
                                ProgressEvent::Warning { message } => {
                                    ("status", format!("warning: {}", message))
                                }
                            };

                            let status_event = TaskStatusUpdateEvent {
                                task_id: task_id.clone(),
                                status: TaskStatus {
                                    state: TaskState::Working,
                                    message: Some(status_msg),
                                    timestamp: Utc::now(),
                                },
                            };
                            yield Ok(Event::default()
                                .event(event_name)
                                .json_data(&status_event)
                                .unwrap_or_else(|_| Event::default()));
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(lagged = n, "SSE consumer lagged, dropped events");
                        }
                    }
                }
                result = async {
                    if let Some(rx) = result_rx.take() {
                        Some(rx.await)
                    } else {
                        // Already consumed, just pend forever
                        std::future::pending::<Option<Result<Result<String, PanoptesError>, _>>>().await
                    }
                } => {
                    if let Some(Ok(agent_result)) = result {
                        match agent_result {
                            Ok(content) => {
                                // Send artifact
                                let artifact = Artifact {
                                    id: Some(uuid::Uuid::new_v4().to_string()),
                                    parts: vec![Part::text(&content)],
                                    metadata: serde_json::Value::Null,
                                };
                                let artifact_event = TaskArtifactUpdateEvent {
                                    task_id: task_id.clone(),
                                    artifact: artifact.clone(),
                                };
                                yield Ok(Event::default()
                                    .event("artifact")
                                    .json_data(&artifact_event)
                                    .unwrap_or_else(|_| Event::default()));

                                // Send completed status
                                let completed = TaskStatusUpdateEvent {
                                    task_id: task_id.clone(),
                                    status: TaskStatus {
                                        state: TaskState::Completed,
                                        message: None,
                                        timestamp: Utc::now(),
                                    },
                                };

                                // Update stored task
                                state.tasks.insert(task_id.clone(), Task {
                                    id: task_id.clone(),
                                    status: completed.status.clone(),
                                    messages: vec![original_message],
                                    artifacts: vec![artifact],
                                });

                                yield Ok(Event::default()
                                    .event("status")
                                    .json_data(&completed)
                                    .unwrap_or_else(|_| Event::default()));
                            }
                            Err(e) => {
                                let failed = TaskStatusUpdateEvent {
                                    task_id: task_id.clone(),
                                    status: TaskStatus {
                                        state: TaskState::Failed,
                                        message: Some(e.to_string()),
                                        timestamp: Utc::now(),
                                    },
                                };
                                state.tasks.insert(task_id.clone(), Task {
                                    id: task_id.clone(),
                                    status: failed.status.clone(),
                                    messages: vec![original_message],
                                    artifacts: vec![],
                                });
                                yield Ok(Event::default()
                                    .event("status")
                                    .json_data(&failed)
                                    .unwrap_or_else(|_| Event::default()));
                            }
                        }
                    }
                    break;
                }
            }
        }
    }
}

/// Handle `tasks/get`.
async fn handle_tasks_get<A: A2aAgent>(
    state: Arc<ServerState<A>>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let params: GetTaskParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                -32602,
                format!("Invalid params: {}", e),
            ));
        }
    };

    match state.tasks.get(&params.task_id) {
        Some(task) => Json(JsonRpcResponse::success(
            request.id,
            serde_json::to_value(task.value()).unwrap(),
        )),
        None => Json(JsonRpcResponse::error(
            request.id,
            -32001,
            format!("Task not found: {}", params.task_id),
        )),
    }
}

/// Handle `tasks/cancel`.
async fn handle_tasks_cancel<A: A2aAgent>(
    state: Arc<ServerState<A>>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let params: CancelTaskParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                -32602,
                format!("Invalid params: {}", e),
            ));
        }
    };

    match state.tasks.get_mut(&params.task_id) {
        Some(mut task) => {
            task.status = TaskStatus {
                state: TaskState::Canceled,
                message: Some("Canceled by client".into()),
                timestamp: Utc::now(),
            };
            Json(JsonRpcResponse::success(
                request.id,
                serde_json::to_value(task.value()).unwrap(),
            ))
        }
        None => Json(JsonRpcResponse::error(
            request.id,
            -32001,
            format!("Task not found: {}", params.task_id),
        )),
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Extract text content from a message's parts.
fn extract_text_from_message(message: &Message) -> String {
    message
        .parts
        .iter()
        .filter_map(|p| p.as_text())
        .collect::<Vec<_>>()
        .join("\n")
}

/// IntoResponse impl for JsonRpcResponse.
impl IntoResponse for JsonRpcResponse {
    fn into_response(self) -> axum::response::Response {
        Json(self).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_text_from_message() {
        let msg = Message {
            role: Role::User,
            parts: vec![Part::text("Hello"), Part::text("World")],
            task_id: None,
        };
        assert_eq!(extract_text_from_message(&msg), "Hello\nWorld");
    }

    #[test]
    fn test_extract_text_skips_non_text() {
        let msg = Message {
            role: Role::User,
            parts: vec![
                Part::text("Hello"),
                Part::Data {
                    data: serde_json::json!({"key": "value"}),
                    mime_type: None,
                },
            ],
            task_id: None,
        };
        assert_eq!(extract_text_from_message(&msg), "Hello");
    }
}
