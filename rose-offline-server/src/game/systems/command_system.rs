use std::time::{Duration, Instant};

use bevy::{
    ecs::query::QueryData,
    math::{Vec3, Vec3Swizzles},
    prelude::{Commands, Entity, MessageWriter, Query, Res, ResMut},
    time::Time,
};

use rose_data::{
    AbilityType, AmmoIndex, EquipmentIndex, ItemClass, SkillActionMode, SkillCooldown, SkillId,
    SkillTargetFilter, SkillType, VehiclePartIndex,
};
use rose_game_common::components::{CharacterGender, CharacterInfo};

use crate::game::{
    bundles::GLOBAL_SKILL_COOLDOWN,
    components::{
        AbilityValues, ClanMembership, ClientEntity, ClientEntitySector, ClientEntityType, Command,
        CommandCastSkillTarget, CommandData, Cooldowns, Equipment, ExperiencePoints, GameClient,
        HealthPoints, Inventory, ItemDrop, ManaPoints, MotionData, MoveMode, MoveSpeed,
        NextCommand, Npc, Owner, PartyMembership, PartyOwner, PersonalStore, Position, Stamina,
        Team,
    },
    events::{
        DamageEvent, ItemLifeEvent, PickupItemEvent, SkillEvent, SkillEventTarget, UseAmmoEvent,
    },
    messages::server::ServerMessage,
    resources::{GameData, ServerMessages},
};

const NPC_MOVE_TO_DISTANCE: f32 = 250.0;
const CHARACTER_MOVE_TO_DISTANCE: f32 = 1000.0;
const DROPPED_ITEM_MOVE_TO_DISTANCE: f32 = 150.0;
const DROPPED_ITEM_PICKUP_DISTANCE: f32 = 200.0;

#[derive(QueryData)]
#[query_data(mutable)]
pub struct QueryCommandEntity<'w> {
    entity: Entity,

    command: &'w mut Command,
    next_command: &'w mut NextCommand,

    ability_values: &'w AbilityValues,
    client_entity: &'w ClientEntity,
    motion_data: &'w MotionData,
    move_mode: &'w MoveMode,
    position: &'w Position,
    team: &'w Team,

    character_info: Option<&'w CharacterInfo>,
    equipment: Option<&'w Equipment>,
    game_client: Option<&'w GameClient>,
    npc: Option<&'w Npc>,
    personal_store: Option<&'w PersonalStore>,
}

#[derive(QueryData)]
pub struct CommandAttackTargetQuery<'w> {
    ability_values: &'w AbilityValues,
    client_entity: &'w ClientEntity,
    health_points: &'w HealthPoints,
    position: &'w Position,
    team: &'w Team,
}

#[derive(QueryData)]
pub struct CommandMoveTargetQuery<'w> {
    client_entity: &'w ClientEntity,
    position: &'w Position,
}

#[derive(QueryData)]
#[query_data(mutable)]
pub struct CommandPickupItemTargetQuery<'w> {
    client_entity: &'w ClientEntity,
    client_entity_sector: &'w ClientEntitySector,
    item_drop: &'w mut ItemDrop,
    position: &'w Position,
    owner: Option<&'w Owner>,
    party_owner: Option<&'w PartyOwner>,
}

fn command_stop(
    command: &mut Command,
    client_entity: &ClientEntity,
    position: &Position,
    server_messages: Option<&mut ServerMessages>,
) {
    if let Some(server_messages) = server_messages {
        server_messages.send_entity_message(
            client_entity,
            ServerMessage::StopMoveEntity {
                entity_id: client_entity.id,
                x: position.position.x,
                y: position.position.y,
                z: position.position.z as u16,
            },
        );
    }

    *command = Command::with_stop();
}

fn is_valid_move_target(target: &CommandMoveTargetQueryItem, position: &Position) -> bool {
    if target.position.zone_id != position.zone_id {
        return false;
    }

    true
}

fn is_valid_attack_target(
    target: &CommandAttackTargetQueryItem,
    position: &Position,
    team: &Team,
) -> bool {
    if target.team.id == team.id {
        return false;
    }

    if target.position.zone_id != position.zone_id {
        return false;
    }

    if target.health_points.hp <= 0 {
        return false;
    }

    true
}

fn is_valid_pickup_target(target: &CommandPickupItemTargetQueryItem, position: &Position) -> bool {
    if target.position.zone_id != position.zone_id {
        return false;
    }

    let distance = position
        .position
        .xy()
        .distance(target.position.position.xy());
    if distance > DROPPED_ITEM_PICKUP_DISTANCE {
        return false;
    }

    true
}

// Inline skill checking functions to avoid QueryData lifetime issues

fn check_skill_cooldown(
    cooldowns: Option<&Cooldowns>,
    now: Instant,
    skill_data: &rose_data::SkillData,
) -> bool {
    let Some(cooldowns) = cooldowns else {
        return true;
    };

    if let Some(global) = cooldowns.skill_global {
        if now - global < GLOBAL_SKILL_COOLDOWN {
            return false;
        }
    }

    match &skill_data.cooldown {
        SkillCooldown::Skill { .. } => {
            if let Some(cooldown_finished) = cooldowns.skill.get(&skill_data.id) {
                if now < *cooldown_finished {
                    return false;
                }
            }
        }
        SkillCooldown::Group { group, .. } => {
            if let Some(cooldown_finished) = cooldowns
                .skill_group
                .get(group.get())
                .and_then(|x: &Option<Instant>| x.as_ref())
            {
                if now < *cooldown_finished {
                    return false;
                }
            }
        }
    }

    true
}

fn check_move_mode(move_mode: &MoveMode, _skill_data: &rose_data::SkillData) -> bool {
    !matches!(move_mode, MoveMode::Drive)
}

