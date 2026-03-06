//! Bot management tools for MCP server

use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::Tool;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::api_client::{ApiClient, CreateBotRequest};
use crate::schemas::{
    CreateBuddyBotParams, GetBotContextParams, GetBotStatusParams, RemoveBotParams,
};
use crate::tools::{create_tool, format_error, format_response, ToolHandler};

/// Tool to create a new buddy bot
pub struct CreateBuddyBotTool {
    api_client: Arc<ApiClient>,
}

impl CreateBuddyBotTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for CreateBuddyBotTool {
    fn tool(&self) -> Tool {
        create_tool(
            "create_buddy_bot",
            "Create a new buddy bot assigned to a player. The bot will act as a companion that can follow the player, chat, and perform actions.",
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name for the bot character"
                    },
                    "level": {
                        "type": "integer",
                        "description": "Level for the bot (optional, defaults based on assigned player)",
                        "minimum": 1
                    },
                    "build": {
                        "type": "string",
                        "description": "Build/class type for the bot",
                        "enum": ["knight", "champion", "mage", "cleric", "raider", "scout", "bourgeois", "artisan"]
                    },
                    "assigned_player": {
                        "type": "string",
                        "description": "Player name to assign the bot to follow"
                    }
                },
                "required": ["name", "assigned_player"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: CreateBuddyBotParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let request = CreateBotRequest {
            name: params.name,
            level: params.level,
            build: params.build,
            assigned_player: params.assigned_player,
        };

        match self.api_client.create_bot(request).await {
            Ok(response) => Ok(format_response(json!({
                "success": true,
                "bot_id": response.bot_id.to_string(),
                "entity_id": response.entity_id,
                "name": response.name,
                "status": response.status
            }))),
            Err(e) => Ok(format_error(format!("Failed to create bot: {}", e))),
        }
    }
}

/// Tool to get bot status
pub struct GetBotStatusTool {
    api_client: Arc<ApiClient>,
}

impl GetBotStatusTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for GetBotStatusTool {
    fn tool(&self) -> Tool {
        create_tool(
            "get_bot_status",
            "Get the current status of a bot including health, position, level, and current action.",
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
        let params: GetBotStatusParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.get_bot_status(&bot_id).await {
            Ok(status) => Ok(format_response(json!({
                "success": true,
                "bot": {
                    "bot_id": status.bot_id.to_string(),
                    "name": status.name,
                    "level": status.level,
                    "job": status.job,
                    "health": {
                        "current": status.health.current,
                        "max": status.health.max,
                        "percent": (status.health.current as f32 / status.health.max as f32 * 100.0) as u8
                    },
                    "mana": {
                        "current": status.mana.current,
                        "max": status.mana.max,
                        "percent": (status.mana.current as f32 / status.mana.max as f32 * 100.0) as u8
                    },
                    "stamina": {
                        "current": status.stamina.current,
                        "max": status.stamina.max
                    },
                    "position": {
                        "x": status.position.x,
                        "y": status.position.y,
                        "z": status.position.z,
                        "zone_id": status.position.zone_id
                    },
                    "current_command": status.current_command,
                    "assigned_player": status.assigned_player,
                    "is_dead": status.is_dead,
                    "is_sitting": status.is_sitting
                }
            }))),
            Err(e) => Ok(format_error(format!("Failed to get bot status: {}", e))),
        }
    }
}

/// Tool to get bot context for LLM
pub struct GetBotContextTool {
    api_client: Arc<ApiClient>,
}

impl GetBotContextTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for GetBotContextTool {
    fn tool(&self) -> Tool {
        create_tool(
            "get_bot_context",
            "Get comprehensive context about a bot optimized for LLM decision-making. Includes nearby threats, items, recent chat, and available actions.",
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
        let params: GetBotContextParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.get_bot_context(&bot_id).await {
            Ok(context) => {
                // Convert to JSON-serializable format
                let bot_info = json!({
                    "name": context.bot.name,
                    "level": context.bot.level,
                    "job": context.bot.job,
                    "health_percent": context.bot.health_percent,
                    "mana_percent": context.bot.mana_percent,
                    "position": {
                        "x": context.bot.position.x,
                        "y": context.bot.position.y,
                        "z": context.bot.position.z
                    },
                    "zone": context.bot.zone
                });
                
                let assigned_player = context.assigned_player.map(|p| json!({
                    "name": p.name,
                    "distance": p.distance,
                    "health_percent": p.health_percent,
                    "is_in_combat": p.is_in_combat
                }));
                
                let nearby_threats: Vec<_> = context.nearby_threats.into_iter().map(|t| json!({
                    "name": t.name,
                    "level": t.level,
                    "distance": t.distance
                })).collect();
                
                let nearby_items: Vec<_> = context.nearby_items.into_iter().map(|i| json!({
                    "name": i.name,
                    "distance": i.distance
                })).collect();
                
                let recent_chat: Vec<_> = context.recent_chat.into_iter().map(|c| json!({
                    "sender": c.sender,
                    "message": c.message
                })).collect();

                Ok(format_response(json!({
                    "success": true,
                    "context": {
                        "bot": bot_info,
                        "assigned_player": assigned_player,
                        "nearby_threats": nearby_threats,
                        "nearby_items": nearby_items,
                        "recent_chat": recent_chat,
                        "available_actions": context.available_actions
                    }
                })))
            }
            Err(e) => Ok(format_error(format!("Failed to get bot context: {}", e))),
        }
    }
}

/// Tool to list all bots
pub struct ListBotsTool {
    api_client: Arc<ApiClient>,
}

impl ListBotsTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for ListBotsTool {
    fn tool(&self) -> Tool {
        create_tool(
            "list_bots",
            "List all active buddy bots with their basic information.",
            json!({
                "type": "object",
                "properties": {}
            }),
        )
    }

