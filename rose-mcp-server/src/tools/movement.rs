//! Movement tools for MCP server

use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::Tool;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::api_client::{ApiClient, FollowRequest, MoveRequest, Position};
use crate::schemas::{FollowPlayerParams, MoveBotParams, StopBotParams};
use crate::tools::{create_tool, format_error, format_response, ToolHandler};

/// Tool to move bot to a position
pub struct MoveBotTool {
    api_client: Arc<ApiClient>,
}

impl MoveBotTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for MoveBotTool {
    fn tool(&self) -> Tool {
        create_tool(
            "move_bot",
            "Move a bot to a specified position in the game world. The bot will navigate to the destination using the specified movement mode.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    },
                    "destination": {
                        "type": "object",
                        "description": "Target destination position",
                        "properties": {
                            "x": { "type": "number", "description": "X coordinate" },
                            "y": { "type": "number", "description": "Y coordinate" },
                            "z": { "type": "number", "description": "Z coordinate (usually 0)" }
                        },
                        "required": ["x", "y", "z"]
                    },
                    "target_entity_id": {
                        "type": "integer",
                        "description": "Optional entity ID to follow while moving"
                    },
                    "move_mode": {
                        "type": "string",
                        "description": "Movement mode",
                        "enum": ["walk", "run"],
                        "default": "run"
                    }
                },
                "required": ["bot_id", "destination"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: MoveBotParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        let request = MoveRequest {
            destination: Position::new(
                params.destination.x,
                params.destination.y,
                params.destination.z,
            ),
            target_entity_id: params.target_entity_id,
            move_mode: params.move_mode.unwrap_or_else(|| "run".to_string()),
        };

        match self.api_client.move_bot(&bot_id, request).await {
            Ok(_) => Ok(format_response(json!({
                "success": true,
                "message": format!("Bot {} moving to ({}, {}, {})", 
                    bot_id, 
                    params.destination.x, 
                    params.destination.y, 
                    params.destination.z
                )
            }))),
            Err(e) => Ok(format_error(format!("Failed to move bot: {}", e))),
        }
    }
}

/// Tool to follow a player
pub struct FollowPlayerTool {
    api_client: Arc<ApiClient>,
}

impl FollowPlayerTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for FollowPlayerTool {
    fn tool(&self) -> Tool {
        create_tool(
            "follow_player",
            "Make a bot follow a specific player. The bot will maintain a specified distance from the player.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    },
                    "player_name": {
                        "type": "string",
                        "description": "Name of the player to follow"
                    },
                    "distance": {
                        "type": "number",
                        "description": "Distance to maintain from the player (default: 50)",
                        "default": 50.0,
                        "minimum": 50.0,
                        "maximum": 1000.0
                    }
                },
                "required": ["bot_id", "player_name"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: FollowPlayerParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        let request = FollowRequest {
            player_name: params.player_name.clone(),
            distance: params.distance.unwrap_or(50.0),
        };

        match self.api_client.follow_player(&bot_id, request).await {
            Ok(_) => Ok(format_response(json!({
                "success": true,
                "message": format!("Bot {} is now following player '{}' at distance {}", 
                    bot_id, 
                    params.player_name,
                    params.distance.unwrap_or(50.0)
                )
            }))),
            Err(e) => Ok(format_error(format!("Failed to follow player: {}", e))),
        }
    }
}

/// Tool to stop bot movement
pub struct StopBotTool {
    api_client: Arc<ApiClient>,
}

impl StopBotTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for StopBotTool {
    fn tool(&self) -> Tool {
        create_tool(
            "stop_bot",
            "Stop a bot's current movement or action. The bot will cease all movement and stand still.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    }
                },
                "required": ["bot_id"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: StopBotParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.stop_bot(&bot_id).await {
            Ok(_) => Ok(format_response(json!({
                "success": true,
                "message": format!("Bot {} stopped", bot_id)
            }))),
            Err(e) => Ok(format_error(format!("Failed to stop bot: {}", e))),
        }
    }
}

/// Tool to teleport bot to player
pub struct TeleportToPlayerTool {
    api_client: Arc<ApiClient>,
}

impl TeleportToPlayerTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for TeleportToPlayerTool {
    fn tool(&self) -> Tool {
        create_tool(
            "teleport_to_player",
            "Instantly teleport the bot to its assigned player's location. Use this if the bot gets stuck or separated from the player.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    }
                },
                "required": ["bot_id"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: TeleportToPlayerParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.teleport_to_player(&bot_id).await {
            Ok(_) => Ok(format_response(json!({
                "success": true,
                "message": format!("Bot {} teleported to player", bot_id)
            }))),
            Err(e) => Ok(format_error(format!("Failed to teleport bot: {}", e))),
        }
    }
}
