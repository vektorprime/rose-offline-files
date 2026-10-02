use std::time::{Duration, Instant};

use arrayvec::ArrayVec;
use bevy::{
    ecs::query::QueryData,
    math::Vec3Swizzles,
    prelude::{Commands, Component, Entity, Query, Res, With},
    time::Time,
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rand::{seq::SliceRandom, Rng};

use crate::game::{
    bots::IDLE_DURATION,
    components::{
        ClientEntityType, Command, DamageSources, HealthPoints, Level, NextCommand, Npc,
        PartyMembership, Position, Team,
    },
    resources::ClientEntityList,
};

use super::{
    bot_personality::{BotPersonality, BotScanCache},
    BotCombatTarget, BotQueryFilterAlive, BotQueryFilterAliveNoTarget,
};

const NEAREST_TARGET_SEARCH_DISTANCE: f32 = 2000.0f32;
/// A monster damaged by someone outside our party this recently is theirs.
const KILL_STEAL_WINDOW_SECS: f64 = 6.0;
/// How often an idle bot looks around for monsters (randomised per scan).
const TARGET_SCAN_INTERVAL_MS: std::ops::Range<u64> = 300..600;
/// Bots pick among this many of the nearest valid targets.
const NEAREST_TARGET_CHOICES: usize = 4;

#[derive(Debug, Clone, Component, ScorerBuilder)]
pub struct FindNearbyTarget {
    pub score: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct AttackRandomNearbyTarget;

#[derive(QueryData)]
pub struct BotQuery<'w> {
    command: &'w Command,
    position: &'w Position,
    team: &'w Team,
    level: &'w Level,
    party_membership: &'w PartyMembership,
    personality: Option<&'w BotPersonality>,
}

#[derive(QueryData)]
pub struct TargetQuery<'w> {
    team: &'w Team,
    health_points: &'w HealthPoints,
    level: Option<&'w Level>,
    npc: Option<&'w Npc>,
    command: &'w Command,
    damage_sources: Option<&'w DamageSources>,
}

fn is_party_member(
    bot: &BotQueryItem,
    other: Entity,
    query_party_membership: &Query<&PartyMembership>,
) -> bool {
    bot.party_membership.party.is_some()
        && query_party_membership
            .get(other)
            .is_ok_and(|membership| membership.party == bot.party_membership.party)
}

/// Whether a bot would pick a fight with `target`: an enemy that is alive,
/// not far above its level, and not already being fought by someone outside
/// its party (players do not like kill stealers).
fn is_good_target(
    bot_entity: Entity,
    bot: &BotQueryItem,
    target: &TargetQueryItem,
    query_party_membership: &Query<&PartyMembership>,
    now_seconds: f64,
) -> bool {
    if target.team.id == bot.team.id || target.health_points.hp <= 0 {
        return false;
    }

    // Enemy characters (PvP) are always fair game.
    if target.npc.is_none() {
        return true;
    }

    let max_levels_above = bot
        .personality
        .map_or(5, |personality| personality.max_levels_above());
    if target.level.is_some_and(|target_level| {
        target_level.level as i32 - bot.level.level as i32 > max_levels_above
    }) {
        return false;
    }

    if let Some(monster_target) = target.command.target_entity() {
        if monster_target != bot_entity
            && !is_party_member(bot, monster_target, query_party_membership)
        {
            return false;
        }
    }

    if let Some(damage_sources) = target.damage_sources {
        if damage_sources.damage_sources.iter().any(|source| {
            now_seconds - source.last_damage_seconds <= KILL_STEAL_WINDOW_SECS
                && source.entity != bot_entity
                && !is_party_member(bot, source.entity, query_party_membership)
        }) {
            return false;
        }
    }

    true
}

fn has_good_target_nearby(
    bot_entity: Entity,
    bot: &BotQueryItem,
    client_entity_list: &ClientEntityList,
    query_target: &Query<TargetQuery>,
    query_party_membership: &Query<&PartyMembership>,
    now_seconds: f64,
) -> bool {
    let Some(zone_entities) = client_entity_list.get_zone(bot.position.zone_id) else {
        return false;
    };
    zone_entities
        .iter_entity_type_within_distance(
            bot.position.position.xy(),
            NEAREST_TARGET_SEARCH_DISTANCE,
            &[ClientEntityType::Character, ClientEntityType::Monster],
        )
        .any(|(nearby_entity, _)| {
            query_target.get(nearby_entity).is_ok_and(|target| {
                is_good_target(
                    bot_entity,
                    bot,
                    &target,
                    query_party_membership,
                    now_seconds,
                )
            })
        })
}

