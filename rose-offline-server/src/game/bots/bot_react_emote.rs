//! Emote reactions: when a player emotes near a bot (wave, cheer, bow, ...),
//! the bot answers with an emote of its own, sometimes with a chat line.

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use bevy::{
    math::Vec3Swizzles,
    prelude::{Commands, Component, Entity, MessageReader, MessageWriter, Query, ResMut, Resource, With, Without},
};
use rand::{seq::SliceRandom, Rng};
use rose_data::MotionId;

use crate::game::{
    components::{
        CharacterInfo, ChatType, ClientEntity, ClientEntityType, Command, CommandData, Dead,
        LlmBuddyBot, NextCommand, Position,
    },
    events::{ChatMessageEvent, PlayerEmoteEvent},
    messages::server::ServerMessage,
    resources::ServerMessages,
};

use super::{is_idle_for_duration, BotBuild, VendorBot, IDLE_DURATION};

/// Maximum distance to react to a player's emote.
const REACT_RANGE: f32 = 500.0;
/// Cooldown between reactions for the same bot.
const REACT_COOLDOWN: Duration = Duration::from_secs(15);
/// Chance to reply with an emote.
const REACT_CHANCE: f64 = 0.7;
/// Chance to also say a line when reacting.
const CHAT_CHANCE: f64 = 0.3;

/// Motion ids a bot may respond with: wave, dance, cheer, bow, clap.
const RESPONSE_MOTIONS: &[u16] = &[15, 16, 17, 18, 19];

const RESPONSE_LINES: &[&str] = &[
    "Thanks!",
    "Hehe, nice!",
    "Haha!",
    "Hi there!",
    "You too!",
];

/// Tracks the last time a bot reacted to an emote.
#[derive(Component, Default)]
pub struct BotReactEmoteState {
    last_reaction: Option<Instant>,
}

/// Remembers each entity's last observed emote state so we only write one
/// `PlayerEmoteEvent` per emote transition, not once per frame.
#[derive(Default, Resource)]
pub struct LastEmoteState {
    emoting: HashMap<Entity, (u16, bool)>,
}

/// Detects when a client entity starts an emote and writes a
/// `PlayerEmoteEvent` for it. Kept separate from `command_system` because
/// that system is already at bevy's parameter limit.
pub fn player_emote_detect_system(
    mut last_state: ResMut<LastEmoteState>,
    mut emote_events: MessageWriter<PlayerEmoteEvent>,
    query: Query<(Entity, &Command), With<ClientEntity>>,
) {
    for (entity, command) in query.iter() {
        if let CommandData::Emote { motion_id, .. } = &command.command {
            let motion_id = motion_id.get();
            if last_state.emoting.get(&entity) != Some(&(motion_id, true)) {
                emote_events.write(PlayerEmoteEvent {
                    entity,
                    motion_id,
                });
                last_state.emoting.insert(entity, (motion_id, true));
            }
        }
    }

    // Forget queried entities that stopped emoting. Only remembered entities
    // need checking (instead of a map remove for every client entity every
    // tick); entities outside the query are kept, exactly as before.
    if !last_state.emoting.is_empty() {
        last_state.emoting.retain(|entity, _| {
            query.get(*entity).map_or(true, |(_, command)| {
                matches!(command.command, CommandData::Emote { .. })
            })
        });
    }
}

pub fn bot_react_emote_system(
    mut commands: Commands,
    mut emote_events: MessageReader<PlayerEmoteEvent>,
    query_players: Query<(&ClientEntity, &Position), (Without<BotBuild>, Without<Dead>)>,
    mut query_bots: Query<
        (
            Entity,
            &ClientEntity,
            &CharacterInfo,
            &Position,
            &Command,
            Option<&mut BotReactEmoteState>,
        ),
        (With<BotBuild>, Without<LlmBuddyBot>, Without<VendorBot>, Without<Dead>),
    >,
    mut server_messages: ResMut<ServerMessages>,
    mut chat_message_events: MessageWriter<ChatMessageEvent>,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();
    // Bots whose first state component was inserted this run: the insert is deferred, so
    // without this they would react again to another emote event in the same frame.
    let mut reacted_without_state = HashSet::new();

    for event in emote_events.read() {
        let Ok((source_client_entity, source_position)) = query_players.get(event.entity) else {
            continue;
        };
        if source_client_entity.entity_type != ClientEntityType::Character {
            continue;
        }

        let source_pos = source_position.position.xy();

        for (bot_entity, client_entity, character_info, position, command, react_state) in
            query_bots.iter_mut()
        {
            if position.zone_id != source_position.zone_id {
                continue;
            }
            if position.position.xy().distance(source_pos) > REACT_RANGE {
                continue;
            }
            if !is_idle_for_duration(command, IDLE_DURATION) {
                continue;
            }

            let on_cooldown = reacted_without_state.contains(&bot_entity)
                || react_state
                    .as_ref()
                    .and_then(|state| state.last_reaction)
                    .map_or(false, |last| {
                        now.saturating_duration_since(last) < REACT_COOLDOWN
                    });
            if on_cooldown || !rng.gen_bool(REACT_CHANCE) {
                continue;
            }

            let motion_id = RESPONSE_MOTIONS.choose(&mut rng).copied().unwrap_or(17);
            commands
                .entity(bot_entity)
                .insert(NextCommand::with_emote(MotionId::new(motion_id), true));

            if rng.gen_bool(CHAT_CHANCE) {
                let line = RESPONSE_LINES.choose(&mut rng).unwrap_or(&"").to_string();
                server_messages.send_entity_message(
                    client_entity,
                    ServerMessage::LocalChat {
                        entity_id: client_entity.id,
                        text: line.clone(),
                    },
                );
                chat_message_events.write(ChatMessageEvent {
                    sender_entity: bot_entity,
                    sender_name: character_info.name.clone(),
                    zone_id: position.zone_id,
                    message: line,
                    chat_type: ChatType::Local,
                });
            }

            // Update an existing state in place so later events this frame already see the
            // cooldown; a deferred insert would not land until after this system runs.
            match react_state {
                Some(mut state) => state.last_reaction = Some(now),
                None => {
                    commands.entity(bot_entity).insert(BotReactEmoteState {
                        last_reaction: Some(now),
                    });
                    reacted_without_state.insert(bot_entity);
                }
            }
        }
    }
}


