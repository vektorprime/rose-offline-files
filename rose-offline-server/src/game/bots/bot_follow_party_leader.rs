use bevy::{
    ecs::query::QueryData,
    math::Vec3Swizzles,
    prelude::{Commands, Component, Entity, Query, With},
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};

use crate::game::{
    bots::{BotQueryFilterAlive, BotQueryFilterAliveNoTarget, BotCombatTarget},
    components::{Command, MoveMode, NextCommand, Party, PartyMembership, Position},
};

use super::Dead;

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

                if distance > 200.0 {
                    // Move to a point close to the leader.
                    let direction = (owner_position.position.xy()
                        - bot.position.position.xy())
                        .normalize();
                    let target_offset = (distance - 200.0).max(0.0);
                    let destination = bot.position.position
                        + bevy::math::Vec3::new(
                            direction.x * target_offset,
                            direction.y * target_offset,
                            0.0,
                        );

                    commands.entity(entity).insert(NextCommand::with_move(
                        destination,
                        None,
                        Some(MoveMode::Run),
                    ));
                }
            }
            ActionState::Cancelled => {
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
