//! Combat tools for MCP server

use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::Tool;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::api_client::{ApiClient, AttackRequest, Position, SkillRequest, SkillTargetType};
use crate::schemas::{AttackTargetParams, UseSkillParams};
use crate::tools::{create_tool, format_error, format_response, ToolHandler};

/// Tool to attack a target
pub struct AttackTargetTool {
    api_client: Arc<ApiClient>,
}

impl AttackTargetTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for AttackTargetTool {
    fn tool(&self) -> Tool {
        create_tool(
            "attack_target",
            "Make a bot attack a specific target. You can specify either target_entity_id OR target_name. If target_entity_id fails, try using target_name instead.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    },
                    "target_entity_id": {
                        "type": "integer",
                        "description": "Entity ID of the target to attack (optional if target_name is provided)"
                    },
                    "target_name": {
                        "type": "string",
                        "description": "Name of the target monster to attack (optional if target_entity_id is provided)"
                    }
                },
                "required": ["bot_id"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: AttackTargetParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        // If target_entity_id is provided, use it directly
        if let Some(entity_id) = params.target_entity_id {
            let request = AttackRequest {
                target_entity_id: entity_id,
            };

            match self.api_client.attack_target(&bot_id, request).await {
                Ok(_) => Ok(format_response(json!({
                    "success": true,
                    "message": format!("Bot {} is attacking target entity {}", bot_id, entity_id)
                }))),
                Err(e) => Ok(format_error(format!("Failed to attack target: {}", e))),
            }
        } else if let Some(target_name) = &params.target_name {
            // Look up the entity by name from nearby entities
            match self.api_client.get_nearby_entities(&bot_id, Some(1000.0), Some("monsters")).await {
                Ok(nearby) => {
                    // Try exact match first, then partial match
                    let target = nearby.entities.iter()
                        .find(|e| e.name == *target_name)
                        .or_else(|| nearby.entities.iter()
                            .find(|e| e.name.to_lowercase().contains(&target_name.to_lowercase())));
                    
                    if let Some(entity) = target {
                        let request = AttackRequest {
                            target_entity_id: entity.entity_id,
                        };

                        match self.api_client.attack_target(&bot_id, request).await {
                            Ok(_) => Ok(format_response(json!({
                                "success": true,
                                "message": format!("Bot {} is attacking '{}' (entity ID: {})", bot_id, entity.name, entity.entity_id)
                            }))),
                            Err(e) => Ok(format_error(format!("Failed to attack target: {}", e))),
                        }
                    } else {
                        let available_names: Vec<&str> = nearby.entities.iter()
                            .take(10)
                            .map(|e| e.name.as_str())
                            .collect();
                        Ok(format_error(format!(
                            "Target '{}' not found nearby. Available monsters: {:?}",
                            target_name, available_names
                        )))
                    }
                }
                Err(e) => Ok(format_error(format!("Failed to get nearby entities: {}", e))),
            }
        } else {
            Ok(format_error("Either target_entity_id or target_name must be provided"))
        }
    }
}

/// Tool to use a skill
pub struct UseSkillTool {
    api_client: Arc<ApiClient>,
}

impl UseSkillTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for UseSkillTool {
    fn tool(&self) -> Tool {
        create_tool(
            "use_skill",
            "Make a bot use a skill. Can target entities, positions, or self depending on the skill type. For entity targeting, you can use either target_entity_id OR target_name. Use get_bot_skills to see available skills.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    },
                    "skill_id": {
                        "type": "integer",
                        "description": "Skill ID to use (use get_bot_skills to see available skills)"
                    },
                    "target_type": {
                        "type": "string",
                        "description": "Type of targeting",
                        "enum": ["entity", "position", "self"]
                    },
                    "target_entity_id": {
                        "type": "integer",
                        "description": "Target entity ID (optional if target_name is provided)"
                    },
                    "target_name": {
                        "type": "string",
                        "description": "Name of the target entity - can be a player name or monster name (optional if target_entity_id is provided)"
                    },
                    "target_position": {
                        "type": "object",
                        "description": "Target position (required if target_type is 'position')",
                        "properties": {
                            "x": { "type": "number" },
                            "y": { "type": "number" },
                            "z": { "type": "number" }
                        },
                        "required": ["x", "y", "z"]
                    }
                },
                "required": ["bot_id", "skill_id", "target_type"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: UseSkillParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        let target_type = match params.target_type.to_lowercase().as_str() {
            "entity" => SkillTargetType::Entity,
            "position" => SkillTargetType::Position,
            "self" | "self_target" => SkillTargetType::SelfTarget,
            other => return Ok(format_error(format!("Invalid target_type: {}. Must be 'entity', 'position', or 'self'", other))),
        };

        // Handle entity targeting - support both entity_id and name-based lookup
        let final_entity_id = if target_type == SkillTargetType::Entity {
            if let Some(entity_id) = params.target_entity_id {
                // Use entity_id directly if provided
                Some(entity_id)
            } else if let Some(target_name) = &params.target_name {
                // Look up entity by name from nearby entities
                match self.api_client.get_nearby_entities(&bot_id, Some(1000.0), None).await {
                    Ok(nearby) => {
                        // Try exact match first, then partial match (case-insensitive)
                        let target = nearby.entities.iter()
                            .find(|e| e.name == *target_name)
                            .or_else(|| nearby.entities.iter()
                                .find(|e| e.name.to_lowercase().contains(&target_name.to_lowercase())));
                        
                        if let Some(entity) = target {
                            Some(entity.entity_id)
                        } else {
                            let available_names: Vec<&str> = nearby.entities.iter()
                                .take(10)
                                .map(|e| e.name.as_str())
                                .collect();
                            return Ok(format_error(format!(
                                "Target '{}' not found nearby. Available entities: {:?}",
                                target_name, available_names
                            )));
                        }
                    }
                    Err(e) => return Ok(format_error(format!("Failed to get nearby entities: {}", e))),
                }
            } else {
                return Ok(format_error("Either target_entity_id or target_name must be provided when target_type is 'entity'"));
            }
        } else {
            params.target_entity_id
        };

        // Validate required parameters based on target type
        match &target_type {
            SkillTargetType::Entity => {
                if final_entity_id.is_none() {
                    return Ok(format_error("target_entity_id or target_name is required when target_type is 'entity'"));
                }
            }
            SkillTargetType::Position => {
                if params.target_position.is_none() {
                    return Ok(format_error("target_position is required when target_type is 'position'"));
                }
            }
            SkillTargetType::SelfTarget => {}
        }

        let request = SkillRequest {
            skill_id: params.skill_id,
            target_type,
            target_entity_id: final_entity_id,
            target_position: params.target_position.map(|p| Position::new(p.x, p.y, p.z)),
        };

        match self.api_client.use_skill(&bot_id, request).await {
            Ok(_) => Ok(format_response(json!({
                "success": true,
                "message": format!("Bot {} is using skill {} with {} targeting",
                    bot_id,
                    params.skill_id,
                    params.target_type
                )
            }))),
            Err(e) => Ok(format_error(format!("Failed to use skill: {}", e))),
        }
    }
}

/// Tool to pickup an item
pub struct PickupItemTool {
    api_client: Arc<ApiClient>,
}

impl PickupItemTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for PickupItemTool {
    fn tool(&self) -> Tool {
        create_tool(
            "pickup_item",
            "Make a bot pick up a dropped item from the ground. Use get_nearby_items to find item entity IDs.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    },
                    "item_entity_id": {
                        "type": "integer",
                        "description": "Entity ID of the item to pick up"
                    }
                },
                "required": ["bot_id", "item_entity_id"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: crate::schemas::PickupItemParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        match self.api_client.pickup_item(&bot_id, params.item_entity_id).await {
            Ok(_) => Ok(format_response(json!({
                "success": true,
                "message": format!("Bot {} is picking up item entity {}", bot_id, params.item_entity_id)
            }))),
            Err(e) => Ok(format_error(format!("Failed to pick up item: {}", e))),
        }
    }
}

/// Tool to use an item on the assigned player
pub struct UseItemOnPlayerTool {
    api_client: Arc<ApiClient>,
}

impl UseItemOnPlayerTool {
    pub fn new(api_client: Arc<ApiClient>) -> Self {
        Self { api_client }
    }
}

#[async_trait]
impl ToolHandler for UseItemOnPlayerTool {
    fn tool(&self) -> Tool {
        create_tool(
            "use_item_on_player",
            "Make a bot use a consumable item (like a potion or scroll) on its assigned player. Use get_bot_inventory to find item slot indices.",
            json!({
                "type": "object",
                "properties": {
                    "bot_id": {
                        "type": "string",
                        "description": "UUID of the bot"
                    },
                    "item_slot": {
                        "type": "integer",
                        "description": "Item slot index in bot's inventory"
                    }
                },
                "required": ["bot_id", "item_slot"]
            }),
        )
    }

