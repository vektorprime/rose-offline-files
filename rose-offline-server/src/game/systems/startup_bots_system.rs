use bevy::{math::Vec3Swizzles, prelude::*};
use rand::{seq::SliceRandom, Rng};

use crate::game::{
    bots::{
        bot_create_random_build, bot_thinker_with_role, is_town_zone, names::BotNames,
        spawn_vendor_bot, BotChatMemory, BotRole, VENDOR_BOTS_PER_TOWN,
    },
    bundles::{CharacterBundle, client_entity_join_zone},
    components::{ClientEntityType, Position},
    resources::{GameConfig, GameData, ClientEntityList},
};

/// The main towns: Zant (zone 1) and Junon (zone 2). Town patrol bots and
/// vendor shops are concentrated here; other zones only get hunters.
const MAIN_TOWN_ZONES: [u16; 2] = [1, 2];

fn is_main_town(zone_id: rose_data::ZoneId) -> bool {
    MAIN_TOWN_ZONES.contains(&zone_id.get())
}

/// Average level of the monsters which spawn in a zone, used to level bots
/// appropriately for the area.
fn zone_monster_level_range(game_data: &GameData, zone_data: &rose_data::ZoneData) -> (u32, u32) {
    let mut total_level = 0i32;
    let mut num_monsters = 0i32;

    for spawn in zone_data.monster_spawns.iter() {
        for npc_id in spawn.basic_spawns.iter().map(|(npc_id, _)| *npc_id) {
            if let Some(npc_data) = game_data.npcs.get_npc(npc_id) {
                total_level += npc_data.level;
                num_monsters += 1;
            }
        }

        for npc_id in spawn.tactic_spawns.iter().map(|(npc_id, _)| *npc_id) {
            if let Some(npc_data) = game_data.npcs.get_npc(npc_id) {
                total_level += npc_data.level;
                num_monsters += 1;
            }
        }
    }

    if num_monsters == 0 {
        // No monsters in this zone, keep bots at low levels.
        return (1, 30);
    }

    let average_level = total_level / num_monsters;
    let min_level = (average_level - 10).clamp(1, 170) as u32;
    let max_level = (average_level + 10).clamp(1, 170) as u32;

    (min_level, max_level)
}

/// Role mix for the main towns (Zant, Junon): mostly social patrol bots with
/// a few defenders and healers.
fn pick_main_town_role(rng: &mut impl Rng) -> BotRole {
    let roll = rng.gen_range(0..100);
    if roll < 65 {
        BotRole::Town
    } else if roll < 90 {
        BotRole::Fighter
    } else {
        BotRole::Healer
    }
}

/// Role mix for smaller towns: half social walkers, half hunters.
fn pick_small_town_role(rng: &mut impl Rng) -> BotRole {
    let roll = rng.gen_range(0..100);
    if roll < 45 {
        BotRole::Town
    } else if roll < 80 {
        BotRole::Fighter
    } else {
        BotRole::Healer
    }
}

/// Role mix for fields: mostly hunters with a support presence.
fn pick_field_role(rng: &mut impl Rng) -> BotRole {
    if rng.gen_range(0..100) < 55 {
        BotRole::Fighter
    } else {
        BotRole::Healer
    }
}

/// Roles for a field party group: fighters with at least one healer, plus a
/// chance for a second healer in larger groups.
fn field_group_roles(rng: &mut impl Rng, group_size: usize) -> Vec<BotRole> {
    match group_size {
        1 => vec![pick_field_role(rng)],
        2 => vec![BotRole::Fighter, BotRole::Healer],
        3 => vec![BotRole::Fighter, BotRole::Fighter, BotRole::Healer],
        _ => {
            let mut roles = vec![BotRole::Fighter, BotRole::Fighter, BotRole::Fighter, BotRole::Healer];
            if rng.gen_bool(0.25) {
                roles[1] = BotRole::Healer;
            }
            roles
        }
    }
}

/// Splits a field zone's bot budget into group sizes: mostly hunting parties
/// of 3-4 with some solos.
fn field_group_sizes(rng: &mut impl Rng, total: usize) -> Vec<usize> {
    let mut groups = Vec::new();
    let mut remaining = total;

    while remaining > 0 {
        let size = if remaining >= 4 && rng.gen_bool(0.6) {
            4
        } else if remaining >= 3 && rng.gen_bool(0.6) {
            3
        } else if remaining >= 2 && rng.gen_bool(0.3) {
            2
        } else {
            1
        };
        let size = size.min(remaining);
        groups.push(size);
        remaining -= size;
    }

    groups
}

