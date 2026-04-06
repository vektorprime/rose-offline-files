//! LLM Event Collector System
//!
//! This system collects game events and converts them to LLM events for
//! processing by the feedback system. It listens to various Bevy events
//! and routes them to the appropriate bot's event queue.

use bevy::prelude::*;
use uuid::Uuid;

use super::event_queue::LlmEventQueue;
use super::event_types::{EventPriority, LlmEvent};
use crate::game::components::{CharacterInfo, ClientEntity, LlmBuddyBot, Position};
use crate::game::events::{ChatMessageEvent, DamageEvent};

/// Maximum distance for a player to be considered "nearby" for chat detection
/// Increased by 10x to allow bots to detect chat from further away.
const CHAT_PROXIMITY_DISTANCE: f32 = 5000.0;

/// System that collects game events and converts them to LLM events.
///
/// This system runs every frame and:
/// 1. Reads game events (chat, damage, etc.)
/// 2. Determines which bots are affected by each event
/// 3. Converts game events to LLM events
/// 4. Pushes LLM events to the appropriate bot's queue
///
/// Event Mapping:
/// | Game Event | LLM Event |
/// |------------|-----------|
/// | ChatMessageEvent | PlayerChat |
/// | DamageEvent | BotDamaged |
///
/// Chat Detection Logic:
/// A chat message is "for a bot" if:
/// - The player is within CHAT_PROXIMITY_DISTANCE units
/// - OR the message contains the bot's name (@BotName or just BotName)
/// - OR the player is the bot's assigned player
#[cfg(feature = "llm-feedback")]
pub fn llm_event_collector_system(
    time: Res<Time>,
    mut chat_events: MessageReader<ChatMessageEvent>,
    mut damage_events: MessageReader<DamageEvent>,
    mut event_queue: ResMut<LlmEventQueue>,
    bot_query: Query<(&LlmBuddyBot, Option<&CharacterInfo>, &Position, Entity)>,
    player_query: Query<&Position>,
    character_info_query: Query<&CharacterInfo>,
) {
    let timestamp = time.elapsed_secs_f64();

    // Process chat events
    for chat_event in chat_events.read() {
        let chat_message = &chat_event.message;
        let sender_name = &chat_event.sender_name;
        let sender_entity = chat_event.sender_entity;

        // Get sender position if available
        let sender_position = player_query.get(sender_entity).ok();

        // Find bots that should receive this chat message
        for (bot, bot_character_info, bot_position, _bot_entity) in bot_query.iter() {
            // Skip messages from the bot itself (if bot has same name as assigned player somehow)
            if bot.assigned_player_name == *sender_name {
                // This is from the assigned player - always process with high priority
                let llm_event = LlmEvent::PlayerChat {
                    bot_id: bot.id,
                    player_name: sender_name.clone(),
                    message: chat_message.clone(),
                };
                event_queue.push_event(bot.id, llm_event, EventPriority::High, timestamp);
                continue;
            }

            // Check proximity - is the sender nearby?
            let is_nearby = if let Some(sender_pos) = sender_position {
                let distance = bot_position.position.distance(sender_pos.position);
                distance <= CHAT_PROXIMITY_DISTANCE
            } else {
                false
            };

            // Check for @mention or name mention in message
            // Use actual bot name from CharacterInfo if available
            let bot_name = bot_character_info
                .map(|ci| ci.name.clone())
                .unwrap_or_else(|| format!("Bot_{}", bot.id));
            let message_lower = chat_message.to_lowercase();
            let name_lower = bot_name.to_lowercase();
            let is_mentioned = message_lower.contains(&format!("@{}", name_lower)) ||
                              message_lower.contains(&name_lower);

            // If nearby or mentioned, add as high-priority event
            if is_nearby || is_mentioned {
                let llm_event = LlmEvent::PlayerChat {
                    bot_id: bot.id,
                    player_name: sender_name.clone(),
                    message: chat_message.clone(),
                };
                event_queue.push_event(bot.id, llm_event, EventPriority::High, timestamp);

                log::debug!(
                    "Chat event queued for bot {} (nearby: {}, mentioned: {}): '{}'",
                    bot.id,
                    is_nearby,
                    is_mentioned,
                    chat_message
                );
            }
        }
    }

    // Process damage events
    for damage_event in damage_events.read() {
        let (attacker_entity, defender_entity, damage_amount) = match damage_event {
            DamageEvent::Attack { attacker, defender, damage } => {
                (*attacker, *defender, damage.amount)
            }
            DamageEvent::Immediate { attacker, defender, damage } => {
                (*attacker, *defender, damage.amount)
            }
            DamageEvent::Skill { attacker, defender, damage, .. } => {
                (*attacker, *defender, damage.amount)
            }
            DamageEvent::Tagged { attacker, defender } => {
                // Tagged events do no damage
                (*attacker, *defender, 0)
            }
        };

        // Check if any bot is the defender (took damage)
        for (bot, _bot_character_info, _bot_position, bot_entity) in bot_query.iter() {
            if bot_entity == defender_entity {
                // Bot took damage - get actual attacker name from CharacterInfo if available
                let attacker_name = character_info_query
                    .get(attacker_entity)
                    .ok()
                    .map(|ci| ci.name.clone())
                    .unwrap_or_else(|| "Unknown".to_string());

                let llm_event = LlmEvent::BotDamaged {
                    bot_id: bot.id,
                    damage: damage_amount,
                    source: attacker_name.clone(),
                };
                event_queue.push_event(bot.id, llm_event, EventPriority::High, timestamp);

                log::debug!(
                    "Damage event queued for bot {}: {} damage from {}",
                    bot.id,
                    damage_amount,
                    attacker_name
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_types() {
        let bot_id = Uuid::new_v4();
        let event = LlmEvent::PlayerChat {
            bot_id,
            player_name: "TestPlayer".to_string(),
            message: "Hello!".to_string(),
        };

        match event {
            LlmEvent::PlayerChat { player_name, message, .. } => {
                assert_eq!(player_name, "TestPlayer");
                assert_eq!(message, "Hello!");
            }
            _ => panic!("Wrong event type"),
        }
    }
}