fn check_skill_target_filter(
    caster_entity: Entity,
    caster_team: &Team,
    caster_clan: Option<&ClanMembership>,
    caster_party: Option<&PartyMembership>,
    target_entity: Entity,
    target_team: &Team,
    target_clan: Option<&ClanMembership>,
    target_party: Option<&PartyMembership>,
    target_hp: i32,
    target_client_entity: &ClientEntity,
    skill_data: &rose_data::SkillData,
) -> bool {
    let target_is_alive = target_hp > 0;
    let target_is_caster = caster_entity == target_entity;

    match skill_data.target_filter {
        SkillTargetFilter::OnlySelf => target_is_alive && target_is_caster,
        SkillTargetFilter::Group => {
            let caster_party_id = caster_party.and_then(|p| p.party);
            let target_party_id = target_party.and_then(|p| p.party);
            target_is_alive
                && (target_is_caster
                    || (caster_party_id.is_some() && caster_party_id == target_party_id))
        }
        SkillTargetFilter::Guild => {
            let caster_clan_id = caster_clan.and_then(|c| c.clan());
            let target_clan_id = target_clan.and_then(|c| c.clan());
            target_is_alive
                && (target_is_caster
                    || (caster_clan_id.is_some() && caster_clan_id == target_clan_id))
        }
        SkillTargetFilter::Allied => {
            target_is_alive
                && (caster_team.id == target_team.id
                    || target_team.id == Team::DEFAULT_CHARACTER_TEAM_ID
                    || target_team.id == Team::DEFAULT_NPC_TEAM_ID)
        }
        SkillTargetFilter::Monster => {
            target_is_alive && matches!(target_client_entity.entity_type, ClientEntityType::Monster)
        }
        SkillTargetFilter::Enemy => {
            target_is_alive
                && target_team.id != Team::DEFAULT_NPC_TEAM_ID
                && caster_team.id != target_team.id
        }
        SkillTargetFilter::EnemyCharacter => {
            target_is_alive
                && caster_team.id != target_team.id
                && matches!(
                    target_client_entity.entity_type,
                    ClientEntityType::Character
                )
        }
        SkillTargetFilter::Character => {
            target_is_alive
                && matches!(
                    target_client_entity.entity_type,
                    ClientEntityType::Character
                )
        }
        SkillTargetFilter::CharacterOrMonster => {
            target_is_alive
                && matches!(
                    target_client_entity.entity_type,
                    ClientEntityType::Character | ClientEntityType::Monster
                )
        }
        SkillTargetFilter::DeadAlliedCharacter => {
            !target_is_alive
                && !target_is_caster
                && caster_team.id == target_team.id
                && matches!(
                    target_client_entity.entity_type,
                    ClientEntityType::Character
                )
        }
        SkillTargetFilter::EnemyMonster => {
            target_is_alive
                && caster_team.id != target_team.id
                && matches!(target_client_entity.entity_type, ClientEntityType::Monster)
        }
    }
}

fn check_use_ability_value(
    ability_values: &AbilityValues,
    health_points: &HealthPoints,
    mana_points: Option<&ManaPoints>,
    experience_points: Option<&ExperiencePoints>,
    inventory: Option<&Inventory>,
    stamina: Option<&Stamina>,
    equipment: Option<&Equipment>,
    skill_data: &rose_data::SkillData,
) -> bool {
    for &(use_ability_type, mut use_ability_value) in skill_data.use_ability.iter() {
        if use_ability_type == AbilityType::Mana {
            let use_mana_rate = (100 - ability_values.get_save_mana()) as f32 / 100.0;
            use_ability_value = (use_ability_value as f32 * use_mana_rate) as i32;
        }

        let ability_value = match use_ability_type {
            AbilityType::Level => ability_values.level,
            AbilityType::Strength => ability_values.strength,
            AbilityType::Dexterity => ability_values.dexterity,
            AbilityType::Intelligence => ability_values.intelligence,
            AbilityType::Concentration => ability_values.concentration,
            AbilityType::Charm => ability_values.charm,
            AbilityType::Sense => ability_values.sense,
            AbilityType::Health => health_points.hp,
            AbilityType::Mana => mana_points.map_or(0, |mp| mp.mp),
            AbilityType::Experience => experience_points
                .map_or(0, |xp| xp.xp)
                .try_into()
                .unwrap_or(i32::MAX),
            AbilityType::Money => inventory
                .map_or(0, |inv| inv.money.0)
                .try_into()
                .unwrap_or(i32::MAX),
            AbilityType::Stamina => stamina
                .map_or(0, |s| s.stamina)
                .try_into()
                .unwrap_or(i32::MAX),
            AbilityType::Fuel => equipment.map_or(0, |eq| {
                eq.get_vehicle_item(VehiclePartIndex::Engine)
                    .map_or(0, |item| item.life as i32)
            }),
            invalid => {
                log::warn!(
                    "SkillId: {} requires invalid use_ability type {:?}",
                    skill_data.id.get(),
                    invalid
                );
                -999
            }
        };

        if ability_value < use_ability_value {
            return false;
        }
    }

    true
}

fn check_equipment(
    game_data: &GameData,
    equipment: Option<&Equipment>,
    skill_data: &rose_data::SkillData,
) -> bool {
    let Some(equipment) = equipment else {
        return true;
    };

    if skill_data.required_equipment_class.is_empty() {
        return true;
    }

    let weapon_class = equipment
        .get_equipment_item(EquipmentIndex::Weapon)
        .and_then(|item| game_data.items.get_base_item(item.item))
        .map(|item_data| item_data.class);
    let sub_weapon_class = equipment
        .get_equipment_item(EquipmentIndex::SubWeapon)
        .and_then(|item| game_data.items.get_base_item(item.item))
        .map(|item_data| item_data.class);

    for &required_equipment_class in skill_data.required_equipment_class.iter() {
        if weapon_class == Some(required_equipment_class) {
            return true;
        }

        if sub_weapon_class == Some(required_equipment_class) {
            return true;
        }
    }

    false
}

