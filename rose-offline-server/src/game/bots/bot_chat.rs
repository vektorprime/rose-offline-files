use bevy::{ecs::query::QueryData, prelude::{Component, MessageWriter, Query, ResMut, With}};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rand::{seq::SliceRandom, Rng};

use crate::game::{
    components::{CharacterInfo, ChatType, ClientEntity, Command, Position},
    events::ChatMessageEvent,
    messages::server::ServerMessage,
    resources::ServerMessages,
};

use super::BotQueryFilterAlive;

const BOT_CHAT_MESSAGES: &[&str] = &[
    "Anyone need a party?",
    "Patrolling this area.",
    "Keeping the monsters under control.",
    "Need heals?",
    "Staying sharp out here.",
    "Let's keep moving.",
];

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldBotChat {
    pub score: f32,
    pub chance_per_tick: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct BotChatLocal;

#[derive(QueryData)]
pub struct BotQuery<'w> {
    command: &'w Command,
    client_entity: &'w ClientEntity,
    character_info: &'w CharacterInfo,
    position: &'w Position,
}

pub fn score_should_bot_chat(
    mut query: Query<(&ShouldBotChat, &Actor, &mut Score)>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
) {
    let mut rng = rand::thread_rng();

    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok(bot) = query_bot.get(entity) else {
            continue;
        };

        if !bot.command.is_stop() {
            continue;
        }

        if rng.gen_bool(scorer.chance_per_tick as f64) {
            score.set(scorer.score);
        }
    }
}

pub fn action_bot_chat_local(
    mut query: Query<(&Actor, &mut ActionState), With<BotChatLocal>>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
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

                let Some(message) = BOT_CHAT_MESSAGES.choose(&mut rng).map(|message| message.to_string()) else {
                    *state = ActionState::Failure;
                    continue;
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

                *state = ActionState::Success;
            }
            ActionState::Cancelled => {
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
