//! LLM Buddy Bot Component
//!
//! This component is attached to bot entities that are controlled by the LLM
//! Buddy Bot REST API. It stores the bot's unique ID, assigned player, and
//! recent chat messages.

use bevy::ecs::prelude::Component;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum number of chat messages to store
pub const MAX_CHAT_MESSAGES: usize = 50;

/// A chat message received by the bot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Timestamp of the message
    pub timestamp: DateTime<Utc>,
    /// Name of the sender
    pub sender_name: String,
    /// Entity ID of the sender
    pub sender_entity_id: u32,
    /// Message content
    pub message: String,
    /// Type of chat (local, shout, etc.)
    pub chat_type: ChatType,
}

/// Type of chat message
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatType {
    /// Proximity-based chat
    Local,
    /// Zone-wide shout
    Shout,
    /// Server announcement
    Announce,
    /// Whisper (private message)
    Whisper,
}

impl Default for ChatType {
    fn default() -> Self {
        Self::Local
    }
}

/// Behavior mode for LLM buddy bots
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotBehaviorMode {
    /// Only follows, doesn't attack
    Passive,
    /// Only attacks what attacks the player
    Defensive,
    /// Attacks any monster in range
    Aggressive,
    /// Prioritizes healing and buffing over attacking
    Support,
}

impl Default for BotBehaviorMode {
    fn default() -> Self {
        Self::Defensive
    }
}

/// Component for LLM-controlled buddy bots
///
/// This component is attached to bot entities that are controlled via
/// the REST API. It tracks the bot's unique ID, the player it's assigned
/// to follow, and maintains a history of recent chat messages.
#[derive(Debug, Clone, Component)]
pub struct LlmBuddyBot {
    /// Unique bot ID for API identification
    pub id: Uuid,
    /// Character ID of the player this bot is assigned to follow
    pub assigned_player_id: u32,
    /// Name of the assigned player (for quick lookup)
    pub assigned_player_name: String,
    /// Distance to maintain from the assigned player
    pub follow_distance: f32,
    /// Recent chat messages (capped at MAX_CHAT_MESSAGES)
    pub chat_messages: Vec<ChatMessage>,
    /// Whether the bot is currently in follow mode
    pub is_following: bool,
    /// Current behavior mode
    pub behavior_mode: BotBehaviorMode,
}

impl LlmBuddyBot {
    /// Create a new LLM Buddy Bot component
    pub fn new(id: Uuid, assigned_player_id: u32, assigned_player_name: String) -> Self {
        Self {
            id,
            assigned_player_id,
            assigned_player_name,
            follow_distance: 300.0,
            chat_messages: Vec::with_capacity(MAX_CHAT_MESSAGES),
            is_following: true,
            behavior_mode: BotBehaviorMode::Defensive,
        }
    }

    /// Create a new LLM Buddy Bot with a random UUID
    pub fn new_random(assigned_player_id: u32, assigned_player_name: String) -> Self {
        Self::new(Uuid::new_v4(), assigned_player_id, assigned_player_name)
    }

    /// Set the follow distance
    pub fn with_follow_distance(mut self, distance: f32) -> Self {
        self.follow_distance = distance;
        self
    }

    /// Add a chat message to the history
    ///
    /// If the history is full, the oldest message is removed.
    pub fn add_chat_message(&mut self, message: ChatMessage) {
        if self.chat_messages.len() >= MAX_CHAT_MESSAGES {
            self.chat_messages.remove(0);
        }
        self.chat_messages.push(message);
    }

    /// Clear all chat messages
    pub fn clear_chat_messages(&mut self) {
        self.chat_messages.clear();
    }

    /// Get the most recent N chat messages
    pub fn recent_messages(&self, n: usize) -> &[ChatMessage] {
        let start = self.chat_messages.len().saturating_sub(n);
        &self.chat_messages[start..]
    }

    /// Set whether the bot is following
    pub fn set_following(&mut self, following: bool) {
        self.is_following = following;
    }

    /// Update the assigned player
    pub fn set_assigned_player(&mut self, player_id: u32, player_name: String) {
        self.assigned_player_id = player_id;
        self.assigned_player_name = player_name;
    }

    /// Set the follow distance
    pub fn set_follow_distance(&mut self, distance: f32) {
        self.follow_distance = distance;
    }
}

impl Default for LlmBuddyBot {
    fn default() -> Self {
        Self::new_random(0, String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_bot() {
        let id = Uuid::new_v4();
        let bot = LlmBuddyBot::new(id, 12345, "TestPlayer".to_string());

        assert_eq!(bot.id, id);
        assert_eq!(bot.assigned_player_id, 12345);
        assert_eq!(bot.assigned_player_name, "TestPlayer");
        assert_eq!(bot.follow_distance, 300.0);
        assert!(bot.chat_messages.is_empty());
        assert!(bot.is_following);
    }

    #[test]
    fn test_add_chat_messages() {
        let mut bot = LlmBuddyBot::new_random(0, String::new());

        for i in 0..10 {
            bot.add_chat_message(ChatMessage {
                timestamp: Utc::now(),
                sender_name: format!("Player{}", i),
                sender_entity_id: i,
                message: format!("Message {}", i),
                chat_type: ChatType::Local,
            });
        }

        assert_eq!(bot.chat_messages.len(), 10);
    }

    #[test]
    fn test_chat_message_cap() {
        let mut bot = LlmBuddyBot::new_random(0, String::new());

        // Add more than max messages
        for i in 0..(MAX_CHAT_MESSAGES + 10) {
            bot.add_chat_message(ChatMessage {
                timestamp: Utc::now(),
                sender_name: format!("Player{}", i),
                sender_entity_id: i as u32,
                message: format!("Message {}", i),
                chat_type: ChatType::Local,
            });
        }

        assert_eq!(bot.chat_messages.len(), MAX_CHAT_MESSAGES);
    }

    #[test]
    fn test_recent_messages() {
        let mut bot = LlmBuddyBot::new_random(0, String::new());

        for i in 0..20 {
            bot.add_chat_message(ChatMessage {
                timestamp: Utc::now(),
                sender_name: format!("Player{}", i),
                sender_entity_id: i as u32,
                message: format!("Message {}", i),
                chat_type: ChatType::Local,
            });
        }

        let recent = bot.recent_messages(5);
        assert_eq!(recent.len(), 5);
        // Should be the last 5 messages (indices 15-19)
        assert_eq!(recent[0].message, "Message 15");
        assert_eq!(recent[4].message, "Message 19");
    }
}
