use std::time::Duration;

use bevy::{
    prelude::{Commands, Component, Entity, Query, Res, With},
    time::Time,
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};

use crate::game::{
    bots::BotCombatTarget,
    components::{Command, DamageSource, DamageSources, HealthPoints, NextCommand, Team},
};

use super::BotQueryFilterAlive;

const RECENT_ATTACK_TIME: Duration = Duration::from_secs(5);

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ThreatIsNotTarget {
    pub score: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct AttackThreat;

/// `DamageSource::last_damage_seconds` is stamped by `damage_system` with
/// `Time::elapsed_secs_f64`, so recency must be measured on the same clock.
fn is_recent_damage(damage_source: &DamageSource, now_seconds: f64) -> bool {
    now_seconds - damage_source.last_damage_seconds <= RECENT_ATTACK_TIME.as_secs_f64()
}

fn find_highest_damage_source(damage_sources: &DamageSources, now_seconds: f64) -> Option<Entity> {
    let mut highest_damage_source = None;
    for damage_source in damage_sources.damage_sources.iter() {
        if !is_recent_damage(damage_source, now_seconds) {
            continue;
        }

        if highest_damage_source.map_or(true, |(total_damage, _)| {
            total_damage < damage_source.total_damage
        }) {
            highest_damage_source = Some((damage_source.total_damage, damage_source.entity));
        }
    }

    highest_damage_source.map(|(_, entity)| entity)
}

pub fn score_threat_is_not_target(
    mut query: Query<(&ThreatIsNotTarget, &Actor, &mut Score)>,
    query_entity: Query<
        (Option<&BotCombatTarget>, &Command, &DamageSources, &Team),
        BotQueryFilterAlive,
    >,
    query_target: Query<(&Team, &HealthPoints)>,
    time: Res<Time>,
) {
    let now_seconds = time.elapsed_secs_f64();

    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok((bot_combat_target, command, damage_sources, team)) = query_entity.get(entity)
        else {
            continue;
        };

        if command.is_dead() {
            // Cannot fight whilst dead
            continue;
        }

        let mut highest_damage_source = None;
        let mut bot_combat_target_damage = 0;

        for damage_source in damage_sources.damage_sources.iter() {
            if !is_recent_damage(damage_source, now_seconds) {
                continue;
            }

            if highest_damage_source.map_or(true, |(total_damage, _)| {
                total_damage < damage_source.total_damage
            }) {
                highest_damage_source = Some((damage_source.total_damage, damage_source.entity));
            }

            if bot_combat_target.map_or(false, |bot_combat_target| {
                bot_combat_target.entity == damage_source.entity
            }) {
                bot_combat_target_damage = damage_source.total_damage;
            }
        }

        let Some((highest_damage, highest_damage_source_entity)) = highest_damage_source else {
            continue;
        };

        if bot_combat_target.map_or(false, |bot_combat_target| {
            bot_combat_target.entity == highest_damage_source_entity
        }) {
            // We are already targeting the highest damage source
            continue;
        }

        if bot_combat_target_damage * 10 >= highest_damage * 8 {
            // Current target damage is within 80% of highest, do not switch target
            continue;
        }

        if let Ok((target_team, target_health_points)) =
            query_target.get(highest_damage_source_entity)
        {
            // Check the target is still valid before starting switch
            if target_team.id != team.id && target_health_points.hp > 0 {
                score.set(scorer.score);
            }
        }
    }
}

pub fn action_attack_threat(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<AttackThreat>>,
    query_entity: Query<(&Team, &DamageSources)>,
    query_target: Query<(&Team, &HealthPoints)>,
    time: Res<Time>,
) {
    let now_seconds = time.elapsed_secs_f64();

    for (&Actor(entity), mut state) in query.iter_mut() {
        let Ok((team, damage_sources)) = query_entity.get(entity) else {
            continue;
        };

        match *state {
            ActionState::Requested => {
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                if let Some(highest_damage_source_entity) =
                    find_highest_damage_source(damage_sources, now_seconds)
                {
                    if let Ok((target_team, target_health_points)) =
                        query_target.get(highest_damage_source_entity)
                    {
                        if target_team.id != team.id && target_health_points.hp > 0 {
                            commands
                                .entity(entity)
                                .insert(BotCombatTarget {
                                    entity: highest_damage_source_entity,
                                })
                                .insert(NextCommand::with_attack(highest_damage_source_entity));

                            *state = ActionState::Success;
                            continue;
                        }
                    }
                }

                *state = ActionState::Failure;
            }
            ActionState::Cancelled => {
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
