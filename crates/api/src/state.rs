//! Application state for the API server.

use crate::auth::ApiKeyConfig;
use crate::rate_limit::{RateLimitConfig, RateLimiter};
use panoptes_agents::{
    CodingAgent, PlanningAgent, ResearchAgent, ReviewAgent, TestingAgent, WritingAgent,
};
use panoptes_coordinator::{Coordinator, CoordinatorConfig};
use panoptes_llm::{LlmClient, LlmConfig, build_llm_client};
use panoptes_memory::{MemoryConfig, MemoryStore};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Container for all specialist agents.
pub struct AgentRegistry {
    /// Coding agent (PTY-MCP based).
    pub coding: Option<Arc<CodingAgent>>,

    /// Research agent (web search with memory).
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

    /// Create a registry with LLM-enabled agents and optional memory.
    pub fn with_llm(
        llm: Arc<dyn LlmClient>,
        memory: Option<Arc<MemoryStore>>,
        search_url: Option<String>,
    ) -> Self {
        let mut research = ResearchAgent::with_default_config();
        if let Some(url) = search_url {
            research = research.with_search_url(url);
        }
        let research = if let Some(ref mem) = memory {
            research
                .with_memory(Arc::clone(mem))
                .with_llm(Arc::clone(&llm))
        } else {
            research.with_llm(Arc::clone(&llm))
        };

        Self {
            coding: Some(Arc::new(CodingAgent::with_default_config())),
            research: Some(Arc::new(research)),
            writing: Some(Arc::new(
                WritingAgent::with_default_config().with_llm(Arc::clone(&llm)),
            )),
            planning: Some(Arc::new(
                PlanningAgent::with_default_config().with_llm(Arc::clone(&llm)),
            )),
            review: Some(Arc::new(
                ReviewAgent::with_default_config().with_llm(Arc::clone(&llm)),
            )),
            testing: Some(Arc::new(
                TestingAgent::with_default_config().with_llm(Arc::clone(&llm)),
            )),
        }
    }

