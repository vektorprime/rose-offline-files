use bevy::{
    ecs::query::QueryData,
    math::{Vec2, Vec3Swizzles},
    prelude::{Commands, Component, Entity, Query, With},
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};

use crate::game::{
    bots::{BotQueryFilterAlive, BotQueryFilterAliveNoTarget, BotCombatTarget},
    components::{Command, MoveMode, NextCommand, Party, PartyMembership, Position},
};

use super::Dead;

/// Followers spread around the leader at these angles (relative to the
/// side they approach from) so a party walks as a loose group instead of
/// stacking on one point.
const FORMATION_ANGLES_DEG: [f32; 4] = [-40.0, 40.0, -95.0, 95.0];
/// Distance followers keep from the leader.
const FORMATION_DISTANCE: f32 = 200.0;
/// Followers within this much of their place do not re-position.
const FORMATION_SLACK: f32 = 100.0;
/// Beyond this distance followers run to catch up whatever the leader does.
const CATCH_UP_DISTANCE: f32 = 1000.0;

/// This follower's place in the party (leader excluded).
fn formation_slot(party: &Party, entity: Entity) -> usize {
    party
        .members
        .iter()
        .filter_map(|member| member.get_entity())
        .filter(|member| *member != party.owner)
        .position(|member| member == entity)
        .unwrap_or(0)
}

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldFollowPartyLeader {
    pub score: f32,
    pub follow_distance: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct FollowPartyLeader;

#[derive(QueryData)]
pub struct BotQuery<'w> {
    entity: Entity,
    command: &'w Command,
    position: &'w Position,
    party_membership: &'w PartyMembership,
}

fn party_leader(bot: &BotQueryItem, query_party: &Query<&Party>) -> Option<Entity> {
    let party_entity = bot.party_membership.party?;
    let party = query_party.get(party_entity).ok()?;
    (party.owner != bot.entity).then_some(party.owner)
}

pub fn score_should_follow_party_leader(
    mut query: Query<(&ShouldFollowPartyLeader, &Actor, &mut Score)>,
    query_bot: Query<BotQuery, BotQueryFilterAliveNoTarget>,
    query_party: Query<&Party>,
    query_owner: Query<(&Position, Option<&Dead>)>,
) {
    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok(bot) = query_bot.get(entity) else {
            continue;
        };

        if !bot.command.is_stop() {
            continue;
        }

        let Some(owner_entity) = party_leader(&bot, &query_party) else {
            continue;
        };

        let Ok((owner_position, owner_dead)) = query_owner.get(owner_entity) else {
            continue;
        };

        if owner_dead.is_some() || owner_position.zone_id != bot.position.zone_id {
            continue;
        }

        let distance = bot
            .position
            .position
            .xy()
            .distance_squared(owner_position.position.xy());

        if distance > scorer.follow_distance * scorer.follow_distance {
            score.set(scorer.score);
        }
    }
}

pub fn action_follow_party_leader(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<FollowPartyLeader>>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    query_party: Query<&Party>,
    query_owner: Query<(&Position, Option<&Dead>)>,
    query_owner_move_mode: Query<&MoveMode>,
) {
    for (&Actor(entity), mut state) in query.iter_mut() {
        let Ok(bot) = query_bot.get(entity) else {
            *state = ActionState::Failure;
            continue;
        };

        match *state {
            ActionState::Requested => {
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                let Some(owner_entity) = party_leader(&bot, &query_party) else {
                    // Party disbanded or we became the leader.
                    *state = ActionState::Success;
                    continue;
                };

                let Ok((owner_position, owner_dead)) = query_owner.get(owner_entity) else {
                    *state = ActionState::Success;
                    continue;
                };

                if owner_dead.is_some() || owner_position.zone_id != bot.position.zone_id {
                    *state = ActionState::Success;
                    continue;
                }

                if !bot.command.is_stop() {
                    continue;
                }

                let distance = bot
                    .position
                    .position
                    .xy()
                    .distance(owner_position.position.xy());

                if distance <= FORMATION_DISTANCE + FORMATION_SLACK {
                    // Caught up: get on with other things until the leader
                    // moves off again.
                    *state = ActionState::Success;
                    continue;
                }

                // Move to this follower's place beside the leader.
                let slot = bot
                    .party_membership
                    .party
                    .and_then(|party| query_party.get(party).ok())
                    .map_or(0, |party| formation_slot(party, entity));
                let angle = FORMATION_ANGLES_DEG[slot % FORMATION_ANGLES_DEG.len()].to_radians();
                let from_leader = (bot.position.position.xy() - owner_position.position.xy())
                    .try_normalize()
                    .unwrap_or(Vec2::X);
                let offset = Vec2::from_angle(angle).rotate(from_leader) * FORMATION_DISTANCE;
                let destination =
                    owner_position.position + bevy::math::Vec3::new(offset.x, offset.y, 0.0);

                // Keep the leader's pace: walk with a walking leader in town,
                // run to catch up when far behind.
                let move_mode = if distance > CATCH_UP_DISTANCE {
                    MoveMode::Run
                } else {
                    query_owner_move_mode
                        .get(owner_entity)
                        .copied()
                        .unwrap_or(MoveMode::Run)
                };

                commands.entity(entity).insert(NextCommand::with_move(
                    destination,
                    None,
                    Some(move_mode),
                ));
            }
            ActionState::Cancelled => {
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
