use bevy::{ecs::query::QueryData, prelude::{Commands, Component, Entity, Query, With}};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};

use crate::game::{
    bots::{BotQueryFilterAlive, BotQueryFilterAliveNoTarget, BotCombatTarget, IDLE_DURATION},
    components::{Command, HealthPoints, NextCommand, Party, PartyMembership, Position, Team},
};

use super::Dead;

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldAttackPartyTarget {
    pub score: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct AttackPartyTarget;

#[derive(QueryData)]
pub struct BotQuery<'w> {
    entity: Entity,
    command: &'w Command,
    position: &'w Position,
    team: &'w Team,
    party_membership: &'w PartyMembership,
}

fn leader_combat_target(
    bot: &BotQueryItem,
    query_party: &Query<&Party>,
    query_leader: &Query<(&BotCombatTarget, &Team, Option<&Dead>)>,
) -> Option<Entity> {
    let party_entity = bot.party_membership.party?;
    let party = query_party.get(party_entity).ok()?;
    if party.owner == bot.entity {
        return None;
    }

    let (target, leader_team, leader_dead) = query_leader.get(party.owner).ok()?;
    if leader_dead.is_some() || leader_team.id != bot.team.id {
        return None;
    }

    Some(target.entity())
}

pub fn score_should_attack_party_target(
    mut query: Query<(&ShouldAttackPartyTarget, &Actor, &mut Score)>,
    query_bot: Query<BotQuery, BotQueryFilterAliveNoTarget>,
    query_party: Query<&Party>,
    query_leader: Query<(&BotCombatTarget, &Team, Option<&Dead>)>,
    query_target: Query<(&Team, &HealthPoints)>,
) {
    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok(bot) = query_bot.get(entity) else {
            continue;
        };

        let Some(target_entity) = leader_combat_target(&bot, &query_party, &query_leader) else {
            continue;
        };

        let Ok((target_team, target_health_points)) = query_target.get(target_entity) else {
            continue;
        };

        if target_team.id != bot.team.id && target_health_points.hp > 0 {
            score.set(scorer.score);
        }
    }
}

pub fn action_attack_party_target(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<AttackPartyTarget>>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    query_party: Query<&Party>,
    query_leader: Query<(&BotCombatTarget, &Team, Option<&Dead>)>,
    query_target: Query<(&Team, &HealthPoints)>,
) {
    for (&Actor(entity), mut state) in query.iter_mut() {
        let Ok(bot) = query_bot.get(entity) else {
            *state = ActionState::Failure;
            continue;
        };

        match *state {
            ActionState::Requested => {
                let Some(target_entity) =
                    leader_combat_target(&bot, &query_party, &query_leader)
                else {
                    *state = ActionState::Failure;
                    continue;
                };

                commands
                    .entity(entity)
                    .insert(BotCombatTarget {
                        entity: target_entity,
                    })
                    .insert(NextCommand::with_attack(target_entity));

                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                let Some(target_entity) =
                    leader_combat_target(&bot, &query_party, &query_leader)
                else {
                    *state = ActionState::Success;
                    continue;
                };

                let Ok((target_team, target_health_points)) = query_target.get(target_entity)
                else {
                    *state = ActionState::Success;
                    continue;
                };

                if target_team.id != bot.team.id && target_health_points.hp > 0 {
                    // Ensure we are attacking the party target.
                    if !bot.command.is_attack_target(target_entity) {
                        commands
                            .entity(entity)
                            .insert(NextCommand::with_attack(target_entity));
                    }
                } else {
                    // Target is dead, wait for the leader to pick a new one.
                    if bot.command.is_stop_for(IDLE_DURATION) {
                        commands.entity(entity).remove::<BotCombatTarget>();
                        *state = ActionState::Success;
                    }
                }
            }
            ActionState::Cancelled => {
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
