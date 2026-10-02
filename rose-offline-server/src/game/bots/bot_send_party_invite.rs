use std::time::{Duration, Instant};

use bevy::{
    ecs::query::QueryData,
    math::Vec3Swizzles,
    prelude::{Component, Entity, MessageWriter, Query, Res, With},
};
use rand::Rng;
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rose_game_common::components::Level;

use crate::game::{
    components::{ClientEntityType, Party, PartyMembership, Position},
    events::PartyEvent,
    resources::ClientEntityList,
};

use super::{bot_personality::BotScanCache, create_bot::BotBuild, BotQueryFilterAlive};

const PARTY_SEARCH_DISTANCE: f32 = 2000.0f32;
const PARTY_LEVEL_DIFFERENCE: i32 = 10;
const PARTY_DESIRED_SIZE: usize = 4;
/// How often a bot looks for party members (randomised per scan).
const PARTY_SCAN_INTERVAL_MS: std::ops::Range<u64> = 2000..5000;

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

    // Already asked, waiting for an answer.
    if nearby_bot
        .party_membership
        .pending_invites
        .contains(&bot.entity)
    {
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

fn has_party_candidate_nearby(
    bot: &BotQueryItem,
    query_bot: &Query<BotQuery, BotQueryFilterAlive>,
    query_party: &Query<&Party>,
    client_entity_list: &ClientEntityList,
) -> bool {
    let Some(zone_entities) = client_entity_list.get_zone(bot.position.zone_id) else {
        return false;
    };

    // Are there any nearby bots which would make a good party member
    zone_entities
        .iter_entity_type_within_distance(
            bot.position.position.xy(),
            PARTY_SEARCH_DISTANCE,
            &[ClientEntityType::Character],
        )
        .any(|(nearby_entity, _)| {
            query_bot
                .get(nearby_entity)
                .ok()
                .map_or(false, |nearby_bot| can_invite(bot, &nearby_bot, query_party))
        })
}

pub fn score_can_party_invite_nearby_bot(
    mut query: Query<(&CanPartyInviteNearbyBot, &Actor, &mut Score)>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    query_party: Query<&Party>,
    mut query_scan: Query<&mut BotScanCache>,
    client_entity_list: Res<ClientEntityList>,
) {
    let now = Instant::now();
    let mut rng = rand::thread_rng();

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

        // Looking around for party members every few seconds is plenty (and
        // gives a natural pause before bots that meet group up).
        let found = match query_scan.get_mut(bot_entity) {
            Ok(mut cache) => {
                if cache.next_party_scan.map_or(true, |next| now >= next) {
                    cache.party_candidate =
                        has_party_candidate_nearby(&bot, &query_bot, &query_party, &client_entity_list);
                    cache.next_party_scan =
                        Some(now + Duration::from_millis(rng.gen_range(PARTY_SCAN_INTERVAL_MS)));
                }
                cache.party_candidate
            }
            Err(_) => has_party_candidate_nearby(&bot, &query_bot, &query_party, &client_entity_list),
        };

        if found {
            score.set(scorer.score);
        }
    }
}

pub fn action_party_invite_nearby_bot(
    mut query: Query<(&Actor, &mut ActionState), With<PartyInviteNearbyBot>>,
    query_bot: Query<BotQuery, BotQueryFilterAlive>,
    query_party: Query<&Party>,
    mut query_scan: Query<&mut BotScanCache>,
    client_entity_list: Res<ClientEntityList>,
    mut party_events: MessageWriter<PartyEvent>,
) {
    let now = Instant::now();
    let mut rng = rand::thread_rng();

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

                // One invite per look around: the cached "candidate nearby"
                // would otherwise keep the thinker on this choice.
                if let Ok(mut cache) = query_scan.get_mut(bot_entity) {
                    cache.party_candidate = false;
                    cache.next_party_scan =
                        Some(now + Duration::from_millis(rng.gen_range(PARTY_SCAN_INTERVAL_MS)));
                }
            }
            ActionState::Cancelled => {
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