#[allow(clippy::too_many_arguments)]
pub fn score_find_nearby_target(
    mut query: Query<(&FindNearbyTarget, &Actor, &mut Score)>,
    query_bot: Query<BotQuery, BotQueryFilterAliveNoTarget>,
    query_target: Query<TargetQuery>,
    query_party_membership: Query<&PartyMembership>,
    mut query_scan: Query<&mut BotScanCache>,
    client_entity_list: Res<ClientEntityList>,
    time: Res<Time>,
) {
    let now = Instant::now();
    let now_seconds = time.elapsed_secs_f64();
    let mut rng = rand::thread_rng();

    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok(bot) = query_bot.get(entity) else {
            continue;
        };

        // Take a moment after a fight before picking the next monster.
        if bot.command.is_stop()
            && bot
                .personality
                .is_some_and(|personality| !bot.command.is_stop_for(personality.reaction))
        {
            continue;
        }

        let found = match query_scan.get_mut(entity) {
            Ok(mut cache) => {
                if cache.next_target_scan.map_or(true, |next| now >= next) {
                    cache.target_found = has_good_target_nearby(
                        entity,
                        &bot,
                        &client_entity_list,
                        &query_target,
                        &query_party_membership,
                        now_seconds,
                    );
                    cache.next_target_scan =
                        Some(now + Duration::from_millis(rng.gen_range(TARGET_SCAN_INTERVAL_MS)));
                }
                cache.target_found
            }
            Err(_) => has_good_target_nearby(
                entity,
                &bot,
                &client_entity_list,
                &query_target,
                &query_party_membership,
                now_seconds,
            ),
        };

        if found {
            score.set(scorer.score);
        }
    }
}

pub fn action_attack_random_nearby_target(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<AttackRandomNearbyTarget>>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    query_target: Query<TargetQuery>,
    query_party_membership: Query<&PartyMembership>,
    mut query_scan: Query<&mut BotScanCache>,
    client_entity_list: Res<ClientEntityList>,
    time: Res<Time>,
) {
    let mut rng = rand::thread_rng();
    let now_seconds = time.elapsed_secs_f64();

    for (&Actor(entity), mut state) in query.iter_mut() {
        match *state {
            ActionState::Requested => {
                let Ok(bot) = query_bot.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                let Some(zone_entities) = client_entity_list.get_zone(bot.position.zone_id) else {
                    *state = ActionState::Failure;
                    continue;
                };

                // Find the nearest valid targets
                let mut nearest_targets: ArrayVec<(f32, Entity), NEAREST_TARGET_CHOICES> =
                    ArrayVec::new();
                for (nearby_entity, nearby_position) in zone_entities
                    .iter_entity_type_within_distance(
                        bot.position.position.xy(),
                        NEAREST_TARGET_SEARCH_DISTANCE,
                        &[ClientEntityType::Character, ClientEntityType::Monster],
                    )
                {
                    if !query_target.get(nearby_entity).is_ok_and(|target| {
                        is_good_target(entity, &bot, &target, &query_party_membership, now_seconds)
                    }) {
                        continue;
                    }

                    let distance = bot
                        .position
                        .position
                        .xy()
                        .distance_squared(nearby_position.xy());
                    let index = nearest_targets
                        .iter()
                        .position(|(nearest_distance, _)| distance < *nearest_distance)
                        .unwrap_or(nearest_targets.len());
                    if index < NEAREST_TARGET_CHOICES {
                        if nearest_targets.is_full() {
                            nearest_targets.pop();
                        }
                        nearest_targets.insert(index, (distance, nearby_entity));
                    }
                }

                // Mostly the closest one, sometimes one of the next few.
                let choice = if rng.gen_bool(0.6) {
                    nearest_targets.first()
                } else {
                    nearest_targets.choose(&mut rng)
                };

                if let Some(&(_, target_entity)) = choice {
                    commands
                        .entity(entity)
                        .insert(NextCommand::with_attack(target_entity))
                        .insert(BotCombatTarget {
                            entity: target_entity,
                        });

                    *state = ActionState::Executing;
                } else {
                    // The cached "target nearby" was stale: rescan next time.
                    if let Ok(mut cache) = query_scan.get_mut(entity) {
                        cache.target_found = false;
                        cache.next_target_scan = None;
                    }
                    *state = ActionState::Failure;
                }
            }
            ActionState::Executing => {
                let Ok(bot) = query_bot.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                if bot.command.is_stop_for(IDLE_DURATION) {
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
