//! Application state for the API server.

use crate::rate_limit::{RateLimitConfig, RateLimiter};
use panoptes_agents::{
    CodingAgent, PlanningAgent, ResearchAgent, ReviewAgent, TestingAgent, WritingAgent,
};
use panoptes_coordinator::{Coordinator, CoordinatorConfig};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Container for all specialist agents.
pub struct AgentRegistry {
    /// Coding agent (PTY-MCP based).
    pub coding: Option<Arc<CodingAgent>>,

    /// Research agent (web search).
    pub research: Option<Arc<ResearchAgent>>,

    /// Writing agent (content creation).
    pub writing: Option<Arc<WritingAgent>>,

    /// Planning agent (task breakdown).
    pub planning: Option<Arc<PlanningAgent>>,

    /// Review agent (code review).
    pub review: Option<Arc<ReviewAgent>>,

    /// Testing agent (test execution).
    pub testing: Option<Arc<TestingAgent>>,
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self {
            coding: Some(Arc::new(CodingAgent::with_default_config())),
            research: Some(Arc::new(ResearchAgent::with_default_config())),
            writing: Some(Arc::new(WritingAgent::with_default_config())),
            planning: Some(Arc::new(PlanningAgent::with_default_config())),
            review: Some(Arc::new(ReviewAgent::with_default_config())),
            testing: Some(Arc::new(TestingAgent::with_default_config())),
        }
    }
}

impl AgentRegistry {
    /// Create an empty registry with no agents.
    pub fn empty() -> Self {
        Self {
            coding: None,
            research: None,
            writing: None,
            planning: None,
            review: None,
            testing: None,
        }
    }

    /// Create a registry with only the coding agent.
    pub fn coding_only() -> Self {
        Self {
            coding: Some(Arc::new(CodingAgent::with_default_config())),
            ..Self::empty()
        }
    }
}

/// Shared application state for the API server.
pub struct AppState {
    /// The coordinator that handles all agent routing.
    pub coordinator: Arc<RwLock<Coordinator>>,

    /// Server start time (for health checks).
    pub start_time: std::time::Instant,

    /// Rate limiter for request throttling (SEC-006).
    pub rate_limiter: Arc<RateLimiter>,

    /// Specialist agents available for direct invocation.
    pub agents: AgentRegistry,
}

impl AppState {
    /// Create new application state with the given coordinator configuration.
    pub fn new(config: CoordinatorConfig) -> panoptes_common::Result<Self> {
        let coordinator = Coordinator::new(config)?;

        Ok(Self {
            coordinator: Arc::new(RwLock::new(coordinator)),
            start_time: std::time::Instant::now(),
            rate_limiter: Arc::new(RateLimiter::new(RateLimitConfig::default())),
            agents: AgentRegistry::default(),
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
            agents: AgentRegistry::default(),
        })
    }

    /// Create application state with a custom agent registry.
    pub fn with_agents(
        config: CoordinatorConfig,
        agents: AgentRegistry,
    ) -> panoptes_common::Result<Self> {
        let coordinator = Coordinator::new(config)?;

        Ok(Self {
            coordinator: Arc::new(RwLock::new(coordinator)),
            start_time: std::time::Instant::now(),
            rate_limiter: Arc::new(RateLimiter::new(RateLimitConfig::default())),
            agents,
        })
    }

    /// Get the uptime in seconds.
    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}
