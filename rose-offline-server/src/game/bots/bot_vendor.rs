//! Vendor shop bots.
//!
//! These bots stand in towns with a personal store full of random gear,
//! consumables and gems at reasonable prices, so towns feel like living
//! marketplaces. They use the existing `PersonalStore` + `Inventory` systems,
//! so players can browse and buy from them exactly like any other character's
//! personal store.

use bevy::prelude::{Commands, Component, Entity, Query, With};
use big_brain::{
    prelude::{ActionBuilder, ActionState, Actor, Highest},
    thinker::{Thinker, ThinkerBuilder},
};
use rand::{seq::SliceRandom, Rng};
use rose_data::{AbilityType, EquipmentItem, Item, ItemReference, ItemType, StackableItem};
use rose_game_common::components::{Inventory, Money};

use crate::game::{
    bots::{
        bot_build_artisan, bot_thinker_with_role, BotBehaviorConfig, BotRole,
        BotQueryFilterAlive,
    },
    bundles::CharacterBundle,
    components::{
        Bank, ClanMembership, Command, CommandData, Cooldowns, DamageSources, HealthPoints,
        Hotbar, ManaPoints, MotionData, MoveMode, MoveSpeed, NextCommand, PartyMembership,
        PassiveRecoveryTime, PersonalStore, Position, QuestState, SkillList, Stamina,
        StatusEffects, StatusEffectsRegen, Team,
    },
    resources::{GameData, ClientEntityList},
};

use super::{
    bot_revive::{IsDead, ReviveCurrentZone},
    create_bot::bot_create_with_build,
    names::BotNames,
};

/// Number of vendor bots spawned per town zone.
pub const VENDOR_BOTS_PER_TOWN: u32 = 12;

/// Maximum items a vendor bot starts with in its store.
const MAX_VENDOR_ITEMS: usize = 24;

/// Marker component identifying vendor shop bots.
#[derive(Component)]
pub struct VendorBot;

/// Keeps the vendor bot standing with its store open.
#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct VendorIdle;

/// Builds the thinker used by vendor bots: revive when dead, otherwise stay
/// in the vending state.
pub fn bot_vendor_thinker() -> ThinkerBuilder {
    Thinker::build()
        .picker(Highest)
        .when(IsDead { score: 1.0 }, ReviveCurrentZone)
        .otherwise(VendorIdle)
}

