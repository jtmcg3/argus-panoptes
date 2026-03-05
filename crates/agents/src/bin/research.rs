//! Standalone A2A server for the Research agent.

use panoptes_a2a::AgentServer;
use panoptes_agents::{AgentConfig, ResearchAgent, ResearchConfig};
use std::sync::Arc;

const PORT: u16 = 9001;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,panoptes=debug")
        .init();

    let config = load_config();

    let mut agent = ResearchAgent::new(
        AgentConfig {
            id: "research".into(),
            name: "Research Agent".into(),
            model: config.model.clone(),
            system_prompt: None,
            mcp_servers: vec![],
            temperature: 0.7,
            max_tokens: 4096,
        },
        ResearchConfig {
            search_url: config.search_url.clone(),
            ..ResearchConfig::default()
        },
    );

    // Wire LLM client
    let llm_config = config.llm_config();
    match panoptes_llm::build_llm_client(&llm_config) {
        Ok(client) => {
            agent = agent.with_llm(client);
            tracing::info!("LLM client initialized ({})", llm_config.model);
        }
        Err(e) => tracing::warn!("Failed to build LLM client, using template fallback: {e}"),
    }

    // Wire memory store
    let mem_config = config.memory_config();
    match panoptes_memory::MemoryStore::new(mem_config).await {
        Ok(store) => {
            agent = agent.with_memory(Arc::new(store));
            tracing::info!("Memory store initialized");
        }
        Err(e) => tracing::warn!("Failed to init memory store, continuing without memory: {e}"),
    }

    // Wire search URL
    if let Some(url) = config.search_url {
        agent = agent.with_search_url(url);
    }

    AgentServer::new(agent, PORT).run().await
}

struct AppConfig {
    model: String,
    search_url: Option<String>,
    llm_table: toml::Value,
    memory_table: toml::Value,
}

impl AppConfig {
    fn llm_config(&self) -> panoptes_llm::LlmConfig {
        self.llm_table
            .clone()
            .try_into()
            .unwrap_or_else(|_| panoptes_llm::LlmConfig {
                provider: "openai".into(),
                model: "lfm2:24b".into(),
                api_url: Some("http://host.orb.internal:11434".into()),
                api_key: None,
                temperature: None,
                max_tokens: None,
                max_concurrent_requests: 2,
                retry: Default::default(),
            })
    }

    fn memory_config(&self) -> panoptes_memory::MemoryConfig {
        self.memory_table.clone().try_into().unwrap_or_default()
    }
}

fn load_config() -> AppConfig {
    let path = std::env::var("PANOPTES_CONFIG").unwrap_or_else(|_| "config/default.toml".into());
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let table: toml::Value = text
        .parse()
        .unwrap_or(toml::Value::Table(Default::default()));

    let model = table
        .get("llm")
        .and_then(|v| v.get("model"))
        .and_then(|v| v.as_str())
        .unwrap_or("lfm2:24b")
        .to_string();

    let search_url = table
        .get("search")
        .and_then(|v| v.get("url"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let llm_table = table
        .get("llm")
        .cloned()
        .unwrap_or(toml::Value::Table(Default::default()));

    let memory_table = table
        .get("memory")
        .cloned()
        .unwrap_or(toml::Value::Table(Default::default()));

    AppConfig {
        model,
        search_url,
        llm_table,
        memory_table,
    }
}
