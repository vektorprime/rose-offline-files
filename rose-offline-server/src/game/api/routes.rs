//! Route definitions for the LLM Buddy Bot REST API

use axum::{
    http::Method,
    routing::{delete, get, post},
    Router,
};

use super::handlers;
use super::state::ApiState;

/// Route information for logging purposes
pub struct RouteInfo {
    pub method: &'static str,
    pub path: &'static str,
}

/// Get all registered routes for logging/debugging purposes
pub fn get_route_info() -> Vec<RouteInfo> {
    vec![
        RouteInfo { method: "GET", path: "/health" },
        RouteInfo { method: "GET", path: "/bots" },
        RouteInfo { method: "POST", path: "/bots" },
        RouteInfo { method: "DELETE", path: "/bots/{bot_id}" },
        RouteInfo { method: "GET", path: "/bots/{bot_id}/status" },
        RouteInfo { method: "GET", path: "/bots/{bot_id}/context" },
        RouteInfo { method: "POST", path: "/bots/{bot_id}/move" },
        RouteInfo { method: "POST", path: "/bots/{bot_id}/follow" },
        RouteInfo { method: "POST", path: "/bots/{bot_id}/stop" },
        RouteInfo { method: "POST", path: "/bots/{bot_id}/attack" },
        RouteInfo { method: "POST", path: "/bots/{bot_id}/skill" },
        RouteInfo { method: "POST", path: "/bots/{bot_id}/sit" },
        RouteInfo { method: "POST", path: "/bots/{bot_id}/stand" },
        RouteInfo { method: "POST", path: "/bots/{bot_id}/pickup" },
        RouteInfo { method: "POST", path: "/bots/{bot_id}/emote" },
        RouteInfo { method: "POST", path: "/bots/{bot_id}/chat" },
        RouteInfo { method: "GET", path: "/bots/{bot_id}/chat/history" },
        RouteInfo { method: "GET", path: "/bots/{bot_id}/nearby" },
        RouteInfo { method: "GET", path: "/bots/{bot_id}/skills" },
        RouteInfo { method: "GET", path: "/bots/{bot_id}/inventory" },
        RouteInfo { method: "GET", path: "/bots/{bot_id}/player_status" },
        RouteInfo { method: "POST", path: "/bots/{bot_id}/teleport_to_player" },
        RouteInfo { method: "GET", path: "/bots/{bot_id}/zone" },
        RouteInfo { method: "POST", path: "/bots/{bot_id}/execute" },
    ]
}

/// Create the API router with all routes configured
///
/// Returns Router<ApiState> which needs to have with_state() called on it
/// before serving. This is the correct pattern for Axum0.8.
pub fn create_router() -> Router<ApiState> {
    log::info!("Creating API router with routes:");
    log::info!("  GET  /health");
    log::info!("  GET  /bots");
    log::info!("  POST /bots");
    log::info!("  DELETE /bots/{{bot_id}}");
    log::info!("  GET  /bots/{{bot_id}}/status");
    log::info!("  GET  /bots/{{bot_id}}/context");
    log::info!("  POST /bots/{{bot_id}}/move");
    log::info!("  POST /bots/{{bot_id}}/follow");
    log::info!("  POST /bots/{{bot_id}}/stop");
    log::info!("  POST /bots/{{bot_id}}/attack");
    log::info!("  POST /bots/{{bot_id}}/skill");
    log::info!("  POST /bots/{{bot_id}}/sit");
    log::info!("  POST /bots/{{bot_id}}/stand");
    log::info!("  POST /bots/{{bot_id}}/pickup");
    log::info!("  POST /bots/{{bot_id}}/emote");
    log::info!("  POST /bots/{{bot_id}}/chat");
    log::info!("  GET  /bots/{{bot_id}}/chat/history");
    log::info!("  GET  /bots/{{bot_id}}/nearby");
    log::info!("  GET  /bots/{{bot_id}}/skills");
    log::info!("  GET  /bots/{{bot_id}}/inventory");
    log::info!("  GET  /bots/{{bot_id}}/player_status");
    log::info!("  POST /bots/{{bot_id}}/teleport_to_player");
    log::info!("  GET  /bots/{{bot_id}}/zone");
    log::info!("  POST /bots/{{bot_id}}/execute");

    // NOTE: Axum 0.8 uses {param} syntax for path parameters (not :param which was Axum 0.7)
    Router::new()
        // Health check
        .route("/health", get(handlers::health_check))
        // Bot management
        .route("/bots", get(handlers::list_bots))
        .route("/bots", post(handlers::create_bot))
        .route("/bots/{bot_id}", delete(handlers::delete_bot))
        // Bot status
        .route("/bots/{bot_id}/status", get(handlers::get_bot_status))
        .route("/bots/{bot_id}/context", get(handlers::get_bot_context))
        // Bot movement
        .route("/bots/{bot_id}/move", post(handlers::move_bot))
        .route("/bots/{bot_id}/follow", post(handlers::follow_player))
        .route("/bots/{bot_id}/stop", post(handlers::stop_bot))
        // Bot combat
        .route("/bots/{bot_id}/attack", post(handlers::attack_target))
        .route("/bots/{bot_id}/skill", post(handlers::use_skill))
        // Bot actions
        .route("/bots/{bot_id}/sit", post(handlers::sit_bot))
        .route("/bots/{bot_id}/stand", post(handlers::stand_bot))
        .route("/bots/{bot_id}/pickup", post(handlers::pickup_item))
        .route("/bots/{bot_id}/emote", post(handlers::perform_emote))
        // Bot chat
        .route("/bots/{bot_id}/chat", post(handlers::send_chat))
        .route(
            "/bots/{bot_id}/chat/history",
            get(handlers::get_chat_history),
        )
        // Bot information
        .route(
            "/bots/{bot_id}/nearby",
            get(handlers::get_nearby_entities),
        )
        .route("/bots/{bot_id}/skills", get(handlers::get_bot_skills))
        .route("/bots/{bot_id}/inventory", get(handlers::get_bot_inventory))
        .route("/bots/{bot_id}/player_status", get(handlers::get_player_status))
        .route("/bots/{bot_id}/teleport_to_player", post(handlers::teleport_to_player))
        .route("/bots/{bot_id}/zone", get(handlers::get_zone_info))
        // LLM integration
        .route(
            "/bots/{bot_id}/execute",
            post(handlers::execute_llm_command),
        )
    // Note: Do NOT call with_state() here - it should be called in server.rs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use std::collections::HashMap;
    use parking_lot::RwLock;
    use std::sync::Arc;

    #[test]
    fn test_create_router() {
        let (tx, _rx) = unbounded();
        let bots_map = Arc::new(RwLock::new(HashMap::new()));
        let state = ApiState::new(tx, bots_map);
        // create_router returns Router<ApiState>, then we apply state to get Router<()>
        let _app: Router<()> = create_router().with_state(state);
    }
}