/// Check if a skill can be used by the caster (inline version to avoid QueryData lifetime issues)
fn skill_can_use_inline(
    now: Instant,
    game_data: &GameData,
    client_entity: &ClientEntity,
    ability_values: &AbilityValues,
    health_points: &HealthPoints,
    move_mode: &MoveMode,
    team: &Team,
    cooldowns: Option<&Cooldowns>,
    equipment: Option<&Equipment>,
    experience_points: Option<&ExperiencePoints>,
    inventory: Option<&Inventory>,
    mana_points: Option<&ManaPoints>,
    party_membership: Option<&PartyMembership>,
    clan_membership: Option<&ClanMembership>,
    stamina: Option<&Stamina>,
    skill_data: &rose_data::SkillData,
) -> bool {
    if !client_entity.is_character() {
        // We only check use requirements for characters
        return true;
    }

    if !check_skill_cooldown(cooldowns, now, skill_data) {
        return false;
    }

    // check_not_disabled - TODO: Check not muted / sleep / fainted / stunned

    // check_weight - TODO: Check weight not too heavy to use skills (110%)

    if !check_move_mode(move_mode, skill_data) {
        return false;
    }

    // check_summon_points - TODO

    if !check_use_ability_value(
        ability_values,
        health_points,
        mana_points,
        experience_points,
        inventory,
        stamina,
        equipment,
        skill_data,
    ) {
        return false;
    }

    if !check_equipment(game_data, equipment, skill_data) {
        return false;
    }

    true
}

