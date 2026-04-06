//! Tool Definitions for LLM Bot Control
//!
//! This module provides the tool definitions that the LLM can use to control
//! the bot. These tools map to the existing REST API commands.

use serde_json::json;

use super::client::{FunctionDefinition, ToolDefinition};

/// Returns all available tool definitions for the LLM.
///
/// These tools allow the LLM to control the bot's actions in the game world.
/// Each tool has a name, description, and JSON Schema for its parameters.
pub fn get_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        follow_player_tool(),
        move_bot_tool(),
        attack_target_tool(),
        use_skill_tool(),
        send_chat_tool(),
        stop_bot_tool(),
        pickup_item_tool(),
        set_behavior_mode_tool(),
    ]
}

/// Creates the follow_player tool definition.
///
/// This tool makes the bot follow a specific player at a given distance.
fn follow_player_tool() -> ToolDefinition {
    ToolDefinition::function(
        FunctionDefinition::new(
            "follow_player",
            "Make the bot follow a specific player. Use this to stay close to your assigned player. \
             The distance is in game units - 150-250 is a comfortable following distance, \
             300-500 is a loose following distance. Always use this when you need to stay near your player."
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "player_name": {
                    "type": "string",
                    "description": "The name of the player to follow. Use your assigned player's name."
                },
                "distance": {
                    "type": "number",
                    "description": "The distance to maintain from the player in game units. \
                                    Typical values: 150-250 (close), 300-400 (medium), 500+ (far)",
                    "default": 200,
                    "minimum": 50,
                    "maximum": 1000
                }
            },
            "required": ["player_name"]
        }))
    )
}

/// Creates the move_bot tool definition.
///
/// This tool moves the bot to a specific destination coordinates.
fn move_bot_tool() -> ToolDefinition {
    ToolDefinition::function(
        FunctionDefinition::new(
            "move_bot",
            "Move the bot to specific coordinates in the game world. \
             Use this when you need to go to a specific location rather than following a player. \
             The destination is an object with x, y, z coordinates."
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "destination": {
                    "type": "object",
                    "description": "The target destination coordinates",
                    "properties": {
                        "x": {
                            "type": "number",
                            "description": "X coordinate in the game world"
                        },
                        "y": {
                            "type": "number",
                            "description": "Y coordinate (height) in the game world"
                        },
                        "z": {
                            "type": "number",
                            "description": "Z coordinate in the game world"
                        }
                    },
                    "required": ["x", "y", "z"]
                },
                "move_mode": {
                    "type": "string",
                    "description": "How to move: 'walk' for slow movement, 'run' for fast movement",
                    "enum": ["walk", "run"],
                    "default": "run"
                }
            },
            "required": ["destination"]
        }))
    )
}

/// Creates the attack_target tool definition.
///
/// This tool makes the bot attack a specific enemy by its entity ID.
fn attack_target_tool() -> ToolDefinition {
    ToolDefinition::function(
        FunctionDefinition::new(
            "attack_target",
            "Attack a specific enemy target. Use this to engage in combat with a monster or hostile entity. \
             You need the target's entity_id from the nearby monsters information. \
             The bot will continue attacking until the target is defeated or you give another command."
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "target_entity_id": {
                    "type": "integer",
                    "description": "The entity ID of the target to attack. \
                                     This is provided in the monster_nearby events and nearby_monsters context.",
                    "minimum": 0
                }
            },
            "required": ["target_entity_id"]
        }))
    )
}

/// Creates the use_skill tool definition.
///
/// This tool makes the bot use a skill on a target.
fn use_skill_tool() -> ToolDefinition {
    ToolDefinition::function(
        FunctionDefinition::new(
            "use_skill",
            "Use a skill or ability. Skills can be attacks, heals, buffs, or other special abilities. \
             The target_type determines what kind of target the skill affects. \
             Some skills target yourself (self), some target enemies (enemy), some target allies (ally). \
             For skills that need a specific target, provide the target_entity_id."
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "skill_id": {
                    "type": "integer",
                    "description": "The ID of the skill to use. Skill IDs are provided in your available_skills context.",
                    "minimum": 0
                },
                "target_type": {
                    "type": "string",
                    "description": "What type of target for this skill",
                    "enum": ["self", "enemy", "ally", "ground"]
                },
                "target_entity_id": {
                    "type": "integer",
                    "description": "The entity ID of the target (required for 'enemy' and 'ally' target types)",
                    "minimum": 0
                }
            },
            "required": ["skill_id", "target_type"]
        }))
    )
}

