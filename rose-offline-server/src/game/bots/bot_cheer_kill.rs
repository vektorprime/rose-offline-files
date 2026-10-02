//! Cheering: when a monster dies nearby, idle bots sometimes cheer or clap
//! at the kill.

use std::time::{Duration, Instant};

use bevy::{
    math::Vec3Swizzles,
    prelude::{Added, Commands, Component, Entity, Query, Res, ResMut, With, Without},
};
use rand::Rng;

use crate::game::{
    components::{ClientEntity, ClientEntityType, Command, Dead, LlmBuddyBot, Position},
    resources::ServerMessages,
};

use super::{
    bot_emotes::{bot_play_emote_kind, BotEmoteKind, BotEmotes},
    bot_personality::{BotActivity, BotLife, BotPersonality},
    is_idle_for_duration, BotBuild, VendorBot, IDLE_DURATION,
};

/// Maximum distance to cheer over a kill.
const CHEER_RANGE: f32 = 600.0;
/// Cooldown before a bot cheers again.
const CHEER_COOLDOWN: Duration = Duration::from_secs(30);
/// Chance to cheer when a kill happens nearby (scaled by sociability).
const CHEER_CHANCE: f64 = 0.6;

/// Tracks when a bot last cheered.
#[derive(Component, Default)]
pub struct BotCheerState {
    last_cheer_at: Option<Instant>,
}

#[allow(clippy::type_complexity)]
pub fn bot_cheer_kill_system(
    mut commands: Commands,
    new_dead: Query<(Entity, &ClientEntity, &Position), Added<Dead>>,
    mut query_bots: Query<
        (
            Entity,
            &ClientEntity,
            &Position,
            &Command,
            Option<&BotPersonality>,
            Option<&BotLife>,
            Option<&mut BotCheerState>,
        ),
        (
            With<BotBuild>,
            Without<LlmBuddyBot>,
            Without<VendorBot>,
            Without<Dead>,
        ),
    >,
    emotes: Res<BotEmotes>,
    mut server_messages: ResMut<ServerMessages>,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();

    for (_, dead_client_entity, dead_position) in new_dead.iter() {
        if dead_client_entity.entity_type != ClientEntityType::Monster {
            continue;
        }

        let dead_pos = dead_position.position.xy();

        for (bot_entity, client_entity, position, command, personality, life, cheer_state) in
            query_bots.iter_mut()
        {
            if position.zone_id != dead_position.zone_id {
                continue;
            }
            if position.position.xy().distance(dead_pos) > CHEER_RANGE {
                continue;
            }
            if !is_idle_for_duration(command, IDLE_DURATION)
                || life.is_some_and(|life| life.activity == BotActivity::Afk)
            {
                continue;
            }

            let on_cooldown = cheer_state
                .as_ref()
                .and_then(|state| state.last_cheer_at)
                .map_or(false, |last| {
                    now.saturating_duration_since(last) < CHEER_COOLDOWN
                });
            let cheer_chance = personality.map_or(CHEER_CHANCE, |personality| {
                CHEER_CHANCE * (0.3 + 0.7 * personality.sociability as f64)
            });
            if on_cooldown || !rng.gen_bool(cheer_chance) {
                continue;
            }

            bot_play_emote_kind(
                &mut commands,
                &mut server_messages,
                &emotes,
                &mut rng,
                bot_entity,
                client_entity,
                BotEmoteKind::Cheer,
            );

            match cheer_state {
                Some(mut state) => state.last_cheer_at = Some(now),
                None => {
                    commands.entity(bot_entity).insert(BotCheerState {
                        last_cheer_at: Some(now),
                    });
                }
            }
        }
    }
}
