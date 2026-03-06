//! LLM Buddy Bot Creation
//!
//! This module provides functions for creating LLM-controlled buddy bot entities.

use bevy::ecs::prelude::*;
use uuid::Uuid;

use crate::game::{
    api::LlmBotCommand,
    bots::{
        bot_build_artisan, bot_build_bourgeois, bot_build_champion, bot_build_cleric,
        bot_build_knight, bot_build_mage, bot_build_raider, bot_build_scout, BotBuild,
    },
    bundles::{client_entity_join_zone, CharacterBundle},
    components::{
        CharacterInfo, ClientEntity, ClientEntityType, ClientEntityVisibility, Command,
        DamageSources, EquipmentItemDatabase, HealthPoints, LlmBuddyBot, ManaPoints, MotionData,
        NextCommand, Position, Stamina, Team,
    },
    events::PartyEvent,
    resources::{ClientEntityList, GameData},
    storage::{
        character::CharacterStorage,
        llm_buddy_bot::{LlmBuddyBotStorage, PositionData, VitalPoints},
    },
    systems::llm_buddy_bot_system::{LlmBotCommandReceiver, LlmBotManagerResource},
};

use rose_game_common::components::{MoveMode, MoveSpeed, StatusEffects};

/// Build type for LLM buddy bots
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmBuddyBotBuildType {
    Knight,
    Champion,
    Mage,
    Cleric,
    Raider,
    Scout,
    Bourgeois,
    Artisan,
    Random,
}

impl Default for LlmBuddyBotBuildType {
    fn default() -> Self {
        Self::Knight
    }
}

impl std::fmt::Display for LlmBuddyBotBuildType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Knight => write!(f, "knight"),
            Self::Champion => write!(f, "champion"),
            Self::Mage => write!(f, "mage"),
            Self::Cleric => write!(f, "cleric"),
            Self::Raider => write!(f, "raider"),
            Self::Scout => write!(f, "scout"),
            Self::Bourgeois => write!(f, "bourgeois"),
            Self::Artisan => write!(f, "artisan"),
            Self::Random => write!(f, "random"),
        }
    }
}

impl std::str::FromStr for LlmBuddyBotBuildType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "knight" => Ok(Self::Knight),
            "champion" => Ok(Self::Champion),
            "mage" => Ok(Self::Mage),
            "muse" | "cleric" => Ok(Self::Cleric),
            "cleric" => Ok(Self::Cleric),
            "raider" => Ok(Self::Raider),
            "scout" => Ok(Self::Scout),
            "bourgeois" => Ok(Self::Bourgeois),
            "artisan" => Ok(Self::Artisan),
            "random" => Ok(Self::Random),
            _ => Err(format!("Unknown build type: {}", s)),
        }
    }
}

/// Get the bot build for a given build type
pub fn get_bot_build(build_type: LlmBuddyBotBuildType) -> BotBuild {
    use rand::seq::SliceRandom;

    match build_type {
        LlmBuddyBotBuildType::Knight => bot_build_knight(),
        LlmBuddyBotBuildType::Champion => bot_build_champion(),
        LlmBuddyBotBuildType::Mage => bot_build_mage(),
        LlmBuddyBotBuildType::Cleric => bot_build_cleric(),
        LlmBuddyBotBuildType::Raider => bot_build_raider(),
        LlmBuddyBotBuildType::Scout => bot_build_scout(),
        LlmBuddyBotBuildType::Bourgeois => bot_build_bourgeois(),
        LlmBuddyBotBuildType::Artisan => bot_build_artisan(),
        LlmBuddyBotBuildType::Random => {
            let mut rng = rand::thread_rng();
            [
                bot_build_knight,
                bot_build_champion,
                bot_build_cleric,
                bot_build_mage,
                bot_build_scout,
                bot_build_raider,
                bot_build_artisan,
                bot_build_bourgeois,
            ]
            .choose(&mut rng)
            .unwrap()()
        }
    }
}

/// Configuration for creating an LLM buddy bot
#[derive(Debug, Clone)]
pub struct LlmBuddyBotConfig {
    /// Unique bot ID (will be generated if None)
    pub bot_id: Option<Uuid>,
    /// Bot name
    pub name: String,
    /// Bot level
    pub level: u16,
    /// Build type
    pub build_type: LlmBuddyBotBuildType,
    /// Gender (None = random)
    pub gender: Option<rose_game_common::components::CharacterGender>,
    /// Assigned player name
    pub assigned_player_name: String,
    /// Assigned player entity ID (character ID)
    pub assigned_player_id: u32,
    /// Initial zone ID
    pub zone_id: rose_data::ZoneId,
    /// Initial position
    pub position: bevy::math::Vec3,
    /// Follow distance
    pub follow_distance: f32,
}

