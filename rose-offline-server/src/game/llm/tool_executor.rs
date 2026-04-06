//! Tool Executor for LLM Bot Commands
//!
//! This module converts LLM tool calls into game commands that can be executed
//! by the existing command processing system. It parses JSON arguments and
//! sends commands through the channel system.

use crossbeam_channel::Sender;
use thiserror::Error;
use uuid::Uuid;

use super::client::ToolCall;
use super::tools::{
    AttackTargetArgs, BehaviorMode, Destination, FollowPlayerArgs, MoveBotArgs, PickupItemArgs,
    SendChatArgs, SetBehaviorModeArgs, UseSkillArgs,
};
use crate::game::api::LlmBotCommand;
use crate::game::api::models::Position;
use crate::game::components::BotBehaviorMode;

/// Errors that can occur during tool execution.
#[derive(Debug, Error)]
pub enum ToolExecutionError {
    /// Failed to parse the tool arguments as JSON
    #[error("Failed to parse arguments: {0}")]
    ParseError(#[source] serde_json::Error),

    /// Unknown tool name
    #[error("Unknown tool: {0}")]
    UnknownTool(String),

    /// Failed to send command through channel
    #[error("Failed to send command: {0}")]
    SendError(String),

    /// Invalid arguments for the tool
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),
}

/// Result of executing a tool call.
#[derive(Debug)]
pub struct ToolExecutionResult {
    /// The tool name that was executed
    pub tool_name: String,
    /// Whether execution was successful
    pub success: bool,
    /// Optional message about the execution
    pub message: Option<String>,
}

/// Executes a single tool call and sends the corresponding command.
///
/// This function parses the tool call arguments, converts them to the
/// appropriate `LlmBotCommand` variant, and sends the command through
/// the provided channel.
///
/// # Arguments
///
/// * `bot_id` - The UUID of the bot executing the tool
/// * `tool_call` - The tool call from the LLM
/// * `command_sender` - Channel sender for game commands
///
/// # Returns
///
/// A result indicating success or the error that occurred.
pub fn execute_tool_call(
    bot_id: Uuid,
    tool_call: &ToolCall,
    command_sender: &Sender<LlmBotCommand>,
) -> Result<ToolExecutionResult, ToolExecutionError> {
    let tool_name = tool_call.function.name.clone();
    let arguments = &tool_call.function.arguments;

    log::info!(
        "[TOOL_DEBUG] Executing tool '{}' for bot {}: {}",
        tool_name,
        bot_id,
        arguments
    );

    let command = match tool_name.as_str() {
        "follow_player" => {
            let cmd = parse_follow_player(bot_id, arguments)?;
            log::info!("[TOOL_DEBUG] Parsed follow_player: {:?}", cmd);
            cmd
        },
        "move_bot" => {
            let cmd = parse_move_bot(bot_id, arguments)?;
            log::info!("[TOOL_DEBUG] Parsed move_bot: {:?}", cmd);
            cmd
        },
        "attack_target" => {
            let cmd = parse_attack_target(bot_id, arguments)?;
            log::info!("[TOOL_DEBUG] Parsed attack_target: {:?}", cmd);
            cmd
        },
        "use_skill" => {
            let cmd = parse_use_skill(bot_id, arguments)?;
            log::info!("[TOOL_DEBUG] Parsed use_skill: {:?}", cmd);
            cmd
        },
        "send_chat" => {
            let cmd = parse_send_chat(bot_id, arguments)?;
            log::info!("[TOOL_DEBUG] Parsed send_chat: {:?}", cmd);
            cmd
        },
        "stop_bot" => {
            log::info!("[TOOL_DEBUG] Parsed stop_bot");
            LlmBotCommand::Stop { bot_id }
        },
        "pickup_item" => {
            let cmd = parse_pickup_item(bot_id, arguments)?;
            log::info!("[TOOL_DEBUG] Parsed pickup_item: {:?}", cmd);
            cmd
        },
        "set_behavior_mode" => {
            let cmd = parse_set_behavior_mode(bot_id, arguments)?;
            log::info!("[TOOL_DEBUG] Parsed set_behavior_mode: {:?}", cmd);
            cmd
        },
        _ => {
            log::error!("[TOOL_DEBUG] Unknown tool: {}", tool_name);
            return Err(ToolExecutionError::UnknownTool(tool_name.clone()));
        }
    };

    // Send the command through the channel
    log::info!("[TOOL_DEBUG] Sending command to channel for bot {}", bot_id);
    command_sender
        .send(command)
        .map_err(|e: crossbeam_channel::SendError<LlmBotCommand>| {
            log::error!("[TOOL_DEBUG] Failed to send command to channel: {}", e);
            ToolExecutionError::SendError(e.to_string())
        })?;

    log::info!("[TOOL_DEBUG] Successfully sent command for tool '{}'", tool_name);

    Ok(ToolExecutionResult {
        tool_name,
        success: true,
        message: Some(format!("Command sent successfully")),
    })
}

