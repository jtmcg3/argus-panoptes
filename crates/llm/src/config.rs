use std::sync::Arc;

use async_trait::async_trait;
use panoptes_common::{PanoptesError, Result};
use serde::{Deserialize, Serialize};

use crate::anthropic::AnthropicClient;
use crate::client::{LlmClient, LlmRequest, LlmResponse};
use crate::openai::OpenAiClient;
use crate::retry::{RetryConfig, RetryingClient};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
    pub api_key: Option<String>,
    pub api_url: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_requests: usize,
    #[serde(default)]
    pub retry: RetryConfig,
}

fn default_max_concurrent() -> usize {
    2
}

pub struct SemaphoredClient {
    inner: Arc<dyn LlmClient>,
    semaphore: Arc<tokio::sync::Semaphore>,
}

impl SemaphoredClient {
    pub fn new(inner: Arc<dyn LlmClient>, max_concurrent: usize) -> Self {
        Self {
            inner,
            semaphore: Arc::new(tokio::sync::Semaphore::new(max_concurrent)),
        }
    }
}

#[async_trait]
impl LlmClient for SemaphoredClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| PanoptesError::Agent(format!("Semaphore acquire failed: {e}")))?;
        self.inner.complete(request).await
    }

    fn model_name(&self) -> &str {
        self.inner.model_name()
    }
}

pub fn build_llm_client(config: &LlmConfig) -> Result<Arc<dyn LlmClient>> {
    let base_client: Box<dyn LlmClient> = match config.provider.as_str() {
        "openai" => Box::new(OpenAiClient::new(
            config.api_url.clone(),
            config.model.clone(),
            config.api_key.clone(),
        )),
        "anthropic" => {
            let api_key = config.api_key.clone().ok_or_else(|| {
                PanoptesError::Config("Anthropic requires an API key".to_string())
            })?;
            Box::new(AnthropicClient::new(config.model.clone(), api_key))
        }
        other => {
            return Err(PanoptesError::Config(format!(
                "Unknown LLM provider: {other}"
            )));
        }
    };

    let retrying: Box<dyn LlmClient> =
        Box::new(RetryingClient::new(base_client, config.retry.clone()));

    let semaphored = SemaphoredClient::new(Arc::from(retrying), config.max_concurrent_requests);

    Ok(Arc::new(semaphored))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML_CONFIG: &str = r#"
provider = "openai"
model = "llama3"
api_url = "http://localhost:11434"
max_concurrent_requests = 4

[retry]
max_retries = 5
initial_delay_ms = 1000
max_delay_ms = 60000
backoff_multiplier = 3.0
"#;

    #[test]
    fn deserialize_config_from_toml() {
        let config: LlmConfig = toml::from_str(TOML_CONFIG).unwrap();
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "llama3");
        assert_eq!(config.api_url.as_deref(), Some("http://localhost:11434"));
        assert!(config.api_key.is_none());
        assert_eq!(config.max_concurrent_requests, 4);
        assert_eq!(config.retry.max_retries, 5);
        assert_eq!(config.retry.initial_delay_ms, 1000);
    }

    #[test]
    fn deserialize_config_defaults() {
        let toml_str = r#"
provider = "anthropic"
model = "claude-sonnet-4-20250514"
api_key = "sk-ant-test"
"#;
        let config: LlmConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.max_concurrent_requests, 2);
        assert_eq!(config.retry.max_retries, 3);
        assert_eq!(config.retry.initial_delay_ms, 500);
    }

    #[test]
    fn build_openai_client() {
        let config = LlmConfig {
            provider: "openai".to_string(),
            model: "llama3".to_string(),
            api_key: None,
            api_url: None,
            temperature: None,
            max_tokens: None,
            max_concurrent_requests: 2,
            retry: RetryConfig::default(),
        };
        let client = build_llm_client(&config).unwrap();
        assert_eq!(client.model_name(), "llama3");
    }

    #[test]
    fn build_anthropic_client() {
        let config = LlmConfig {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_key: Some("sk-ant-test".to_string()),
            api_url: None,
            temperature: None,
            max_tokens: None,
            max_concurrent_requests: 2,
            retry: RetryConfig::default(),
        };
        let client = build_llm_client(&config).unwrap();
        assert_eq!(client.model_name(), "claude-sonnet-4-20250514");
    }

    #[test]
    fn build_anthropic_without_key_fails() {
        let config = LlmConfig {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_key: None,
            api_url: None,
            temperature: None,
            max_tokens: None,
            max_concurrent_requests: 2,
            retry: RetryConfig::default(),
        };
        assert!(build_llm_client(&config).is_err());
    }

    #[test]
    fn build_unknown_provider_fails() {
        let config = LlmConfig {
            provider: "gemini".to_string(),
            model: "gemini-pro".to_string(),
            api_key: None,
            api_url: None,
            temperature: None,
            max_tokens: None,
            max_concurrent_requests: 2,
            retry: RetryConfig::default(),
        };
        assert!(build_llm_client(&config).is_err());
    }

    #[tokio::test]
    async fn semaphored_client_limits_concurrency() {
        use crate::client::LlmResponse;
        use std::sync::atomic::{AtomicU32, Ordering};

        struct CountingClient {
            concurrent: Arc<AtomicU32>,
            max_seen: Arc<AtomicU32>,
        }

        #[async_trait]
        impl LlmClient for CountingClient {
            async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse> {
                let current = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                // Update max seen concurrency
                self.max_seen.fetch_max(current, Ordering::SeqCst);
                // Simulate work
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                self.concurrent.fetch_sub(1, Ordering::SeqCst);
                Ok(LlmResponse {
                    content: "ok".to_string(),
                    model: "test".to_string(),
                    usage: None,
                    finish_reason: None,
                })
            }
            fn model_name(&self) -> &str {
                "test"
            }
        }

        let concurrent = Arc::new(AtomicU32::new(0));
        let max_seen = Arc::new(AtomicU32::new(0));

        let inner = Arc::new(CountingClient {
            concurrent: concurrent.clone(),
            max_seen: max_seen.clone(),
        });

        let semaphored = Arc::new(SemaphoredClient::new(inner, 2));

        let mut handles = vec![];
        for _ in 0..6 {
            let client = semaphored.clone();
            handles.push(tokio::spawn(async move {
                client.complete(LlmRequest::default()).await.unwrap();
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        // Max concurrency should never exceed 2
        assert!(max_seen.load(Ordering::SeqCst) <= 2);
    }
}
