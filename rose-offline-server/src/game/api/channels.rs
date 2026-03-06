//! Command channels for communication between API and game world
//!
//! This module provides the command types and channel infrastructure for
//! sending commands from the REST API to the game world.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::Entity;
use crossbeam_channel::{Receiver, Sender};
use parking_lot::RwLock;
use uuid::Uuid;

use super::models::Position;
use super::state::BotInfo;
use crate::game::components::BotBehaviorMode;

/// Response from a DeleteBot command
#[derive(Debug, Clone)]
pub struct DeleteBotResponse {
    /// Whether the deletion was successful
    pub success: bool,
    /// Error message if deletion failed
    pub error: Option<String>,
}

/// Response from a GetBotContext command
#[derive(Debug, Clone)]
pub struct GetBotContextResponse {
    /// Whether the query was successful
    pub success: bool,
    /// Error message if query failed
    pub error: Option<String>,
    /// Nearby entities (monsters, players, items)
    pub entities: Vec<crate::game::api::models::NearbyEntity>,
}

/// Response from a GetBotSkills command
#[derive(Debug, Clone)]
pub struct GetBotSkillsResponse {
    /// Whether the query was successful
    pub success: bool,
    /// Error message if query failed
    pub error: Option<String>,
    /// Skills the bot has
    pub skills: Vec<crate::game::api::models::SkillInfo>,
}

/// Response from a GetChatHistory command
#[derive(Debug, Clone)]
pub struct GetChatHistoryResponse {
    /// Whether the query was successful
    pub success: bool,
    /// Error message if query failed
    pub error: Option<String>,
    /// Chat messages
    pub messages: Vec<crate::game::api::models::ChatMessage>,
}

/// Bot summary data for list responses
#[derive(Debug, Clone)]
pub struct BotSummaryData {
    /// Bot ID
    pub bot_id: Uuid,
    /// Bot name
    pub name: String,
    /// Bot level
    pub level: u16,
    /// Health points
    pub health: crate::game::api::models::VitalPoints,
    /// Position in zone
    pub position: crate::game::api::models::ZonePosition,
    /// Assigned player
    pub assigned_player: Option<String>,
    /// Status string
    pub status: String,
}

/// Response from a GetBotList command
#[derive(Debug, Clone)]
pub struct GetBotListResponse {
    /// Whether the query was successful
    pub success: bool,
    /// Error message if query failed
    pub error: Option<String>,
    /// List of bots with their data
    pub bots: Vec<BotSummaryData>,
}

/// Commands that can be sent to control LLM buddy bots
#[derive(Clone)]
pub enum LlmBotCommand {
    /// Create a new bot
    CreateBot {
        /// Unique bot ID
        bot_id: Uuid,
        /// Bot name
        name: String,
        /// Level for the bot
        level: u16,
        /// Class/build type
        class: String,
        /// Gender (male or female)
        gender: Option<String>,
        /// Player name to assign bot to
        assigned_player: String,
    },

    /// Delete a bot
    DeleteBot {
        /// Bot ID to delete
        bot_id: Uuid,
        /// Response channel to send confirmation back to API handler
        response_tx: Sender<DeleteBotResponse>,
    },

    /// Move bot to a position
    Move {
        /// Bot ID
        bot_id: Uuid,
        /// Target position
        destination: Position,
        /// Optional entity to follow
        target_entity: Option<u32>,
        /// Movement mode (walk/run)
        move_mode: String,
    },

    /// Follow a player
    Follow {
        /// Bot ID
        bot_id: Uuid,
        /// Player name to follow
        player_name: String,
        /// Distance to maintain
        distance: f32,
    },

    /// Attack a target
    Attack {
        /// Bot ID
        bot_id: Uuid,
        /// Target entity ID
        target_entity_id: u32,
    },

    /// Use a skill
    UseSkill {
        /// Bot ID
        bot_id: Uuid,
        /// Skill ID to use
        skill_id: u16,
        /// Target entity ID (if targeting entity)
        target_entity_id: Option<u32>,
        /// Target position (if targeting ground)
        target_position: Option<Position>,
    },

    /// Send a chat message
    Chat {
        /// Bot ID
        bot_id: Uuid,
        /// Message content
        message: String,
        /// Chat type (local, shout)
        chat_type: String,
    },

    /// Stop current action
    Stop {
        /// Bot ID
        bot_id: Uuid,
    },

    /// Sit down
    Sit {
        /// Bot ID
        bot_id: Uuid,
    },