/// Bots per zone: towns get more than fields so streets look busy.
fn bots_per_zone_count(game_config: &GameConfig, is_town: bool) -> u32 {
    if is_town {
        game_config.startup_bots_per_zone + 10
    } else {
        game_config.startup_bots_per_zone
    }
}

fn spawn_bot(
    commands: &mut Commands,
    game_data: &GameData,
    client_entity_list: &mut ClientEntityList,
    bot_names: &mut BotNames,
    role: BotRole,
    name: String,
    level: u32,
    position: Position,
) {
    let (bot_build, bot_data) = bot_create_random_build(game_data, name, level);

    let entity = commands
        .spawn((
            bot_thinker_with_role(&crate::game::bots::BotBehaviorConfig::default(), role),
            bot_build,
            role,
            BotChatMemory::default(),
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
        ))
        .id();

    client_entity_join_zone(
        commands,
        client_entity_list,
        entity,
        ClientEntityType::Character,
        &position,
    )
    .expect("Failed to join startup bot into zone");
}

/// How far a spawn spot may move to stand on walkable ground.
const SPAWN_GROUND_SNAP_CM: f32 = 3000.0;

/// `point` moved onto the nearest walkable ground of the zone's anchored
/// areas, standing on the floor (spawn anchors are XY placements: random
/// offsets land on cliffs, in moats or inside buildings).
fn on_walkable_ground(zone_data: &rose_data::ZoneData, point: bevy::math::Vec3) -> bevy::math::Vec3 {
    zone_data
        .nav_grid
        .as_ref()
        .and_then(|nav_grid| nav_grid.rescue_point(point, SPAWN_GROUND_SNAP_CM))
        .unwrap_or(point)
}

/// Spawns a bot at a random point within `spread` cm of the given anchor.
fn spawn_at_anchor(
    rng: &mut impl Rng,
    anchor: bevy::math::Vec3,
    spread: f32,
    zone_data: &rose_data::ZoneData,
) -> Position {
    let position = anchor
        + Vec3::new(
            rng.gen_range(-spread..spread),
            rng.gen_range(-spread..spread),
            0.0,
        );
    Position::new(on_walkable_ground(zone_data, position), zone_data.id)
}

/// Town bot spawn: a random spot spread across the town, anchored on a random
/// NPC or the town centre.
fn town_spawn_position(
    rng: &mut impl Rng,
    zone_data: &rose_data::ZoneData,
) -> Position {
    const TOWN_SPREAD: f32 = 450.0;

    let base = if rng.gen_bool(0.5) {
        zone_data
            .npcs
            .choose(rng)
            .map(|npc_spawn| npc_spawn.position)
            .unwrap_or(zone_data.start_position)
    } else {
        zone_data.start_position
    };

    spawn_at_anchor(rng, base, TOWN_SPREAD, zone_data)
}

/// Spawns a vendor bot at a random spot away from NPCs and other vendors, so
/// the market stalls are spread out across the town.
fn spawn_vendor_position(
    rng: &mut impl Rng,
    zone_data: &rose_data::ZoneData,
    used_positions: &mut Vec<bevy::math::Vec2>,
) -> Position {
    const MIN_VENDOR_SEPARATION: f32 = 300.0;
    const VENDOR_OFFSET_MIN: f32 = 350.0;
    const VENDOR_OFFSET_MAX: f32 = 550.0;

    // Try a few random spots around random NPC anchors; keep vendors apart
    // from each other and away from the NPCs themselves.
    for _ in 0..16 {
        let base = if let Some(npc_spawn) = zone_data.npcs.choose(rng) {
            npc_spawn.position
        } else {
            zone_data.start_position
        };

        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        let distance = rng.gen_range(VENDOR_OFFSET_MIN..VENDOR_OFFSET_MAX);
        let candidate = base
            + bevy::math::Vec3::new(
                angle.cos() * distance,
                angle.sin() * distance,
                0.0,
            );
        let candidate = on_walkable_ground(zone_data, candidate);
        let candidate_xy = candidate.xy();

        if used_positions
            .iter()
            .all(|position| position.distance(candidate_xy) > MIN_VENDOR_SEPARATION)
        {
            used_positions.push(candidate_xy);
            return Position::new(candidate, zone_data.id);
        }
    }

    // Fallback: a random open spot anywhere in the town.
    let position = on_walkable_ground(
        zone_data,
        zone_data.start_position
            + Vec3::new(
                rng.gen_range(-800.0..800.0),
                rng.gen_range(-800.0..800.0),
                0.0,
            ),
    );
    used_positions.push(position.xy());
    Position::new(position, zone_data.id)
}

