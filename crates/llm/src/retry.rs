use async_trait::async_trait;
use panoptes_common::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::client::{LlmClient, LlmRequest, LlmResponse};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 500,
            max_delay_ms: 30_000,
            backoff_multiplier: 2.0,
        }
    }
}

pub struct RetryingClient<T: LlmClient> {
    inner: T,
    config: RetryConfig,
}

impl<T: LlmClient> RetryingClient<T> {
    pub fn new(inner: T, config: RetryConfig) -> Self {
        Self { inner, config }
    }

    fn is_retryable(error_msg: &str) -> bool {
        let lower = error_msg.to_lowercase();
        lower.contains("429")
            || lower.contains("rate limit")
            || lower.contains("500")
            || lower.contains("502")
            || lower.contains("503")
            || lower.contains("504")
            || lower.contains("server error")
            || lower.contains("internal server error")
            || lower.contains("bad gateway")
            || lower.contains("service unavailable")
            || lower.contains("gateway timeout")
    }

    fn parse_retry_after(error_msg: &str) -> Option<u64> {
        // Try to find "retry-after: N" or "Retry-After: N" in the error
        let lower = error_msg.to_lowercase();
        if let Some(pos) = lower.find("retry-after") {
            let after = &error_msg[pos..];
            // Skip past the header name and colon/whitespace
            for word in after.split_whitespace().skip(1) {
                let cleaned = word.trim_end_matches(|c: char| !c.is_ascii_digit());
                if let Ok(secs) = cleaned.parse::<u64>() {
                    return Some(secs * 1000); // convert to ms
                }
            }
        }
        None
    }

    fn compute_delay(&self, attempt: u32) -> u64 {
        let base = self.config.initial_delay_ms as f64
            * self.config.backoff_multiplier.powi(attempt as i32);
        let jitter = (base * 0.1 * rand_jitter(attempt)) as u64;
        let delay = (base as u64).saturating_add(jitter);
        delay.min(self.config.max_delay_ms)
    }
}

/// Simple deterministic jitter based on attempt number (no external rand crate needed).
fn rand_jitter(attempt: u32) -> f64 {
    // Use a simple hash-like function for pseudo-random jitter
    let x = attempt.wrapping_mul(2654435761);
    (x % 100) as f64 / 100.0
}

#[async_trait]
impl<T: LlmClient> LlmClient for RetryingClient<T> {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let mut last_error = None;

        for attempt in 0..=self.config.max_retries {
            match self.inner.complete(request.clone()).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    let error_msg = e.to_string();

                    if attempt == self.config.max_retries || !Self::is_retryable(&error_msg) {
                        return Err(e);
                    }

                    let delay = Self::parse_retry_after(&error_msg)
                        .unwrap_or_else(|| self.compute_delay(attempt));

                    warn!(
                        attempt = attempt + 1,
                        max_retries = self.config.max_retries,
                        delay_ms = delay,
                        error = %error_msg,
                        "Retrying LLM request"
                    );

                    tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap())
    }

    fn model_name(&self) -> &str {
        self.inner.model_name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_retry_config() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_delay_ms, 500);
        assert_eq!(config.max_delay_ms, 30_000);
        assert!((config.backoff_multiplier - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn retryable_error_detection() {
        assert!(RetryingClient::<DummyClient>::is_retryable(
            "OpenAI API error 429 Too Many Requests: rate limit exceeded"
        ));
        assert!(RetryingClient::<DummyClient>::is_retryable(
            "Anthropic API error 500 Internal Server Error"
        ));
        assert!(RetryingClient::<DummyClient>::is_retryable(
            "server error: 502 bad gateway"
        ));
        assert!(RetryingClient::<DummyClient>::is_retryable(
            "503 Service Unavailable"
        ));
        assert!(!RetryingClient::<DummyClient>::is_retryable(
            "API error 401 Unauthorized"
        ));
        assert!(!RetryingClient::<DummyClient>::is_retryable(
            "Invalid request: missing model field"
        ));
    }

    #[test]
    fn parse_retry_after_from_error() {
        let msg = "429 Too Many Requests, Retry-After: 5";
        let delay = RetryingClient::<DummyClient>::parse_retry_after(msg);
        assert_eq!(delay, Some(5000));
    }

    #[test]
    fn compute_delay_respects_max() {
        let client = RetryingClient {
            inner: DummyClient,
            config: RetryConfig {
                max_retries: 5,
                initial_delay_ms: 500,
                max_delay_ms: 2000,
                backoff_multiplier: 10.0,
            },
        };
        // Even with high multiplier, should not exceed max
        let delay = client.compute_delay(5);
        assert!(delay <= 2000);
    }

    struct DummyClient;

    #[async_trait]
    impl LlmClient for DummyClient {
        async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse> {
            Ok(LlmResponse {
                content: "dummy".to_string(),
                model: "dummy".to_string(),
                usage: None,
                finish_reason: None,
            })
        }
        fn model_name(&self) -> &str {
            "dummy"
        }
    }
}
