//! System to restore and teleport LLM buddy bots when their assigned player logs in

use std::collections::HashMap;
use bevy::prelude::*;
use bevy::math::Vec3;
use uuid::Uuid;

use crate::game::{
    api::BotInfo,
    bundles::{client_entity_join_zone, client_entity_leave_zone, CharacterBundle},
    components::{
        CharacterInfo, ClientEntity, ClientEntitySector, ClientEntityType, ClientEntityVisibility,
        Command, CommandData, DamageSources, EquipmentItemDatabase, HealthPoints, LlmBuddyBot,
        ManaPoints, MotionData, NextCommand, PartyMembership, Position, Stamina, Team,
    },
    events::PartyEvent,
    resources::{ClientEntityList, GameData},
    storage::llm_buddy_bot::LlmBuddyBotStorage,
    systems::LlmBotManagerResource,
};

use rose_game_common::components::{MoveMode, MoveSpeed, StatusEffects};

/// System that restores and teleports LLM buddy bots when their assigned player logs in
pub fn llm_bot_teleport_to_player_on_login_system(
    mut commands: Commands,
    players_joining: Query<
        (&Position, &CharacterInfo, &ClientEntity, Entity),
        (Added<ClientEntityVisibility>, Without<LlmBuddyBot>),
    >,
    mut bot_query: Query<
        (
            &LlmBuddyBot,
            &mut Position,
            &mut NextCommand,
            &mut Command,
            &ClientEntity,
            &ClientEntitySector,
            Entity,
        ),
        With<LlmBuddyBot>,
    >,
    bot_manager: Res<LlmBotManagerResource>,
    mut client_entity_list: ResMut<ClientEntityList>,
    game_data: Res<GameData>,
    mut party_events: MessageWriter<PartyEvent>,
) {
    for (player_position, character_info, player_client_entity, player_entity) in players_joining.iter() {
        let player_name = character_info.name.clone();
        let player_pos = player_position.position;
        let player_zone = player_position.zone_id;
        let player_entity_id = player_client_entity.id.0 as u32;

        log::info!(
            "Player '{}' joined zone {:?}, checking for assigned bots",
            player_name,
            player_zone
        );

        let saved_bots = match LlmBuddyBotStorage::load_for_player(&player_name) {
            Ok(bots) => bots,
            Err(e) => {
                log::error!("Failed to load saved bots for player '{}': {:?}", player_name, e);
                continue;
            }
        };

        if saved_bots.is_empty() {
            log::debug!("No saved bots found for player '{}'", player_name);
            continue;
        }

        let existing_bots: HashMap<Uuid, BotInfo> = {
            let bots_map = bot_manager.bots_map.read();
            saved_bots
                .iter()
                .filter_map(|bot_storage| {
                    bots_map.get(&bot_storage.bot_id).map(|bot_info| {
                        (bot_storage.bot_id, bot_info.clone())
                    })
                })
                .collect()
        };

        for bot_storage in saved_bots {
            let bot_id = bot_storage.bot_id;

            if let Some(bot_info) = existing_bots.get(&bot_id) {
                teleport_bot_to_player(
                    &mut commands,
                    &mut client_entity_list,
                    &mut bot_query,
                    bot_info,
                    &player_name,
                    player_pos,
                    player_zone,
                );
            } else {
                spawn_bot_for_player(
                    &mut commands,
                    &mut client_entity_list,
                    &bot_manager,
                    &game_data,
                    bot_storage,
                    player_pos,
                    player_zone,
                    player_entity_id,
                    player_entity,
                    &mut party_events,
                );
            }
        }
    }
}

fn teleport_bot_to_player(
    commands: &mut Commands,
    client_entity_list: &mut ResMut<ClientEntityList>,
    bot_query: &mut Query<
        (
            &LlmBuddyBot,
            &mut Position,
            &mut NextCommand,
            &mut Command,
            &ClientEntity,
            &ClientEntitySector,
            Entity,
        ),
        With<LlmBuddyBot>,
    >,
    bot_info: &BotInfo,
    player_name: &str,
    player_pos: Vec3,
    player_zone: rose_data::ZoneId,
) {
    let entity = bot_info.entity;

    if let Ok((
        _buddy_bot,
        mut bot_position,
        mut next_command,
        mut command,
        bot_client_entity,
        bot_client_entity_sector,
        _bot_entity,
    )) = bot_query.get_mut(entity)
    {
        let previous_position = bot_position.clone();
        let new_position = Position::new(player_pos, player_zone);

        client_entity_leave_zone(
            commands,
            client_entity_list,
            entity,
            bot_client_entity,
            bot_client_entity_sector,
            &previous_position,
        );

        bot_position.position = player_pos;
        bot_position.zone_id = player_zone;

        match client_entity_join_zone(
            commands,
            client_entity_list,
            entity,
            ClientEntityType::Character,
            &new_position,
        ) {
            Ok(_) => {
                commands.entity(entity).insert(ClientEntityVisibility::new());

                let offset = Vec3::new(50.0, 0.0, 50.0);
                let teleport_dest = player_pos + offset;

                *command = Command::with_stop();
                next_command.command = Some(CommandData::Move {
                    destination: teleport_dest,
                    target: None,
                    move_mode: Some(MoveMode::Run),
                });
                next_command.has_sent_server_message = false;

                log::info!(
                    "Teleported LLM buddy bot '{}' to player '{}' at position {:?} in zone {:?}",
                    bot_info.name,
                    player_name,
                    player_pos,
                    player_zone
                );
            }
            Err(e) => {
                log::error!(
                    "Failed to join zone for bot '{}' after teleport: {:?}",
                    bot_info.name,
                    e
                );
            }
        }
    } else {
        log::warn!(
            "Could not find bot entity {:?} for bot '{}' assigned to player '{}'",
            entity,
            bot_info.name,
            player_name
        );
    }
}