pub fn startup_bots_system(
    mut commands: Commands,
    game_config: Res<GameConfig>,
    game_data: Res<GameData>,
    mut client_entity_list: ResMut<ClientEntityList>,
    mut bot_names: ResMut<BotNames>,
) {
    if !game_config.spawn_bots_on_startup {
        return;
    }

    let mut rng = rand::thread_rng();

    if !game_config.bot_spawn_all_zones {
        // Legacy single-zone spawn.
        let zone_id = game_config.startup_bot_zone;
        let position = Position::new(
            bevy::math::Vec3::new(
                rng.gen_range(-1000.0..1000.0),
                rng.gen_range(-1000.0..1000.0),
                0.0,
            ),
            rose_data::ZoneId::new(zone_id as u16).expect("Invalid startup bot zone ID"),
        );

        for i in 0..game_config.startup_bot_count {
            let level = rng
                .gen_range(game_config.startup_bot_level_min..=game_config.startup_bot_level_max);
            let role = pick_field_role(&mut rng);
            let bot_name = bot_names.next_unique_name(&mut rng);
            spawn_bot(
                &mut commands,
                &game_data,
                &mut client_entity_list,
                &mut bot_names,
                role,
                bot_name,
                level,
                position.clone(),
            );
        }
        return;
    }

    // Spawn bots across every zone, scaled to the local monster levels.
    let mut spawned = 0u32;
    let zone_ids: Vec<rose_data::ZoneId> = game_data.zones.iter_zone_ids().collect();
    let zone_count = zone_ids.len();

    'zones: for zone_id in zone_ids {
        if spawned >= game_config.bot_total_cap {
            break;
        }

        let Some(zone_data) = game_data.zones.get_zone(zone_id) else {
            continue;
        };

        let is_town = is_town_zone(zone_data);
        let main_town = is_main_town(zone_id);
        let (level_min, level_max) = zone_monster_level_range(&game_data, zone_data);
        let bots_this_zone = bots_per_zone_count(&game_config, is_town)
            .min(game_config.bot_total_cap - spawned);

        if !is_town {
            // Fields: spawn hunting groups directly on monster spawn points.
            // Group members spawn close together so they can party up, while
            // solo bots roam alone.
            let fallback_anchor = zone_data.start_position;
            for group_size in field_group_sizes(&mut rng, bots_this_zone as usize) {
                if spawned >= game_config.bot_total_cap {
                    break 'zones;
                }

                let anchor = zone_data
                    .monster_spawns
                    .choose(&mut rng)
                    .map(|spawn_point| spawn_point.position)
                    .unwrap_or(fallback_anchor);

                for role in field_group_roles(&mut rng, group_size) {
                    if spawned >= game_config.bot_total_cap {
                        break 'zones;
                    }

                    let level = rng.gen_range(level_min..=level_max);
                    let position = spawn_at_anchor(&mut rng, anchor, 150.0, zone_data);
                    let bot_name = bot_names.next_unique_name(&mut rng);

                    spawn_bot(
                        &mut commands,
                        &game_data,
                        &mut client_entity_list,
                        &mut bot_names,
                        role,
                        bot_name,
                        level,
                        position,
                    );

                    spawned += 1;
                }
            }
        } else {
            // Towns: spread walkers and defenders across the whole town.
            for _ in 0..bots_this_zone {
                if spawned >= game_config.bot_total_cap {
                    break 'zones;
                }

                let role = if main_town {
                    pick_main_town_role(&mut rng)
                } else {
                    pick_small_town_role(&mut rng)
                };
                let level = rng.gen_range(level_min..=level_max);
                let position = town_spawn_position(&mut rng, zone_data);
                let bot_name = bot_names.next_unique_name(&mut rng);

                spawn_bot(
                    &mut commands,
                    &game_data,
                    &mut client_entity_list,
                    &mut bot_names,
                    role,
                    bot_name,
                    level,
                    position,
                );

                spawned += 1;
            }
        }

        // Vendor shop bots only in the main towns (Zant, Junon).
        if main_town && spawned < game_config.bot_total_cap {
            let vendors_this_zone = VENDOR_BOTS_PER_TOWN.min(game_config.bot_total_cap - spawned);
            let mut used_vendor_positions: Vec<bevy::math::Vec2> = Vec::new();

            for _ in 0..vendors_this_zone {
                let level = rng.gen_range(level_min..=level_max);
                let position =
                    spawn_vendor_position(&mut rng, zone_data, &mut used_vendor_positions);

                spawn_vendor_bot(
                    &mut commands,
                    &game_data,
                    &mut client_entity_list,
                    &mut bot_names,
                    level,
                    position,
                );

                spawned += 1;
            }
        }
    }

    log::info!(
        "Spawned {} bots across {} zones",
        spawned,
        zone_count
    );
}