/// Keeps the vending state active, re-opening the store after a revive.
pub fn action_vendor_idle(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<VendorIdle>>,
    query_command: Query<&Command, BotQueryFilterAlive>,
) {
    for (&Actor(entity), mut state) in query.iter_mut() {
        match *state {
            ActionState::Requested => {
                *state = ActionState::Executing;
            }
            ActionState::Executing => {
                if let Ok(command) = query_command.get(entity) {
                    if !matches!(command.command, CommandData::PersonalStore) {
                        commands
                            .entity(entity)
                            .insert(NextCommand::with_personal_store());
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

/// Extracts the level requirement of an item, defaulting to 0.
fn item_level_requirement(game_data: &GameData, item: ItemReference) -> u32 {
    game_data
        .items
        .get_base_item(item)
        .and_then(|item_data| {
            item_data
                .equip_ability_requirement
                .iter()
                .find(|(ability_type, _)| *ability_type == AbilityType::Level)
                .map(|(_, level)| *level)
        })
        .unwrap_or(0)
}

/// Maximum item number that fits the network protocol's 10-bit item_number
/// field. Items with larger numbers cannot be sent to clients.
const MAX_PACKET_ITEM_NUMBER: usize = 1023;

/// Picks random level-appropriate items of the given type.
fn pick_items(
    game_data: &GameData,
    rng: &mut impl Rng,
    item_type: ItemType,
    max_level: u32,
    count: usize,
) -> Vec<ItemReference> {
    let mut candidates: Vec<ItemReference> = game_data
        .items
        .iter_items(item_type)
        .filter(|item| item.item_number <= MAX_PACKET_ITEM_NUMBER)
        .filter(|item| item_level_requirement(game_data, *item) <= max_level)
        .collect();

    candidates.shuffle(rng);
    candidates.truncate(count);
    candidates
}

/// Calculates a reasonable vendor price for an item: the standard NPC buy
/// price with a small random discount/markup, rounded to a neat value.
fn vendor_price(game_data: &GameData, rng: &mut impl Rng, item: ItemReference) -> Money {
    let base_price = game_data
        .ability_value_calculator
        .calculate_npc_store_item_buy_price(&game_data.items, item, 0, 50, 50)
        .unwrap_or(1)
        .max(1) as f32;

    let price = (base_price * rng.gen_range(0.8..1.2)).round();
    let price = (price / 10.0).round() * 10.0;

    Money(price.max(1.0) as i64)
}

/// Creates a random item for the vendor's inventory.
fn create_vendor_item(
    game_data: &GameData,
    rng: &mut impl Rng,
    item: ItemReference,
) -> Option<Item> {
    if item.item_type.is_stackable_item() {
        let quantity = rng.gen_range(5..=50);
        StackableItem::new(item, quantity).map(Item::from)
    } else {
        let durability = game_data
            .items
            .get_base_item(item)
            .map(|item_data| item_data.durability)
            .unwrap_or(0);
        EquipmentItem::new(item, durability).map(Item::from)
    }
}

/// Spawns a vendor shop bot in a town.
pub fn spawn_vendor_bot(
    commands: &mut Commands,
    game_data: &GameData,
    client_entity_list: &mut ClientEntityList,
    bot_names: &mut BotNames,
    level: u32,
    position: Position,
) -> Entity {
    let mut rng = rand::thread_rng();

    let name = bot_names.next_unique_name(&mut rng);
    let bot_build = bot_build_artisan();
    let bot_data = bot_create_with_build(game_data, name, level, &bot_build);

    // Build the vendor inventory with random gear, consumables and gems.
    let mut inventory = Inventory::new();
    inventory.money = Money(rng.gen_range(100_000..500_000));

    let max_item_level = level + 10;
    let mut store = PersonalStore::new(format!("{}'s Shop", bot_data.info.name), 0);
    let mut items_added = 0usize;

    // Mix of equipment, consumables and gems.
    let equipment_types = [
        ItemType::Weapon,
        ItemType::SubWeapon,
        ItemType::Head,
        ItemType::Body,
        ItemType::Hands,
        ItemType::Feet,
        ItemType::Back,
    ];

    let mut equipment_items: Vec<ItemReference> = Vec::new();
    for item_type in equipment_types.iter() {
        let count = if *item_type == ItemType::Weapon { 3 } else { 2 };
        equipment_items.extend(pick_items(
            game_data,
            &mut rng,
            *item_type,
            max_item_level,
            count,
        ));
    }

    let consumables = pick_items(game_data, &mut rng, ItemType::Consumable, max_item_level, 6);
    let gems = pick_items(game_data, &mut rng, ItemType::Gem, u32::MAX, 4);

    for item in equipment_items
        .iter()
        .chain(consumables.iter())
        .chain(gems.iter())
    {
        if items_added >= MAX_VENDOR_ITEMS {
            break;
        }

        let price = vendor_price(game_data, &mut rng, *item);
        let Some(item) = create_vendor_item(game_data, &mut rng, *item) else {
            continue;
        };

        if let Ok((slot, _)) = inventory.try_add_item(item) {
            if store.add_sell_item(slot, price).is_ok() {
                items_added += 1;
            }
        }
    }

    let entity = commands
        .spawn((
            bot_thinker_with_role(&BotBehaviorConfig::default(), BotRole::Vendor),
            bot_build,
            BotRole::Vendor,
            VendorBot,
            CharacterBundle {
                ability_values: game_data.ability_value_calculator.calculate(
                    &bot_data.info,
                    &bot_data.level,
                    &bot_data.equipment,
                    &bot_data.basic_stats,
                    &bot_data.skill_list,
                    &StatusEffects::new(),
                ),
                basic_stats: bot_data.basic_stats,
                bank: Bank::default(),
                cooldowns: Cooldowns::default(),
                command: Command::default(),
                damage_sources: DamageSources::default_character(),
                equipment: bot_data.equipment,
                experience_points: bot_data.experience_points,
                health_points: bot_data.health_points,
                hotbar: Hotbar::default(),
                inventory,
                level: bot_data.level,
                mana_points: bot_data.mana_points,
                motion_data: MotionData::from_character(&game_data.motions, 0, bot_data.info.gender),
                info: bot_data.info,
                move_mode: MoveMode::Walk,
                move_speed: MoveSpeed::new(100.0),
                next_command: NextCommand::with_personal_store(),
                party_membership: PartyMembership::default(),
                passive_recovery_time: PassiveRecoveryTime::default(),
                position: position.clone(),
                quest_state: QuestState::default(),
                skill_list: bot_data.skill_list,
                skill_points: bot_data.skill_points,
                stamina: bot_data.stamina,
                stat_points: bot_data.stat_points,
                status_effects: StatusEffects::new(),
                status_effects_regen: StatusEffectsRegen::new(),
                team: Team::default_character(),
                union_membership: bot_data.union_membership,
                clan_membership: ClanMembership::default(),
            },
            store,
        ))
        .id();

    crate::game::bundles::client_entity_join_zone(
        commands,
        client_entity_list,
        entity,
        crate::game::components::ClientEntityType::Character,
        &position,
    )
    .expect("Failed to join vendor bot into zone");

    entity
}
