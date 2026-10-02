use std::time::Duration;

use bevy::{
    ecs::query::QueryData,
    math::Vec3Swizzles,
    prelude::{Commands, Component, Query, Res, Vec3, With},
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rand::{seq::SliceRandom, Rng};

use crate::game::{
    bots::{is_idle_for_duration, is_town_zone, BotQueryFilterAlive, IDLE_DURATION},
    components::{Command, MoveMode, NextCommand, Party, PartyMembership, Position},
    resources::GameData,
};

const WANDER_OFFSET: f32 = 750.0;
const DWELL_DURATION: Duration = Duration::from_secs(3);

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldWanderTown {
    pub score: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct WanderTown;

#[derive(QueryData)]
pub struct BotQuery<'w> {
    command: &'w Command,
    position: &'w Position,
    party_membership: Option<&'w PartyMembership>,
}

pub fn score_should_wander_town(
    mut query: Query<(&ShouldWanderTown, &Actor, &mut Score)>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    query_party: Query<&Party>,
    game_data: Res<GameData>,
) {
    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok(bot) = query_bot.get(entity) else {
            continue;
        };

        if !is_idle_for_duration(bot.command, IDLE_DURATION) {
            continue;
        }

        let Some(zone_data) = game_data.zones.get_zone(bot.position.zone_id) else {
            continue;
        };

        if !is_town_zone(zone_data) {
            continue;
        }

        // Party members follow the leader instead of wandering freely.
        if let Some(party_entity) = bot.party_membership.and_then(|m| m.party) {
            if let Ok(party) = query_party.get(party_entity) {
                if party.owner != entity {
                    continue;
                }
            }
        }

        score.set(scorer.score);
    }
}

pub fn action_wander_town(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<WanderTown>>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    game_data: Res<GameData>,
) {
    let mut rng = rand::thread_rng();

    for (&Actor(entity), mut state) in query.iter_mut() {
        let Ok(bot) = query_bot.get(entity) else {
            *state = ActionState::Failure;
            continue;
        };

        match *state {
            ActionState::Requested => {
                if !is_idle_for_duration(bot.command, IDLE_DURATION) {
                    // Already walking (e.g. re-selected as fallback mid-walk).
                    *state = ActionState::Executing;
                    continue;
                }

                let Some(zone_data) = game_data.zones.get_zone(bot.position.zone_id) else {
                    *state = ActionState::Failure;
                    continue;
                };

                // Wander to a random spot somewhere in the town: half the
                // time near a random NPC, otherwise around the town centre.
                let base = if rng.gen_bool(0.5) {
                    zone_data
                        .npcs
                        .choose(&mut rng)
                        .map(|npc_spawn| npc_spawn.position)
                        .unwrap_or(zone_data.start_position)
                } else {
                    zone_data.start_position
                };

                let destination = base
                    + Vec3::new(
                        rng.gen_range(-WANDER_OFFSET..WANDER_OFFSET),
                        rng.gen_range(-WANDER_OFFSET..WANDER_OFFSET),
                        0.0,
                    );

                // Don't wander to a spot we are already standing on.
                let current = bot.position.position.xy();
                let target = destination.xy();
                if current.distance_squared(target) < 100.0 {
                    *state = ActionState::Success;
                    continue;
                }

                commands.entity(entity).insert(NextCommand::with_move(
                    destination,
                    None,
                    Some(MoveMode::Walk),
                ));
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                if bot.command.is_stop_for(DWELL_DURATION) {
                    *state = ActionState::Success;
                }
            }
            ActionState::Cancelled => {
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
