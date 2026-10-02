use std::time::{Duration, Instant};

use bevy::{
    math::Vec3Swizzles,
    prelude::{Commands, Component, Entity, Query, Res, With},
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rand::Rng;

use rose_data::{Item, ItemType};
use rose_game_common::components::{Inventory, InventoryPageType};

use crate::game::{
    bots::IDLE_DURATION,
    components::{ClientEntityType, Command, DroppedItem, ItemDrop, NextCommand, Owner, Position},
    resources::{ClientEntityList, GameData},
};

use super::{
    bot_chat::{BotChatEvent, BotChatter},
    bot_personality::BotScanCache,
    BotQueryFilterAlive,
};

const ITEM_DROP_SEARCH_DISTANCE: f32 = 1000.0f32;
/// Within this distance of a drop the pickup is started; the server allows
/// pickups up to 200 cm and the targeted move stops 150 cm short.
const PICKUP_REACH: f32 = 190.0;
/// A walk to a drop that stopped this long short of it is abandoned.
const GIVE_UP_STOPPED: Duration = Duration::from_millis(800);
/// How often a bot looks around for loot (randomised per scan).
const ITEM_SCAN_INTERVAL_MS: std::ops::Range<u64> = 350..700;

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct FindNearbyItemDrop {
    pub score: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct PickupNearestItemDrop;

#[derive(Component)]
pub struct PickupItemDrop {
    entity: Entity,
}

fn has_item_drop_nearby(
    entity: Entity,
    position: &Position,
    client_entity_list: &ClientEntityList,
    query_owner: &Query<&Owner>,
) -> bool {
    let Some(zone_entities) = client_entity_list.get_zone(position.zone_id) else {
        return false;
    };

    // Any item drop nearby which we own, or has no owner
    zone_entities
        .iter_entity_type_within_distance(
            position.position.xy(),
            ITEM_DROP_SEARCH_DISTANCE,
            &[ClientEntityType::ItemDrop],
        )
        .any(|(nearby_entity, _)| {
            query_owner
                .get(nearby_entity)
                .map_or(true, |owner| owner.entity == entity)
        })
}

pub fn score_find_nearby_item_drop_system(
    mut query: Query<(&FindNearbyItemDrop, &Actor, &mut Score)>,
    query_entity: Query<(&Command, &Inventory, &Position), BotQueryFilterAlive>,
    query_owner: Query<&Owner>,
    mut query_scan: Query<&mut BotScanCache>,
    client_entity_list: Res<ClientEntityList>,
) {
    let now = Instant::now();
    let mut rng = rand::thread_rng();

    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok((command, inventory, position)) = query_entity.get(entity) else {
            continue;
        };

        if command.is_dead() {
            // Cannot pick up items whilst dead
            continue;
        }

        if !inventory.has_empty_slot(InventoryPageType::Equipment)
            || !inventory.has_empty_slot(InventoryPageType::Consumables)
            || !inventory.has_empty_slot(InventoryPageType::Materials)
            || !inventory.has_empty_slot(InventoryPageType::Vehicles)
        {
            // Only pickup items if we have a space
            continue;
        }

        let found = match query_scan.get_mut(entity) {
            Ok(mut cache) => {
                if cache.next_item_scan.map_or(true, |next| now >= next) {
                    cache.item_found =
                        has_item_drop_nearby(entity, position, &client_entity_list, &query_owner);
                    cache.next_item_scan =
                        Some(now + Duration::from_millis(rng.gen_range(ITEM_SCAN_INTERVAL_MS)));
                }
                cache.item_found
            }
            Err(_) => has_item_drop_nearby(entity, position, &client_entity_list, &query_owner),
        };

        if found {
            score.set(scorer.score);
        }
    }
}

/// Name of a dropped item worth mentioning: gear and gems, not materials,
/// consumables or money.
fn notable_drop_name<'a>(game_data: &'a GameData, item_drop: &ItemDrop) -> Option<&'a str> {
    let Some(DroppedItem::Item(item)) = item_drop.item.as_ref() else {
        return None;
    };
    let notable = match item {
        Item::Equipment(_) => true,
        Item::Stackable(stackable) => stackable.item.item_type == ItemType::Gem,
    };
    if !notable {
        return None;
    }
    game_data
        .items
        .get_base_item(item.get_item_reference())
        .map(|item_data| item_data.name.as_str())
        .filter(|name| !name.is_empty())
}

