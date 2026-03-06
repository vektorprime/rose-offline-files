//! Information tools for MCP server

use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::Tool;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::api_client::ApiClient;
use crate::schemas::{GetBotSkillsParams, GetBotInventoryParams, GetPlayerStatusParams, TeleportToPlayerParams};
use crate::tools::{create_tool, format_error, format_response, ToolHandler};

/// Tool to get nearby entities with optional filtering
pub struct GetNearbyEntitiesTool {
    api_client: Arc<ApiClient>,
}

impl GetNearbyEntitiesTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for GetNearbyEntitiesTool {
    fn tool(&self) -> Tool {
        create_tool(
            "get_nearby_entities",
            "Get entities near the bot with optional filtering by type. Use this to find players, monsters, NPCs, or items in the area.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    },
                    "radius": {
                        "type": "number",
                        "description": "Search radius (default: 1000)",
                        "default": 1000.0,
                        "minimum": 100.0,
                        "maximum": 5000.0
                    },
                    "entity_types": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "enum": ["players", "monsters", "npcs", "items"]
                        },
                        "description": "Types of entities to include (optional, defaults to all types)"
                    }
                },
                "required": ["bot_id"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: crate::schemas::GetNearbyEntitiesParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        // Convert entity_types array to comma-separated string for API
        let entity_types_filter = params.entity_types.as_ref().map(|types| {
            types.join(",")
        });

        match self.api_client.get_nearby_entities(&bot_id, params.radius, entity_types_filter.as_deref()).await {
            Ok(response) => {
                // Group entities by type
                let mut players = Vec::new();
                let mut monsters = Vec::new();
                let mut npcs = Vec::new();
                let mut items = Vec::new();

                for e in response.entities {
                    match e.entity_type {
                        crate::api_client::NearbyEntityType::Player => {
                            players.push(json!({
                                "entity_id": e.entity_id,
                                "name": e.name,
                                "level": e.level,
                                "position": e.position,
                                "distance": e.distance,
                                "health_percent": e.health_percent
                            }));
                        }
                        crate::api_client::NearbyEntityType::Monster => {
                            monsters.push(json!({
                                "entity_id": e.entity_id,
                                "name": e.name,
                                "level": e.level,
                                "position": e.position,
                                "distance": e.distance,
                                "health_percent": e.health_percent
                            }));
                        }
                        crate::api_client::NearbyEntityType::Npc => {
                            npcs.push(json!({
                                "entity_id": e.entity_id,
                                "name": e.name,
                                "position": e.position,
                                "distance": e.distance
                            }));
                        }
                        crate::api_client::NearbyEntityType::Item => {
                            items.push(json!({
                                "entity_id": e.entity_id,
                                "name": e.name,
                                "position": e.position,
                                "distance": e.distance
                            }));
                        }
                    }
                }

                Ok(format_response(json!({
                    "success": true,
                    "bot_id": bot_id.to_string(),
                    "players": players,
                    "monsters": monsters,
                    "npcs": npcs,
                    "items": items,
                    "counts": {
                        "players": players.len(),
                        "monsters": monsters.len(),
                        "npcs": npcs.len(),
                        "items": items.len()
                    }
                })))
            }
            Err(e) => Ok(format_error(format!("Failed to get nearby entities: {}", e))),
        }
    }
}

/// Tool to get bot skills
pub struct GetBotSkillsTool {
    api_client: Arc<ApiClient>,
}

