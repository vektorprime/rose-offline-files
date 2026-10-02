//! Greeting behaviour: bots wave and say hello when a player walks up to
//! them, and say goodbye when the player walks away again.
//!
//! Not every bot greets every passer-by (a player walking through a crowded
//! plaza used to be greeted by every bot at once): friendly personalities
//! greet more often, AFK bots never, and only a couple of bots greet the
//! same player within a short window. A bot that decides not to greet still
//! remembers the player so it does not re-roll every tick.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use bevy::{
    ecs::change_detection::DetectChangesMut,
    math::{Vec2, Vec3Swizzles},
    prelude::{
        Commands, Component, Entity, MessageWriter, Query, Res, ResMut, Resource, With, Without,
    },
};
use rand::{seq::SliceRandom, Rng};
use rose_data::ZoneId;

use crate::game::{
    components::{
        CharacterInfo, ChatType, ClientEntity, ClientEntityType, Command, Dead, LlmBuddyBot,
        Position,
    },
    events::ChatMessageEvent,
    messages::server::ServerMessage,
    resources::{GameData, ServerMessages},
};

use super::{
    bot_chat::{fill_template, first_name},
    bot_emotes::{bot_play_emote_kind, BotEmoteKind, BotEmotes},
    bot_personality::{BotActivity, BotLife, BotPersonality},
    is_idle_for_duration, is_town_zone, BotBuild, VendorBot, IDLE_DURATION,
};

/// Distance a player must come within for a bot to greet them.
const GREET_RANGE: f32 = 350.0;
/// Distance a player must move beyond for a bot to say goodbye.
const FAREWELL_RANGE: f32 = 500.0;
/// How long before a bot will greet the same player again.
const GREET_COOLDOWN: Duration = Duration::from_secs(90);
/// Minimum time between greeting and farewell for the same player.
const MIN_GREET_AGE: Duration = Duration::from_secs(3);
/// At most this many bots greet one player within `GREET_CROWD_WINDOW`.
const MAX_GREETERS_PER_PLAYER: u8 = 2;
const GREET_CROWD_WINDOW: Duration = Duration::from_secs(20);

const GREETING_LINES: &[&str] = &[
    "Hello there!",
    "Hi! Good to see you.",
    "Hey there, welcome!",
    "Hello! Enjoying your day?",
    "Hi {name}!",
    "Hey {name}!",
    "Oh, hi {name}.",
];

const FAREWELL_LINES: &[&str] = &[
    "See you around!",
    "Take care!",
    "Goodbye, come back soon!",
    "See you later!",
    "Bye {name}!",
];

/// Tracks which players a bot has recently met, and whether it greeted
/// them out loud.
#[derive(Component, Default)]
pub struct BotGreetState {
    greeted: Vec<(Entity, Instant, bool)>,
}

/// How many bots greeted each player recently.
#[derive(Resource, Default)]
pub struct BotGreetLimiter {
    recent: HashMap<Entity, (Instant, u8)>,
}