/// Creates the send_chat tool definition.
///
/// This tool makes the bot send a chat message.
fn send_chat_tool() -> ToolDefinition {
    ToolDefinition::function(
        FunctionDefinition::new(
            "send_chat",
            "Send a chat message to communicate with players. \
             Use this to respond when players talk to you, or to announce what you're doing. \
             Keep messages brief and natural. Don't spam chat unnecessarily."
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message to send. Keep it brief (1-2 sentences) and natural.",
                    "maxLength": 200
                },
                "chat_type": {
                    "type": "string",
                    "description": "The type of chat channel to use",
                    "enum": ["local", "party", "shout"],
                    "default": "local"
                }
            },
            "required": ["message"]
        }))
    )
}

/// Creates the stop_bot tool definition.
///
/// This tool stops all current bot actions.
fn stop_bot_tool() -> ToolDefinition {
    ToolDefinition::function(
        FunctionDefinition::new(
            "stop_bot",
            "Stop all current actions immediately. The bot will cease moving, attacking, or any other action. \
             Use this when you need to immediately halt what you're doing, \
             such as when receiving an urgent command or when something unexpected happens."
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {},
            "required": []
        }))
    )
}

/// Creates the pickup_item tool definition.
///
/// This tool makes the bot pick up an item from the ground.
fn pickup_item_tool() -> ToolDefinition {
    ToolDefinition::function(
        FunctionDefinition::new(
            "pickup_item",
            "Pick up an item from the ground. Use this to collect dropped loot or items. \
             You need the item's entity_id from the item_dropped events or nearby_items context. \
             Only pick up items that are reasonably close to you."
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "item_entity_id": {
                    "type": "integer",
                    "description": "The entity ID of the item to pick up. \
                                     This is provided in item_dropped events and nearby_items context.",
                    "minimum": 0
                }
            },
            "required": ["item_entity_id"]
        }))
    )
}

/// Creates the set_behavior_mode tool definition.
///
/// This tool changes the bot's behavior mode.
fn set_behavior_mode_tool() -> ToolDefinition {
    ToolDefinition::function(
        FunctionDefinition::new(
            "set_behavior_mode",
            "Change the bot's behavior mode. This affects how the bot automatically responds to situations. \
             'passive': Only follow, never attack first. Good for safe exploration. \
             'defensive': Attack back when attacked. Balanced mode for most situations. \
             'aggressive': Proactively attack nearby monsters. Good for grinding. \
             'support': Focus on healing and buffing allies. Good for party play."
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "description": "The behavior mode to set",
                    "enum": ["passive", "defensive", "aggressive", "support"]
                }
            },
            "required": ["mode"]
        }))
    )
}

// ============================================================================
// Argument Parsing Structures
// ============================================================================

/// Arguments for the follow_player tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct FollowPlayerArgs {
    /// The name of the player to follow
    pub player_name: String,
    /// The distance to maintain (optional, defaults to 200)
    pub distance: Option<f32>,
}

/// Arguments for the move_bot tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct MoveBotArgs {
    /// The destination coordinates
    pub destination: Destination,
    /// The movement mode (walk or run)
    pub move_mode: Option<String>,
}

/// Destination coordinates for movement.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Destination {
    /// X coordinate
    pub x: f32,
    /// Y coordinate (height)
    pub y: f32,
    /// Z coordinate
    pub z: f32,
}

/// Arguments for the attack_target tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct AttackTargetArgs {
    /// The entity ID of the target
    pub target_entity_id: u32,
}

/// Arguments for the use_skill tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct UseSkillArgs {
    /// The skill ID to use
    pub skill_id: u32,
    /// The type of target
    pub target_type: String,
    /// The entity ID of the target (optional for self-targeted skills)
    pub target_entity_id: Option<u32>,
}

/// Arguments for the send_chat tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SendChatArgs {
    /// The message to send
    pub message: String,
    /// The chat type (optional, defaults to "local")
    pub chat_type: Option<String>,
}

/// Arguments for the pickup_item tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PickupItemArgs {
    /// The entity ID of the item
    pub item_entity_id: u32,
}

/// Arguments for the set_behavior_mode tool.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SetBehaviorModeArgs {
    /// The behavior mode to set
    pub mode: String,
}

/// Parses the behavior mode string into a typed value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehaviorMode {
    /// Only follow, never attack first
    Passive,
    /// Attack back when attacked
    Defensive,
    /// Proactively attack nearby monsters
    Aggressive,
    /// Focus on healing and buffing
    Support,
}

impl std::fmt::Display for BehaviorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BehaviorMode::Passive => write!(f, "passive"),
            BehaviorMode::Defensive => write!(f, "defensive"),
            BehaviorMode::Aggressive => write!(f, "aggressive"),
            BehaviorMode::Support => write!(f, "support"),
        }
    }
}

