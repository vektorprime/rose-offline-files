//! Axum HTTP server setup for the LLM Buddy Bot REST API

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{Method, Request, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Json,
};
use log::{info, warn, debug};
use serde_json::Value;
use tokio::sync::Notify;
use tower_http::cors::{Any, CorsLayer};

use super::routes::{create_router, get_route_info};
use super::state::ApiState;

/// Default API server port
pub const DEFAULT_API_PORT: u16 = 8080;

/// Configuration for the API server
#[derive(Debug, Clone)]
pub struct ApiServerConfig {
    /// Port to listen on
    pub port: u16,
    /// Host to bind to
    pub host: String,
}

impl Default for ApiServerConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_API_PORT,
            host: "127.0.0.1".to_string(),
        }
    }
}

impl ApiServerConfig {
    /// Create a new configuration with the specified port
    pub fn new(port: u16) -> Self {
        Self {
            port,
            ..Default::default()
        }
    }

    /// Set the host to bind to
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Get the socket address to bind to
    pub fn socket_addr(&self) -> Result<SocketAddr, String> {
        let addr: SocketAddr = format!("{}:{}", self.host, self.port)
            .parse()
            .map_err(|e| format!("Invalid address: {}", e))?;
        Ok(addr)
    }
}

/// Middleware to log all incoming requests with detailed information
async fn log_requests(req: Request<Body>, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().map(|q| q.to_string()).unwrap_or_default();
    let content_type = req.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("none")
        .to_string();
    
    // Collect request details
    let request_id: u64 = rand::random();
    info!("[REQ-{:016x}] ========== API Request ==========", request_id);
    info!("[REQ-{:016x}] Method: {}", request_id, method);
    info!("[REQ-{:016x}] Path: {}", request_id, path);
    if !query.is_empty() {
        info!("[REQ-{:016x}] Query: {}", request_id, query);
    }
    info!("[REQ-{:016x}] Content-Type: {}", request_id, content_type);
    
    let response = next.run(req).await;
    
    let elapsed = start.elapsed();
    let status = response.status();
    
    // Use different log levels based on status code
    if status.is_success() {
        info!("[REQ-{:016x}] Response: {} in {:.2}ms", request_id, status, elapsed.as_secs_f64() * 1000.0);
    } else if status.is_client_error() {
        warn!("[REQ-{:016x}] Response: {} in {:.2}ms", request_id, status, elapsed.as_secs_f64() * 1000.0);
    } else {
        info!("[REQ-{:016x}] Response: {} in {:.2}ms", request_id, status, elapsed.as_secs_f64() * 1000.0);
    }
    
    response
}

/// Request body logging middleware wrapper
/// This logs the body for POST/PUT/PATCH requests
async fn log_request_body(req: Request<Body>, next: Next) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path().to_string();
    
    // Only log body for methods that typically have a body
    if matches!(method, Method::POST | Method::PUT | Method::PATCH) {
        // Split the request into parts and body
        let (parts, body) = req.into_parts();
        
        // Collect the body bytes
        let bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!("Failed to read request body for logging: {}", e);
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from("Failed to read request body"))
                    .unwrap();
            }
        };
        
        // Try to log the body as JSON if possible
        if !bytes.is_empty() {
            if let Ok(body_str) = std::str::from_utf8(&bytes) {
                if let Ok(json) = serde_json::from_str::<Value>(body_str) {
                    info!("API Request Body ({} {}): {}", method, path, serde_json::to_string_pretty(&json).unwrap_or_else(|_| body_str.to_string()));
                } else {
                    // Not JSON, log as raw string (truncated if too long)
                    let truncated = if body_str.len() > 500 {
                        format!("{}... (truncated, {} bytes total)", &body_str[..500], body_str.len())
                    } else {
                        body_str.to_string()
                    };
                    info!("API Request Body ({} {}): {}", method, path, truncated);
                }
            } else {
                info!("API Request Body ({} {}): <binary data, {} bytes>", method, path, bytes.len());
            }
        }
        
        // Reconstruct the request
        let req = Request::from_parts(parts, Body::from(bytes));
        next.run(req).await
    } else {
        next.run(req).await
    }
}