/// Create an LLM buddy bot entity
///
/// This function creates a complete bot entity with all necessary components
/// for LLM control through the REST API.
pub fn create_llm_buddy_bot(
    commands: &mut Commands,
    game_data: &GameData,
    config: LlmBuddyBotConfig,
) -> Entity {
    // Get the bot build
    let bot_build = get_bot_build(config.build_type);

    // Create bot character data
    let bot_data = create_bot_character_data(game_data, &config, &bot_build);

    // Create the LlmBuddyBot component
    let bot_id = config.bot_id.unwrap_or_else(Uuid::new_v4);
    let mut llm_buddy_bot = LlmBuddyBot::new(
        bot_id,
        config.assigned_player_id,
        config.assigned_player_name.clone(),
    );
    llm_buddy_bot.set_follow_distance(config.follow_distance);

    // Calculate ability values
    let ability_values = game_data.ability_value_calculator.calculate(
        &bot_data.info,
        &bot_data.level,
        &bot_data.equipment,
        &bot_data.basic_stats,
        &bot_data.skill_list,
        &StatusEffects::new(),
    );

    // Calculate move speed
    let move_speed = MoveSpeed {
        speed: ability_values.run_speed as f32,
    };

    // Get weapon motion type from equipped weapon
    use rose_data::EquipmentIndex;
    let weapon_motion_type = game_data
        .items
        .get_equipped_weapon_item_data(&bot_data.equipment, EquipmentIndex::Weapon)
        .map(|item_data| item_data.motion_type)
        .unwrap_or(0) as usize;

    // Create motion data
    let motion_data = MotionData::from_character(
        game_data.motions.as_ref(),
        weapon_motion_type,
        bot_data.info.gender,
    );

    // Spawn the bot entity using CharacterBundle
    let entity = commands
        .spawn((
            // LLM Buddy Bot component
            llm_buddy_bot,
            // Character bundle
            CharacterBundle {
                ability_values,
                basic_stats: bot_data.basic_stats,
                bank: Default::default(),
                cooldowns: Default::default(),
                command: Command::default(),
                damage_sources: DamageSources::default_character(),
                equipment: bot_data.equipment,
                experience_points: bot_data.experience_points,
                health_points: HealthPoints {
                    hp: bot_data.health_points.hp,
                },
                hotbar: bot_data.hotbar,
                info: bot_data.info,
                inventory: bot_data.inventory,
                level: bot_data.level,
                mana_points: ManaPoints {
                    mp: bot_data.mana_points.mp,
                },
                motion_data,
                move_mode: MoveMode::Run,
                move_speed,
                next_command: NextCommand::default(),
                party_membership: Default::default(),
                passive_recovery_time: Default::default(),
                position: Position::new(config.position, config.zone_id),
                quest_state: bot_data.quest_state,
                skill_list: bot_data.skill_list,
                skill_points: bot_data.skill_points,
                stamina: Stamina {
                    stamina: bot_data.stamina.stamina,
                },
                stat_points: bot_data.stat_points,
                status_effects: StatusEffects::new(),
                status_effects_regen: Default::default(),
                team: Team::default_character(),
                union_membership: bot_data.union_membership,
                clan_membership: Default::default(),
            },
        ))
        .id();

    log::info!(
        "Created LLM buddy bot '{}' ({}), level {}, entity {:?}",
        config.name,
        bot_id,
        config.level,
        entity
    );

    entity
}