impl std::str::FromStr for BehaviorMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "passive" => Ok(BehaviorMode::Passive),
            "defensive" => Ok(BehaviorMode::Defensive),
            "aggressive" => Ok(BehaviorMode::Aggressive),
            "support" => Ok(BehaviorMode::Support),
            _ => Err(format!("Unknown behavior mode: {}", s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_tool_definitions_count() {
        let tools = get_tool_definitions();
        assert_eq!(tools.len(), 8);
    }

    #[test]
    fn test_tool_names() {
        let tools = get_tool_definitions();
        let names: Vec<&str> = tools.iter().map(|t| t.function.name.as_str()).collect();

        assert!(names.contains(&"follow_player"));
        assert!(names.contains(&"move_bot"));
        assert!(names.contains(&"attack_target"));
        assert!(names.contains(&"use_skill"));
        assert!(names.contains(&"send_chat"));
        assert!(names.contains(&"stop_bot"));
        assert!(names.contains(&"pickup_item"));
        assert!(names.contains(&"set_behavior_mode"));
    }

    #[test]
    fn test_tool_has_parameters_schema() {
        let tools = get_tool_definitions();

        for tool in &tools {
            assert!(
                tool.function.parameters.is_some(),
                "Tool {} should have parameters schema",
                tool.function.name
            );
        }
    }

    #[test]
    fn test_follow_player_tool_serialization() {
        let tool = follow_player_tool();
        let json = serde_json::to_string(&tool).unwrap();

        assert!(json.contains("follow_player"));
        assert!(json.contains("player_name"));
        assert!(json.contains("distance"));
    }

    #[test]
    fn test_stop_bot_has_no_required_params() {
        let tool = stop_bot_tool();
        let params = tool.function.parameters.as_ref().unwrap();

        let required = params.get("required").unwrap();
        assert_eq!(required.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_behavior_mode_parsing() {
        assert_eq!(BehaviorMode::from_str("passive").unwrap(), BehaviorMode::Passive);
        assert_eq!(BehaviorMode::from_str("defensive").unwrap(), BehaviorMode::Defensive);
        assert_eq!(BehaviorMode::from_str("aggressive").unwrap(), BehaviorMode::Aggressive);
        assert_eq!(BehaviorMode::from_str("support").unwrap(), BehaviorMode::Support);
        assert!(BehaviorMode::from_str("invalid").is_err());
    }

    #[test]
    fn test_behavior_mode_display() {
        assert_eq!(BehaviorMode::Passive.to_string(), "passive");
        assert_eq!(BehaviorMode::Defensive.to_string(), "defensive");
        assert_eq!(BehaviorMode::Aggressive.to_string(), "aggressive");
        assert_eq!(BehaviorMode::Support.to_string(), "support");
    }

    #[test]
    fn test_parse_follow_player_args() {
        let json = json!({
            "player_name": "TestPlayer",
            "distance": 250.0
        });

        let args: FollowPlayerArgs = serde_json::from_value(json).unwrap();
        assert_eq!(args.player_name, "TestPlayer");
        assert_eq!(args.distance, Some(250.0));
    }

    #[test]
    fn test_parse_move_bot_args() {
        let json = json!({
            "destination": {
                "x": 100.0,
                "y": 50.0,
                "z": 200.0
            },
            "move_mode": "run"
        });

        let args: MoveBotArgs = serde_json::from_value(json).unwrap();
        assert_eq!(args.destination.x, 100.0);
        assert_eq!(args.destination.y, 50.0);
        assert_eq!(args.destination.z, 200.0);
        assert_eq!(args.move_mode, Some("run".to_string()));
    }

    #[test]
    fn test_parse_attack_target_args() {
        let json = json!({
            "target_entity_id": 12345
        });

        let args: AttackTargetArgs = serde_json::from_value(json).unwrap();
        assert_eq!(args.target_entity_id, 12345);
    }

    #[test]
    fn test_parse_send_chat_args() {
        let json = json!({
            "message": "Hello, world!",
            "chat_type": "party"
        });

        let args: SendChatArgs = serde_json::from_value(json).unwrap();
        assert_eq!(args.message, "Hello, world!");
        assert_eq!(args.chat_type, Some("party".to_string()));
    }

    #[test]
    fn test_parse_set_behavior_mode_args() {
        let json = json!({
            "mode": "aggressive"
        });

        let args: SetBehaviorModeArgs = serde_json::from_value(json).unwrap();
        assert_eq!(args.mode, "aggressive");
    }
}
