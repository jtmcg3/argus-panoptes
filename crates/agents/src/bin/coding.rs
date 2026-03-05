//! Standalone A2A server for the Coding agent.

use panoptes_a2a::AgentServer;
use panoptes_agents::{AgentConfig, CodingAgent};

const PORT: u16 = 9006;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,panoptes=debug")
        .init();

    let model = load_model();
    let agent = CodingAgent::new(AgentConfig {
        id: "coding".into(),
        name: "Coding Agent".into(),
        model,
        system_prompt: None,
        mcp_servers: vec![],
        temperature: 0.7,
        max_tokens: 4096,
    });

    AgentServer::new(agent, PORT).run().await
}

fn load_model() -> String {
    let path = std::env::var("PANOPTES_CONFIG").unwrap_or_else(|_| "config/default.toml".into());
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let table: toml::Value = text
        .parse()
        .unwrap_or(toml::Value::Table(Default::default()));
    table
        .get("llm")
        .and_then(|v| v.get("model"))
        .and_then(|v| v.as_str())
        .unwrap_or("lfm2:24b")
        .to_string()
}
