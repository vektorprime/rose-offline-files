use bevy::{
    ecs::query::QueryData,
    math::Vec3Swizzles,
    prelude::{Component, Entity, MessageWriter, Query, Res, With},
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rose_game_common::components::Level;

use crate::game::{
    components::{ClientEntityType, Party, PartyMembership, Position},
    events::PartyEvent,
    resources::ClientEntityList,
};

use super::{create_bot::BotBuild, BotQueryFilterAlive};

const PARTY_SEARCH_DISTANCE: f32 = 2000.0f32;
const PARTY_LEVEL_DIFFERENCE: i32 = 10;
const PARTY_DESIRED_SIZE: usize = 4;

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct CanPartyInviteNearbyBot {
    pub score: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct PartyInviteNearbyBot;

#[derive(QueryData)]
pub struct BotQuery<'w> {
    entity: Entity,
    bot_build: &'w BotBuild,
    party_membership: &'w PartyMembership,
    position: &'w Position,
    level: &'w Level,
}

fn can_invite(
    bot: &BotQueryItem,
    nearby_bot: &BotQueryItem,
    query_party: &Query<&Party>,
) -> bool {
    if nearby_bot.party_membership.party.is_some() {
        return false;
    }

    if nearby_bot.bot_build.job_id == bot.bot_build.job_id {
        return false;
    }

    if (nearby_bot.level.level as i32 - bot.level.level as i32).abs() > PARTY_LEVEL_DIFFERENCE {
        return false;
    }

    if let Some(party_entity) = bot.party_membership.party {
        if let Ok(party) = query_party.get(party_entity) {
            if party.owner != bot.entity || party.members.len() >= PARTY_DESIRED_SIZE {
                return false;
            }
        }
    }

    true
}

pub fn score_can_party_invite_nearby_bot(
    mut query: Query<(&CanPartyInviteNearbyBot, &Actor, &mut Score)>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    query_party: Query<&Party>,
    client_entity_list: Res<ClientEntityList>,
) {
    for (scorer, &Actor(bot_entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok(bot) = query_bot.get(bot_entity) else {
            continue;
        };

        if bot.party_membership.party.is_some() {
            if let Some(party_entity) = bot.party_membership.party {
                if let Ok(party) = query_party.get(party_entity) {
                    if party.owner != bot_entity {
                        // Only party owner can send invites
                        continue;
                    }
                }
            }
        }

        let Some(zone_entities) = client_entity_list.get_zone(bot.position.zone_id) else {
            continue;
        };

        // Are there any nearby bots which would make a good party member
        if zone_entities
            .iter_entity_type_within_distance(
                bot.position.position.xy(),
                PARTY_SEARCH_DISTANCE,
                &[ClientEntityType::Character],
            )
            .any(|(nearby_entity, _)| {
                query_bot
                    .get(nearby_entity)
                    .ok()
                    .map_or(false, |nearby_bot| can_invite(&bot, &nearby_bot, &query_party))
            })
        {
            score.set(scorer.score);
        }
    }
}

pub fn action_party_invite_nearby_bot(
    mut query: Query<(&Actor, &mut ActionState), With<PartyInviteNearbyBot>>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    query_party: Query<&Party>,
    client_entity_list: Res<ClientEntityList>,
    mut party_events: MessageWriter<PartyEvent>,
) {
    for (&Actor(bot_entity), mut state) in query.iter_mut() {
        let Ok(bot) = query_bot.get(bot_entity) else {
            continue;
        };

        match *state {
            ActionState::Requested => {
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                let Some(zone_entities) = client_entity_list.get_zone(bot.position.zone_id) else {
                    *state = ActionState::Failure;
                    continue;
                };

                *state = ActionState::Failure;

                for nearby_bot in zone_entities
                    .iter_entity_type_within_distance(
                        bot.position.position.xy(),
                        PARTY_SEARCH_DISTANCE,
                        &[ClientEntityType::Character],
                    )
                    .filter_map(|(nearby_entity, _)| query_bot.get(nearby_entity).ok())
                {
                    if can_invite(&bot, &nearby_bot, &query_party) {
                        party_events.write(PartyEvent::Invite {
                            owner_entity: bot.entity,
                            invited_entity: nearby_bot.entity,
                        });
                        *state = ActionState::Success;
                        break;
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