fn spawn_bot_for_player(
    commands: &mut Commands,
    client_entity_list: &mut ResMut<ClientEntityList>,
    bot_manager: &Res<LlmBotManagerResource>,
    game_data: &Res<GameData>,
    bot_storage: LlmBuddyBotStorage,
    player_pos: Vec3,
    player_zone: rose_data::ZoneId,
    player_entity_id: u32,
    player_entity: Entity,
    party_events: &mut MessageWriter<PartyEvent>,
) {
    log::info!(
        "Spawning saved bot '{}' ({}) for player '{}'",
        bot_storage.name,
        bot_storage.bot_id,
        bot_storage.assigned_player
    );

    let build_type = bot_storage.build_type.parse().unwrap_or_default();
    let bot_build = super::super::bots::get_bot_build(build_type);
    let bot_data = create_bot_character_data_from_storage(game_data, &bot_storage, &bot_build);

    let mut llm_buddy_bot = LlmBuddyBot::new(
        bot_storage.bot_id,
        player_entity_id,
        bot_storage.assigned_player.clone(),
    );
    llm_buddy_bot.set_follow_distance(bot_storage.follow_distance);
    // Always set is_following to true when restoring bot on player login
    // The player expects their bot to follow them after relogin
    llm_buddy_bot.is_following = true;

    let ability_values = game_data.ability_value_calculator.calculate(
        &bot_data.info,
        &bot_data.level,
        &bot_data.equipment,
        &bot_data.basic_stats,
        &bot_data.skill_list,
        &StatusEffects::new(),
    );

    let move_speed = MoveSpeed {
        speed: ability_values.run_speed as f32,
    };

    use rose_data::EquipmentIndex;
    let weapon_motion_type = game_data
        .items
        .get_equipped_weapon_item_data(&bot_data.equipment, EquipmentIndex::Weapon)
        .map(|item_data| item_data.motion_type)
        .unwrap_or(0) as usize;

    let motion_data = MotionData::from_character(
        game_data.motions.as_ref(),
        weapon_motion_type,
        bot_data.info.gender,
    );

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
                    hp: bot_storage.health.max,
                },
                hotbar: bot_data.hotbar,
                info: bot_data.info,
                inventory: bot_data.inventory,
                level: bot_data.level,
                mana_points: ManaPoints {
                    mp: bot_storage.mana.max,
                },
                motion_data,
                move_mode: MoveMode::Run,
                move_speed,
                next_command: NextCommand::default(),
                party_membership: Default::default(),
                passive_recovery_time: Default::default(),
                position: Position::new(player_pos, player_zone),
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

    let position = Position::new(player_pos, player_zone);
    match client_entity_join_zone(
        commands,
        client_entity_list,
        entity,
        ClientEntityType::Character,
        &position,
    ) {
        Ok(_) => {
            commands.entity(entity).insert(ClientEntityVisibility::new());
            log::info!(
                "Successfully spawned LLM buddy bot '{}' ({}) in zone {:?}",
                bot_storage.name,
                bot_storage.bot_id,
                player_zone
            );
        }
        Err(e) => {
            log::error!(
                "Failed to register spawned bot '{}' with ClientEntityList: {:?}",
                bot_storage.name,
                e
            );
        }
    }

    let bot_info = BotInfo::new(
        entity,
        bot_storage.name.clone(),
        Some(bot_storage.assigned_player.clone()),
        bot_storage.level,
        bot_storage.build_type.clone(),
    );
    bot_manager.bots_map.write().insert(bot_storage.bot_id, bot_info);
    log::info!(
        "Registered spawned LLM buddy bot {} -> entity {:?}",
        bot_storage.bot_id,
        entity
    );

    // Send party invite from player to bot - the bot will auto-accept via llm_buddy_bot_auto_accept_party_system
    log::info!(
        "Sending party invite from player entity {:?} to bot '{}' (entity {:?})",
        player_entity,
        bot_storage.name,
        entity
    );
    party_events.write(PartyEvent::Invite {
        owner_entity: player_entity,
        invited_entity: entity,
    });
}

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

    if target_level >= 70 {
        bot_data.info.job = bot_build.job_id.get();
    } else if target_level >= 10 {
        bot_data.info.job = (bot_build.job_id.get() / 100) * 100 + 11;
    }

    super::super::bots::spend_stat_points(
        game_data,
        bot_build,
        &mut bot_data.stat_points,
        &mut bot_data.basic_stats,
    );

    let mut ability_values = game_data.ability_value_calculator.calculate(
        &bot_data.info,
        &bot_data.level,
        &bot_data.equipment,
        &bot_data.basic_stats,
        &bot_data.skill_list,
        &StatusEffects::new(),
    );

    super::super::bots::spend_skill_points(
        game_data,
        bot_build,
        &mut bot_data,
        &mut ability_values,
    );

    super::super::bots::choose_equipment_items(game_data, bot_build, &mut bot_data, target_level);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_exists() {}
}