    async fn handle(&self, arguments: serde_json::Value) -> Result<String> {
        let params: crate::schemas::UseItemOnPlayerParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return Ok(format_error(format!("Invalid parameters: {}", e))),
        };

        let bot_id = match params.bot_id.parse::<Uuid>() {
            Ok(id) => id,
            Err(e) => return Ok(format_error(format!("Invalid bot_id: {}", e))),
        };

        // First, get bot status to find assigned player's entity ID
        match self.api_client.get_bot_status(&bot_id).await {
            Ok(status) => {
                if let Some(player_name) = status.assigned_player {
                    // Get nearby entities to find the player's entity ID
                    match self.api_client.get_nearby_entities(&bot_id, Some(1000.0), Some("players")).await {
                        Ok(nearby) => {
                            let player_entity_id = nearby.entities.iter()
                                .find(|e| e.name == player_name)
                                .map(|e| e.entity_id);
                            
                            match self.api_client.use_item(&bot_id, params.item_slot, player_entity_id).await {
                                Ok(_) => Ok(format_response(json!({
                                    "success": true,
                                    "message": format!("Bot {} used item in slot {} on player {}", bot_id, params.item_slot, player_name)
                                }))),
                                Err(e) => Ok(format_error(format!("Failed to use item: {}", e))),
                            }
                        }
                        Err(e) => Ok(format_error(format!("Failed to find nearby player: {}", e))),
                    }
                } else {
                    Ok(format_error("Bot has no assigned player"))
                }
            }
            Err(e) => Ok(format_error(format!("Failed to get bot status: {}", e))),
        }
    }
}