/// Create the character storage data for a bot
fn create_bot_character_data(
    game_data: &GameData,
    config: &LlmBuddyBotConfig,
    bot_build: &BotBuild,
) -> CharacterStorage {
    use rand::seq::SliceRandom;

    const BOT_GENDERS: &[rose_game_common::components::CharacterGender] = &[
        rose_game_common::components::CharacterGender::Male,
        rose_game_common::components::CharacterGender::Female,
    ];
    const BOT_FACES: &[u8] = &[1, 8, 15, 22, 29, 36, 43];
    const BOT_HAIRS: &[u8] = &[0, 5, 10, 15, 20];

    let mut rng = rand::thread_rng();

    // Use specified gender or random
    let gender = config.gender.unwrap_or_else(|| *BOT_GENDERS.choose(&mut rng).unwrap());

    // Create base character
    let mut bot_data = game_data
        .character_creator
        .create(
            config.name.clone(),
            gender,
            1,
            *BOT_FACES.choose(&mut rng).unwrap(),
            *BOT_HAIRS.choose(&mut rng).unwrap(),
        )
        .expect("Failed to create bot character data");

    // Level up to target level
    level_up_bot(game_data, config.level as u32, &mut bot_data);

    // Set job based on level
    if config.level >= 70 {
        bot_data.info.job = bot_build.job_id.get();
    } else if config.level >= 10 {
        bot_data.info.job = (bot_build.job_id.get() / 100) * 100 + 11;
    }

    // Spend stat points
    crate::game::bots::spend_stat_points(
        game_data,
        bot_build,
        &mut bot_data.stat_points,
        &mut bot_data.basic_stats,
    );

    // Calculate initial ability values for skill spending
    let mut ability_values = game_data.ability_value_calculator.calculate(
        &bot_data.info,
        &bot_data.level,
        &bot_data.equipment,
        &bot_data.basic_stats,
        &bot_data.skill_list,
        &StatusEffects::new(),
    );

    // Spend skill points
    crate::game::bots::spend_skill_points(
        game_data,
        bot_build,
        &mut bot_data,
        &mut ability_values,
    );

    // Choose equipment
    choose_equipment_items(game_data, bot_build, &mut bot_data, config.level as u32);

    // Set initial HP/MP/Stamina to max
    let final_ability_values = game_data.ability_value_calculator.calculate(
        &bot_data.info,
        &bot_data.level,
        &bot_data.equipment,
        &bot_data.basic_stats,
        &bot_data.skill_list,
        &StatusEffects::new(),
    );

    bot_data.health_points.hp = final_ability_values.max_health;
    bot_data.mana_points.mp = final_ability_values.max_mana;
    bot_data.stamina.stamina = rose_game_common::components::MAX_STAMINA;

    bot_data
}

/// Level up a bot to the target level
fn level_up_bot(game_data: &GameData, level: u32, bot_data: &mut CharacterStorage) {
    while bot_data.level.level < level {
        bot_data.level.level += 1;

        bot_data.skill_points.points += game_data
            .ability_value_calculator
            .calculate_levelup_reward_skill_points(bot_data.level.level);

        bot_data.stat_points.points += game_data
            .ability_value_calculator
            .calculate_levelup_reward_stat_points(bot_data.level.level);
    }
}

/// Choose equipment items for the bot
pub fn choose_equipment_items(
    game_data: &GameData,
    bot_build: &BotBuild,
    bot_data: &mut CharacterStorage,
    level: u32,
) {
    use rose_data::{
        AbilityType, AmmoIndex, EquipmentIndex, EquipmentItem, ItemReference,
        ItemType, StackableItem,
    };

    let equipment = &mut bot_data.equipment;

    // Create a list of JobClassId which applies to selected job
    let mut valid_job_classes = Vec::new();
    for job_class in game_data.job_class.iter() {
        if job_class.jobs.contains(&bot_build.job_id) {
            valid_job_classes.push(job_class.id);
        }
    }

    // Choose armour items
    for equipment_index in [
        EquipmentIndex::Head,
        EquipmentIndex::Body,
        EquipmentIndex::Hands,
        EquipmentIndex::Feet,
    ] {
        let mut best_item = None;
        let mut best_item_level = 0;

        for item_reference in game_data.items.iter_items(equipment_index.into()) {
            let Some(item) = game_data.items.get_base_item(item_reference) else {
                continue;
            };

            // Find item which requires our job
            if !item
                .equip_job_class_requirement
                .map_or(false, |job_class| valid_job_classes.contains(&job_class))
            {
                continue;
            }

            // Choose item with highest level which we can equip
            if let Some((_, item_level)) = item
                .equip_ability_requirement
                .iter()
                .find(|(ability_type, _)| *ability_type == AbilityType::Level)
            {
                if best_item_level < *item_level && *item_level < level {
                    best_item = Some(item);
                    best_item_level = *item_level;
                }
            }
        }

        if let Some(item_data) = best_item {
            equipment.equipped_items[equipment_index] =
                EquipmentItem::new(item_data.id, item_data.durability);
        }
    }

    // Choose weapon item
    equipment.equipped_items[EquipmentIndex::Weapon] =
        choose_highest_level_item_by_class(
            game_data,
            ItemType::Weapon,
            bot_build.weapon_type,
            level,
        )
        .or_else(|| {
            // Fallback to Wooden Sword if not appropriate weapon was found
            game_data
                .items
                .get_base_item(ItemReference::weapon(1))
                .and_then(EquipmentItem::from_item_data)
        });

    if let Some(subweapon_type) = bot_build.subweapon_type {
        equipment.equipped_items[EquipmentIndex::SubWeapon] =
            choose_highest_level_item_by_class(
                game_data,
                ItemType::SubWeapon,
                subweapon_type,
                level,
            );
    }

    // Add ammo
    equipment.equipped_ammo[AmmoIndex::Arrow] = StackableItem::new(ItemReference::material(304), 999);
    equipment.equipped_ammo[AmmoIndex::Bullet] = StackableItem::new(ItemReference::material(323), 999);
    equipment.equipped_ammo[AmmoIndex::Throw] = StackableItem::new(ItemReference::material(342), 999);
}

