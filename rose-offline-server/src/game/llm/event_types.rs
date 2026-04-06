//! Event Types for the LLM Feedback System
//!
//! This module defines the event types used for LLM-controlled bot feedback.
//! These events represent game state changes that are relevant for LLM decision-making.

use std::fmt;
use uuid::Uuid;

/// Priority level for LLM events.
///
/// Higher priority events should be processed first by the LLM feedback system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventPriority {
    /// Low priority: informational events like item drops, distant monsters
    Low,
    /// Normal priority: status changes, routine updates
    Normal,
    /// High priority: damage received, player chat, combat events
    High,
}

impl Default for EventPriority {
    fn default() -> Self {
        Self::Normal
    }
}

impl fmt::Display for EventPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventPriority::Low => write!(f, "low"),
            EventPriority::Normal => write!(f, "normal"),
            EventPriority::High => write!(f, "high"),
        }
    }
}

/// Events relevant to LLM decision-making for bot control.
///
/// Each event variant includes the `bot_id` to identify which bot
/// the event is relevant to.
#[derive(Debug, Clone)]
pub enum LlmEvent {
    /// A player sent a chat message near the bot
    PlayerChat {
        /// The bot this event is for
        bot_id: Uuid,
        /// Name of the player who sent the message
        player_name: String,
        /// The chat message content
        message: String,
    },

    /// The bot received damage from some source
    BotDamaged {
        /// The bot this event is for
        bot_id: Uuid,
        /// Amount of damage received
        damage: u32,
        /// Source of the damage (e.g., monster name, player name, "environment")
        source: String,
    },

    /// The bot's health has dropped to a concerning level
    BotLowHealth {
        /// The bot this event is for
        bot_id: Uuid,
        /// Current health as a percentage (0-100)
        health_percent: u8,
    },

    /// A monster has been detected near the bot
    MonsterNearby {
        /// The bot this event is for
        bot_id: Uuid,
        /// Name of the monster
        monster_name: String,
        /// Level of the monster
        level: u16,
        /// Distance from the bot
        distance: f32,
    },

    /// A player has moved relative to the bot
    PlayerMoved {
        /// The bot this event is for
        bot_id: Uuid,
        /// Distance from the bot to the player
        distance_from_bot: f32,
    },

    /// An item has been dropped on the ground nearby
    ItemDropped {
        /// The bot this event is for
        bot_id: Uuid,
        /// Name of the dropped item
        item_name: String,
        /// Distance from the bot to the item
        distance: f32,
    },

    /// Combat has started with a target
    CombatStarted {
        /// The bot this event is for
        bot_id: Uuid,
        /// Name or identifier of the combat target
        target: String,
    },

    /// Combat has ended
    CombatEnded {
        /// The bot this event is for
        bot_id: Uuid,
        /// Whether the bot was victorious
        victory: bool,
    },

    /// The bot received a party invite
    PartyInviteReceived {
        /// The bot this event is for
        bot_id: Uuid,
        /// Name of the player who sent the invite
        inviter_name: String,
    },
}

impl LlmEvent {
    /// Returns the bot ID associated with this event
    pub fn bot_id(&self) -> Uuid {
        match self {
            LlmEvent::PlayerChat { bot_id, .. } => *bot_id,
            LlmEvent::BotDamaged { bot_id, .. } => *bot_id,
            LlmEvent::BotLowHealth { bot_id, .. } => *bot_id,
            LlmEvent::MonsterNearby { bot_id, .. } => *bot_id,
            LlmEvent::PlayerMoved { bot_id, .. } => *bot_id,
            LlmEvent::ItemDropped { bot_id, .. } => *bot_id,
            LlmEvent::CombatStarted { bot_id, .. } => *bot_id,
            LlmEvent::CombatEnded { bot_id, .. } => *bot_id,
            LlmEvent::PartyInviteReceived { bot_id, .. } => *bot_id,
        }
    }

    /// Returns the default priority for this event type
    pub fn default_priority(&self) -> EventPriority {
        match self {
            LlmEvent::PlayerChat { .. } => EventPriority::High,
            LlmEvent::BotDamaged { .. } => EventPriority::High,
            LlmEvent::BotLowHealth { .. } => EventPriority::High,
            LlmEvent::MonsterNearby { .. } => EventPriority::Normal,
            LlmEvent::PlayerMoved { .. } => EventPriority::Low,
            LlmEvent::ItemDropped { .. } => EventPriority::Low,
            LlmEvent::CombatStarted { .. } => EventPriority::High,
            LlmEvent::CombatEnded { .. } => EventPriority::High,
            LlmEvent::PartyInviteReceived { .. } => EventPriority::High,
        }
    }
}

