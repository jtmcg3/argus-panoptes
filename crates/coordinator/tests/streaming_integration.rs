use async_trait::async_trait;
use panoptes_a2a::{
    A2aAgent, AgentCapabilities, AgentCard, AgentServer, AgentSkill, ProgressEvent,
};
use panoptes_coordinator::{AgentRegistry, Orchestrator};
use panoptes_llm::{LlmClient, LlmRequest, LlmResponse};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

struct MockLlm;

#[async_trait]
impl LlmClient for MockLlm {
    async fn complete(&self, _request: LlmRequest) -> panoptes_common::Result<LlmResponse> {
        Err(panoptes_common::PanoptesError::Agent(
            "Mock: no LLM available".into(),
        ))
    }

    fn model_name(&self) -> &str {
        "mock"
    }
}

struct TestResearchAgent;

#[async_trait]
impl A2aAgent for TestResearchAgent {
    fn name(&self) -> &str {
        "panoptes-research"
    }

    fn description(&self) -> &str {
        "Test streaming research agent"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn skills(&self) -> Vec<AgentSkill> {
        vec![AgentSkill {
            id: "web-research".into(),
            name: "Web Research".into(),
            description: "Research skill".into(),
            tags: vec!["research".into()],
            examples: vec![],
        }]
    }

    async fn handle(
        &self,
        _input: &str,
        progress_tx: Option<broadcast::Sender<ProgressEvent>>,
    ) -> Result<String, panoptes_common::PanoptesError> {
        if let Some(tx) = progress_tx {
            let _ = tx.send(ProgressEvent::Phase {
                name: "research".into(),
                detail: "Searching source index".into(),
            });
        }
        Ok("streamed artifact content".into())
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("read local addr")
        .port()
}

async fn wait_for_agent(base_url: &str) {
    let client = reqwest::Client::new();
    for _ in 0..50 {
        if client
            .get(format!("{}/.well-known/agent.json", base_url))
            .send()
            .await
            .map(|resp| resp.status().is_success())
            .unwrap_or(false)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("Agent did not become ready: {}", base_url);
}

#[tokio::test]
#[ignore = "requires loopback socket bind permissions"]
async fn coordinator_streaming_parses_status_and_artifact() {
    let port = free_port();
    let base_url = format!("http://127.0.0.1:{port}");

    let server_handle = tokio::spawn(async move {
        AgentServer::new(TestResearchAgent, port)
            .run()
            .await
            .expect("test agent server");
    });

    wait_for_agent(&base_url).await;

    let mut registry = AgentRegistry::new();
    registry.register(
        base_url.clone(),
        AgentCard {
            name: "panoptes-research".into(),
            description: "Test streaming research agent".into(),
            url: Some(format!("{}/", base_url)),
            version: "0.1.0".into(),
            capabilities: AgentCapabilities {
                streaming: true,
                push_notifications: false,
            },
            skills: vec![AgentSkill {
                id: "web-research".into(),
                name: "Web Research".into(),
                description: "Research skill".into(),
                tags: vec!["research".into()],
                examples: vec![],
            }],
            default_input_modes: vec!["text/plain".into()],
            default_output_modes: vec!["text/plain".into()],
        },
    );

    let orchestrator = Orchestrator::new(Arc::new(MockLlm), registry, None);
    let (tx, mut rx) = broadcast::channel(32);

    let result = orchestrator
        .process("research rust async channels", Some(tx))
        .await
        .expect("orchestrator process");

    assert_eq!(result, "streamed artifact content");

    let mut saw_agent_status = false;
    for _ in 0..16 {
        let evt = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for progress event")
            .expect("progress receive");

        if let ProgressEvent::Phase { name, detail } = evt
            && name == "agent-status"
            && detail.contains("Searching source index")
        {
            saw_agent_status = true;
            break;
        }
    }
    assert!(saw_agent_status, "expected forwarded agent status event");

    server_handle.abort();
}
