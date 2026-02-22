//! REST/WebSocket API gateway for Argus-Panoptes.
//!
//! This crate provides external client access to the Panoptes multi-agent system
//! via HTTP REST endpoints and WebSocket connections.
//!
//! # Endpoints
//!
//! ## Core
//! - `GET /health` - Health check
//! - `POST /api/v1/messages` - Send a message to the coordinator
//! - `GET /api/v1/sessions/:id` - Get PTY session status
//! - `WS /api/v1/ws` - WebSocket for real-time streaming
//!
//! ## Specialist Agents
//! - `POST /api/v1/coding` - Execute coding tasks via PTY-MCP
//! - `POST /api/v1/research` - Execute research/search tasks
//! - `POST /api/v1/writing` - Execute content creation tasks
//! - `POST /api/v1/planning` - Execute planning tasks
//! - `POST /api/v1/workflow` - Execute multi-agent workflows
//!
//! # Security Features
//!
//! - **SEC-006**: Rate limiting on all endpoints
//! - Request body size limits (default 1MB)
//! - Per-IP concurrent connection limits
//! - WebSocket connection limits
//!
//! # Architecture
//!
//! ```text
//! Client (ZeroClaw/Telegram/etc.)
//!    │
//!    ▼
//! ┌─────────────────┐
//! │   API Gateway   │ ◄── This crate
//! │     (Axum)      │
//! └────────┬────────┘
//!          │
//!          ├──────────────────┬──────────────────┐
//!          ▼                  ▼                  ▼
//! ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
//! │   Coordinator   │ │  Specialist     │ │   Workflows     │
//! │    (Triage)     │ │    Agents       │ │                 │
//! └─────────────────┘ └─────────────────┘ └─────────────────┘
//! ```

pub mod rate_limit;
pub mod routes;
pub mod state;

use axum::{
    Router,
    routing::{get, post},
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

pub use rate_limit::{RateLimitConfig, RateLimiter};
pub use state::{AgentRegistry, AppState};

/// Create the API router with all routes configured.
pub fn create_router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health check
        .route("/health", get(routes::health))
        // API v1 - Core routes
        .route("/api/v1/messages", post(routes::send_message))
        .route("/api/v1/sessions/{id}", get(routes::get_session))
        .route("/api/v1/ws", get(routes::websocket_handler))
        // API v1 - Specialist agent routes
        .route("/api/v1/coding", post(routes::coding_handler))
        .route("/api/v1/research", post(routes::research_handler))
        .route("/api/v1/writing", post(routes::writing_handler))
        .route("/api/v1/planning", post(routes::planning_handler))
        .route("/api/v1/workflow", post(routes::workflow_handler))
        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

/// Start the API server on the given address.
///
/// SEC-006: Server configured with ConnectInfo for IP-based rate limiting.
pub async fn serve(state: Arc<AppState>, addr: SocketAddr) -> anyhow::Result<()> {
    let router = create_router(state);

    info!(%addr, "Starting Panoptes API server");

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Use into_make_service_with_connect_info to enable IP tracking (SEC-006)
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