pub fn command_system(
    mut commands: Commands,
    mut query_command_entity: Query<QueryCommandEntity>,
    query_move_target: Query<CommandMoveTargetQuery>,
    query_attack_target: Query<CommandAttackTargetQuery>,
    mut query_pickup_item: Query<CommandPickupItemTargetQuery>,
    query_position: Query<(&ClientEntity, &Position)>,
    // Use explicit tuples instead of QueryData types to avoid lifetime issues
    query_skill_caster: Query<(
        Entity,
        &AbilityValues,
        &ClientEntity,
        &HealthPoints,
        &MoveMode,
        &Team,
        Option<&ClanMembership>,
        Option<&Cooldowns>,
        Option<&Equipment>,
        Option<&ExperiencePoints>,
        Option<&Inventory>,
        Option<&ManaPoints>,
        Option<&PartyMembership>,
        Option<&Stamina>,
    )>,
    query_skill_target: Query<(
        Entity,
        &ClientEntity,
        &HealthPoints,
        &Team,
        Option<&ClanMembership>,
        Option<&PartyMembership>,
    )>,
    game_data: Res<GameData>,
    time: Res<Time>,
    mut damage_events: MessageWriter<DamageEvent>,
    mut skill_events: MessageWriter<SkillEvent>,
    mut pickup_item_event: MessageWriter<PickupItemEvent>,
    mut item_life_event: MessageWriter<ItemLifeEvent>,
    mut use_ammo_event: MessageWriter<UseAmmoEvent>,
    mut server_messages: ResMut<ServerMessages>,
) {
    let now = Instant::now();

    for mut command_entity in query_command_entity.iter_mut() {
        if command_entity.command.is_dead() {
            // Ignore all requested commands whilst dead.
            command_entity.next_command.command = None;
        }

        if !command_entity.next_command.has_sent_server_message
            && command_entity.next_command.command.is_some()
        {
            // Send any server message required for update client next command
            match command_entity.next_command.command.as_mut().unwrap() {
                CommandData::Die { .. } => {
                    panic!("Next command should never be set to die, set current command")
                }
                CommandData::Sit | CommandData::Sitting | CommandData::Standing => {}
                CommandData::Stop { .. } => {}
                CommandData::PersonalStore => {}
                CommandData::PickupItemDrop { .. } => {}
                CommandData::Emote { .. } => {}
                CommandData::Move {
                    destination,
                    target,
                    move_mode: command_move_mode,
                } => {
                    let mut target_entity_id = None;
                    if let Some(target_entity) = *target {
                        if let Some(target) = query_move_target
                            .get(target_entity)
                            .ok()
                            .filter(|target| is_valid_move_target(target, command_entity.position))
                        {
                            *destination = target.position.position;
                            target_entity_id = Some(target.client_entity.id);
                        } else {
                            *target = None;
                        }
                    }

                    let distance = command_entity
                        .position
                        .position
                        .xy()
                        .distance(destination.xy());
                    server_messages.send_entity_message(
                        command_entity.client_entity,
                        ServerMessage::MoveEntity {
                            entity_id: command_entity.client_entity.id,
                            target_entity_id,
                            distance: distance as u16,
                            x: destination.x,
                            y: destination.y,
                            z: destination.z as u16,
                            move_mode: *command_move_mode,
                        },
                    );
                }
                &mut CommandData::Attack {
                    target: target_entity,
                } => {
                    if let Some(target) =
                        query_attack_target
                            .get(target_entity)
                            .ok()
                            .filter(|target| {
                                is_valid_attack_target(
                                    target,
                                    command_entity.position,
                                    command_entity.team,
                                )
                            })
                    {
                        let distance = command_entity
                            .position
                            .position
                            .xy()
                            .distance(target.position.position.xy());

                        server_messages.send_entity_message(
                            command_entity.client_entity,
                            ServerMessage::AttackEntity {
                                entity_id: command_entity.client_entity.id,
                                target_entity_id: target.client_entity.id,
                                distance: distance as u16,
                                x: target.position.position.x,
                                y: target.position.position.y,
                                z: target.position.position.z as u16,
                            },
                        );
                    } else {
                        *command_entity.next_command = NextCommand::with_stop(true);
                    }
                }
                &mut CommandData::CastSkill {
                    skill_id,
                    ref skill_target,
                    cast_motion_id,
                    ..
                } => {
                    // Inline skill validation to avoid QueryData lifetime issues
                    let Some(skill_data) = game_data.skills.get_skill(skill_id) else {
                        continue;
                    };

                    let Ok(skill_caster) = query_skill_caster.get(command_entity.entity) else {
                        continue;
                    };

                    let (
                        caster_entity,
                        caster_ability_values,
                        caster_client_entity,
                        caster_health_points,
                        caster_move_mode,
                        caster_team,
                        caster_clan_membership,
                        caster_cooldowns,
                        caster_equipment,
                        caster_experience_points,
                        caster_inventory,
                        caster_mana_points,
                        caster_party_membership,
                        caster_stamina,
                    ) = skill_caster;

                    if !skill_can_use_inline(
                        now,
                        &game_data,
                        caster_client_entity,
                        caster_ability_values,
                        caster_health_points,
                        caster_move_mode,
                        caster_team,
                        caster_cooldowns,
                        caster_equipment,
                        caster_experience_points,
                        caster_inventory,
                        caster_mana_points,
                        caster_party_membership,
                        caster_clan_membership,
                        caster_stamina,
                        skill_data,
                    ) {
                        continue;
                    }

                    let can_cast = match skill_target {
                        Some(CommandCastSkillTarget::Entity(target_entity)) => {
                            match query_skill_target.get(*target_entity) {
                                Ok((
                                    target_entity_val,
                                    target_client_entity,
                                    target_health_points,
                                    target_team,
                                    target_clan_membership,
                                    target_party_membership,
                                )) => check_skill_target_filter(
                                    caster_entity,
                                    caster_team,
                                    caster_clan_membership,
                                    caster_party_membership,
                                    target_entity_val,
                                    target_team,
                                    target_clan_membership,
                                    target_party_membership,
                                    target_health_points.hp,
                                    target_client_entity,
                                    skill_data,
                                ),
                                Err(_) => false,
                            }
                        }
                        Some(CommandCastSkillTarget::Position(_)) => {
                            matches!(skill_data.skill_type, SkillType::AreaTarget)
                        }
                        None => {
                            matches!(
                                skill_data.skill_type,
                                SkillType::SelfBoundDuration
                                    | SkillType::SelfBound
                                    | SkillType::SelfStateDuration
                                    | SkillType::SummonPet
                                    | SkillType::SelfDamage
                            ) || check_skill_target_filter(
                                caster_entity,
                                caster_team,
                                caster_clan_membership,
                                caster_party_membership,
                                caster_entity,
                                caster_team,
                                caster_clan_membership,
                                caster_party_membership,
                                caster_health_points.hp,
                                caster_client_entity,
                                skill_data,
                            )
                        }
                    };

                    if can_cast {
                        match skill_target {
                            Some(CommandCastSkillTarget::Entity(target_entity)) => {
                                let (target_client_entity, target_position) =
                                    query_position.get(*target_entity).unwrap();
                                let distance = command_entity
                                    .position
                                    .position
                                    .xy()
                                    .distance(target_position.position.xy());

                                server_messages.send_entity_message(
                                    command_entity.client_entity,
                                    ServerMessage::CastSkillTargetEntity {
                                        entity_id: command_entity.client_entity.id,
                                        skill_id,
                                        target_entity_id: target_client_entity.id,
                                        target_distance: distance,
                                        target_position: target_position.position.xy(),
                                        cast_motion_id,
                                    },
                                );
                            }
                            Some(CommandCastSkillTarget::Position(target_position)) => {
                                server_messages.send_entity_message(
                                    command_entity.client_entity,
                                    ServerMessage::CastSkillTargetPosition {
                                        entity_id: command_entity.client_entity.id,
                                        skill_id,
                                        target_position: *target_position,
                                        cast_motion_id,
                                    },
                                );
                            }
                            None => {
                                server_messages.send_entity_message(
                                    command_entity.client_entity,
                                    ServerMessage::CastSkillSelf {
                                        entity_id: command_entity.client_entity.id,
                                        skill_id,
                                        cast_motion_id,
                                    },
                                );
                            }
                        }
                    } else {
                        // Send explicit rejection message for invalid target
                        server_messages.send_entity_message(
                            command_entity.client_entity,
                            ServerMessage::CancelCastingSkill {
                                entity_id: command_entity.client_entity.id,
                                reason: crate::game::messages::server::CancelCastingSkillReason::InvalidTarget,
                            },
                        );
                    }
                }
            }

            command_entity.next_command.has_sent_server_message = true;
        }

        command_entity.command.duration += time.delta();

        let required_duration = match &mut command_entity.command.command {
            CommandData::Attack { .. } => {
                let attack_speed =
                    i32::max(command_entity.ability_values.get_attack_speed(), 30) as f32 / 100.0;
                command_entity
                    .command
                    .required_duration
                    .map(|duration| duration.div_f32(attack_speed))
            }
            CommandData::Emote { .. } => {
                // Any command can interrupt an emote
                if command_entity.next_command.command.is_some() {
                    None
                } else {
                    command_entity.command.required_duration
                }
            }
            _ => command_entity.command.required_duration,
        };

        let command_motion_completed = required_duration.map_or_else(
            || true,
            |required_duration| command_entity.command.duration >= required_duration,
        );

        if !command_motion_completed {
            // Current command still in animation
            continue;
        }

        match command_entity.command.command {
            CommandData::Die { .. } => {
                // We can't perform NextCommand if we are dead!
                continue;
            }
            CommandData::Sitting => {
                // When sitting animation is complete transition to Sit
                *command_entity.command = Command::with_sit();
            }
            _ => {}
        }

        if command_entity.next_command.command.is_none() {
            // If we have completed current command, and there is no next command, then clear current.
            // This does not apply for some commands which must be manually completed, such as Sit
            // where you need to stand after.
            if command_motion_completed && !command_entity.command.command.is_manual_complete() {
                *command_entity.command = Command::default();
            }

            // Nothing to do when there is no next command
            continue;
        }

        if matches!(command_entity.command.command, CommandData::Sit) {
            // If current command is sit, we must stand before performing NextCommand
            let duration = command_entity
                .motion_data
                .get_sit_standing()
                .map(|motion_data| motion_data.duration)
                .unwrap_or_else(|| Duration::from_secs(0));

            *command_entity.command = Command::with_standing(duration);

            server_messages.send_entity_message(
                command_entity.client_entity,
                ServerMessage::SitToggle {
                    entity_id: command_entity.client_entity.id,
                },
            );
            continue;
        }

        let weapon_item_data = command_entity.equipment.as_ref().and_then(|equipment| {
            equipment
                .get_equipment_item(EquipmentIndex::Weapon)
                .and_then(|weapon_item| {
                    game_data
                        .items
                        .get_weapon_item(weapon_item.item.item_number)
                })
        });
        let weapon_motion_type = weapon_item_data
            .map(|weapon_item_data| weapon_item_data.motion_type as usize)
            .unwrap_or(0);
        let weapon_motion_gender = command_entity
            .character_info
            .map(|character_info| match character_info.gender {
                CharacterGender::Male => 0,
                CharacterGender::Female => 1,
            })
            .unwrap_or(0);

        match command_entity.next_command.command.as_mut().unwrap() {
            &mut CommandData::Stop { send_message } => {
                command_stop(
                    &mut command_entity.command,
                    command_entity.client_entity,
                    command_entity.position,
                    if send_message {
                        Some(&mut server_messages)
                    } else {
                        None
                    },
                );
                *command_entity.next_command = NextCommand::default();
            }
            CommandData::Move {
                destination,
                target,
                move_mode: command_move_mode,
            } => {
                let mut entity_commands = commands.entity(command_entity.entity);

                if let Some(target_entity) = *target {
                    if let Some(target) = query_move_target
                        .get(target_entity)
                        .ok()
                        .filter(|target| is_valid_move_target(target, command_entity.position))
                    {
                        let required_distance = match target.client_entity.entity_type {
                            ClientEntityType::Character => Some(CHARACTER_MOVE_TO_DISTANCE),
                            ClientEntityType::Npc => Some(NPC_MOVE_TO_DISTANCE),
                            ClientEntityType::ItemDrop => Some(DROPPED_ITEM_MOVE_TO_DISTANCE),
                            _ => None,
                        };

                        if let Some(required_distance) = required_distance {
                            let distance = command_entity
                                .position
                                .position
                                .xy()
                                .distance(target.position.position.xy());
                            if distance < required_distance {
                                // We are already within required distance, so no need to move further
                                *destination = command_entity.position.position;
                            } else {
                                let offset = (target.position.position.xy()
                                    - command_entity.position.position.xy())
                                .normalize()
                                    * required_distance;
                                destination.x = target.position.position.x - offset.x;
                                destination.y = target.position.position.y - offset.y;
                                destination.z = target.position.position.z;
                            }
                        } else {
                            *destination = target.position.position;
                        }
                    } else {
                        *target = None;
                    }
                }

                // If this move command has a different move mode, update move mode and move speed
                if let Some(command_move_mode) = command_move_mode.as_ref() {
                    if command_move_mode != command_entity.move_mode {
                        entity_commands.insert((
                            *command_move_mode,
                            MoveSpeed::new(
                                command_entity
                                    .ability_values
                                    .get_move_speed(command_move_mode),
                            ),
                        ));
                    }
                }

                let distance = command_entity
                    .position
                    .position
                    .xy()
                    .distance(destination.xy());
                if distance < 0.1 {
                    *command_entity.command = Command::with_stop();
                } else {
                    *command_entity.command =
                        Command::with_move(*destination, *target, *command_move_mode);
                }
            }
            &mut CommandData::PickupItemDrop {
                target: target_entity,
            } => {
                if query_pickup_item
                    .get_mut(target_entity)
                    .ok()
                    .map_or(false, |target| {
                        is_valid_pickup_target(&target, command_entity.position)
                    })
                {
                    pickup_item_event.write(PickupItemEvent {
                        pickup_entity: command_entity.entity,
                        item_entity: target_entity,
                    });

                    // Update our current command
                    let motion_duration = command_entity
                        .motion_data
                        .get_pickup_item_drop()
                        .map_or_else(|| Duration::from_secs(1), |motion| motion.duration);

                    *command_entity.command =
                        Command::with_pickup_item_drop(target_entity, motion_duration);
                } else {
                    *command_entity.command = Command::with_stop();
                }

                *command_entity.next_command = NextCommand::default();
            }
            &mut CommandData::Attack {
                target: target_entity,
            } => {
                log::info!(
                    "[COMBAT_DEBUG] Processing Attack command: entity={:?}, target={:?}, client_entity_id={:?}",
                    command_entity.entity,
                    target_entity,
                    command_entity.client_entity.id
                );

                let Some(target) = query_attack_target.get(target_entity).ok() else {
                    // Cannot attack target, cancel command.
                    log::warn!(
                        "[COMBAT_DEBUG] Attack target missing required components: entity={:?}, target={:?}",
                        command_entity.entity,
                        target_entity
                    );
                    command_stop(
                        &mut command_entity.command,
                        command_entity.client_entity,
                        command_entity.position,
                        Some(&mut server_messages),
                    );
                    *command_entity.next_command = NextCommand::default();
                    continue;
                };

                if !is_valid_attack_target(&target, command_entity.position, command_entity.team) {
                    // Cannot attack target, cancel command.
                    log::warn!(
                        "[COMBAT_DEBUG] Attack target failed validation: attacker={:?}, target={:?}, attacker_team={}, target_team={}, attacker_zone={:?}, target_zone={:?}, target_hp={}",
                        command_entity.entity,
                        target_entity,
                        command_entity.team.id,
                        target.team.id,
                        command_entity.position.zone_id,
                        target.position.zone_id,
                        target.health_points.hp
                    );
                    command_stop(
                        &mut command_entity.command,
                        command_entity.client_entity,
                        command_entity.position,
                        Some(&mut server_messages),
                    );
                    *command_entity.next_command = NextCommand::default();
                    continue;
                }

                let attack_range = command_entity.ability_values.get_attack_range() as f32;
                let distance = command_entity
                    .position
                    .position
                    .xy()
                    .distance(target.position.position.xy());

                log::info!(
                    "[COMBAT_DEBUG] Attack range check: attack_range={}, distance={}, in_range={}",
                    attack_range,
                    distance,
                    attack_range >= distance
                );

                // Use 3x attack range to allow bot to start moving toward target earlier
                // This prevents the bot from standing still when target is far away
                let effective_attack_range = attack_range * 3.0;

                if effective_attack_range < distance {
                    let direction_to_target =
                        target.position.position.xy() - command_entity.position.position.xy();
                    let move_destination = if direction_to_target.length_squared() > 0.0 {
                        let offset = direction_to_target.normalize() * attack_range;
                        Vec3::new(
                            target.position.position.x - offset.x,
                            target.position.position.y - offset.y,
                            target.position.position.z,
                        )
                    } else {
                        target.position.position
                    };

                    // Not in range, set current command to move AND update next_command
                    // so the bot actually moves instead of re-processing Attack every frame
                    *command_entity.command = Command::with_move(
                        move_destination,
                        Some(target_entity),
                        Some(MoveMode::Run),
                    );
                    // Keep the Attack command in next_command so bot attacks when in range
                    // but mark it as needing to send server message so Move is broadcast
                    command_entity.next_command.has_sent_server_message = false;
                    continue;
                }

                if attack_range < distance {
                    // Close enough to start attack animation, but need to move closer
                    // The attack will execute and the bot will close distance naturally
                    log::info!(
                        "[COMBAT_DEBUG] Within effective range ({}), starting attack (actual range={})",
                        effective_attack_range,
                        attack_range
                    );
                }

                let mut cancel_attack = false;
                let mut cancel_no_attack_motion = false;
                let mut cancel_broken_vehicle_engine = false;
                let mut cancel_broken_vehicle_arms = false;
                let mut cancel_broken_weapon = false;
                let mut cancel_not_enough_ammo = false;

                let (attack_duration, hit_count) =
                    if let Some(attack_motion) = command_entity.motion_data.get_attack() {
                        log::info!(
                            "[COMBAT_DEBUG] Attack motion found: duration={:?}, hit_count={}",
                            attack_motion.duration,
                            attack_motion.total_attack_frames
                        );
                        (attack_motion.duration, attack_motion.total_attack_frames)
                    } else {
                        // No attack animation, cancel attack
                        log::warn!("[COMBAT_DEBUG] No attack motion found, cancelling attack!");
                        cancel_attack = true;
                        cancel_no_attack_motion = true;
                        (Duration::ZERO, 0)
                    };

                if matches!(command_entity.move_mode, MoveMode::Drive) {
                    if let Some(equipment) = command_entity.equipment.as_ref() {
                        if equipment
                            .get_vehicle_item(VehiclePartIndex::Engine)
                            .map_or(false, |equipment_item| equipment_item.life == 0)
                        {
                            // Vehicle engine is broken, cancel attack
                            cancel_attack = true;
                            cancel_broken_vehicle_engine = true;
                        }

                        if equipment
                            .get_vehicle_item(VehiclePartIndex::Arms)
                            .map_or(false, |equipment_item| equipment_item.life == 0)
                        {
                            // Vehicle weapon item is broken, cancel attack
                            cancel_attack = true;
                            cancel_broken_vehicle_arms = true;
                        }
                    }
                } else {
                    if let Some(equipment) = command_entity.equipment.as_ref() {
                        if equipment
                            .get_equipment_item(EquipmentIndex::Weapon)
                            .map_or(false, |equipment_item| equipment_item.life == 0)
                        {
                            // Weapon item is broken, cancel attack
                            cancel_attack = true;
                            cancel_broken_weapon = true;
                        }
                    }

                    // If the weapon uses ammo, we must consume the ammo
                    if !cancel_attack {
                        if let Some(equipment) = command_entity.equipment {
                            if let Some(weapon_item_data) = weapon_item_data {
                                let ammo_index = match weapon_item_data.item_data.class {
                                    ItemClass::Bow | ItemClass::Crossbow => Some(AmmoIndex::Arrow),
                                    ItemClass::Gun | ItemClass::DualGuns => Some(AmmoIndex::Bullet),
                                    ItemClass::Launcher => Some(AmmoIndex::Throw),
                                    _ => None,
                                };

                                if let Some(ammo_index) = ammo_index {
                                    if equipment
                                        .get_ammo_item(ammo_index)
                                        .map_or(false, |ammo_item| {
                                            ammo_item.quantity >= hit_count as u32
                                        })
                                    {
                                        use_ammo_event.write(UseAmmoEvent {
                                            entity: command_entity.entity,
                                            ammo_index,
                                            quantity: hit_count,
                                        });
                                    } else {
                                        // Not enough ammo, cancel attack
                                        cancel_attack = true;
                                        cancel_not_enough_ammo = true;
                                    }
                                }
                            }
                        }
                    }
                }

                if cancel_attack {
                    // Attack requirements not met, cancel attack
                    log::warn!(
                        "[COMBAT_DEBUG] Attack cancelled: entity={:?}, target={:?}, no_attack_motion={}, broken_vehicle_engine={}, broken_vehicle_arms={}, broken_weapon={}, not_enough_ammo={}, hit_count={}",
                        command_entity.entity,
                        target_entity,
                        cancel_no_attack_motion,
                        cancel_broken_vehicle_engine,
                        cancel_broken_vehicle_arms,
                        cancel_broken_weapon,
                        cancel_not_enough_ammo,
                        hit_count,
                    );
                    command_stop(
                        &mut command_entity.command,
                        command_entity.client_entity,
                        command_entity.position,
                        Some(&mut server_messages),
                    );
                    *command_entity.next_command = NextCommand::default();
                    continue;
                }

                if matches!(command_entity.move_mode, MoveMode::Drive) {
                    // Decrease vehicle engine item life on attack
                    item_life_event.write(ItemLifeEvent::DecreaseVehicleEngineLife {
                        entity: command_entity.entity,
                        amount: None,
                    });
                }

                // Decrease weapon item life on attack
                if command_entity.character_info.is_some() {
                    item_life_event.write(ItemLifeEvent::DecreaseWeaponLife {
                        entity: command_entity.entity,
                    });
                }

                // In range, set current command to attack
                *command_entity.command = Command::with_attack(target_entity, attack_duration);

                // Broadcast each attack cycle start so clients restart attack animation
                // for continuous server-authoritative combat instead of only on the
                // first queued attack intent.
                server_messages.send_entity_message(
                    command_entity.client_entity,
                    ServerMessage::AttackEntity {
                        entity_id: command_entity.client_entity.id,
                        target_entity_id: target.client_entity.id,
                        distance: distance as u16,
                        x: target.position.position.x,
                        y: target.position.position.y,
                        z: target.position.position.z as u16,
                    },
                );

                // Calculate damage
                let damage = game_data.ability_value_calculator.calculate_damage(
                    command_entity.ability_values,
                    target.ability_values,
                    hit_count as i32,
                );

                // DEBUG: Log damage event being sent
                log::info!(
                    "[COMBAT_DEBUG] Sending DamageEvent: attacker={:?}, defender={:?}, damage={}, is_critical={}",
                    command_entity.entity,
                    target_entity,
                    damage.amount,
                    damage.is_critical
                );

                // Send damage event to damage system
                damage_events.write(DamageEvent::Attack {
                    attacker: command_entity.entity,
                    defender: target_entity,
                    damage,
                });
            }
            &mut CommandData::CastSkill {
                skill_id,
                skill_target,
                ref use_item,
                cast_motion_id,
                action_motion_id,
            } => {
                // Inline skill validation to avoid QueryData lifetime issues
                let Some(skill_data) = game_data.skills.get_skill(skill_id) else {
                    // Cannot use skill, cancel command.
                    command_stop(
                        &mut command_entity.command,
                        command_entity.client_entity,
                        command_entity.position,
                        Some(&mut server_messages),
                    );
                    *command_entity.next_command = NextCommand::default();
                    continue;
                };

                let Ok(skill_caster) = query_skill_caster.get(command_entity.entity) else {
                    // Cannot use skill, cancel command.
                    command_stop(
                        &mut command_entity.command,
                        command_entity.client_entity,
                        command_entity.position,
                        Some(&mut server_messages),
                    );
                    *command_entity.next_command = NextCommand::default();
                    continue;
                };

                let (
                    caster_entity,
                    caster_ability_values,
                    caster_client_entity,
                    caster_health_points,
                    caster_move_mode,
                    caster_team,
                    caster_clan_membership,
                    caster_cooldowns,
                    caster_equipment,
                    caster_experience_points,
                    caster_inventory,
                    caster_mana_points,
                    caster_party_membership,
                    caster_stamina,
                ) = skill_caster;

                if !skill_can_use_inline(
                    now,
                    &game_data,
                    caster_client_entity,
                    caster_ability_values,
                    caster_health_points,
                    caster_move_mode,
                    caster_team,
                    caster_cooldowns,
                    caster_equipment,
                    caster_experience_points,
                    caster_inventory,
                    caster_mana_points,
                    caster_party_membership,
                    caster_clan_membership,
                    caster_stamina,
                    skill_data,
                ) {
                    // Cannot use skill, cancel command.
                    command_stop(
                        &mut command_entity.command,
                        command_entity.client_entity,
                        command_entity.position,
                        Some(&mut server_messages),
                    );
                    *command_entity.next_command = NextCommand::default();
                    continue;
                }

                // Check target validity
                let target_valid = match &skill_target {
                    Some(CommandCastSkillTarget::Entity(target_entity)) => {
                        match query_skill_target.get(*target_entity) {
                            Ok((
                                target_entity_val,
                                target_client_entity,
                                target_health_points,
                                target_team,
                                target_clan_membership,
                                target_party_membership,
                            )) => check_skill_target_filter(
                                caster_entity,
                                caster_team,
                                caster_clan_membership,
                                caster_party_membership,
                                target_entity_val,
                                target_team,
                                target_clan_membership,
                                target_party_membership,
                                target_health_points.hp,
                                target_client_entity,
                                skill_data,
                            ),
                            Err(_) => false,
                        }
                    }
                    Some(CommandCastSkillTarget::Position(_)) => {
                        matches!(skill_data.skill_type, SkillType::AreaTarget)
                    }
                    None => {
                        matches!(
                            skill_data.skill_type,
                            SkillType::SelfBoundDuration
                                | SkillType::SelfBound
                                | SkillType::SelfStateDuration
                                | SkillType::SummonPet
                                | SkillType::SelfDamage
                        ) || check_skill_target_filter(
                            caster_entity,
                            caster_team,
                            caster_clan_membership,
                            caster_party_membership,
                            caster_entity,
                            caster_team,
                            caster_clan_membership,
                            caster_party_membership,
                            caster_health_points.hp,
                            caster_client_entity,
                            skill_data,
                        )
                    }
                };

                if !target_valid {
                    // Cannot use skill, cancel command.
                    command_stop(
                        &mut command_entity.command,
                        command_entity.client_entity,
                        command_entity.position,
                        Some(&mut server_messages),
                    );
                    *command_entity.next_command = NextCommand::default();
                    continue;
                }

                let (target_position, target_entity) = match skill_target {
                    Some(CommandCastSkillTarget::Entity(target_entity)) => {
                        let (_, target_position) = query_position.get(target_entity).unwrap();
                        (Some(target_position.position), Some(target_entity))
                    }
                    Some(CommandCastSkillTarget::Position(target_position)) => (
                        // Note: Vec2 position for skills is ground-targeted, Z=0 is intentional
                        Some(Vec3::new(target_position.x, target_position.y, 0.0)),
                        None,
                    ),
                    None => (None, None),
                };

                let cast_range = if skill_data.cast_range > 0 {
                    skill_data.cast_range as f32
                } else {
                    command_entity.ability_values.get_attack_range() as f32
                };

                let in_distance = target_position.map_or(true, |target_position| {
                    command_entity
                        .position
                        .position
                        .xy()
                        .distance_squared(target_position.xy())
                        < cast_range * cast_range
                });
                if !in_distance {
                    // Not in range, set current command to move
                    // TODO: By changing command to move here we affect SkillActionMode::Restore, should save current command
                    *command_entity.command = Command::with_move(
                        target_position.unwrap(),
                        target_entity,
                        Some(MoveMode::Run),
                    );
                    continue;
                }

                let casting_duration = cast_motion_id
                    .or(skill_data.casting_motion_id)
                    .and_then(|motion_id| {
                        if let Some(npc) = command_entity.npc {
                            game_data.npcs.get_npc_motion(npc.id, motion_id)
                        } else {
                            game_data.motions.find_first_character_motion(
                                motion_id,
                                weapon_motion_type,
                                weapon_motion_gender,
                            )
                        }
                    })
                    .map(|motion_data| motion_data.duration)
                    .unwrap_or_else(|| Duration::from_secs(0))
                    .mul_f32(skill_data.casting_motion_speed);

                let action_duration = action_motion_id
                    .or(skill_data.action_motion_id)
                    .and_then(|motion_id| {
                        if let Some(npc) = command_entity.npc {
                            game_data.npcs.get_npc_motion(npc.id, motion_id)
                        } else {
                            game_data.motions.find_first_character_motion(
                                motion_id,
                                weapon_motion_type,
                                weapon_motion_gender,
                            )
                        }
                    })
                    .map(|motion_data| motion_data.duration)
                    .unwrap_or_else(|| Duration::from_secs(0))
                    .mul_f32(skill_data.action_motion_speed);

                // For skills which target an entity, we must send a message indicating start of skill
                if target_entity.is_some() {
                    server_messages.send_entity_message(
                        command_entity.client_entity,
                        ServerMessage::StartCastingSkill {
                            entity_id: command_entity.client_entity.id,
                        },
                    );
                }

                // Send skill event for effect to be applied after casting motion
                skill_events.write(SkillEvent::new(
                    command_entity.entity,
                    now + casting_duration,
                    skill_id,
                    match skill_target {
                        None => SkillEventTarget::Entity(command_entity.entity),
                        Some(CommandCastSkillTarget::Entity(target_entity)) => {
                            SkillEventTarget::Entity(target_entity)
                        }
                        Some(CommandCastSkillTarget::Position(target_position)) => {
                            SkillEventTarget::Position(target_position)
                        }
                    },
                    use_item.clone(),
                ));

                // Update next command
                match skill_data.action_mode {
                    SkillActionMode::Stop => *command_entity.next_command = NextCommand::default(),
                    SkillActionMode::Attack => {
                        *command_entity.next_command =
                            target_entity.map_or_else(NextCommand::default, |target| {
                                NextCommand::with_command_skip_server_message(CommandData::Attack {
                                    target,
                                })
                            })
                    }
                    SkillActionMode::Restore => match command_entity.command.command {
                        CommandData::Stop { .. }
                        | CommandData::Move { .. }
                        | CommandData::Attack { .. } => {
                            *command_entity.next_command =
                                NextCommand::with_command_skip_server_message(
                                    command_entity.command.command.clone(),
                                )
                        }
                        _ => *command_entity.next_command = NextCommand::default(),
                    },
                }

                // Update current command
                *command_entity.command = Command::with_cast_skill(
                    skill_id,
                    skill_target.clone(),
                    casting_duration,
                    action_duration,
                );
            }
            CommandData::Sit | CommandData::Sitting => {
                // Set current command to sitting transition
                let duration = command_entity
                    .motion_data
                    .get_sit_sitting()
                    .map(|motion_data| motion_data.duration)
                    .unwrap_or_else(|| Duration::from_secs(0));

                *command_entity.command = Command::with_sitting(duration);
                *command_entity.next_command = NextCommand::default();

                server_messages.send_entity_message(
                    command_entity.client_entity,
                    ServerMessage::SitToggle {
                        entity_id: command_entity.client_entity.id,
                    },
                );
            }
            CommandData::Standing => {
                // Set current command to stand-up transition
                let duration = command_entity
                    .motion_data
                    .get_sit_standing()
                    .map(|motion_data| motion_data.duration)
                    .unwrap_or_else(|| Duration::from_secs(0));

                *command_entity.command = Command::with_standing(duration);
                *command_entity.next_command = NextCommand::default();

                server_messages.send_entity_message(
                    command_entity.client_entity,
                    ServerMessage::SitToggle {
                        entity_id: command_entity.client_entity.id,
                    },
                );
            }
            _ => {}
        }
    }
}
