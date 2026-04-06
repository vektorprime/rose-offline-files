//! Configuration for the LLM Feedback System
//!
//! This module provides configuration options for the LLM-controlled bot feedback loop.

use bevy::prelude::Resource;

/// Configuration resource for the LLM feedback system.
///
/// This resource controls how the game server communicates with the LLM server.
/// Default values are set for local development with a compatible LLM server.
///
/// # Environment Variables
///
/// The following environment variables can override default values:
/// - `LLM_SERVER_URL`: Override the server URL
/// - `LLM_API_KEY`: Override the API key
/// - `LLM_ENABLED`: Enable or disable the LLM feedback ("true" or "false")
#[derive(Debug, Clone, Resource)]
pub struct LlmConfig {
    /// URL of the LLM server (OpenAI-compatible API endpoint).
    /// Default: "http://localhost:8080"
    pub server_url: String,

    /// API key for authentication with the LLM server.
    /// For local servers, any value typically works.
    /// Default: "any-key-works"
    pub api_key: String,

    /// Model name to use for LLM requests.
    /// For local servers, this is often ignored but required by the API format.
    /// Default: "local-model"
    pub model: String,

    /// Interval in seconds between LLM polling cycles.
    /// Default: 5.0
    pub poll_interval_secs: f64,

    /// Whether the LLM feedback system is enabled.
    /// Default: true
    pub enabled: bool,

    /// Maximum number of tokens to request in LLM responses.
    /// Default: 10000
    pub max_tokens: u32,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            server_url: std::env::var("LLM_SERVER_URL")
                .unwrap_or_else(|_| "http://localhost:8080".to_string()),
            api_key: std::env::var("LLM_API_KEY")
                .unwrap_or_else(|_| "any-key-works".to_string()),
            model: std::env::var("LLM_MODEL")
                .unwrap_or_else(|_| "local-model".to_string()),
            poll_interval_secs: 5.0,
            enabled: std::env::var("LLM_ENABLED")
                .map(|v| v.eq_ignore_ascii_case("true"))
                .unwrap_or(true),
            max_tokens: 10000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = LlmConfig::default();
        assert_eq!(config.server_url, "http://localhost:8080");
        assert_eq!(config.api_key, "any-key-works");
        assert_eq!(config.model, "local-model");
        assert_eq!(config.poll_interval_secs, 5.0);
        assert!(config.enabled);
        assert_eq!(config.max_tokens, 10000);
    }
}