impl GetBotSkillsTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for GetBotSkillsTool {
    fn tool(&self) -> Tool {
        create_tool(
            "get_bot_skills",
            "Get all skills available to a bot. Use this to see which skills can be used with the use_skill tool.",
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
        let params: GetBotSkillsParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.get_bot_skills(&bot_id).await {
            Ok(response) => {
                let skills: Vec<_> = response
                    .skills
                    .into_iter()
                    .map(|s| {
                        json!({
                            "slot": s.slot,
                            "skill_id": s.skill_id,
                            "name": s.name,
                            "level": s.level,
                            "mp_cost": s.mp_cost,
                            "cooldown": s.cooldown
                        })
                    })
                    .collect();

                Ok(format_response(json!({
                    "success": true,
                    "bot_id": bot_id.to_string(),
                    "skills": skills,
                    "count": skills.len()
                })))
            }
            Err(e) => Ok(format_error(format!("Failed to get bot skills: {}", e))),
        }
    }
}

/// Tool to get nearby items
pub struct GetNearbyItemsTool {
    api_client: Arc<ApiClient>,
}

impl GetNearbyItemsTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for GetNearbyItemsTool {
    fn tool(&self) -> Tool {
        create_tool(
            "get_nearby_items",
            "Get all dropped items near the bot. Use this to find loot to pick up.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    },
                    "radius": {
                        "type": "number",
                        "description": "Search radius (default: 1000)",
                        "default": 1000.0,
                        "minimum": 100.0,
                        "maximum": 5000.0
                    }
                },
                "required": ["bot_id"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: crate::schemas::GetNearbyItemsParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.get_nearby_entities(&bot_id, params.radius, Some("items")).await {
            Ok(response) => {
                let items: Vec<_> = response
                    .entities
                    .into_iter()
                    .filter(|e| e.entity_type == crate::api_client::NearbyEntityType::Item)
                    .map(|e| {
                        json!({
                            "entity_id": e.entity_id,
                            "name": e.name,
                            "position": e.position,
                            "distance": e.distance
                        })
                    })
                    .collect();

                Ok(format_response(json!({
                    "success": true,
                    "bot_id": bot_id.to_string(),
                    "items": items,
                    "count": items.len()
                })))
            }
            Err(e) => Ok(format_error(format!("Failed to get nearby items: {}", e))),
        }
    }
}

/// Tool to get bot inventory
pub struct GetBotInventoryTool {
    api_client: Arc<ApiClient>,
}

impl GetBotInventoryTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for GetBotInventoryTool {
    fn tool(&self) -> Tool {
        create_tool(
            "get_bot_inventory",
            "Get the current inventory of a bot. Use this to see what items the bot is carrying.",
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
        let params: GetBotInventoryParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.get_bot_inventory(&bot_id).await {
            Ok(response) => {
                Ok(format_response(json!({
                    "success": true,
                    "bot_id": bot_id.to_string(),
                    "items": response.items,
                    "count": response.items.len()
                })))
            }
            Err(e) => Ok(format_error(format!("Failed to get bot inventory: {}", e))),
        }
    }
}

/// Tool to get player status
pub struct GetPlayerStatusTool {
    api_client: Arc<ApiClient>,
}

impl GetPlayerStatusTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for GetPlayerStatusTool {
    fn tool(&self) -> Tool {
        create_tool(
            "get_player_status",
            "Get the status of the player assigned to a bot. Use this to monitor the player's health and mana.",
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
        let params: GetPlayerStatusParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.get_player_status(&bot_id).await {
            Ok(response) => {
                Ok(format_response(json!({
                    "success": true,
                    "bot_id": bot_id.to_string(),
                    "status": response.status
                })))
            }
            Err(e) => Ok(format_error(format!("Failed to get player status: {}", e))),
        }
    }
}

/// Tool to get zone info
pub struct GetZoneInfoTool {
    api_client: Arc<ApiClient>,
}

impl GetZoneInfoTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for GetZoneInfoTool {
    fn tool(&self) -> Tool {
        create_tool(
            "get_zone_info",
            "Get information about the current zone the bot is in, including recommended levels.",
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
        let params: crate::schemas::GetZoneInfoParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.get_zone_info(&bot_id).await {
            Ok(response) => {
                Ok(format_response(json!({
                    "success": true,
                    "bot_id": bot_id.to_string(),
                    "zone": {
                        "name": response.zone_name,
                        "id": response.zone_id,
                        "recommended_level_min": response.recommended_level_min,
                        "recommended_level_max": response.recommended_level_max
                    }
                })))
            }
            Err(e) => Ok(format_error(format!("Failed to get zone info: {}", e))),
        }
    }
}