#[allow(clippy::too_many_arguments)]
pub fn action_pickup_nearest_item_drop(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<PickupNearestItemDrop>>,
    query_entity: Query<(&Command, &Position, Option<&PickupItemDrop>)>,
    query_owner: Query<&Owner>,
    query_item_drop: Query<(&ItemDrop, &Position)>,
    mut query_bot_extra: Query<(Option<&mut BotChatter>, Option<&mut BotScanCache>)>,
    client_entity_list: Res<ClientEntityList>,
    game_data: Res<GameData>,
) {
    let mut rng = rand::thread_rng();

    for (&Actor(entity), mut state) in query.iter_mut() {
        let Ok((command, position, pickup_item_drop)) = query_entity.get(entity) else {
            continue;
        };

        match *state {
            ActionState::Requested => {
                let Some(zone_entities) = client_entity_list.get_zone(position.zone_id) else {
                    continue;
                };

                let mut nearest_item_drop = None;
                for (nearby_entity, nearby_position) in zone_entities
                    .iter_entity_type_within_distance(
                        position.position.xy(),
                        ITEM_DROP_SEARCH_DISTANCE,
                        &[ClientEntityType::ItemDrop],
                    )
                {
                    if query_owner
                        .get(nearby_entity)
                        .map_or(true, |owner| owner.entity == entity)
                    {
                        let distance = position
                            .position
                            .xy()
                            .distance_squared(nearby_position.xy());

                        if nearest_item_drop
                            .map_or(true, |(nearest_distance, _, _)| distance < nearest_distance)
                        {
                            nearest_item_drop = Some((distance, nearby_position, nearby_entity));
                        }
                    }
                }

                if let Some((_, nearest_position, nearest_entity)) = nearest_item_drop {
                    commands
                        .entity(entity)
                        .insert(NextCommand::with_move(
                            nearest_position,
                            Some(nearest_entity),
                            None,
                        ))
                        .insert(PickupItemDrop {
                            entity: nearest_entity,
                        });

                    *state = ActionState::Executing;
                } else {
                    // The cached "loot nearby" was stale: rescan next time.
                    if let Ok((_, Some(mut cache))) = query_bot_extra.get_mut(entity) {
                        cache.item_found = false;
                        cache.next_item_scan = None;
                    }
                    *state = ActionState::Failure;
                }
            }
            ActionState::Executing => {
                if let Some(pickup_item_drop) = pickup_item_drop {
                    let Ok((item_drop, drop_position)) =
                        query_item_drop.get(pickup_item_drop.entity)
                    else {
                        // Someone else got it, or it expired.
                        commands.entity(entity).remove::<PickupItemDrop>();
                        *state = ActionState::Failure;
                        continue;
                    };

                    let distance = position.position.xy().distance(drop_position.position.xy());
                    if distance > PICKUP_REACH {
                        // Still walking over (the targeted move follows the
                        // drop). Give up if the walk stopped short.
                        if command.is_stop_for(GIVE_UP_STOPPED) {
                            commands.entity(entity).remove::<PickupItemDrop>();
                            *state = ActionState::Failure;
                        }
                        continue;
                    }

                    // Next to the drop: pick it up.
                    commands
                        .entity(entity)
                        .insert(NextCommand::with_pickup_item_drop(pickup_item_drop.entity))
                        .remove::<PickupItemDrop>();

                    // A nice drop is worth a word.
                    if let Some(name) = notable_drop_name(&game_data, item_drop) {
                        if let Ok((Some(mut chatter), _)) = query_bot_extra.get_mut(entity) {
                            chatter.notify(&mut rng, BotChatEvent::Loot(name.to_string()));
                        }
                    }
                    continue;
                } else if command.is_stop_for(IDLE_DURATION) {
                    // We must have finished picking the item up
                    *state = ActionState::Success;
                }
            }
            ActionState::Cancelled => {
                commands
                    .entity(entity)
                    .insert(NextCommand::with_stop(true))
                    .remove::<PickupItemDrop>();

                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