    async fn handle(&self, _arguments: serde_json::Value) -> Result<String> {
        match self.api_client.list_bots().await {
            Ok(response) => {
                let bots: Vec<_> = response
                    .bots
                    .into_iter()
                    .map(|b| {
                        json!({
                            "bot_id": b.bot_id.to_string(),
                            "name": b.name,
                            "level": b.level,
                            "health": {
                                "current": b.health.current,
                                "max": b.health.max
                            },
                            "position": {
                                "x": b.position.x,
                                "y": b.position.y,
                                "z": b.position.z,
                                "zone_id": b.position.zone_id
                            },
                            "assigned_player": b.assigned_player,
                            "status": b.status
                        })
                    })
                    .collect();

                Ok(format_response(json!({
                    "success": true,
                    "bots": bots,
                    "count": bots.len()
                })))
            }
            Err(e) => Ok(format_error(format!("Failed to list bots: {}", e))),
        }
    }
}

/// Tool to remove a bot
pub struct RemoveBotTool {
    api_client: Arc<ApiClient>,
}

impl RemoveBotTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for RemoveBotTool {
    fn tool(&self) -> Tool {
        create_tool(
            "remove_bot",
            "Remove/delete a buddy bot from the game.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot to remove"
                    }
                },
                "required": ["bot_id"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: RemoveBotParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.delete_bot(&bot_id).await {
            Ok(_) => Ok(format_response(json!({
                "success": true,
                "message": format!("Bot {} removed successfully", bot_id)
            }))),
            Err(e) => Ok(format_error(format!("Failed to remove bot: {}", e))),
        }
    }
}

/// Tool to set bot behavior mode
pub struct SetBotBehaviorModeTool {
    api_client: Arc<ApiClient>,
}

impl SetBotBehaviorModeTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for SetBotBehaviorModeTool {
    fn tool(&self) -> Tool {
        create_tool(
            "set_bot_behavior_mode",
            "Set the high-level AI behavior mode for a bot.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    },
                    "mode": {
                        "type": "string",
                        "description": "Behavior mode",
                        "enum": ["passive", "defensive", "aggressive", "support"]
                    }
                },
                "required": ["bot_id", "mode"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: crate::schemas::SetBotBehaviorModeParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.set_bot_behavior_mode(&bot_id, &params.mode).await {
            Ok(_) => Ok(format_response(json!({
                "success": true,
                "message": format!("Bot {} behavior mode set to {}", bot_id, params.mode)
            }))),
            Err(e) => Ok(format_error(format!("Failed to set behavior mode: {}", e))),
        }
    }
}
