use bevy::{
    math::Vec3Swizzles,
    prelude::{Commands, Component, Query, Res, Vec3, With},
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor};

use rand::{seq::SliceRandom, Rng};
use rose_game_common::components::Level;

use crate::game::{
    components::{ClientEntityType, Command, MoveMode, NextCommand, PartyMembership, Position},
    resources::ClientEntityList,
    GameData,
};

use super::{
    bot_personality::{BotArchetype, BotPersonality},
    BotQueryFilterAlive, IDLE_DURATION,
};

/// Characters counted around a spawn to judge how crowded it is.
const CROWD_RADIUS_MIN: f32 = 1000.0;
/// Random spots tried around a spawn before falling back to its center.
const SPAWN_SPOT_TRIES: usize = 4;
/// How far a spawn spot may move to reach walkable ground the bot can get to.
const SPAWN_SPOT_SNAP_CM: f32 = 1500.0;

#[derive(Debug, Default, Clone, Component, ActionBuilder)]
pub struct FindMonsterSpawns;

#[allow(clippy::type_complexity)]
pub fn action_find_monster_spawn(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<FindMonsterSpawns>>,
    query_entity: Query<
        (
            &Command,
            &Level,
            &Position,
            &PartyMembership,
            Option<&BotPersonality>,
        ),
        BotQueryFilterAlive,
    >,
    query_party_membership: Query<&PartyMembership>,
    client_entity_list: Res<ClientEntityList>,
    game_data: Res<GameData>,
) {
    let mut rng = rand::thread_rng();

    for (&Actor(entity), mut state) in query.iter_mut() {
        let Ok((command, level, position, party_membership, personality)) =
            query_entity.get(entity)
        else {
            continue;
        };

        match *state {
            ActionState::Requested => {
                // Stand a moment before heading off, like a player looking
                // around for the next spot (fights nearby still interrupt).
                if let Some(personality) = personality {
                    if command.is_stop()
                        && !command.is_stop_for(personality.reaction + personality.hunt_pause)
                    {
                        continue;
                    }
                }

                let Some(zone_data) = game_data.zones.get_zone(position.zone_id) else {
                    *state = ActionState::Failure;
                    continue;
                };

                let mut potential_spawns = Vec::with_capacity(zone_data.monster_spawns.len());

                for (i, spawn) in zone_data.monster_spawns.iter().enumerate() {
                    let mut total_level = 0;
                    let mut num_monsters = 0;

                    for npc_id in spawn
                        .basic_spawns
                        .iter()
                        .chain(spawn.tactic_spawns.iter())
                        .map(|(npc_id, _)| *npc_id)
                    {
                        if let Some(npc_data) = game_data.npcs.get_npc(npc_id) {
                            total_level += npc_data.level;
                            num_monsters += 1;
                        }
                    }

                    if num_monsters == 0 {
                        continue;
                    }

                    let average_level = total_level / num_monsters;
                    let level_difference = (level.level as i32 - average_level).abs();
                    let distance = position.position.xy().distance_squared(spawn.position.xy());

                    potential_spawns.push((distance, level_difference, i));
                }

                if potential_spawns.is_empty() {
                    *state = ActionState::Failure;
                    continue;
                }

                let explorer = personality
                    .is_some_and(|personality| personality.archetype == BotArchetype::Explorer);

                // Take the 10 spawns with smallest level delta to our character
                potential_spawns.sort_by(|lhs, rhs| lhs.1.cmp(&rhs.1));
                potential_spawns.truncate(10);

                // Then the closest ones (explorers roam further)
                potential_spawns.sort_by(|lhs, rhs| lhs.0.total_cmp(&rhs.0));
                potential_spawns.truncate(if explorer { 8 } else { 5 });

                // Prefer spots other hunters are not already camping.
                let zone_entities = client_entity_list.get_zone(position.zone_id);
                let weighted: Vec<(usize, f32)> = potential_spawns
                    .iter()
                    .map(|&(_, _, index)| {
                        let spawn = &zone_data.monster_spawns[index];
                        let radius = ((spawn.range * 100) as f32).max(CROWD_RADIUS_MIN);
                        let crowd = zone_entities.map_or(0, |zone_entities| {
                            zone_entities
                                .iter_entity_type_within_distance(
                                    spawn.position.xy(),
                                    radius,
                                    &[ClientEntityType::Character],
                                )
                                .filter(|(other, _)| {
                                    *other != entity
                                        && !(party_membership.party.is_some()
                                            && query_party_membership.get(*other).is_ok_and(
                                                |membership| {
                                                    membership.party == party_membership.party
                                                },
                                            ))
                                })
                                .count()
                        });
                        (index, 1.0 / (1.0 + crowd as f32).powi(2))
                    })
                    .collect();

                // Choose one randomly
                let Some(spawn_point) = weighted
                    .choose_weighted(&mut rng, |(_, weight)| *weight)
                    .ok()
                    .and_then(|(index, _)| zone_data.monster_spawns.get(*index))
                else {
                    *state = ActionState::Failure;
                    continue;
                };

                // Players run between hunting spots; some relaxed ones walk.
                let move_mode = if personality
                    .is_some_and(|personality| personality.archetype == BotArchetype::Casual)
                    && rng.gen_bool(0.3)
                {
                    MoveMode::Walk
                } else {
                    MoveMode::Run
                };

                // Move near the center of spawn, to a spot the bot can walk to
                // (spawn squares often reach over cliffs, banks and water).
                let range = (spawn_point.range * 100).max(500) as f32;
                let nav_grid = zone_data.nav_grid.as_ref();
                let destination = (0..SPAWN_SPOT_TRIES)
                    .map(|_| {
                        spawn_point.position
                            + Vec3::new(
                                rng.gen_range(-range..range),
                                rng.gen_range(-range..range),
                                0.0,
                            )
                    })
                    .chain(std::iter::once(spawn_point.position))
                    .find_map(|candidate| match nav_grid {
                        Some(nav_grid) => nav_grid.reachable_point(
                            position.position,
                            candidate,
                            SPAWN_SPOT_SNAP_CM,
                        ),
                        None => Some(candidate),
                    });
                let Some(destination) = destination else {
                    // Nothing near this spawn is reachable from here.
                    *state = ActionState::Failure;
                    continue;
                };
                commands.entity(entity).insert(NextCommand::with_move(
                    destination,
                    None,
                    Some(move_mode),
                ));
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                if command.is_stop_for(IDLE_DURATION) {
                    *state = ActionState::Success;
                }
            }
            ActionState::Cancelled => {
                *state = ActionState::Success;
            }
            _ => {}
        }
    }
}
