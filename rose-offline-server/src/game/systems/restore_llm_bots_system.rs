//! System to restore LLM Buddy Bots from persistent storage on startup
//!
//! This system runs at startup and recreates any previously saved LLM buddy bots.

use bevy::ecs::prelude::*;
use uuid::Uuid;

use crate::game::{
    api::LlmBotCommand,
    bundles::{client_entity_join_zone, CharacterBundle},
    components::{
        CharacterInfo, ClientEntity, ClientEntityType, ClientEntityVisibility, Command,
        DamageSources, EquipmentItemDatabase, HealthPoints, LlmBuddyBot, ManaPoints, MotionData,
        NextCommand, Position, Stamina, Team,
    },
    resources::{ClientEntityList, GameData},
    storage::llm_buddy_bot::LlmBuddyBotStorage,
    systems::llm_buddy_bot_system::LlmBotManagerResource,
};

use rose_game_common::components::{MoveMode, MoveSpeed, StatusEffects};

/// Startup system to restore LLM buddy bots from persistent storage
///
/// This system loads all saved bots and recreates them in the game world.
/// It should run after the game data is loaded but before the main game loop starts.
pub fn restore_llm_buddy_bots_system(
    mut commands: Commands,
    game_data: bevy::ecs::system::Res<GameData>,
    mut bot_manager: bevy::ecs::system::ResMut<LlmBotManagerResource>,
    mut client_entity_list: bevy::ecs::system::ResMut<ClientEntityList>,
    player_query: Query<(&Position, &CharacterInfo, &ClientEntity), Without<LlmBuddyBot>>,
) {
    log::info!("Checking for saved LLM buddy bots to restore...");

    // Load all saved bots
    let saved_bots = match LlmBuddyBotStorage::load_all() {
        Ok(bots) => bots,
        Err(e) => {
            log::error!("Failed to load saved LLM buddy bots: {:?}", e);
            return;
        }
    };

    if saved_bots.is_empty() {
        log::info!("No saved LLM buddy bots found");
        return;
    }

    log::info!("Found {} saved LLM buddy bot(s) to restore", saved_bots.len());

    for bot_storage in saved_bots {
        log::info!(
            "Restoring LLM buddy bot '{}' ({}) for player '{}'",
            bot_storage.name,
            bot_storage.bot_id,
            bot_storage.assigned_player
        );

        // Try to find the assigned player's current position
        let mut player_position: Option<(bevy::math::Vec3, rose_data::ZoneId, u32)> = None;
        for (pos, character_info, client_entity) in player_query.iter() {
            if character_info.name == bot_storage.assigned_player {
                player_position = Some((
                    pos.position,
                    pos.zone_id,
                    client_entity.id.0 as u32,
                ));
                log::info!(
                    "Found player '{}' at position {:?} in zone {:?}",
                    bot_storage.assigned_player,
                    pos.position,
                    pos.zone_id
                );
                break;
            }
        }

        // Use saved position if player not found, otherwise spawn near player
        let (spawn_position, zone_id, assigned_player_id) = player_position.unwrap_or_else(|| {
            log::warn!(
                "Player '{}' not found for bot '{}', using saved position",
                bot_storage.assigned_player,
                bot_storage.name
            );
            (
                bevy::math::Vec3::new(
                    bot_storage.position.x,
                    bot_storage.position.y,
                    bot_storage.position.z,
                ),
                rose_data::ZoneId::new(bot_storage.zone_id).unwrap_or_else(|| {
                    rose_data::ZoneId::new(1).unwrap_or_else(|| rose_data::ZoneId::new(2).unwrap())
                }),
                bot_storage.assigned_player_entity_id,
            )
        });

        // Parse build type
        let build_type = bot_storage.build_type.parse().unwrap_or_default();
        let bot_build = super::super::bots::get_bot_build(build_type);

        // Create bot character data
        let bot_data = create_bot_character_data_from_storage(&game_data, &bot_storage, &bot_build);

        // Create the LlmBuddyBot component
        let mut llm_buddy_bot = LlmBuddyBot::new(
            bot_storage.bot_id,
            assigned_player_id,
            bot_storage.assigned_player.clone(),
        );
        llm_buddy_bot.set_follow_distance(bot_storage.follow_distance);
        llm_buddy_bot.is_following = bot_storage.is_following;

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

        // Spawn the bot entity
        let entity = commands
            .spawn((
                llm_buddy_bot,
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
                        hp: bot_storage.health.current,
                    },
                    hotbar: bot_data.hotbar,
                    info: bot_data.info,
                    inventory: bot_data.inventory,
                    level: bot_data.level,
                    mana_points: ManaPoints {
                        mp: bot_storage.mana.current,
                    },
                    motion_data,
                    move_mode: MoveMode::Run,
                    move_speed,
                    next_command: NextCommand::default(),
                    party_membership: Default::default(),
                    passive_recovery_time: Default::default(),
                    position: Position::new(spawn_position, zone_id),
                    quest_state: bot_data.quest_state,
                    skill_list: bot_data.skill_list,
                    skill_points: bot_data.skill_points,
                    stamina: Stamina {
                        stamina: bot_storage.stamina,
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

        // Register with ClientEntityList
        let position = Position::new(spawn_position, zone_id);
        match client_entity_join_zone(
            &mut commands,
            &mut client_entity_list,
            entity,
            ClientEntityType::Character,
            &position,
        ) {
            Ok(_) => {
                commands.entity(entity).insert(ClientEntityVisibility::new());
                log::info!(
                    "Successfully restored LLM buddy bot '{}' ({}) in zone {:?}",
                    bot_storage.name,
                    bot_storage.bot_id,
                    zone_id
                );
            }
            Err(e) => {
                log::error!(
                    "Failed to register restored bot '{}' with ClientEntityList: {:?}",
                    bot_storage.name,
                    e
                );
            }
        }

        // Register with manager - create BotInfo with full data
        let bot_info = crate::game::api::BotInfo::new(
            entity,
            bot_storage.name.clone(),
            Some(bot_storage.assigned_player.clone()),
            bot_storage.level,
            bot_storage.build_type.clone(),
        );
        bot_manager.bots_map.write().insert(bot_storage.bot_id, bot_info);
        log::info!(
            "Registered restored LLM buddy bot {} -> entity {:?}",
            bot_storage.bot_id,
            entity
        );
    }
}

/// Create character data for a restored bot from storage
fn create_bot_character_data_from_storage(
    game_data: &GameData,
    bot_storage: &LlmBuddyBotStorage,
    bot_build: &super::super::bots::BotBuild,
) -> crate::game::storage::character::CharacterStorage {
    use rand::seq::SliceRandom;

    const BOT_GENDERS: &[rose_game_common::components::CharacterGender] = &[
        rose_game_common::components::CharacterGender::Male,
        rose_game_common::components::CharacterGender::Female,
    ];
    const BOT_FACES: &[u8] = &[1, 8, 15, 22, 29, 36, 43];
    const BOT_HAIRS: &[u8] = &[0, 5, 10, 15, 20];

    let mut rng = rand::thread_rng();

    // Create base character
    let mut bot_data = game_data
        .character_creator
        .create(
            bot_storage.name.clone(),
            *BOT_GENDERS.choose(&mut rng).unwrap(),
            1,
            *BOT_FACES.choose(&mut rng).unwrap(),
            *BOT_HAIRS.choose(&mut rng).unwrap(),
        )
        .expect("Failed to create bot character data");

    // Level up to target level
    let target_level = bot_storage.level as u32;
    while bot_data.level.level < target_level {
        bot_data.level.level += 1;

        bot_data.skill_points.points += game_data
            .ability_value_calculator
            .calculate_levelup_reward_skill_points(bot_data.level.level);

        bot_data.stat_points.points += game_data
            .ability_value_calculator
            .calculate_levelup_reward_stat_points(bot_data.level.level);
    }

    // Set job based on level
    if target_level >= 70 {
        bot_data.info.job = bot_build.job_id.get();
    } else if target_level >= 10 {
        bot_data.info.job = (bot_build.job_id.get() / 100) * 100 + 11;
    }

    // Spend stat points
    super::super::bots::spend_stat_points(
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
    super::super::bots::spend_skill_points(
        game_data,
        bot_build,
        &mut bot_data,
        &mut ability_values,
    );

    // Choose equipment - use the function from bots module
    crate::game::bots::choose_equipment_items(game_data, bot_build, &mut bot_data, target_level);

    // Set HP/MP/Stamina from storage (use max if current is 0)
    let final_ability_values = game_data.ability_value_calculator.calculate(
        &bot_data.info,
        &bot_data.level,
        &bot_data.equipment,
        &bot_data.basic_stats,
        &bot_data.skill_list,
        &StatusEffects::new(),
    );

    bot_data.health_points.hp = if bot_storage.health.current > 0 {
        bot_storage.health.current
    } else {
        final_ability_values.max_health
    };
    bot_data.mana_points.mp = if bot_storage.mana.current > 0 {
        bot_storage.mana.current
    } else {
        final_ability_values.max_mana
    };
    bot_data.stamina.stamina = if bot_storage.stamina > 0 {
        bot_storage.stamina
    } else {
        rose_game_common::components::MAX_STAMINA
    };

    bot_data
}