impl BotGreetLimiter {
    fn try_take(&mut self, player: Entity, now: Instant) -> bool {
        let entry = self.recent.entry(player).or_insert((now, 0));
        if now.saturating_duration_since(entry.0) >= GREET_CROWD_WINDOW {
            *entry = (now, 0);
        }
        if entry.1 >= MAX_GREETERS_PER_PLAYER {
            return false;
        }
        entry.1 += 1;
        true
    }
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

fn compose_line(
    rng: &mut impl Rng,
    lines: &[&str],
    player_name: &str,
    personality: Option<&BotPersonality>,
) -> String {
    let template = lines.choose(rng).copied().unwrap_or("");
    let line = fill_template(template, &[("name", first_name(player_name))]);
    match personality {
        Some(personality) => personality.stylize(rng, &line),
        None => line,
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn bot_greet_system(
    mut commands: Commands,
    mut query_bots: Query<
        (
            Entity,
            &ClientEntity,
            &CharacterInfo,
            &Position,
            &Command,
            Option<&BotPersonality>,
            Option<&BotLife>,
            Option<&mut BotGreetState>,
        ),
        (
            With<BotBuild>,
            Without<LlmBuddyBot>,
            Without<VendorBot>,
            Without<Dead>,
        ),
    >,
    query_players: Query<
        (Entity, &ClientEntity, &Position, &CharacterInfo),
        (Without<BotBuild>, Without<Dead>),
    >,
    game_data: Res<GameData>,
    emotes: Res<BotEmotes>,
    mut limiter: ResMut<BotGreetLimiter>,
    mut server_messages: ResMut<ServerMessages>,
    mut chat_message_events: MessageWriter<ChatMessageEvent>,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();

    if limiter.recent.len() > 256 {
        limiter
            .recent
            .retain(|_, (since, _)| now.saturating_duration_since(*since) < GREET_CROWD_WINDOW);
    }

    // Characters per zone, built once per run in query order.
    let mut players_by_zone: Option<HashMap<ZoneId, Vec<(Entity, Vec2)>>> = None;

    for (
        bot_entity,
        client_entity,
        character_info,
        position,
        command,
        personality,
        life,
        mut greet_state,
    ) in query_bots.iter_mut()
    {
        let Some(zone_data) = game_data.zones.get_zone(position.zone_id) else {
            continue;
        };
        if !is_town_zone(zone_data) {
            continue;
        }

        let players_by_zone = players_by_zone.get_or_insert_with(|| {
            let mut by_zone: HashMap<ZoneId, Vec<(Entity, Vec2)>> = HashMap::new();
            for (player_entity, player_client_entity, player_position, _) in query_players.iter() {
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
        greeted.retain(|(_, greeted_at, _)| {
            now.saturating_duration_since(*greeted_at) < GREET_COOLDOWN
        });
        let mut changed = greeted.len() != greeted_len_before;

        let bot_pos = position.position.xy();
        let idle = is_idle_for_duration(command, IDLE_DURATION);
        let afk = life.is_some_and(|life| life.activity == BotActivity::Afk);
        let greet_chance =
            personality.map_or(0.5, |personality| 0.15 + 0.55 * personality.sociability);

        let zone_players = players_by_zone
            .get(&position.zone_id)
            .map_or(&[][..], |players| players.as_slice());
        for &(player_entity, player_pos) in zone_players {
            let distance = bot_pos.distance(player_pos);
            let greeted_index = greeted
                .iter()
                .position(|(entity, _, _)| *entity == player_entity);

            if distance <= GREET_RANGE {
                if greeted_index.is_none() && idle {
                    let greet_aloud = !afk
                        && rng.gen_bool(greet_chance as f64)
                        && limiter.try_take(player_entity, now);
                    if greet_aloud {
                        let player_name = query_players
                            .get(player_entity)
                            .map_or("", |(_, _, _, info)| info.name.as_str());
                        let line = compose_line(&mut rng, GREETING_LINES, player_name, personality);
                        say(
                            &mut server_messages,
                            &mut chat_message_events,
                            client_entity,
                            bot_entity,
                            &character_info.name,
                            position.zone_id,
                            &line,
                        );
                        bot_play_emote_kind(
                            &mut commands,
                            &mut server_messages,
                            &emotes,
                            &mut rng,
                            bot_entity,
                            client_entity,
                            BotEmoteKind::Greet,
                        );
                    }
                    greeted.push((player_entity, now, greet_aloud));
                    changed = true;
                }
            } else if distance > FAREWELL_RANGE {
                if let Some(index) = greeted_index {
                    let (_, greeted_at, greeted_aloud) = greeted[index];
                    if !greeted_aloud {
                        greeted.remove(index);
                        changed = true;
                    } else if now.saturating_duration_since(greeted_at) >= MIN_GREET_AGE && idle {
                        let player_name = query_players
                            .get(player_entity)
                            .map_or("", |(_, _, _, info)| info.name.as_str());
                        let line = compose_line(&mut rng, FAREWELL_LINES, player_name, personality);
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
