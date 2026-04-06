//! JSON schemas for MCP tool parameters

use schemars::schema_for;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Position parameter for tools
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PositionSchema {
    /// X coordinate in game world
    pub x: f32,
    /// Y coordinate in game world
    pub y: f32,
    /// Z coordinate (usually 0 for ground)
    pub z: f32,
}

// ================================
// Bot Management Schemas
// ================================

/// Parameters for create_buddy_bot tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateBuddyBotParams {
    /// Name for the bot character
    pub name: String,
    /// Level for the bot (optional, defaults based on assigned player)
    #[serde(default)]
    pub level: Option<u16>,
    /// Build/class type for the bot (knight, champion, mage, cleric, raider, scout, bourgeois, artisan)
    #[serde(default)]
    pub build: Option<String>,
    /// Player name to assign the bot to follow
    pub assigned_player: String,
}

/// Parameters for get_bot_status tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetBotStatusParams {
    /// UUID of the bot
    pub bot_id: String,
}

/// Parameters for get_bot_context tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetBotContextParams {
    /// UUID of the bot
    pub bot_id: String,
}

/// Parameters for list_bots tool (no parameters needed)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListBotsParams {}

/// Parameters for remove_bot tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemoveBotParams {
    /// UUID of the bot to remove
    pub bot_id: String,
}

/// Parameters for set_bot_behavior_mode tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetBotBehaviorModeParams {
    /// UUID of the bot
    pub bot_id: String,
    /// Behavior mode: "passive", "defensive", "aggressive", or "support"
    pub mode: String,
}

// ================================
// Movement Schemas
// ================================

/// Parameters for move_bot tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MoveBotParams {
    /// UUID of the bot
    pub bot_id: String,
    /// Target destination position
    pub destination: PositionSchema,
    /// Optional entity ID to follow while moving
    #[serde(default)]
    pub target_entity_id: Option<u32>,
    /// Movement mode: "walk" or "run" (default: "run")
    #[serde(default)]
    pub move_mode: Option<String>,
}

/// Parameters for follow_player tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FollowPlayerParams {
    /// UUID of the bot
    pub bot_id: String,
    /// Player name to follow
    pub player_name: String,
    /// Distance to maintain from the player (default: 300)
    #[serde(default)]
    pub distance: Option<f32>,
}

/// Parameters for stop_bot tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StopBotParams {
    /// UUID of the bot
    pub bot_id: String,
}

// ================================
// Combat Schemas
// ================================

/// Parameters for attack_target tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AttackTargetParams {
    /// UUID of the bot
    pub bot_id: String,
    /// Entity ID of the target to attack (optional if target_name is provided)
    #[serde(default)]
    pub target_entity_id: Option<u32>,
    /// Name of the target monster to attack (optional if target_entity_id is provided)
    #[serde(default)]
    pub target_name: Option<String>,
}

/// Parameters for use_skill tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UseSkillParams {
    /// UUID of the bot
    pub bot_id: String,
    /// Skill ID to use
    pub skill_id: u16,
    /// Type of targeting: "entity", "position", or "self"
    pub target_type: String,
    /// Target entity ID (optional if target_name is provided and target_type is "entity")
    #[serde(default)]
    pub target_entity_id: Option<u32>,
    /// Target name for entity targeting (optional if target_entity_id is provided)
    #[serde(default)]
    pub target_name: Option<String>,
    /// Target position (required if target_type is "position")
    #[serde(default)]
    pub target_position: Option<PositionSchema>,
}

/// Parameters for pickup_item tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PickupItemParams {
    /// UUID of the bot
    pub bot_id: String,
    /// Entity ID of the item to pickup
    pub item_entity_id: u32,
}

/// Parameters for use_item_on_player tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UseItemOnPlayerParams {
    /// UUID of the bot
    pub bot_id: String,
    /// Item slot index in bot's inventory
    pub item_slot: u16,
}

// ================================
// Chat Schemas
// ================================

/// Parameters for send_chat tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SendChatParams {
    /// UUID of the bot
    pub bot_id: String,
    /// Message content to send
    pub message: String,
    /// Chat type: "local" or "shout" (default: "local")
    #[serde(default)]
    pub chat_type: Option<String>,
}

/// Parameters for get_chat_history tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetChatHistoryParams {
    /// UUID of the bot
    pub bot_id: String,
}

// ================================
// Information Schemas
// ================================

/// Parameters for get_nearby_entities tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetNearbyEntitiesParams {
    /// UUID of the bot
    pub bot_id: String,
    /// Search radius (default: 1000)
    #[serde(default)]
    pub radius: Option<f32>,
    /// Types of entities to filter: "players", "monsters", "npcs", "items"
    #[serde(default)]
    pub entity_types: Option<Vec<String>>,
}

/// Parameters for get_nearby_items tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetNearbyItemsParams {
    /// UUID of the bot
    pub bot_id: String,
    /// Search radius (default: 1000)
    #[serde(default)]
    pub radius: Option<f32>,
}

/// Parameters for get_bot_skills tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetBotSkillsParams {
    /// UUID of the bot
    pub bot_id: String,
}

/// Parameters for get_bot_inventory tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetBotInventoryParams {
    /// UUID of the bot
    pub bot_id: String,
}

/// Parameters for get_player_status tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetPlayerStatusParams {
    /// UUID of the bot
    pub bot_id: String,
}

/// Parameters for teleport_to_player tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TeleportToPlayerParams {
    /// UUID of the bot
    pub bot_id: String,
}

/// Parameters for get_zone_info tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetZoneInfoParams {
    /// UUID of the bot
    pub bot_id: String,
}

/// Generate JSON schema for a type
pub fn schema_string<T: JsonSchema>() -> String {
    serde_json::to_string(&schema_for!(T)).expect("Failed to generate schema")
}

/// Generate JSON schema value for a type
pub fn schema_value<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(&schema_for!(T)).expect("Failed to generate schema")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_bot_schema() {
        let schema = schema_string::<CreateBuddyBotParams>();
        assert!(schema.contains("name"));
        assert!(schema.contains("assigned_player"));
    }
}
