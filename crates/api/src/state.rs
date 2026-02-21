//! Application state for the API server.

use crate::rate_limit::{RateLimitConfig, RateLimiter};
use panoptes_coordinator::{Coordinator, CoordinatorConfig};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared application state for the API server.
pub struct AppState {
    /// The coordinator that handles all agent routing.
    pub coordinator: Arc<RwLock<Coordinator>>,

    /// Server start time (for health checks).
    pub start_time: std::time::Instant,

    /// Rate limiter for request throttling (SEC-006).
    pub rate_limiter: Arc<RateLimiter>,
}

impl AppState {
    /// Create new application state with the given coordinator configuration.
    pub fn new(config: CoordinatorConfig) -> panoptes_common::Result<Self> {
        let coordinator = Coordinator::new(config)?;

        Ok(Self {
            coordinator: Arc::new(RwLock::new(coordinator)),
            start_time: std::time::Instant::now(),
            rate_limiter: Arc::new(RateLimiter::new(RateLimitConfig::default())),
        })
    }

    /// Create new application state with custom rate limit configuration.
    pub fn with_rate_limit(
        config: CoordinatorConfig,
        rate_limit_config: RateLimitConfig,
    ) -> panoptes_common::Result<Self> {
        let coordinator = Coordinator::new(config)?;

        Ok(Self {
            coordinator: Arc::new(RwLock::new(coordinator)),
            start_time: std::time::Instant::now(),
            rate_limiter: Arc::new(RateLimiter::new(rate_limit_config)),
        })
    }

    /// Get the uptime in seconds.
    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}