    /// Create a registry with memory-enabled research agent.
    pub fn with_memory(memory_store: Arc<MemoryStore>, search_url: Option<String>) -> Self {
        let mut research = ResearchAgent::with_default_config();
        if let Some(url) = search_url {
            research = research.with_search_url(url);
        }
        let research = research.with_memory(Arc::clone(&memory_store));

        Self {
            coding: Some(Arc::new(CodingAgent::with_default_config())),
            research: Some(Arc::new(research)),
            writing: Some(Arc::new(WritingAgent::with_default_config())),
            planning: Some(Arc::new(PlanningAgent::with_default_config())),
            review: Some(Arc::new(ReviewAgent::with_default_config())),
            testing: Some(Arc::new(TestingAgent::with_default_config())),
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

    /// Shared memory store for knowledge persistence.
    pub memory_store: Option<Arc<MemoryStore>>,

    /// API key configuration for authentication (SEC-011).
    /// None = unauthenticated mode (dev/local only).
    pub api_key_config: Option<ApiKeyConfig>,
}

impl AppState {
    /// Wire agents from the registry into the coordinator for routing dispatch.
    fn wire_agents(coordinator: &mut Coordinator, agents: &AgentRegistry) {
        if let Some(ref agent) = agents.research {
            coordinator.set_research_agent(Arc::clone(agent) as Arc<dyn panoptes_common::Agent>);
        }
        if let Some(ref agent) = agents.writing {
            coordinator.set_writing_agent(Arc::clone(agent) as Arc<dyn panoptes_common::Agent>);
        }
        if let Some(ref agent) = agents.planning {
            coordinator.set_planning_agent(Arc::clone(agent) as Arc<dyn panoptes_common::Agent>);
        }
        if let Some(ref agent) = agents.review {
            coordinator.set_review_agent(Arc::clone(agent) as Arc<dyn panoptes_common::Agent>);
        }
        if let Some(ref agent) = agents.testing {
            coordinator.set_testing_agent(Arc::clone(agent) as Arc<dyn panoptes_common::Agent>);
        }
    }

    /// Create new application state with the given coordinator configuration.
    pub fn new(config: CoordinatorConfig) -> panoptes_common::Result<Self> {
        let mut coordinator = Coordinator::new(config)?;
        let agents = AgentRegistry::default();
        Self::wire_agents(&mut coordinator, &agents);

        Ok(Self {
            coordinator: Arc::new(RwLock::new(coordinator)),
            start_time: std::time::Instant::now(),
            rate_limiter: Arc::new(RateLimiter::new(RateLimitConfig::default())),
            agents,
            memory_store: None,
            api_key_config: None,
        })
    }

    /// Set the API key configuration for authentication (SEC-011).
    pub fn with_api_key(mut self, config: ApiKeyConfig) -> Self {
        self.api_key_config = Some(config);
        self
    }

    /// Create new application state with memory enabled.
    ///
    /// This is the recommended way to create AppState for production use,
    /// as it enables the ResearchAgent to persist findings.
    pub async fn with_memory(
        config: CoordinatorConfig,
        memory_config: MemoryConfig,
        search_url: Option<String>,
    ) -> anyhow::Result<Self> {
        let mut coordinator = Coordinator::new(config)?;

        // Initialize memory store
        info!(
            db_path = %memory_config.db_path.display(),
            "Initializing shared memory store"
        );
        let memory_store = Arc::new(MemoryStore::new(memory_config).await?);

        // Warmup embedding model
        info!("Warming up embedding model...");
        memory_store.warmup().await?;
        info!("Embedding model ready");

        // Create agents with shared memory and wire into coordinator
        let agents = AgentRegistry::with_memory(Arc::clone(&memory_store), search_url);
        Self::wire_agents(&mut coordinator, &agents);

        Ok(Self {
            coordinator: Arc::new(RwLock::new(coordinator)),
            start_time: std::time::Instant::now(),
            rate_limiter: Arc::new(RateLimiter::new(RateLimitConfig::default())),
            agents,
            memory_store: Some(memory_store),
            api_key_config: None,
        })
    }

    /// Create new application state with LLM support and optional memory.
    pub async fn with_llm(
        config: CoordinatorConfig,
        llm_config: LlmConfig,
        memory_config: Option<MemoryConfig>,
        search_url: Option<String>,
    ) -> anyhow::Result<Self> {
        let mut coordinator = Coordinator::new(config)?;

        // Build LLM client
        let llm_client = build_llm_client(&llm_config)?;

        // Optionally build memory
        let (agents, memory_store) = if let Some(mem_config) = memory_config {
            let store = Arc::new(MemoryStore::new(mem_config).await?);
            store.warmup().await?;
            (
                AgentRegistry::with_llm(llm_client, Some(Arc::clone(&store)), search_url),
                Some(store),
            )
        } else {
            (AgentRegistry::with_llm(llm_client, None, search_url), None)
        };

        Self::wire_agents(&mut coordinator, &agents);

        Ok(Self {
            coordinator: Arc::new(RwLock::new(coordinator)),
            start_time: std::time::Instant::now(),
            rate_limiter: Arc::new(RateLimiter::new(RateLimitConfig::default())),
            agents,
            memory_store,
            api_key_config: None,
        })
    }

    /// Create new application state with custom rate limit configuration.
    pub fn with_rate_limit(
        config: CoordinatorConfig,
        rate_limit_config: RateLimitConfig,
    ) -> panoptes_common::Result<Self> {
        let mut coordinator = Coordinator::new(config)?;
        let agents = AgentRegistry::default();
        Self::wire_agents(&mut coordinator, &agents);

        Ok(Self {
            coordinator: Arc::new(RwLock::new(coordinator)),
            start_time: std::time::Instant::now(),
            rate_limiter: Arc::new(RateLimiter::new(rate_limit_config)),
            agents,
            memory_store: None,
            api_key_config: None,
        })
    }

    /// Create application state with a custom agent registry.
    pub fn with_agents(
        config: CoordinatorConfig,
        agents: AgentRegistry,
    ) -> panoptes_common::Result<Self> {
        let mut coordinator = Coordinator::new(config)?;
        Self::wire_agents(&mut coordinator, &agents);

        Ok(Self {
            coordinator: Arc::new(RwLock::new(coordinator)),
            start_time: std::time::Instant::now(),
            rate_limiter: Arc::new(RateLimiter::new(RateLimitConfig::default())),
            agents,
            memory_store: None,
            api_key_config: None,
        })
    }

    /// Get the uptime in seconds.
    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    /// Get the memory store if available.
    pub fn memory(&self) -> Option<&Arc<MemoryStore>> {
        self.memory_store.as_ref()
    }

    /// Get memory statistics.
    pub async fn memory_stats(&self) -> Option<MemoryStats> {
        if let Some(ref store) = self.memory_store {
            let count = store.count().await.ok()?;
            let working_memory = store.get_working_memory().await;
            Some(MemoryStats {
                total_memories: count,
                working_memory_size: working_memory.len(),
            })
        } else {
            None
        }
    }
}

/// Statistics about the memory system.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryStats {
    /// Total memories in LanceDB.
    pub total_memories: usize,
    /// Current working memory size.
    pub working_memory_size: usize,
}

/// Default memory configuration for the API server.
pub fn default_memory_config() -> MemoryConfig {
    MemoryConfig {
        db_path: PathBuf::from("./data/memory"),
        ..Default::default()
    }
}
