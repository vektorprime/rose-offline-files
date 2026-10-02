//! Cheering: when a monster dies nearby, idle bots cheer or clap at the kill.

use std::time::{Duration, Instant};

use bevy::{
    math::Vec3Swizzles,
    prelude::{Added, Commands, Component, Entity, Query, With, Without},
};
use rand::{seq::SliceRandom, Rng};
use rose_data::MotionId;

use crate::game::components::{
    ClientEntity, ClientEntityType, Command, Dead, LlmBuddyBot, NextCommand, Position,
};

use super::{is_idle_for_duration, BotBuild, VendorBot, IDLE_DURATION};

/// Maximum distance to cheer over a kill.
const CHEER_RANGE: f32 = 600.0;
/// Cooldown before a bot cheers again.
const CHEER_COOLDOWN: Duration = Duration::from_secs(30);
/// Chance to cheer when a kill happens nearby.
const CHEER_CHANCE: f64 = 0.6;

/// Cheer and clap motion ids.
const CHEER_MOTIONS: &[u16] = &[17, 19];

/// Tracks when a bot last cheered.
#[derive(Component, Default)]
pub struct BotCheerState {
    last_cheer_at: Option<Instant>,
}

pub fn bot_cheer_kill_system(
    mut commands: Commands,
    new_dead: Query<(Entity, &ClientEntity, &Position), Added<Dead>>,
    mut query_bots: Query<
        (Entity, &Position, &Command, Option<&mut BotCheerState>),
        (With<BotBuild>, Without<LlmBuddyBot>, Without<VendorBot>, Without<Dead>),
    >,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();

    for (_, dead_client_entity, dead_position) in new_dead.iter() {
        if dead_client_entity.entity_type != ClientEntityType::Monster {
            continue;
        }

        let dead_pos = dead_position.position.xy();

        for (bot_entity, position, command, cheer_state) in query_bots.iter_mut() {
            if position.zone_id != dead_position.zone_id {
                continue;
            }
            if position.position.xy().distance(dead_pos) > CHEER_RANGE {
                continue;
            }
            if !is_idle_for_duration(command, IDLE_DURATION) {
                continue;
            }

            let on_cooldown = cheer_state
                .as_ref()
                .and_then(|state| state.last_cheer_at)
                .map_or(false, |last| now.saturating_duration_since(last) < CHEER_COOLDOWN);
            if on_cooldown || !rng.gen_bool(CHEER_CHANCE) {
                continue;
            }

            let motion_id = CHEER_MOTIONS.choose(&mut rng).copied().unwrap_or(17);
            commands
                .entity(bot_entity)
                .insert(NextCommand::with_emote(MotionId::new(motion_id), true));

            let mut state = match cheer_state {
                Some(mut state) => std::mem::take(&mut *state),
                None => BotCheerState::default(),
            };
            state.last_cheer_at = Some(now);
            commands.entity(bot_entity).insert(state);
        }
    }
}


