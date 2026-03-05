//! Standalone A2A server for the Coordinator.
//!
//! The coordinator is itself an A2A server on port 8080.
//! It triages incoming requests and delegates to specialist agents.

use async_trait::async_trait;
use panoptes_a2a::{A2aAgent, AgentServer, AgentSkill, ProgressEvent};
use panoptes_common::PanoptesError;
use panoptes_coordinator::{AgentRegistry, CoordinatorConfig, Orchestrator};
use panoptes_llm::{LlmConfig, build_llm_client};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::info;

/// The coordinator wrapped as an A2aAgent.
struct CoordinatorAgent {
    orchestrator: Arc<Orchestrator>,
}

#[async_trait]
impl A2aAgent for CoordinatorAgent {
    fn name(&self) -> &str {
        "panoptes-coordinator"
    }

    fn description(&self) -> &str {
        "Intelligent triage coordinator that routes requests to specialist agents"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn skills(&self) -> Vec<AgentSkill> {
        vec![AgentSkill {
            id: "triage".into(),
            name: "Request Triage".into(),
            description: "Analyze requests and route to the appropriate specialist agent".into(),
            tags: vec!["triage".into(), "routing".into(), "orchestration".into()],
            examples: vec![
                "Fix the bug in parser.rs".into(),
                "Research Rust async patterns".into(),
                "Write documentation for the API".into(),
            ],
        }]
    }

    async fn handle(
        &self,
        input: &str,
        progress_tx: Option<broadcast::Sender<ProgressEvent>>,
    ) -> Result<String, PanoptesError> {
        self.orchestrator.process(input, progress_tx).await
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,panoptes=debug")
        .init();

    let config = load_config()?;

    // Build LLM client for triage
    let llm_config = LlmConfig {
        provider: config.triage.provider.clone(),
        model: config.triage.model.clone(),
        api_key: config.triage.api_key.clone(),
        api_url: Some(config.triage.api_url.clone()),
        temperature: Some(config.triage.temperature),
        max_tokens: Some(256),
        max_concurrent_requests: 1,
        retry: Default::default(),
    };
    let llm_client = build_llm_client(&llm_config)?;

    // Discover agents
    let mut registry = AgentRegistry::new();
    info!(
        agent_urls = ?config.agent_urls,
        "Discovering agents"
    );
    registry.discover(&config.agent_urls).await?;

    info!(agents = registry.len(), "Agent discovery complete");

    // Build orchestrator
    let delegate_timeout = config.delegate_timeout_ms.map(Duration::from_millis);
    let orchestrator = Arc::new(Orchestrator::new(llm_client, registry, delegate_timeout));

    let agent = CoordinatorAgent { orchestrator };

    info!(port = config.port, "Starting coordinator");
    AgentServer::new(agent, config.port).run().await
}

fn load_config() -> anyhow::Result<CoordinatorConfig> {
    let path = std::env::var("PANOPTES_CONFIG").unwrap_or_else(|_| "config/default.toml".into());
    match CoordinatorConfig::from_file(&path) {
        Ok(config) => Ok(config),
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path,
                "Failed to load config, using defaults"
            );
            Ok(CoordinatorConfig::default())
        }
    }
}
