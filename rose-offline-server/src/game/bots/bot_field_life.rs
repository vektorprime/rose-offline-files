//! Field routine for hunting bots: breaks, restock trips and potions.
//!
//! Real players do not grind forever without pause. Every so often a hunter
//! takes a break (sits or stands around for a minute or two), walks over to a
//! merchant NPC when its bags are full / it ran out of potions / it is just
//! time for a trip, and drinks potions when a fight goes badly. Potions are
//! used through `UseItemEvent::Item`, which applies the potion and broadcasts
//! the drinking effect to nearby players without needing an inventory slot;
//! the bot's stock is a counter in `BotLife` that restock trips refill.

use std::time::{Duration, Instant};

use bevy::{
    math::{Vec2, Vec3, Vec3Swizzles},
    prelude::{
        Commands, Component, Entity, MessageWriter, Query, Res, ResMut, Resource, With, Without,
    },
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rand::Rng;
use rose_data::{
    AbilityType, Item, ItemClass, ItemReference, ItemType, StackableItem, StatusEffectType,
};

use crate::game::{
    components::{
        AbilityValues, Command, CommandData, Dead, HealthPoints, Inventory, InventoryPageType,
        Level, LlmBuddyBot, ManaPoints, Money, MoveMode, NextCommand, Party, PartyMembership,
        Position, StatusEffects,
    },
    events::UseItemEvent,
    resources::GameData,
};

use super::{
    bot_chat::{BotChatEvent, BotChatter},
    bot_personality::{BotActivity, BotLife, BotPersonality},
    BotBuild, BotCombatTarget, BotQueryFilterAlive, BotQueryFilterAliveNoTarget, VendorBot,
};

/// Within this distance of the merchant the bot starts "shopping".
const SHOP_RADIUS: f32 = 250.0;
/// A stopped bot that is not at the merchant re-issues its walk after this.
const REORDER_DELAY: Duration = Duration::from_millis(1200);

fn is_party_follower(
    entity: Entity,
    party_membership: &PartyMembership,
    query_party: &Query<&Party>,
) -> bool {
    party_membership
        .party
        .and_then(|party| query_party.get(party).ok())
        .is_some_and(|party| party.owner != entity)
}

// ---------------------------------------------------------------------------
// Breaks
// ---------------------------------------------------------------------------

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldTakeBreak {
    pub score: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct TakeBreak;

pub fn score_should_take_break(
    mut query: Query<(&ShouldTakeBreak, &Actor, &mut Score)>,
    query_bot: Query<(&Command, &BotLife, &PartyMembership), BotQueryFilterAliveNoTarget>,
    query_party: Query<&Party>,
) {
    let now = Instant::now();

    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok((command, life, party_membership)) = query_bot.get(entity) else {
            continue;
        };

        // A running break keeps its score (also after a fight interrupted it).
        if life.break_until.is_some() {
            score.set(scorer.score);
            continue;
        }

        if now < life.next_break_at
            || is_party_follower(entity, party_membership, &query_party)
            || !(command.is_stop() || command.is_sit())
        {
            continue;
        }

        score.set(scorer.score);
    }
}

#[allow(clippy::too_many_arguments)]
fn end_break(
    commands: &mut Commands,
    rng: &mut impl Rng,
    entity: Entity,
    command: &Command,
    personality: &BotPersonality,
    life: &mut BotLife,
    chatter: &mut BotChatter,
    now: Instant,
) {
    if command.is_sit() {
        commands.entity(entity).insert(NextCommand::with_standing());
    }
    life.break_until = None;
    life.next_break_at = now + personality.next_break_in(rng);
    life.activity = BotActivity::Idle;
    chatter.notify(rng, BotChatEvent::BreakEnd);
}

pub fn action_take_break(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<TakeBreak>>,
    mut query_bot: Query<
        (&Command, &BotPersonality, &mut BotLife, &mut BotChatter),
        BotQueryFilterAlive,
    >,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();

    for (&Actor(entity), mut state) in query.iter_mut() {
        let Ok((command, personality, mut life, mut chatter)) = query_bot.get_mut(entity) else {
            *state = ActionState::Failure;
            continue;
        };

        match *state {
            ActionState::Requested => {
                let break_until = life.break_until;
                match break_until {
                    Some(until) if now >= until => {
                        // The break ran out while the bot was busy.
                        end_break(
                            &mut commands,
                            &mut rng,
                            entity,
                            command,
                            personality,
                            &mut life,
                            &mut chatter,
                            now,
                        );
                        *state = ActionState::Success;
                        continue;
                    }
                    Some(_) => {}
                    None => {
                        life.break_until = Some(now + personality.break_length(&mut rng));
                        chatter.notify(&mut rng, BotChatEvent::BreakStart);
                    }
                }

                life.activity = BotActivity::OnBreak;
                // Most sit down; some just stand around.
                if !command.is_sit() && rng.gen_bool(0.75) {
                    commands.entity(entity).insert(NextCommand::with_sitting());
                }
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                if life.break_until.map_or(true, |until| now >= until) {
                    end_break(
                        &mut commands,
                        &mut rng,
                        entity,
                        command,
                        personality,
                        &mut life,
                        &mut chatter,
                        now,
                    );
                    *state = ActionState::Success;
                }
            }
            ActionState::Cancelled => {
                // Keep `break_until`: the break resumes after the interruption.
                life.activity = BotActivity::Idle;
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Restock trips
// ---------------------------------------------------------------------------

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct NeedsRestock {
    pub score: f32,
}

#[derive(Debug, Clone, Default, Component, ActionBuilder)]
pub struct RestockTrip {
    destination: Option<Vec3>,
    ordered_at: Option<Instant>,
    deadline: Option<Instant>,
    shopping_until: Option<Instant>,
}

pub fn score_needs_restock(
    mut query: Query<(&NeedsRestock, &Actor, &mut Score)>,
    query_bot: Query<(&BotLife, &PartyMembership), BotQueryFilterAliveNoTarget>,
    query_party: Query<&Party>,
) {
    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok((life, party_membership)) = query_bot.get(entity) else {
            continue;
        };

        if life.restock_due && !is_party_follower(entity, party_membership, &query_party) {
            score.set(scorer.score);
        }
    }
}

/// Spot in front of the merchant NPC closest to `from`, or the zone start
/// (usually a camp or the zone entrance) when the zone has no NPCs.
fn merchant_spot(zone_data: &rose_data::ZoneData, from: Vec3, rng: &mut impl Rng) -> Vec3 {
    let anchor = zone_data
        .npcs
        .iter()
        .min_by(|a, b| {
            a.position
                .xy()
                .distance_squared(from.xy())
                .total_cmp(&b.position.xy().distance_squared(from.xy()))
        })
        .map(|npc| npc.position)
        .unwrap_or(zone_data.start_position);

    let direction = (from.xy() - anchor.xy()).try_normalize().unwrap_or(Vec2::X);
    let distance = rng.gen_range(160.0..220.0);
    let spot = anchor + Vec3::new(direction.x * distance, direction.y * distance, 0.0);

    // A spot the bot can walk to (NPCs stand behind counters and on
    // platforms); the trip deadline covers the rare unreachable merchant.
    zone_data
        .nav_grid
        .as_ref()
        .and_then(|nav_grid| nav_grid.reachable_point(from, spot, 1200.0))
        .unwrap_or(spot)
}

/// "Sells" the loot (equipment and material pages) for roughly the NPC buy
/// price and restocks potions.
fn complete_restock(
    rng: &mut impl Rng,
    game_data: &GameData,
    life: &mut BotLife,
    inventory: &mut Inventory,
    now: Instant,
) {
    let mut earned: i64 = 0;
    for slot in inventory
        .equipment
        .slots
        .iter_mut()
        .chain(inventory.materials.slots.iter_mut())
    {
        if let Some(item) = slot.take() {
            let quantity = match &item {
                Item::Stackable(stackable) => stackable.quantity as i64,
                Item::Equipment(_) => 1,
            };
            let price = game_data
                .items
                .get_base_item(item.get_item_reference())
                .map_or(0, |item_data| item_data.base_price as i64);
            earned += price * quantity / 4;
        }
    }
    inventory.try_add_money(Money(earned)).ok();

    life.hp_potions = life.hp_potions.max(rng.gen_range(15..=30));
    if life.mp_potion_user {
        life.mp_potions = life.mp_potions.max(rng.gen_range(5..=15));
    }
    life.restock_due = false;
    life.next_restock_at = now + Duration::from_secs(rng.gen_range(30 * 60..80 * 60));
    life.activity = BotActivity::Idle;
}

#[allow(clippy::type_complexity)]
pub fn action_restock_trip(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState, &mut RestockTrip)>,
    mut query_bot: Query<
        (
            &Position,
            &Command,
            &mut BotLife,
            &mut BotChatter,
            &mut Inventory,
        ),
        BotQueryFilterAlive,
    >,
    game_data: Res<GameData>,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();

    for (&Actor(entity), mut state, mut trip) in query.iter_mut() {
        let Ok((position, command, mut life, mut chatter, mut inventory)) =
            query_bot.get_mut(entity)
        else {
            *state = ActionState::Failure;
            continue;
        };

        match *state {
            ActionState::Requested => {
                let Some(zone_data) = game_data.zones.get_zone(position.zone_id) else {
                    *state = ActionState::Failure;
                    continue;
                };

                let destination = merchant_spot(zone_data, position.position, &mut rng);
                let distance = position.position.xy().distance(destination.xy());
                commands.entity(entity).insert(NextCommand::with_move(
                    destination,
                    None,
                    Some(MoveMode::Run),
                ));

                trip.destination = Some(destination);
                trip.ordered_at = Some(now);
                trip.deadline =
                    Some(now + Duration::from_secs_f32((distance / 150.0).max(30.0) + 30.0));
                trip.shopping_until = None;

                if life.activity != BotActivity::Restocking {
                    life.activity = BotActivity::Restocking;
                    chatter.notify(&mut rng, BotChatEvent::Restocking);
                }
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                let Some(destination) = trip.destination else {
                    *state = ActionState::Failure;
                    continue;
                };

                if let Some(shopping_until) = trip.shopping_until {
                    if now >= shopping_until {
                        complete_restock(&mut rng, &game_data, &mut life, &mut inventory, now);
                        *state = ActionState::Success;
                    }
                    continue;
                }

                // Busy with a pickup, emote, ...: wait.
                if !matches!(
                    command.command,
                    CommandData::Stop { .. } | CommandData::Move { target: None, .. }
                ) {
                    continue;
                }

                if position.position.xy().distance(destination.xy()) <= SHOP_RADIUS
                    || trip.deadline.is_some_and(|deadline| now >= deadline)
                {
                    // At the merchant: browse and trade for a while.
                    trip.shopping_until = Some(now + Duration::from_secs(rng.gen_range(12..35)));
                    if !command.is_stop() {
                        commands.entity(entity).insert(NextCommand::with_stop(true));
                    }
                } else if command.is_stop_for(REORDER_DELAY)
                    && trip.ordered_at.is_some_and(|ordered_at| {
                        now.saturating_duration_since(ordered_at) > REORDER_DELAY
                    })
                {
                    trip.ordered_at = Some(now);
                    commands.entity(entity).insert(NextCommand::with_move(
                        destination,
                        None,
                        Some(MoveMode::Run),
                    ));
                }
            }
            ActionState::Cancelled => {
                // `restock_due` stays set: the trip resumes after the interruption.
                life.activity = BotActivity::Idle;
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Potions and upkeep
// ---------------------------------------------------------------------------

/// HP and MP potions bots drink, sorted from weakest to strongest, with
/// their level requirement.
#[derive(Resource, Default)]
pub struct BotPotionTable {
    hp: Vec<(u32, i32, ItemReference)>,
    mp: Vec<(u32, i32, ItemReference)>,
}

impl BotPotionTable {
    /// The potion a player of `level` would typically carry: stronger ones as
    /// the level goes up.
    fn pick(list: &[(u32, i32, ItemReference)], level: u32) -> Option<ItemReference> {
        let usable: Vec<ItemReference> = list
            .iter()
            .filter(|(required_level, _, _)| *required_level <= level)
            .map(|(_, _, item)| *item)
            .collect();
        if usable.is_empty() {
            return None;
        }
        let index = ((level.min(150) as f32 / 150.0) * (usable.len() - 1) as f32).round() as usize;
        usable.get(index.min(usable.len() - 1)).copied()
    }

    pub fn hp_potion(&self, level: u32) -> Option<ItemReference> {
        Self::pick(&self.hp, level)
    }

    pub fn mp_potion(&self, level: u32) -> Option<ItemReference> {
        Self::pick(&self.mp, level)
    }
}

/// Finds the shop potions (medicine / food that restore HP or MP over time).
pub fn bot_potion_table_init_system(mut table: ResMut<BotPotionTable>, game_data: Res<GameData>) {
    for item_reference in game_data.items.iter_items(ItemType::Consumable) {
        let Some(item_data) = game_data
            .items
            .get_consumable_item(item_reference.item_number)
        else {
            continue;
        };
        if !matches!(
            item_data.item_data.class,
            ItemClass::Medicine | ItemClass::Food
        ) || item_data.item_data.name.is_empty()
            || item_data.item_data.base_price == 0
        {
            continue;
        }

        let required_level = match item_data.ability_requirement {
            None => 0,
            Some((AbilityType::Level, level)) => level.max(0) as u32,
            Some(_) => continue,
        };
        let Some((base_status_effect_id, value)) = item_data.apply_status_effect else {
            continue;
        };
        let Some(base_status_effect) = game_data
            .status_effects
            .get_status_effect(base_status_effect_id)
        else {
            continue;
        };

        let restores = |hp: bool| {
            base_status_effect
                .apply_status_effects
                .iter()
                .any(|(id, _)| {
                    game_data
                        .status_effects
                        .get_status_effect(*id)
                        .is_some_and(|data| {
                            if hp {
                                matches!(data.status_effect_type, StatusEffectType::IncreaseHp)
                            } else {
                                matches!(data.status_effect_type, StatusEffectType::IncreaseMp)
                            }
                        })
                })
        };

        if restores(true) {
            table.hp.push((required_level, value, item_reference));
        } else if restores(false) {
            table.mp.push((required_level, value, item_reference));
        }
    }

    table.hp.sort_by_key(|(_, value, _)| *value);
    table.mp.sort_by_key(|(_, value, _)| *value);
    log::info!(
        "Bot potions: {} hp potions, {} mp potions",
        table.hp.len(),
        table.mp.len()
    );
}

/// Drinks potions in fights and flags restock trips (no potions left, bags
/// full, or simply time for one). Each bot is checked a couple of times a
/// second rather than every tick.
#[allow(clippy::type_complexity)]
pub fn bot_upkeep_system(
    mut query: Query<
        (
            Entity,
            &Command,
            &Level,
            &HealthPoints,
            &ManaPoints,
            &AbilityValues,
            &StatusEffects,
            &Inventory,
            Option<&BotCombatTarget>,
            &BotPersonality,
            &mut BotLife,
            &mut BotChatter,
        ),
        (
            With<BotBuild>,
            Without<LlmBuddyBot>,
            Without<VendorBot>,
            Without<Dead>,
        ),
    >,
    potion_table: Res<BotPotionTable>,
    mut use_item_events: MessageWriter<UseItemEvent>,
) {
    let mut rng = rand::thread_rng();
    let now = Instant::now();

    for (
        entity,
        command,
        level,
        health_points,
        mana_points,
        ability_values,
        status_effects,
        inventory,
        combat_target,
        personality,
        mut life,
        mut chatter,
    ) in query.iter_mut()
    {
        if now < life.next_upkeep_at {
            continue;
        }
        life.next_upkeep_at = now + Duration::from_millis(rng.gen_range(500..900));

        if !life.restock_due
            && (now >= life.next_restock_at
                || !inventory.has_empty_slot(InventoryPageType::Equipment)
                || !inventory.has_empty_slot(InventoryPageType::Materials))
        {
            life.restock_due = true;
        }

        if command.is_dead() || now < life.next_potion_at {
            continue;
        }
        let in_fight = combat_target.is_some()
            || matches!(
                command.command,
                CommandData::Attack { .. } | CommandData::CastSkill { .. }
            );
        if !in_fight {
            continue;
        }

        let max_health = ability_values.get_max_health().max(1);
        let max_mana = ability_values.get_max_mana();
        let health_ratio = health_points.hp as f32 / max_health as f32;
        let mana_ratio = if max_mana > 0 {
            mana_points.mp as f32 / max_mana as f32
        } else {
            1.0
        };

        let potion = if health_ratio < personality.potion_below_hp
            && life.hp_potions > 0
            && status_effects.active[StatusEffectType::IncreaseHp].is_none()
        {
            potion_table
                .hp_potion(level.level)
                .map(|potion| (potion, true))
        } else if mana_ratio < 0.2
            && life.mp_potions > 0
            && status_effects.active[StatusEffectType::IncreaseMp].is_none()
        {
            potion_table
                .mp_potion(level.level)
                .map(|potion| (potion, false))
        } else {
            None
        };

        let Some((potion, is_hp)) = potion else {
            continue;
        };
        let Some(item) = StackableItem::new(potion, 1).map(Item::from) else {
            continue;
        };

        use_item_events.write(UseItemEvent::from_item(entity, item));
        life.next_potion_at = now + Duration::from_millis(rng.gen_range(6000..10000));
        if is_hp {
            life.hp_potions -= 1;
            if life.hp_potions == 0 {
                life.restock_due = true;
                chatter.notify(&mut rng, BotChatEvent::OutOfPotions);
            }
        } else {
            life.mp_potions -= 1;
        }
    }
}
