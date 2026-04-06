//! Shared state for the LLM Buddy Bot REST API

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::Entity;
use crossbeam_channel::Sender;
use parking_lot::RwLock;
use uuid::Uuid;

use super::channels::LlmBotCommand;

/// Information about a registered bot
#[derive(Debug, Clone)]
pub struct BotInfo {
    /// The Bevy entity for this bot
    pub entity: Entity,
    /// Bot name
    pub name: String,
    /// Player name this bot is assigned to
    pub assigned_player: Option<String>,
    /// Bot level
    pub level: u16,
    /// Bot class/build
    pub class: String,
}

impl BotInfo {
    /// Create a new BotInfo with placeholder entity
    pub fn new_placeholder(name: String, assigned_player: Option<String>) -> Self {
        Self {
            entity: Entity::from_raw_u32(0).unwrap(),
            name,
            assigned_player,
            level: 1,
            class: String::new(),
        }
    }

    /// Create a new BotInfo with full data
    pub fn new(
        entity: Entity,
        name: String,
        assigned_player: Option<String>,
        level: u16,
        class: String,
    ) -> Self {
        Self {
            entity,
            name,
            assigned_player,
            level,
            class,
        }
    }

    /// Update the entity ID (called when game thread creates the actual entity)
    pub fn update_entity(&mut self, entity: Entity) {
        self.entity = entity;
    }
}

/// Shared state for the API server
///
/// This struct holds all the state needed by the API handlers to communicate
/// with the game server. It uses thread-safe primitives to allow concurrent
/// access from the async HTTP handlers.
#[derive(Clone)]
pub struct ApiState {
    /// Channel sender for bot commands to the game world
    pub command_sender: Sender<LlmBotCommand>,
    /// Map of bot IDs to bot information (shared with LlmBotManager)
    pub bots: Arc<RwLock<HashMap<Uuid, BotInfo>>>,
}

impl ApiState {
    /// Create a new API state with shared bots map from LlmBotManager
    ///
    /// This is the only way to create an ApiState - it MUST use the shared bots map
    /// from LlmBotManager to ensure consistency between the API handlers and game thread.
    pub fn new(
        command_sender: Sender<LlmBotCommand>,
        bots: Arc<RwLock<HashMap<Uuid, BotInfo>>>,
    ) -> Self {
        log::info!("ApiState::new created with bots map pointer: {:p}", Arc::as_ptr(&bots));
        Self {
            command_sender,
            bots,
        }
    }

    /// Register a new bot with the state (with placeholder entity)
    pub fn register_bot(&self, bot_id: Uuid, name: String, assigned_player: Option<String>) {
        let info = BotInfo::new_placeholder(name.clone(), assigned_player.clone());
        self.bots.write().insert(bot_id, info);
        log::info!("ApiState.register_bot: {} ({}) - map now has {} bots, pointer: {:p}",
            bot_id, name, self.bots.read().len(), Arc::as_ptr(&self.bots));
    }

    /// Register a new bot with full information
    pub fn register_bot_full(
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

    /// Update bot's entity ID (called when game thread creates the entity)
    pub fn update_bot_entity(&self, bot_id: Uuid, entity: Entity) {
        if let Some(info) = self.bots.write().get_mut(&bot_id) {
            info.update_entity(entity);
        }
    }

    /// Unregister a bot from the state
    pub fn unregister_bot(&self, bot_id: &Uuid) {
        self.bots.write().remove(bot_id);
    }

    /// Get the entity for a bot ID
    pub fn get_bot_entity(&self, bot_id: &Uuid) -> Option<Entity> {
        self.bots.read().get(bot_id).map(|info| info.entity)
    }

    /// Get the bot info for a bot ID
    pub fn get_bot_info(&self, bot_id: &Uuid) -> Option<BotInfo> {
        self.bots.read().get(bot_id).cloned()
    }

    /// Get the name for a bot ID
    pub fn get_bot_name(&self, bot_id: &Uuid) -> Option<String> {
        self.bots.read().get(bot_id).map(|info| info.name.clone())
    }

    /// List all registered bot IDs
    pub fn list_bots(&self) -> Vec<Uuid> {
        let bots: Vec<Uuid> = self.bots.read().keys().copied().collect();
        log::debug!("list_bots: returning {} bots", bots.len());
        bots
    }

    /// Check if a bot exists
    pub fn bot_exists(&self, bot_id: &Uuid) -> bool {
        let bots = self.bots.read();
        let exists = bots.contains_key(bot_id);
        log::info!("bot_exists check for {}: {} (map has {} bots, pointer: {:p})",
            bot_id, exists, bots.len(), Arc::as_ptr(&self.bots));
        if !exists && !bots.is_empty() {
            log::info!("  Keys in map: {:?}", bots.keys().take(5).collect::<Vec<_>>());
        }
        exists
    }

    /// Send a command to the game world
    pub fn send_command(&self, command: LlmBotCommand) -> Result<(), String> {
        log::info!("ApiState::send_command - sending command through channel");
        let result = self.command_sender
            .send(command)
            .map_err(|e| format!("Failed to send command: {}", e));
        match &result {
            Ok(_) => log::info!("ApiState::send_command - command sent successfully"),
            Err(e) => log::error!("ApiState::send_command - failed: {}", e),
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;

    #[test]
    fn test_register_and_unregister_bot() {
        let (tx, _rx) = unbounded();
        let bots_map = Arc::new(RwLock::new(HashMap::new()));
        let state = ApiState::new(tx, bots_map);
        let bot_id = Uuid::new_v4();
        let entity = Entity::from(42);

        state.register_bot_full(
            bot_id,
            entity,
            "TestBot".to_string(),
            Some("Player1".to_string()),
            10,
            "knight".to_string(),
        );

        assert!(state.bot_exists(&bot_id));
        assert_eq!(state.get_bot_entity(&bot_id), Some(entity));
        assert_eq!(state.get_bot_name(&bot_id), Some("TestBot".to_string()));
        
        let info = state.get_bot_info(&bot_id).unwrap();
        assert_eq!(info.assigned_player, Some("Player1".to_string()));
        assert_eq!(info.level, 10);
        assert_eq!(info.class, "knight");

        state.unregister_bot(&bot_id);

        assert!(!state.bot_exists(&bot_id));
        assert_eq!(state.get_bot_entity(&bot_id), None);
    }

    #[test]
    fn test_update_bot_entity() {
        let (tx, _rx) = unbounded();
        let bots_map = Arc::new(RwLock::new(HashMap::new()));
        let state = ApiState::new(tx, bots_map);
        let bot_id = Uuid::new_v4();
        let initial_entity = Entity::from(0);
        let real_entity = Entity::from(42);

        state.register_bot(bot_id, "TestBot".to_string(), Some("Player1".to_string()));
        assert_eq!(state.get_bot_entity(&bot_id), Some(initial_entity));

        state.update_bot_entity(bot_id, real_entity);
        assert_eq!(state.get_bot_entity(&bot_id), Some(real_entity));
    }

    #[test]
    fn test_send_command() {
        let (tx, rx) = unbounded();
        let bots_map = Arc::new(RwLock::new(HashMap::new()));
        let state = ApiState::new(tx, bots_map);

        let result = state.send_command(LlmBotCommand::Stop { bot_id: Uuid::new_v4() });
        assert!(result.is_ok());

        let received = rx.try_recv();
        assert!(received.is_ok());
    }
}
