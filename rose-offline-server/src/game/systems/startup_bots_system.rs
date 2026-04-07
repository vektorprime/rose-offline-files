use bevy::prelude::*;
use rand::Rng;

use crate::game::{
    bots::{bot_create_random_build, bot_thinker, BotBuild},
    bundles::{CharacterBundle, client_entity_join_zone},
    components::{Position, ClientEntityType},
    resources::{GameConfig, GameData, ClientEntityList},
};

pub fn startup_bots_system(
    mut commands: Commands,
    game_config: Res<GameConfig>,
    game_data: Res<GameData>,
    mut client_entity_list: ResMut<ClientEntityList>,
) {
    if !game_config.spawn_bots_on_startup {
        return;
    }

    let mut rng = rand::thread_rng();
    let zone_id = game_config.startup_bot_zone;

    for i in 0..game_config.startup_bot_count {
        let name = format!("Bot_{}_{}", zone_id, i);
        let level = rng.gen_range(game_config.startup_bot_level_min..=game_config.startup_bot_level_max);
        
        let (bot_build, bot_data) = bot_create_random_build(&game_data, name.clone(), level);
        
        // Random position in the zone (simplified)
        // In a real scenario, we might want to pick a random spawn point
        let position = Position::new(
            bevy::math::Vec3::new(
                rng.gen_range(-1000.0..1000.0),
                rng.gen_range(-1000.0..1000.0),
                0.0,
            ),
            rose_data::ZoneId::new(zone_id as u16).expect("Invalid startup bot zone ID"),
        );

        let entity = commands.spawn((
            bot_thinker(),
            CharacterBundle {
            ability_values: game_data.ability_value_calculator.calculate(
                &bot_data.info,
                &bot_data.level,
                &bot_data.equipment,
                &bot_data.basic_stats,
                &bot_data.skill_list,
                &crate::game::components::StatusEffects::new(),
            ),
            basic_stats: bot_data.basic_stats,
            bank: crate::game::components::Bank::default(),
            cooldowns: crate::game::components::Cooldowns::default(),
            command: crate::game::components::Command::default(),
            damage_sources: crate::game::components::DamageSources::default_character(),
            equipment: bot_data.equipment,
            experience_points: bot_data.experience_points,
            health_points: bot_data.health_points,
            hotbar: crate::game::components::Hotbar::default(),
            inventory: bot_data.inventory,
            level: bot_data.level,
            mana_points: bot_data.mana_points,
            motion_data: crate::game::components::MotionData::from_character(
                &game_data.motions,
                0,
                bot_data.info.gender,
            ),
            info: bot_data.info,
            move_mode: crate::game::components::MoveMode::Walk,
            move_speed: crate::game::components::MoveSpeed::new(100.0),
            next_command: crate::game::components::NextCommand::default(),
            party_membership: crate::game::components::PartyMembership::default(),
            passive_recovery_time: crate::game::components::PassiveRecoveryTime::default(),
            position: position.clone(),
            quest_state: crate::game::components::QuestState::default(),
            skill_list: bot_data.skill_list,
            skill_points: bot_data.skill_points,
            stamina: bot_data.stamina,
            stat_points: bot_data.stat_points,
            status_effects: crate::game::components::StatusEffects::new(),
            status_effects_regen: crate::game::components::StatusEffectsRegen::new(),
            team: crate::game::components::Team::default_character(),
            union_membership: bot_data.union_membership,
            clan_membership: crate::game::components::ClanMembership::default(),
        },
        bot_build,
    ))
    .id();


        client_entity_join_zone(
            &mut commands,
            &mut client_entity_list,
            entity,
            ClientEntityType::Character,
            &position,
        )
        .expect("Failed to join startup bot into zone");
    }
}