/// Choose the highest level item of a given class
fn choose_highest_level_item_by_class(
    game_data: &GameData,
    item_type: rose_data::ItemType,
    item_class: rose_data::ItemClass,
    level: u32,
) -> Option<rose_data::EquipmentItem> {
    use rose_data::{AbilityType, EquipmentItem};

    let mut best_item = None;
    let mut best_item_level = 0;

    for item_reference in game_data.items.iter_items(item_type) {
        let Some(item) = game_data.items.get_base_item(item_reference) else {
            continue;
        };

        if item.class != item_class {
            continue;
        }

        if let Some((_, item_level)) = item
            .equip_ability_requirement
            .iter()
            .find(|(ability_type, _)| *ability_type == AbilityType::Level)
        {
            if best_item_level < *item_level && *item_level < level {
                best_item = Some(item);
                best_item_level = *item_level;
            }
        }
    }

    best_item.and_then(|item| EquipmentItem::new(item.id, item.durability))
}

/// Process pending bot creation commands
///
/// This system processes CreateBot commands that were queued by the command processing system.
/// When a bot is created, it automatically sends a party invite from the player to the bot.
pub fn process_llm_bot_creations_system(
    mut commands: Commands,
    game_data: Res<GameData>,
    mut bot_manager: ResMut<LlmBotManagerResource>,
    mut client_entity_list: ResMut<ClientEntityList>,
    player_query: Query<(Entity, &Position, &CharacterInfo, &ClientEntity), Without<LlmBuddyBot>>,
    mut party_events: EventWriter<PartyEvent>,
) {
    // Note: Command receiving has been moved to process_llm_bot_commands_system
    // to avoid race conditions where this system consumes and drops non-CreateBot commands.
    // This system now only processes pending_creates that were queued by process_llm_bot_commands_system.

    // Process pending creates
    let pending: std::collections::HashMap<Uuid, LlmBotCommand> =
        bot_manager.pending_creates.drain().collect();

    for (_, create_cmd) in pending {
        if let LlmBotCommand::CreateBot {
            bot_id,
            name,
            level,
            class,
            gender,
            assigned_player,
        } = create_cmd
        {
            // Parse build type
            let build_type = class.parse().unwrap_or_default();

            // Parse gender
            let parsed_gender = gender.and_then(|g| match g.to_lowercase().as_str() {
                "male" => Some(rose_game_common::components::CharacterGender::Male),
                "female" => Some(rose_game_common::components::CharacterGender::Female),
                _ => None,
            });

            // Look up the player's position by name
            let mut player_info: Option<(Entity, bevy::math::Vec3, rose_data::ZoneId, u32)> = None;
            for (player_entity, pos, character_info, client_entity) in player_query.iter() {
                if character_info.name == assigned_player {
                    player_info = Some((
                        player_entity,
                        pos.position,
                        pos.zone_id,
                        client_entity.id.0 as u32,
                    ));
                    log::info!(
                        "Found player '{}' at position {:?} in zone {:?}, entity_id {}",
                        assigned_player,
                        pos.position,
                        pos.zone_id,
                        client_entity.id.0
                    );
                    break;
                }
            }

            // Use player position if found, otherwise use default spawn
            let (player_entity, spawn_position, zone_id, assigned_player_id) = player_info.unwrap_or_else(|| {
                log::warn!(
                    "Player '{}' not found, using default spawn position",
                    assigned_player
                );
                (
                    Entity::PLACEHOLDER,
                    bevy::math::Vec3::new(520000.0, 520000.0, 0.0),
                    rose_data::ZoneId::new(1).unwrap_or_else(|| rose_data::ZoneId::new(2).unwrap()),
                    0u32,
                )
            });

            // Clone values before moving into config (needed for storage later)
            let name_for_storage = name.clone();
            let assigned_player_for_storage = assigned_player.clone();

            // Create config
            let config = LlmBuddyBotConfig {
                bot_id: Some(bot_id),
                name,
                level,
                build_type,
                gender: parsed_gender,
                assigned_player_name: assigned_player,
                assigned_player_id,
                zone_id,
                position: spawn_position,
                follow_distance: 300.0,
            };

            // Create the bot entity
            let entity = create_llm_buddy_bot(&mut commands, &game_data, config);

            // Register the bot with ClientEntityList so it's visible to other clients
            let position = Position::new(spawn_position, zone_id);
            match client_entity_join_zone(
                &mut commands,
                &mut client_entity_list,
                entity,
                ClientEntityType::Character,
                &position,
            ) {
                Ok(_) => {
                    // Add visibility component for broadcasting
                    commands.entity(entity).insert(ClientEntityVisibility::new());
                    log::info!(
                        "Successfully registered LLM buddy bot '{}' with ClientEntityList in zone {:?}",
                        bot_id,
                        zone_id
                    );
                }
                Err(e) => {
                    log::error!(
                        "Failed to register LLM buddy bot '{}' with ClientEntityList: {:?}",
                        bot_id,
                        e
                    );
                }
            }

            // Register with manager - update the existing placeholder entry with the real entity
            {
                let mut bots_map = bot_manager.bots_map.write();
                if let Some(bot_info) = bots_map.get_mut(&bot_id) {
                    bot_info.update_entity(entity);
                    log::info!("Updated LLM buddy bot {} with entity {:?}", bot_id, entity);
                } else {
                    // Bot wasn't pre-registered by API, create a new entry
                    let info = crate::game::api::BotInfo::new(
                        entity,
                        name_for_storage.clone(),
                        Some(assigned_player_for_storage.clone()),
                        level,
                        build_type.to_string(),
                    );
                    bots_map.insert(bot_id, info);
                    log::info!("Registered LLM buddy bot {} -> entity {:?}", bot_id, entity);
                }
            }

            // Save bot to persistent storage
            let bot_storage = LlmBuddyBotStorage::new(
                bot_id,
                name_for_storage.clone(),
                level,
                build_type.to_string(),
                assigned_player_for_storage.clone(),
                zone_id.0.get(),
                PositionData {
                    x: spawn_position.x,
                    y: spawn_position.y,
                    z: spawn_position.z,
                },
            );
            
            if let Err(e) = bot_storage.save() {
                log::error!("Failed to save LLM buddy bot '{}' to storage: {:?}", name_for_storage, e);
            } else {
                log::info!("Saved LLM buddy bot '{}' to persistent storage", name_for_storage);
            }

            // Auto-party: Send a party invite from the player to the bot
            // The bot_accept_party_invite system will automatically accept this invite
            if player_entity != Entity::PLACEHOLDER {
                log::info!(
                    "Sending party invite from player '{}' (entity {:?}) to bot '{}' (entity {:?})",
                    assigned_player_for_storage,
                    player_entity,
                    name_for_storage,
                    entity
                );
                party_events.send(PartyEvent::Invite {
                    owner_entity: player_entity,
                    invited_entity: entity,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_build_type_from_str() {
        assert_eq!(
            LlmBuddyBotBuildType::from_str("knight").unwrap(),
            LlmBuddyBotBuildType::Knight
        );
        assert_eq!(
            LlmBuddyBotBuildType::from_str("MAGE").unwrap(),
            LlmBuddyBotBuildType::Mage
        );
        assert!(LlmBuddyBotBuildType::from_str("invalid").is_err());
    }

    #[test]
    fn test_build_type_display() {
        assert_eq!(format!("{}", LlmBuddyBotBuildType::Knight), "knight");
        assert_eq!(format!("{}", LlmBuddyBotBuildType::Mage), "mage");
    }
}
