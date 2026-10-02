//! Greeting behaviour: bots wave and say hello when a player walks up to
//! them, and say goodbye when the player walks away again.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use bevy::{
    ecs::change_detection::DetectChangesMut,
    math::{Vec2, Vec3Swizzles},
    prelude::{Commands, Component, Entity, MessageWriter, Query, Res, ResMut, With, Without},
};
use rand::{seq::SliceRandom, Rng};
use rose_data::{MotionId, ZoneId};

use crate::game::{
    components::{
        CharacterInfo, ChatType, ClientEntity, ClientEntityType, Command, Dead, LlmBuddyBot,
        NextCommand, Position,
    },
    events::ChatMessageEvent,
    messages::server::ServerMessage,
    resources::{GameData, ServerMessages},
};

use super::{is_idle_for_duration, is_town_zone, BotBuild, VendorBot, IDLE_DURATION};

/// Distance a player must come within for a bot to greet them.
const GREET_RANGE: f32 = 350.0;
/// Distance a player must move beyond for a bot to say goodbye.
const FAREWELL_RANGE: f32 = 500.0;
/// How long before a bot will greet the same player again.
const GREET_COOLDOWN: Duration = Duration::from_secs(90);
/// Minimum time between greeting and farewell for the same player.
const MIN_GREET_AGE: Duration = Duration::from_secs(3);
/// Wave motion id.
const WAVE_MOTION_ID: u16 = 15;

const GREETING_LINES: &[&str] = &[
    "Hello there!",
    "Hi! Good to see you.",
    "Hey there, welcome!",
    "Hello! Enjoying your day?",
];

const FAREWELL_LINES: &[&str] = &[
    "See you around!",
    "Take care!",
    "Goodbye, come back soon!",
    "See you later!",
];

/// Tracks which players a bot has recently greeted.
#[derive(Component, Default)]
pub struct BotGreetState {
    greeted: Vec<(Entity, Instant)>,
}

/// Sends a local chat line from a bot.
fn say(
    server_messages: &mut ServerMessages,
    chat_message_events: &mut MessageWriter<ChatMessageEvent>,
    client_entity: &ClientEntity,
    bot_entity: Entity,
    bot_name: &str,
    zone_id: rose_data::ZoneId,
    message: &str,
) {
    server_messages.send_entity_message(
        client_entity,
        ServerMessage::LocalChat {
            entity_id: client_entity.id,
            text: message.to_string(),
        },
    );

    chat_message_events.write(ChatMessageEvent {
        sender_entity: bot_entity,
        sender_name: bot_name.to_string(),
        zone_id,
        message: message.to_string(),
        chat_type: ChatType::Local,
    });
}

pub fn bot_greet_system(
    mut commands: Commands,
    mut query_bots: Query<
        (
            Entity,
            &ClientEntity,
            &CharacterInfo,
            &Position,
            &Command,
            Option<&mut BotGreetState>,
        ),
        (With<BotBuild>, Without<LlmBuddyBot>, Without<VendorBot>, Without<Dead>),
    >,
    query_players: Query<(Entity, &ClientEntity, &Position), (Without<BotBuild>, Without<Dead>)>,
    game_data: Res<GameData>,
    mut server_messages: ResMut<ServerMessages>,
    mut chat_message_events: MessageWriter<ChatMessageEvent>,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();

    // Characters per zone, built once per run in query order, so each bot
    // visits the same players in the same order (and draws the RNG the same
    // way) as a filtered scan of all client entities would.
    let mut players_by_zone: Option<HashMap<ZoneId, Vec<(Entity, Vec2)>>> = None;

    for (bot_entity, client_entity, character_info, position, command, mut greet_state) in
        query_bots.iter_mut()
    {
        let Some(zone_data) = game_data.zones.get_zone(position.zone_id) else {
            continue;
        };
        if !is_town_zone(zone_data) {
            continue;
        }

        let players_by_zone = players_by_zone.get_or_insert_with(|| {
            let mut by_zone: HashMap<ZoneId, Vec<(Entity, Vec2)>> = HashMap::new();
            for (player_entity, player_client_entity, player_position) in query_players.iter() {
                if player_client_entity.entity_type == ClientEntityType::Character {
                    by_zone
                        .entry(player_position.zone_id)
                        .or_default()
                        .push((player_entity, player_position.position.xy()));
                }
            }
            by_zone
        });

        // Edit the stored list in place (no per-tick copy); bots without one
        // get a fresh list inserted below.
        let mut fresh_greeted = Vec::new();
        let greeted = match greet_state.as_mut() {
            Some(state) => &mut state.bypass_change_detection().greeted,
            None => &mut fresh_greeted,
        };
        let greeted_len_before = greeted.len();
        greeted.retain(|(_, greeted_at)| now.saturating_duration_since(*greeted_at) < GREET_COOLDOWN);
        let mut changed = greeted.len() != greeted_len_before;

        let bot_pos = position.position.xy();
        let idle = is_idle_for_duration(command, IDLE_DURATION);

        let zone_players = players_by_zone
            .get(&position.zone_id)
            .map_or(&[][..], |players| players.as_slice());
        for &(player_entity, player_pos) in zone_players {
            let distance = bot_pos.distance(player_pos);
            let greeted_index = greeted
                .iter()
                .position(|(entity, _)| *entity == player_entity);

            if distance <= GREET_RANGE {
                if greeted_index.is_none() && idle {
                    let line = GREETING_LINES.choose(&mut rng).unwrap_or(&"").to_string();
                    say(
                        &mut server_messages,
                        &mut chat_message_events,
                        client_entity,
                        bot_entity,
                        &character_info.name,
                        position.zone_id,
                        &line,
                    );
                    commands
                        .entity(bot_entity)
                        .insert(NextCommand::with_emote(MotionId::new(WAVE_MOTION_ID), true));
                    greeted.push((player_entity, now));
                    changed = true;
                }
            } else if distance > FAREWELL_RANGE {
                if let Some(index) = greeted_index {
                    let greeted_at = greeted[index].1;
                    if now.saturating_duration_since(greeted_at) >= MIN_GREET_AGE && idle {
                        let line = FAREWELL_LINES.choose(&mut rng).unwrap_or(&"").to_string();
                        say(
                            &mut server_messages,
                            &mut chat_message_events,
                            client_entity,
                            bot_entity,
                            &character_info.name,
                            position.zone_id,
                            &line,
                        );
                        greeted.remove(index);
                        changed = true;
                    }
                }
            }
        }

        match greet_state.as_mut() {
            Some(state) => {
                if changed {
                    state.set_changed();
                }
            }
            None => {
                commands.entity(bot_entity).insert(BotGreetState {
                    greeted: fresh_greeted,
                });
            }
        }
    }
}