/// Executes multiple tool calls in sequence.
///
/// This function iterates through a list of tool calls and executes each one.
/// If a tool call fails, it logs the error and continues with the next one.
///
/// # Arguments
///
/// * `bot_id` - The UUID of the bot executing the tools
/// * `tool_calls` - The list of tool calls from the LLM
/// * `command_sender` - Channel sender for game commands
///
/// # Returns
///
/// A vector of results for each tool call.
pub fn execute_multiple_tool_calls(
    bot_id: Uuid,
    tool_calls: &[ToolCall],
    command_sender: &Sender<LlmBotCommand>,
) -> Vec<Result<ToolExecutionResult, ToolExecutionError>> {
    tool_calls
        .iter()
        .map(|tool_call| execute_tool_call(bot_id, tool_call, command_sender))
        .collect()
}

// ============================================================================
// Tool Parsing Functions
// ============================================================================

/// Parses follow_player tool arguments.
fn parse_follow_player(
    bot_id: Uuid,
    arguments: &str,
) -> Result<LlmBotCommand, ToolExecutionError> {
    let args: FollowPlayerArgs = serde_json::from_str(arguments).map_err(ToolExecutionError::ParseError)?;

    let distance = args.distance.unwrap_or(200.0);

    Ok(LlmBotCommand::Follow {
        bot_id,
        player_name: args.player_name,
        distance,
    })
}

/// Parses move_bot tool arguments.
fn parse_move_bot(bot_id: Uuid, arguments: &str) -> Result<LlmBotCommand, ToolExecutionError> {
    let args: MoveBotArgs = serde_json::from_str(arguments).map_err(ToolExecutionError::ParseError)?;

    let destination = Position::new(args.destination.x, args.destination.y, args.destination.z);
    let move_mode = args.move_mode.unwrap_or_else(|| "run".to_string());

    Ok(LlmBotCommand::Move {
        bot_id,
        destination,
        target_entity: None,
        move_mode,
    })
}

/// Parses attack_target tool arguments.
fn parse_attack_target(
    bot_id: Uuid,
    arguments: &str,
) -> Result<LlmBotCommand, ToolExecutionError> {
    let args: AttackTargetArgs = serde_json::from_str(arguments).map_err(ToolExecutionError::ParseError)?;

    Ok(LlmBotCommand::Attack {
        bot_id,
        target_entity_id: args.target_entity_id,
    })
}

/// Parses use_skill tool arguments.
fn parse_use_skill(bot_id: Uuid, arguments: &str) -> Result<LlmBotCommand, ToolExecutionError> {
    let args: UseSkillArgs = serde_json::from_str(arguments).map_err(ToolExecutionError::ParseError)?;

    // Determine target based on target_type and target_entity_id
    let (target_entity_id, target_position) = match args.target_type.as_str() {
        "self" => (None, None),
        "enemy" | "ally" => (args.target_entity_id, None),
        "ground" => (None, None), // Would need position parsing if ground targeting is needed
        _ => (None, None),
    };

    Ok(LlmBotCommand::UseSkill {
        bot_id,
        skill_id: args.skill_id as u16,
        target_entity_id,
        target_position,
    })
}

/// Parses send_chat tool arguments.
fn parse_send_chat(bot_id: Uuid, arguments: &str) -> Result<LlmBotCommand, ToolExecutionError> {
    let args: SendChatArgs = serde_json::from_str(arguments).map_err(ToolExecutionError::ParseError)?;

    let chat_type = args.chat_type.unwrap_or_else(|| "local".to_string());

    Ok(LlmBotCommand::Chat {
        bot_id,
        message: args.message,
        chat_type,
    })
}

