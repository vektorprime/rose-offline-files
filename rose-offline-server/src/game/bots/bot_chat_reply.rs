use bevy::{ecs::query::QueryData, prelude::{Component, MessageWriter, Query, ResMut, With}};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rand::Rng;

use crate::game::{
    bots::{BotChatMemory, BOT_CHAT_REPLY_COOLDOWN},
    components::{CharacterInfo, ChatType, ClientEntity, Command, Position},
    events::ChatMessageEvent,
    messages::server::ServerMessage,
    resources::ServerMessages,
};

use super::{bot_chat::line_for_topic, BotQueryFilterAlive};

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldReplyToChat {
    pub score: f32,
    pub chance_per_tick: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct BotChatReply;

#[derive(QueryData)]
pub struct BotQuery<'w> {
    command: &'w Command,
    client_entity: &'w ClientEntity,
    character_info: &'w CharacterInfo,
    position: &'w Position,
}

pub fn score_should_reply_to_chat(
    mut query: Query<(&ShouldReplyToChat, &Actor, &mut Score)>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    query_memory: Query<&BotChatMemory>,
) {
    let mut rng = rand::thread_rng();
    // One clock reading for the whole run instead of one per memory entry.
    let now = std::time::Instant::now();

    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok(bot) = query_bot.get(entity) else {
            continue;
        };

        if !bot.command.is_stop() {
            continue;
        }

        let Ok(memory) = query_memory.get(entity) else {
            continue;
        };

        // Only reply to recent chat, and not too often. Both checks are pure
        // and precede the RNG draw, so the cheap cooldown check goes first.
        if memory
            .last_reply_time
            .is_some_and(|last| now.saturating_duration_since(last) < BOT_CHAT_REPLY_COOLDOWN)
        {
            continue;
        }

        if memory.replyable_entry_at(entity, now).is_none() {
            continue;
        }

        if rng.gen_bool(scorer.chance_per_tick as f64) {
            score.set(scorer.score);
        }
    }
}

pub fn action_bot_chat_reply(
    mut query: Query<(&Actor, &mut ActionState), With<BotChatReply>>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    mut query_memory: Query<&mut BotChatMemory>,
    mut server_messages: ResMut<ServerMessages>,
    mut chat_message_events: MessageWriter<ChatMessageEvent>,
) {
    let mut rng = rand::thread_rng();

    for (&Actor(entity), mut state) in query.iter_mut() {
        match *state {
            ActionState::Requested => {
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                let Ok(bot) = query_bot.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                let Ok(mut memory) = query_memory.get_mut(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                let Some(entry) = memory.replyable_entry(entity).cloned() else {
                    *state = ActionState::Failure;
                    continue;
                };

                let line = line_for_topic(&mut rng, entry.topic);
                // Sometimes address the sender by name to look like a real reply.
                let message = if rng.gen_bool(0.4) {
                    format!("{} {}", line, entry.sender_name)
                } else {
                    line.to_string()
                };

                server_messages.send_entity_message(
                    bot.client_entity,
                    ServerMessage::LocalChat {
                        entity_id: bot.client_entity.id,
                        text: message.clone(),
                    },
                );

                chat_message_events.write(ChatMessageEvent {
                    sender_entity: entity,
                    sender_name: bot.character_info.name.clone(),
                    zone_id: bot.position.zone_id,
                    message,
                    chat_type: ChatType::Local,
                });

                // Mark the memory as replied so we don't reply again immediately.
                memory.last_reply_time = Some(std::time::Instant::now());

                *state = ActionState::Success;
            }
            ActionState::Cancelled => {
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
