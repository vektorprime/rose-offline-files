//! Configuration for the MCP server

use std::env;

/// Default API URL for the ROSE Offline server
pub const DEFAULT_API_URL: &str = "http://localhost:3000";

/// Environment variable name for the API URL
pub const API_URL_ENV_VAR: &str = "ROSE_API_URL";

/// Configuration for the MCP server
#[derive(Debug, Clone)]
pub struct Config {
    /// Base URL for the ROSE Offline REST API
    pub api_url: String,
}

impl Config {
    /// Create a new configuration with the given API URL
    pub fn new(api_url: impl Into<String>) -> Self {
        Self {
            api_url: api_url.into(),
        }
    }

    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        let api_url = env::var(API_URL_ENV_VAR)
            .unwrap_or_else(|_| DEFAULT_API_URL.to_string());
        Self { api_url }
    }

    /// Get the full URL for an API endpoint
    pub fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.api_url, path)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::from_env()
    }
}