/// Parses pickup_item tool arguments.
fn parse_pickup_item(bot_id: Uuid, arguments: &str) -> Result<LlmBotCommand, ToolExecutionError> {
    let args: PickupItemArgs = serde_json::from_str(arguments).map_err(ToolExecutionError::ParseError)?;

    Ok(LlmBotCommand::Pickup {
        bot_id,
        item_entity_id: args.item_entity_id,
    })
}

/// Parses set_behavior_mode tool arguments.
fn parse_set_behavior_mode(
    bot_id: Uuid,
    arguments: &str,
) -> Result<LlmBotCommand, ToolExecutionError> {
    let args: SetBehaviorModeArgs = serde_json::from_str(arguments).map_err(ToolExecutionError::ParseError)?;

    let mode = parse_behavior_mode(&args.mode)?;

    Ok(LlmBotCommand::SetBehaviorMode { bot_id, mode })
}

/// Parses a behavior mode string into the component enum.
pub fn parse_behavior_mode(mode: &str) -> Result<BotBehaviorMode, ToolExecutionError> {
    match mode.to_lowercase().as_str() {
        "passive" => Ok(BotBehaviorMode::Passive),
        "defensive" => Ok(BotBehaviorMode::Defensive),
        "aggressive" => Ok(BotBehaviorMode::Aggressive),
        "support" => Ok(BotBehaviorMode::Support),
        _ => Err(ToolExecutionError::InvalidArguments(format!(
            "Unknown behavior mode: {}",
            mode
        ))),
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Converts a Destination from tools.rs to Position for commands.
pub fn destination_to_position(dest: &Destination) -> Position {
    Position::new(dest.x, dest.y, dest.z)
}

/// Validates that a skill ID is within reasonable bounds.
pub fn is_valid_skill_id(skill_id: u16) -> bool {
    skill_id > 0 && skill_id < 10000 // Assuming skill IDs are 1-9999
}

/// Validates that an entity ID is valid.
pub fn is_valid_entity_id(entity_id: u32) -> bool {
    entity_id > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_tool_call(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "test_id".to_string(),
            call_type: "function".to_string(),
            function: super::super::client::FunctionCall {
                name: name.to_string(),
                arguments: serde_json::to_string(&arguments).unwrap(),
            },
        }
    }

    #[test]
    fn test_parse_follow_player() {
        let bot_id = Uuid::new_v4();
        let args = json!({
            "player_name": "TestPlayer",
            "distance": 250.0
        });
        let tool_call = create_test_tool_call("follow_player", args);

        let (sender, receiver) = crossbeam_channel::unbounded();
        let result = execute_tool_call(bot_id, &tool_call, &sender);

        assert!(result.is_ok());
        let cmd = receiver.try_recv().unwrap();
        match cmd {
            LlmBotCommand::Follow { bot_id: id, player_name, distance } => {
                assert_eq!(id, bot_id);
                assert_eq!(player_name, "TestPlayer");
                assert!((distance - 250.0).abs() < f32::EPSILON);
            }
            _ => panic!("Expected Follow command"),
        }
    }

    #[test]
    fn test_parse_follow_player_default_distance() {
        let bot_id = Uuid::new_v4();
        let args = json!({
            "player_name": "TestPlayer"
        });
        let tool_call = create_test_tool_call("follow_player", args);

        let (sender, receiver) = crossbeam_channel::unbounded();
        let result = execute_tool_call(bot_id, &tool_call, &sender);

        assert!(result.is_ok());
        let cmd = receiver.try_recv().unwrap();
        match cmd {
            LlmBotCommand::Follow { distance, .. } => {
                assert!((distance - 200.0).abs() < f32::EPSILON); // Default distance
            }
            _ => panic!("Expected Follow command"),
        }
    }

    #[test]
    fn test_parse_move_bot() {
        let bot_id = Uuid::new_v4();
        let args = json!({
            "destination": {
                "x": 100.0,
                "y": 50.0,
                "z": 200.0
            },
            "move_mode": "run"
        });
        let tool_call = create_test_tool_call("move_bot", args);

        let (sender, receiver) = crossbeam_channel::unbounded();
        let result = execute_tool_call(bot_id, &tool_call, &sender);

        assert!(result.is_ok());
        let cmd = receiver.try_recv().unwrap();
        match cmd {
            LlmBotCommand::Move { bot_id: id, destination, move_mode, .. } => {
                assert_eq!(id, bot_id);
                assert!((destination.x - 100.0).abs() < f32::EPSILON);
                assert!((destination.y - 50.0).abs() < f32::EPSILON);
                assert!((destination.z - 200.0).abs() < f32::EPSILON);
                assert_eq!(move_mode, "run");
            }
            _ => panic!("Expected Move command"),
        }
    }

    #[test]
    fn test_parse_attack_target() {
        let bot_id = Uuid::new_v4();
        let args = json!({
            "target_entity_id": 12345
        });
        let tool_call = create_test_tool_call("attack_target", args);

        let (sender, receiver) = crossbeam_channel::unbounded();
        let result = execute_tool_call(bot_id, &tool_call, &sender);

        assert!(result.is_ok());
        let cmd = receiver.try_recv().unwrap();
        match cmd {
            LlmBotCommand::Attack { bot_id: id, target_entity_id } => {
                assert_eq!(id, bot_id);
                assert_eq!(target_entity_id, 12345);
            }
            _ => panic!("Expected Attack command"),
        }
    }

    #[test]
    fn test_parse_use_skill_self() {
        let bot_id = Uuid::new_v4();
        let args = json!({
            "skill_id": 5,
            "target_type": "self"
        });
        let tool_call = create_test_tool_call("use_skill", args);

        let (sender, receiver) = crossbeam_channel::unbounded();
        let result = execute_tool_call(bot_id, &tool_call, &sender);

        assert!(result.is_ok());
        let cmd = receiver.try_recv().unwrap();
        match cmd {
            LlmBotCommand::UseSkill { bot_id: id, skill_id, target_entity_id, .. } => {
                assert_eq!(id, bot_id);
                assert_eq!(skill_id, 5);
                assert!(target_entity_id.is_none());
            }
            _ => panic!("Expected UseSkill command"),
        }
    }

    #[test]
    fn test_parse_use_skill_enemy() {
        let bot_id = Uuid::new_v4();
        let args = json!({
            "skill_id": 10,
            "target_type": "enemy",
            "target_entity_id": 999
        });
        let tool_call = create_test_tool_call("use_skill", args);

        let (sender, receiver) = crossbeam_channel::unbounded();
        let result = execute_tool_call(bot_id, &tool_call, &sender);

        assert!(result.is_ok());
        let cmd = receiver.try_recv().unwrap();
        match cmd {
            LlmBotCommand::UseSkill { bot_id: id, skill_id, target_entity_id, .. } => {
                assert_eq!(id, bot_id);
                assert_eq!(skill_id, 10);
                assert_eq!(target_entity_id, Some(999));
            }
            _ => panic!("Expected UseSkill command"),
        }
    }

    #[test]
    fn test_parse_send_chat() {
        let bot_id = Uuid::new_v4();
        let args = json!({
            "message": "Hello, world!",
            "chat_type": "local"
        });
        let tool_call = create_test_tool_call("send_chat", args);

        let (sender, receiver) = crossbeam_channel::unbounded();
        let result = execute_tool_call(bot_id, &tool_call, &sender);

        assert!(result.is_ok());
        let cmd = receiver.try_recv().unwrap();
        match cmd {
            LlmBotCommand::Chat { bot_id: id, message, chat_type } => {
                assert_eq!(id, bot_id);
                assert_eq!(message, "Hello, world!");
                assert_eq!(chat_type, "local");
            }
            _ => panic!("Expected Chat command"),
        }
    }

    #[test]
    fn test_parse_stop_bot() {
        let bot_id = Uuid::new_v4();
        let args = json!({});
        let tool_call = create_test_tool_call("stop_bot", args);

        let (sender, receiver) = crossbeam_channel::unbounded();
        let result = execute_tool_call(bot_id, &tool_call, &sender);

        assert!(result.is_ok());
        let cmd = receiver.try_recv().unwrap();
        match cmd {
            LlmBotCommand::Stop { bot_id: id } => {
                assert_eq!(id, bot_id);
            }
            _ => panic!("Expected Stop command"),
        }
    }

    #[test]
    fn test_parse_pickup_item() {
        let bot_id = Uuid::new_v4();
        let args = json!({
            "item_entity_id": 54321
        });
        let tool_call = create_test_tool_call("pickup_item", args);

        let (sender, receiver) = crossbeam_channel::unbounded();
        let result = execute_tool_call(bot_id, &tool_call, &sender);

        assert!(result.is_ok());
        let cmd = receiver.try_recv().unwrap();
        match cmd {
            LlmBotCommand::Pickup { bot_id: id, item_entity_id } => {
                assert_eq!(id, bot_id);
                assert_eq!(item_entity_id, 54321);
            }
            _ => panic!("Expected Pickup command"),
        }
    }

    #[test]
    fn test_parse_set_behavior_mode() {
        let bot_id = Uuid::new_v4();
        let args = json!({
            "mode": "aggressive"
        });
        let tool_call = create_test_tool_call("set_behavior_mode", args);

        let (sender, receiver) = crossbeam_channel::unbounded();
        let result = execute_tool_call(bot_id, &tool_call, &sender);

        assert!(result.is_ok());
        let cmd = receiver.try_recv().unwrap();
        match cmd {
            LlmBotCommand::SetBehaviorMode { bot_id: id, mode } => {
                assert_eq!(id, bot_id);
                assert_eq!(mode, BotBehaviorMode::Aggressive);
            }
            _ => panic!("Expected SetBehaviorMode command"),
        }
    }

    #[test]
    fn test_unknown_tool() {
        let bot_id = Uuid::new_v4();
        let args = json!({});
        let tool_call = create_test_tool_call("unknown_tool", args);

        let (sender, _) = crossbeam_channel::unbounded();
        let result = execute_tool_call(bot_id, &tool_call, &sender);

        assert!(result.is_err());
        match result.unwrap_err() {
            ToolExecutionError::UnknownTool(name) => {
                assert_eq!(name, "unknown_tool");
            }
            _ => panic!("Expected UnknownTool error"),
        }
    }

    #[test]
    fn test_invalid_arguments() {
        let bot_id = Uuid::new_v4();
        let args = json!({
            "invalid_field": "invalid_value"
        });
        let tool_call = create_test_tool_call("follow_player", args);

        let (sender, _) = crossbeam_channel::unbounded();
        let result = execute_tool_call(bot_id, &tool_call, &sender);

        // Should fail because player_name is required but not provided
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_multiple_tool_calls() {
        let bot_id = Uuid::new_v4();
        let tool_calls = vec![
            create_test_tool_call("stop_bot", json!({})),
            create_test_tool_call("send_chat", json!({
                "message": "Hello!"
            })),
        ];

        let (sender, receiver) = crossbeam_channel::unbounded();
        let results = execute_multiple_tool_calls(bot_id, &tool_calls, &sender);

        assert_eq!(results.len(), 2);
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());

        // Should have received 2 commands
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn test_parse_behavior_mode() {
        assert_eq!(parse_behavior_mode("passive").unwrap(), BotBehaviorMode::Passive);
        assert_eq!(parse_behavior_mode("defensive").unwrap(), BotBehaviorMode::Defensive);
        assert_eq!(parse_behavior_mode("aggressive").unwrap(), BotBehaviorMode::Aggressive);
        assert_eq!(parse_behavior_mode("support").unwrap(), BotBehaviorMode::Support);

        // Case insensitive
        assert_eq!(parse_behavior_mode("PASSIVE").unwrap(), BotBehaviorMode::Passive);
        assert_eq!(parse_behavior_mode("Aggressive").unwrap(), BotBehaviorMode::Aggressive);

        // Invalid
        assert!(parse_behavior_mode("invalid").is_err());
    }

    #[test]
    fn test_is_valid_skill_id() {
        assert!(!is_valid_skill_id(0)); // 0 is not valid
        assert!(is_valid_skill_id(1));
        assert!(is_valid_skill_id(100));
        assert!(is_valid_skill_id(9999));
        assert!(!is_valid_skill_id(10000)); // Too high
    }

    #[test]
    fn test_is_valid_entity_id() {
        assert!(!is_valid_entity_id(0)); // 0 is not valid
        assert!(is_valid_entity_id(1));
        assert!(is_valid_entity_id(100));
        assert!(is_valid_entity_id(u32::MAX));
    }
}