/// Fallback handler for unmatched routes with detailed logging
async fn fallback_handler(uri: Uri, method: Method) -> (StatusCode, Json<serde_json::Value>) {
    let path = uri.path();
    
    warn!("========== 404 Route Not Found ==========");
    warn!("Request: {} {}", method, path);
    warn!("Query: {}", uri.query().unwrap_or("(none)"));
    
    // Get available routes for this method
    let routes = get_route_info();
    let matching_method_routes: Vec<&str> = routes
        .iter()
        .filter(|r| r.method == method.as_str())
        .map(|r| r.path)
        .collect();
    
    // Try to find similar routes (simple prefix matching)
    let path_parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let similar_routes: Vec<&str> = matching_method_routes
        .iter()
        .filter(|route| {
            let route_parts: Vec<&str> = route.split('/').filter(|p| !p.is_empty()).collect();
            // Check if first few parts match
            path_parts.len() > 0 && route_parts.len() > 0 && 
            (path_parts[0] == route_parts[0] || 
             (path_parts.len() > 1 && route_parts.len() > 1 && path_parts[0] == route_parts[0]))
        })
        .copied()
        .collect();
    
    if !matching_method_routes.is_empty() {
        if !similar_routes.is_empty() {
            warn!("Similar routes for {} {}:", method, path);
            for route in &similar_routes {
                warn!("  {} {}", method, route);
            }
        }
        info!("All available routes for {}:", method);
        for route in &matching_method_routes {
            info!("  {} {}", method, route);
        }
    } else {
        warn!("No routes available for method {}", method);
        info!("Available methods:");
        let methods: std::collections::HashSet<&str> = routes.iter().map(|r| r.method).collect();
        for m in methods {
            info!("  {}", m);
        }
    }
    warn!("==========================================");
    
    let error_response = serde_json::json!({
        "error": "Not Found",
        "message": format!("No route matches {} {}", method, path),
        "path": path,
        "method": method.as_str(),
        "available_routes": matching_method_routes,
        "hint": if similar_routes.len() == 1 {
            format!("Did you mean {} {}?", method, similar_routes[0])
        } else if !similar_routes.is_empty() {
            format!("Similar routes found: {}", similar_routes.join(", "))
        } else {
            "Check the API documentation for available routes".to_string()
        }
    });
    
    (StatusCode::NOT_FOUND, Json(error_response))
}

/// Start the API server
///
/// This function starts the HTTP server and blocks until the server
/// is shut down. It should be run in a separate tokio runtime.
///
/// # Arguments
///
/// * `state` - The shared API state
/// * `config` - Server configuration
/// * `shutdown` - Optional shutdown signal
///
/// # Returns
///
/// Returns `Ok(())` on successful shutdown or an error if the server fails.
pub async fn start_api_server(
    state: ApiState,
    config: ApiServerConfig,
    shutdown: Option<Arc<Notify>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = config.socket_addr()?;
    
    // Log all registered routes at startup
    info!("Registered API routes:");
    for route in get_route_info() {
        info!("  {} {}", route.method, route.path);
    }

    // Create the router with middleware layers
    let app = create_router()
        .with_state(state)
        // Add request body logging middleware (for POST/PUT/PATCH)
        .layer(middleware::from_fn(log_request_body))
        // Add general request logging middleware
        .layer(middleware::from_fn(log_requests))
        // Add fallback handler for 404s
        .fallback(fallback_handler);

    info!("Starting LLM Buddy Bot API server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    if let Some(shutdown_signal) = shutdown {
        // Run with graceful shutdown
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_signal.notified().await;
                info!("API server shutdown signal received");
            })
            .await?;
    } else {
        // Run without graceful shutdown
        axum::serve(listener, app).await?;
    }

    info!("API server stopped");
    Ok(())
}

/// Run the API server in a blocking manner
///
/// This function creates a new tokio runtime and runs the API server.
/// It's designed to be called from a non-async context.
pub fn run_api_server_blocking(
    state: ApiState,
    config: ApiServerConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::runtime::Builder::new_multi_thread()
        .thread_name("llm-api-server")
        .enable_all()
        .build()?
        .block_on(async { start_api_server(state, config, None).await })
}

/// Run the API server with graceful shutdown in a blocking manner
pub fn run_api_server_with_shutdown_blocking(
    state: ApiState,
    config: ApiServerConfig,
    shutdown: Arc<Notify>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::runtime::Builder::new_multi_thread()
        .thread_name("llm-api-server")
        .enable_all()
        .build()?
        .block_on(async { start_api_server(state, config, Some(shutdown)).await })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;

    #[test]
    fn test_config_default() {
        let config = ApiServerConfig::default();
        assert_eq!(config.port, DEFAULT_API_PORT);
        assert_eq!(config.host, "127.0.0.1");
    }

    #[test]
    fn test_config_socket_addr() {
        let config = ApiServerConfig::new(9000).with_host("0.0.0.0");
        let addr = config.socket_addr().unwrap();
        assert_eq!(addr.port(), 9000);
        assert_eq!(addr.ip().to_string(), "0.0.0.0");
    }

    #[test]
    fn test_config_invalid_address() {
        let config = ApiServerConfig {
            port: 8080,
            host: "invalid:host".to_string(),
        };
        assert!(config.socket_addr().is_err());
    }
}
