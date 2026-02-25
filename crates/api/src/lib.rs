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
//! - `POST /api/v1/review` - Execute code review tasks
//! - `POST /api/v1/testing` - Execute testing tasks
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

pub mod auth;
pub mod rate_limit;
pub mod routes;
pub mod state;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

pub use auth::ApiKeyConfig;
pub use rate_limit::{RateLimitConfig, RateLimiter};
pub use state::{AgentRegistry, AppState, MemoryStats, default_memory_config};

/// Create the API router with all routes configured.
///
/// # Arguments
/// * `state` - Application state
/// * `allowed_origins` - CORS allowed origins. None = localhost only. Some(["*"]) = wildcard.
pub fn create_router(state: Arc<AppState>, allowed_origins: Option<Vec<String>>) -> Router {
    // SEC-009b: CORS restriction - default to localhost only
    let cors = build_cors_layer(allowed_origins);

    // SEC-006: Framework-level body size limit
    let max_body = state.rate_limiter.max_body_size();

    let mut router = Router::new()
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
        .route("/api/v1/review", post(routes::review_handler))
        .route("/api/v1/testing", post(routes::testing_handler))
        .route("/api/v1/workflow", post(routes::workflow_handler));

    // SEC-011: Conditionally add auth middleware if API key is configured
    if let Some(ref api_key_config) = state.api_key_config {
        let config = api_key_config.clone();
        router = router.layer(middleware::from_fn(move |req, next| {
            let config = config.clone();
            auth::api_key_auth(config, req, next)
        }));
    }

    router
        // Middleware
        .layer(DefaultBodyLimit::max(max_body))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

/// Build CORS layer based on allowed origins configuration (SEC-009b).
fn build_cors_layer(allowed_origins: Option<Vec<String>>) -> CorsLayer {
    use axum::http::{HeaderValue, Method};

    let methods = vec![Method::GET, Method::POST, Method::OPTIONS];

    match allowed_origins {
        Some(origins) if origins.len() == 1 && origins[0] == "*" => {
            warn!("CORS configured with wildcard '*' - this should only be used in development");
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(methods)
                .allow_headers(Any)
        }
        Some(origins) if !origins.is_empty() => {
            let parsed: Vec<HeaderValue> = origins
                .iter()
                .filter_map(|o| o.parse::<HeaderValue>().ok())
                .collect();
            info!(origins = ?origins, "CORS configured with specific origins");
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(parsed))
                .allow_methods(methods)
                .allow_headers(Any)
        }
        _ => {
            // Default: localhost only
            let localhost_origins: Vec<HeaderValue> = vec![
                "http://localhost".parse().unwrap(),
                "http://127.0.0.1".parse().unwrap(),
                "http://localhost:3000".parse().unwrap(),
                "http://localhost:5173".parse().unwrap(),
                "http://localhost:8080".parse().unwrap(),
                "http://127.0.0.1:3000".parse().unwrap(),
                "http://127.0.0.1:5173".parse().unwrap(),
                "http://127.0.0.1:8080".parse().unwrap(),
            ];
            info!("CORS configured for localhost origins only");
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(localhost_origins))
                .allow_methods(methods)
                .allow_headers(Any)
        }
    }
}

/// Start the API server on the given address.
///
/// SEC-006: Server configured with ConnectInfo for IP-based rate limiting.
pub async fn serve(
    state: Arc<AppState>,
    addr: SocketAddr,
    allowed_origins: Option<Vec<String>>,
) -> anyhow::Result<()> {
    let router = create_router(state, allowed_origins);

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