impl fmt::Display for LlmEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmEvent::PlayerChat { player_name, message, .. } => {
                write!(f, "Player '{}' said: {}", player_name, message)
            }
            LlmEvent::BotDamaged { damage, source, .. } => {
                write!(f, "Took {} damage from {}", damage, source)
            }
            LlmEvent::BotLowHealth { health_percent, .. } => {
                write!(f, "Low health: {}%", health_percent)
            }
            LlmEvent::MonsterNearby { monster_name, level, distance, .. } => {
                write!(f, "Monster '{}' (lv.{}) nearby at distance {:.1}", monster_name, level, distance)
            }
            LlmEvent::PlayerMoved { distance_from_bot, .. } => {
                write!(f, "Player moved, now at distance {:.1}", distance_from_bot)
            }
            LlmEvent::ItemDropped { item_name, distance, .. } => {
                write!(f, "Item '{}' dropped at distance {:.1}", item_name, distance)
            }
            LlmEvent::CombatStarted { target, .. } => {
                write!(f, "Combat started with {}", target)
            }
            LlmEvent::CombatEnded { victory, .. } => {
                if *victory {
                    write!(f, "Combat ended victoriously")
                } else {
                    write!(f, "Combat ended in defeat")
                }
            }
            LlmEvent::PartyInviteReceived { inviter_name, .. } => {
                write!(f, "Party invite received from {}", inviter_name)
            }
        }
    }
}

/// An event with timestamp and priority information.
///
/// This struct wraps an [`LlmEvent`] with additional metadata needed
/// for queue management and LLM context building.
#[derive(Debug, Clone)]
pub struct TimestampedLlmEvent {
    /// The wrapped event
    pub event: LlmEvent,
    /// Priority level for this event
    pub priority: EventPriority,
    /// Timestamp when the event occurred (seconds since game start)
    pub timestamp: f64,
}

impl TimestampedLlmEvent {
    /// Creates a new timestamped event with the given priority
    pub fn new(event: LlmEvent, priority: EventPriority, timestamp: f64) -> Self {
        Self {
            event,
            priority,
            timestamp,
        }
    }

    /// Creates a new timestamped event with default priority for the event type
    pub fn with_default_priority(event: LlmEvent, timestamp: f64) -> Self {
        let priority = event.default_priority();
        Self::new(event, priority, timestamp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_priority_ordering() {
        assert!(EventPriority::High > EventPriority::Normal);
        assert!(EventPriority::Normal > EventPriority::Low);
        assert!(EventPriority::High > EventPriority::Low);
    }

    #[test]
    fn test_event_default_priority() {
        let bot_id = Uuid::nil();
        
        let chat_event = LlmEvent::PlayerChat {
            bot_id,
            player_name: "Test".to_string(),
            message: "Hello".to_string(),
        };
        assert_eq!(chat_event.default_priority(), EventPriority::High);

        let monster_event = LlmEvent::MonsterNearby {
            bot_id,
            monster_name: "Slime".to_string(),
            level: 1,
            distance: 100.0,
        };
        assert_eq!(monster_event.default_priority(), EventPriority::Normal);

        let item_event = LlmEvent::ItemDropped {
            bot_id,
            item_name: "Potion".to_string(),
            distance: 50.0,
        };
        assert_eq!(item_event.default_priority(), EventPriority::Low);
    }

    #[test]
    fn test_timestamped_event_creation() {
        let bot_id = Uuid::nil();
        let event = LlmEvent::BotDamaged {
            bot_id,
            damage: 10,
            source: "Slime".to_string(),
        };
        
        let timestamped = TimestampedLlmEvent::new(event.clone(), EventPriority::Normal, 123.45);
        assert_eq!(timestamped.event.bot_id(), bot_id);
        assert_eq!(timestamped.priority, EventPriority::Normal);
        assert!((timestamped.timestamp - 123.45).abs() < f64::EPSILON);

        let timestamped_default = TimestampedLlmEvent::with_default_priority(event, 100.0);
        assert_eq!(timestamped_default.priority, EventPriority::High);
    }
}