    /// Stand up
    Stand {
        /// Bot ID
        bot_id: Uuid,
    },

    /// Pickup an item
    Pickup {
        /// Bot ID
        bot_id: Uuid,
        /// Item entity ID
        item_entity_id: u32,
    },

    /// Perform an emote
    Emote {
        /// Bot ID
        bot_id: Uuid,
        /// Emote ID
        emote_id: u16,
        /// Is stop emote
        is_stop: bool,
    },

    /// Get bot context (nearby threats and items)
    GetBotContext {
        /// Bot ID
        bot_id: Uuid,
        /// Response channel to send context back to API handler
        response_tx: Sender<GetBotContextResponse>,
    },

    /// Get bot skills
    GetBotSkills {
        /// Bot ID
        bot_id: Uuid,
        /// Response channel to send skills back to API handler
        response_tx: Sender<GetBotSkillsResponse>,
    },

    /// Get chat history for a bot
    GetChatHistory {
        /// Bot ID
        bot_id: Uuid,
        /// Response channel to send chat history back to API handler
        response_tx: Sender<GetChatHistoryResponse>,
    },

    /// Get list of all bots with their current status
    GetBotList {
        /// Response channel to send bot list back to API handler
        response_tx: Sender<GetBotListResponse>,
    },
    /// Get bot inventory
    GetBotInventory {
        /// Bot ID
        bot_id: Uuid,
        /// Response channel
        response_tx: Sender<GetBotInventoryResponse>,
    },
    /// Get player status
    GetPlayerStatus {
        /// Bot ID (to find the assigned player)
        bot_id: Uuid,
        /// Response channel
        response_tx: Sender<GetPlayerStatusResponse>,
    },
    /// Teleport bot to player
    TeleportToPlayer {
        /// Bot ID
        bot_id: Uuid,
    },
    /// Use an item
    UseItem {
        /// Bot ID
        bot_id: Uuid,
        /// Item slot index
        item_slot: u16,
        /// Target entity ID
        target_entity_id: Option<u32>,
    },
    /// Set bot behavior mode
    SetBehaviorMode {
        /// Bot ID
        bot_id: Uuid,
        /// Behavior mode
        mode: BotBehaviorMode,
    },
    /// Get zone info
    GetZoneInfo {
        /// Bot ID
        bot_id: Uuid,
        /// Response channel
        response_tx: Sender<GetZoneInfoResponse>,
    },
}

#[derive(Debug, Clone)]
pub struct GetBotInventoryResponse {
    pub success: bool,
    pub error: Option<String>,
    pub items: Vec<crate::game::api::models::InventoryItemInfo>,
}

#[derive(Debug, Clone)]
pub struct GetPlayerStatusResponse {
    pub success: bool,
    pub error: Option<String>,
    pub status: Option<crate::game::api::models::PlayerStatus>,
}

#[derive(Debug, Clone)]
pub struct GetZoneInfoResponse {
    pub success: bool,
    pub error: Option<String>,
    pub zone_name: String,
    pub zone_id: u16,
    pub recommended_level_min: u16,
    pub recommended_level_max: u16,
}

/// Manager for LLM bot commands and entity tracking
///
/// This struct provides thread-safe access to bot commands and entity mappings.
/// It's designed to be shared between the API server and the game world.
pub struct LlmBotManager {
    /// Sender for bot commands
    command_sender: Sender<LlmBotCommand>,
    /// Receiver for bot commands (used by game world)
    command_receiver: Receiver<LlmBotCommand>,
    /// Map of bot IDs to bot information
    bots: Arc<RwLock<HashMap<Uuid, BotInfo>>>,
}

