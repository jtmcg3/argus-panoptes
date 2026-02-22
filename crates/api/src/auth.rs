//! API key authentication middleware.
//!
//! SEC-011: Provides bearer token authentication for API endpoints.
//! The `/health` endpoint is exempted from authentication.

use axum::{
    body::Body,
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use tracing::warn;

/// Configuration for API key authentication.
#[derive(Debug, Clone)]
pub struct ApiKeyConfig {
    /// The expected API key (stored for constant-time comparison).
    key_bytes: Vec<u8>,
}

impl ApiKeyConfig {
    /// Create a new API key configuration.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key_bytes: key.into().into_bytes(),
        }
    }

    /// Constant-time comparison to prevent timing attacks.
    fn verify(&self, provided: &[u8]) -> bool {
        if self.key_bytes.len() != provided.len() {
            return false;
        }
        // XOR all bytes and accumulate — constant time regardless of where mismatch occurs
        let mut result: u8 = 0;
        for (a, b) in self.key_bytes.iter().zip(provided.iter()) {
            result |= a ^ b;
        }
        result == 0
    }
}

/// Error response for authentication failures.
#[derive(Debug, serde::Serialize)]
struct AuthError {
    error: String,
    code: &'static str,
}

/// Extract bearer token from Authorization header.
fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// API key authentication middleware (SEC-011).
///
/// Exempts the `/health` endpoint from authentication.
/// Returns 401 Unauthorized if the API key is missing or invalid.
pub async fn api_key_auth(
    api_key_config: ApiKeyConfig,
    request: Request<Body>,
    next: Next,
) -> Response {
    // Exempt health endpoint from auth
    if request.uri().path() == "/health" {
        return next.run(request).await;
    }

    // Extract and verify bearer token
    match extract_bearer_token(request.headers()) {
        Some(token) => {
            if api_key_config.verify(token.as_bytes()) {
                next.run(request).await
            } else {
                warn!("Invalid API key provided");
                (
                    StatusCode::UNAUTHORIZED,
                    Json(AuthError {
                        error: "Invalid API key".into(),
                        code: "INVALID_API_KEY",
                    }),
                )
                    .into_response()
            }
        }
        None => {
            warn!("Missing Authorization header");
            (
                StatusCode::UNAUTHORIZED,
                Json(AuthError {
                    error: "Missing or invalid Authorization header. Use: Authorization: Bearer <key>".into(),
                    code: "MISSING_API_KEY",
                }),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_key_config_verify_correct() {
        let config = ApiKeyConfig::new("test-key-123");
        assert!(config.verify(b"test-key-123"));
    }

    #[test]
    fn test_api_key_config_verify_wrong() {
        let config = ApiKeyConfig::new("test-key-123");
        assert!(!config.verify(b"wrong-key"));
    }

    #[test]
    fn test_api_key_config_verify_empty() {
        let config = ApiKeyConfig::new("test-key-123");
        assert!(!config.verify(b""));
    }

    #[test]
    fn test_api_key_config_verify_different_length() {
        let config = ApiKeyConfig::new("short");
        assert!(!config.verify(b"longer-key-here"));
    }

    #[test]
    fn test_extract_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer my-secret-key".parse().unwrap());
        assert_eq!(extract_bearer_token(&headers), Some("my-secret-key"));
    }

    #[test]
    fn test_extract_bearer_token_missing() {
        let headers = HeaderMap::new();
        assert_eq!(extract_bearer_token(&headers), None);
    }

    #[test]
    fn test_extract_bearer_token_wrong_scheme() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Basic dXNlcjpwYXNz".parse().unwrap());
        assert_eq!(extract_bearer_token(&headers), None);
    }
}