impl LlmBotManager {
    /// Create a new LLM bot manager
    pub fn new() -> Self {
        let (command_sender, command_receiver) = crossbeam_channel::unbounded();
        log::info!("LlmBotManager::new() - created unbounded channel");
        Self {
            command_sender,
            command_receiver,
            bots: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get a sender for sending commands
    pub fn command_sender(&self) -> Sender<LlmBotCommand> {
        let sender = self.command_sender.clone();
        log::info!("LlmBotManager::command_sender() - cloned sender, receiver is_empty: {}", self.command_receiver.is_empty());
        sender
    }

    /// Get the command receiver (for game world to consume)
    pub fn command_receiver(&self) -> &Receiver<LlmBotCommand> {
        log::info!("LlmBotManager::command_receiver() - returning receiver reference, is_empty: {}", self.command_receiver.is_empty());
        &self.command_receiver
    }

    /// Try to receive a command without blocking
    pub fn try_recv_command(&self) -> Option<LlmBotCommand> {
        self.command_receiver.try_recv().ok()
    }

    /// Receive a command, blocking if none available
    pub fn recv_command(&self) -> Result<LlmBotCommand, crossbeam_channel::RecvError> {
        self.command_receiver.recv()
    }

    /// Register a bot with placeholder entity (called by API before game thread creates entity)
    pub fn register_bot(&self, bot_id: Uuid, name: String, assigned_player: Option<String>) {
        let info = BotInfo::new_placeholder(name, assigned_player);
        self.bots.write().insert(bot_id, info);
    }

    /// Register a bot entity (called by game thread after entity creation)
    pub fn register_bot_entity(&self, bot_id: Uuid, entity: Entity) {
        // If bot already exists (registered by API), update the entity
        // Otherwise create a new entry
        let mut bots = self.bots.write();
        if let Some(info) = bots.get_mut(&bot_id) {
            info.update_entity(entity);
        } else {
            // Shouldn't happen normally, but handle it gracefully
            log::warn!("Registering bot entity {} that wasn't pre-registered by API", bot_id);
            bots.insert(bot_id, BotInfo::new(entity, String::new(), None, 1, String::new()));
        }
    }

    /// Update bot info after creation (called by game thread)
    pub fn update_bot_info(
        &self,
        bot_id: Uuid,
        entity: Entity,
        name: String,
        assigned_player: Option<String>,
        level: u16,
        class: String,
    ) {
        let info = BotInfo::new(entity, name, assigned_player, level, class);
        self.bots.write().insert(bot_id, info);
    }

    /// Unregister a bot entity
    pub fn unregister_bot(&self, bot_id: &Uuid) {
        self.bots.write().remove(bot_id);
    }

    /// Get a bot's entity
    pub fn get_bot_entity(&self, bot_id: &Uuid) -> Option<Entity> {
        self.bots.read().get(bot_id).map(|info| info.entity)
    }

    /// Get a bot's info
    pub fn get_bot_info(&self, bot_id: &Uuid) -> Option<BotInfo> {
        self.bots.read().get(bot_id).cloned()
    }

    /// Check if a bot exists
    pub fn bot_exists(&self, bot_id: &Uuid) -> bool {
        self.bots.read().contains_key(bot_id)
    }

    /// List all registered bot IDs
    pub fn list_bots(&self) -> Vec<Uuid> {
        self.bots.read().keys().copied().collect()
    }

    /// Get the bots map for read access
    pub fn bots_map(&self) -> Arc<RwLock<HashMap<Uuid, BotInfo>>> {
        Arc::clone(&self.bots)
    }

    /// Send a command
    pub fn send_command(&self, command: LlmBotCommand) -> Result<(), String> {
        self.command_sender
            .send(command)
            .map_err(|e| format!("Failed to send command: {}", e))
    }
}

impl Default for LlmBotManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manager_creation() {
        let manager = LlmBotManager::new();
        assert!(manager.list_bots().is_empty());
    }

    #[test]
    fn test_register_and_get_bot() {
        let manager = LlmBotManager::new();
        let bot_id = Uuid::new_v4();
        let entity = Entity::from_raw(42);

        manager.register_bot(bot_id, "TestBot".to_string(), Some("Player1".to_string()));
        manager.register_bot_entity(bot_id, entity);

        assert!(manager.bot_exists(&bot_id));
        assert_eq!(manager.get_bot_entity(&bot_id), Some(entity));
        
        let info = manager.get_bot_info(&bot_id).unwrap();
        assert_eq!(info.name, "TestBot");
        assert_eq!(info.assigned_player, Some("Player1".to_string()));
    }

    #[test]
    fn test_unregister_bot() {
        let manager = LlmBotManager::new();
        let bot_id = Uuid::new_v4();
        let entity = Entity::from_raw(42);

        manager.register_bot(bot_id, "TestBot".to_string(), None);
        manager.register_bot_entity(bot_id, entity);
        manager.unregister_bot(&bot_id);

        assert!(!manager.bot_exists(&bot_id));
    }

    #[test]
    fn test_send_and_receive_command() {
        let manager = LlmBotManager::new();
        let bot_id = Uuid::new_v4();

        let command = LlmBotCommand::Stop { bot_id };
        manager.send_command(command.clone()).unwrap();

        let received = manager.try_recv_command();
        assert!(received.is_some());
    }
}
