use bevy::{
    prelude::{Commands, Component, Entity, Query, Res, With},
    time::Time,
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rose_data::{SkillTargetFilter, SkillType};

use crate::game::{
    bundles::GLOBAL_SKILL_COOLDOWN,
    components::{
        AbilityValues, ClanMembership, ClientEntity, Command, CommandData, Cooldowns, Equipment,
        ExperiencePoints, HealthPoints, Inventory, ManaPoints, MoveMode, NextCommand,
        PartyMembership, SkillList, Stamina, StatusEffects, Team,
    },
    GameData,
};

use super::{BotQueryFilterAlive, BotQueryFilterAliveNoTarget};

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldUseBuffSkill {
    pub score: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct UseBuffSkill;

/// Check if a skill can be used (inline version to avoid QueryData lifetime issues)
fn skill_can_use_inline(
    now: std::time::Instant,
    game_data: &GameData,
    client_entity: &ClientEntity,
    ability_values: &AbilityValues,
    health_points: &HealthPoints,
    move_mode: &MoveMode,
    team: &Team,
    cooldowns: Option<&Cooldowns>,
    equipment: Option<&Equipment>,
    mana_points: Option<&ManaPoints>,
    skill_data: &rose_data::SkillData,
) -> bool {
    if !client_entity.is_character() {
        // We only check use requirements for characters
        return true;
    }

    // Check cooldown
    if let Some(cooldowns) = cooldowns {
        if let Some(global) = cooldowns.skill_global {
            if now - global < GLOBAL_SKILL_COOLDOWN {
                return false;
            }
        }

        match &skill_data.cooldown {
            rose_data::SkillCooldown::Skill { .. } => {
                if let Some(cooldown_finished) = cooldowns.skill.get(&skill_data.id) {
                    if now < *cooldown_finished {
                        return false;
                    }
                }
            }
            rose_data::SkillCooldown::Group { group, .. } => {
                if let Some(cooldown_finished) = cooldowns
                    .skill_group
                    .get(group.get())
                    .and_then(|x: &Option<std::time::Instant>| x.as_ref())
                {
                    if now < *cooldown_finished {
                        return false;
                    }
                }
            }
        }
    }

    // Check move mode
    if matches!(move_mode, MoveMode::Drive) {
        return false;
    }

    // Check mana
    if skill_data.use_ability.iter().any(|(ability_type, _)| {
        *ability_type == rose_data::AbilityType::Mana && mana_points.is_none()
    }) {
        return false;
    }

    // Check equipment
    if !skill_data.required_equipment_class.is_empty() {
        if let Some(equipment) = equipment {
            let weapon_class = equipment
                .get_equipment_item(rose_data::EquipmentIndex::Weapon)
                .and_then(|item| game_data.items.get_base_item(item.item))
                .map(|item_data| item_data.class);
            let sub_weapon_class = equipment
                .get_equipment_item(rose_data::EquipmentIndex::SubWeapon)
                .and_then(|item| game_data.items.get_base_item(item.item))
                .map(|item_data| item_data.class);

            let has_required_equipment = skill_data
                .required_equipment_class
                .iter()
                .any(|&required| weapon_class == Some(required) || sub_weapon_class == Some(required));

            if !has_required_equipment {
                return false;
            }
        }
    }

    // Check use ability values (simplified - just check mana)
    for &(use_ability_type, use_ability_value) in skill_data.use_ability.iter() {
        if use_ability_type == rose_data::AbilityType::Mana {
            let use_mana_rate = (100 - ability_values.get_save_mana()) as f32 / 100.0;
            let adjusted_value = (use_ability_value as f32 * use_mana_rate) as i32;
            if let Some(mp) = mana_points {
                if mp.mp < adjusted_value {
                    return false;
                }
            }
        }
    }

    true
}

pub fn score_should_use_buff_skill(
    mut query: Query<(&ShouldUseBuffSkill, &Actor, &mut Score)>,
    query_skill_list: Query<&SkillList, BotQueryFilterAliveNoTarget>,
    query_status_effects: Query<&StatusEffects, BotQueryFilterAliveNoTarget>,
    // Use explicit tuple instead of QueryData type
    query_skill_caster: Query<(
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
    ), BotQueryFilterAliveNoTarget>,
    query_is_using_skill: Query<Option<&UseBuffSkill>, BotQueryFilterAliveNoTarget>,
    game_data: Res<GameData>,
    time: Res<Time>,
) {
    let now = std::time::Instant::now();
    let _ = time.elapsed();

    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        let Ok(is_using_skill) = query_is_using_skill.get(entity) else {
            continue;
        };

        if is_using_skill.is_some() {
            score.set(scorer.score);
            continue;
        }

        let Ok((
            ability_values,
            client_entity,
            health_points,
            move_mode,
            team,
            _clan_membership,
            cooldowns,
            equipment,
            _experience_points,
            _inventory,
            mana_points,
            _party_membership,
            _stamina,
        )) = query_skill_caster.get(entity) else {
            continue;
        };

        let Some(mana_points) = mana_points else {
            continue;
        };

        if (mana_points.mp as f32 / ability_values.get_max_mana() as f32) < 0.25 {
            continue;
        }

        let Ok(skill_list) = query_skill_list.get(entity) else {
            continue;
        };

        let Ok(status_effects) = query_status_effects.get(entity) else {
            continue;
        };

        let Some(active_skill_page) = skill_list.pages.get(1) else {
            continue;
        };

        for skill_data in active_skill_page
            .skills
            .iter()
            .filter_map(|skill_slot| skill_slot.as_ref())
            .filter_map(|skill_id| game_data.skills.get_skill(*skill_id))
        {
            if (skill_data.status_effects[0].is_none() && skill_data.status_effects[1].is_none())
                || !matches!(
                    skill_data.skill_type,
                    SkillType::SelfBoundDuration
                        | SkillType::SelfStateDuration
                        | SkillType::TargetBoundDuration
                        | SkillType::TargetStateDuration
                        | SkillType::SelfBound
                        | SkillType::TargetBound
                )
                || !matches!(
                    skill_data.target_filter,
                    SkillTargetFilter::OnlySelf
                        | SkillTargetFilter::Allied
                        | SkillTargetFilter::Group
                )
            {
                // Only looking for buffs which can apply to self
                continue;
            }

            let already_has_status_effect = skill_data
                .status_effects
                .iter()
                .filter_map(|x| *x)
                .filter_map(|status_effect_id| {
                    game_data.status_effects.get_status_effect(status_effect_id)
                })
                .any(|status_effect_data| {
                    status_effects
                        .get_status_effect_value(status_effect_data.status_effect_type)
                        .is_some()
                });

            if already_has_status_effect {
                continue;
            }

            if skill_can_use_inline(
                now,
                &game_data,
                client_entity,
                ability_values,
                health_points,
                move_mode,
                team,
                cooldowns,
                equipment,
                Some(mana_points),
                skill_data,
            ) {
                score.set(scorer.score);
                break;
            }
        }
    }
}

pub fn action_use_buff_skill(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<UseBuffSkill>>,
    query_skill_list: Query<&SkillList, BotQueryFilterAlive>,
    query_status_effects: Query<&StatusEffects, BotQueryFilterAlive>,
    // Use explicit tuple instead of QueryData type
    query_skill_caster: Query<(
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
    ), BotQueryFilterAlive>,
    query_command: Query<(&Command, &NextCommand)>,
    game_data: Res<GameData>,
    time: Res<Time>,
) {
    let now = std::time::Instant::now();
    let _ = time.elapsed();

    for (&Actor(entity), mut state) in query.iter_mut() {
        match *state {
            ActionState::Requested => {
                let Ok(skill_list) = query_skill_list.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                let Ok(status_effects) = query_status_effects.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                let Ok((
                    ability_values,
                    client_entity,
                    health_points,
                    move_mode,
                    team,
                    _clan_membership,
                    cooldowns,
                    equipment,
                    _experience_points,
                    _inventory,
                    mana_points,
                    _party_membership,
                    _stamina,
                )) = query_skill_caster.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                let Some(active_skill_page) = skill_list.pages.get(1) else {
                    *state = ActionState::Failure;
                    continue;
                };

                *state = ActionState::Failure;

                for skill_data in active_skill_page
                    .skills
                    .iter()
                    .filter_map(|skill_slot| skill_slot.as_ref())
                    .filter_map(|skill_id| game_data.skills.get_skill(*skill_id))
                {
                    if (skill_data.status_effects[0].is_none()
                        && skill_data.status_effects[1].is_none())
                        || !matches!(
                            skill_data.skill_type,
                            SkillType::SelfBoundDuration
                                | SkillType::SelfStateDuration
                                | SkillType::TargetBoundDuration
                                | SkillType::TargetStateDuration
                                | SkillType::SelfBound
                                | SkillType::TargetBound
                        )
                        || !matches!(
                            skill_data.target_filter,
                            SkillTargetFilter::OnlySelf
                                | SkillTargetFilter::Allied
                                | SkillTargetFilter::Group
                        )
                    {
                        // Only looking for buffs which can apply to self
                        continue;
                    }

                    let already_has_status_effect = skill_data
                        .status_effects
                        .iter()
                        .filter_map(|x| *x)
                        .filter_map(|status_effect_id| {
                            game_data.status_effects.get_status_effect(status_effect_id)
                        })
                        .any(|status_effect_data| {
                            status_effects
                                .get_status_effect_value(status_effect_data.status_effect_type)
                                .is_some()
                        });

                    if already_has_status_effect {
                        continue;
                    }

                    if skill_can_use_inline(
                        now,
                        &game_data,
                        client_entity,
                        ability_values,
                        health_points,
                        move_mode,
                        team,
                        cooldowns,
                        equipment,
                        mana_points,
                        skill_data,
                    ) {
                        commands
                            .entity(entity)
                            .insert(NextCommand::with_cast_skill_target_self(
                                skill_data.id,
                                None,
                            ));
                        *state = ActionState::Executing;
                        break;
                    }
                }
            }
            ActionState::Executing => {
                let Ok((command, next_command)) = query_command.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                // Wait until we are not casting any skills
                if !matches!(command.command, CommandData::CastSkill { .. })
                    && !matches!(next_command.command, Some(CommandData::CastSkill { .. }))
                {
                    *state = ActionState::Success;
                }
            }
            ActionState::Cancelled => {
                *state = ActionState::Failure;
            }
            _ => {}
        }
    }
}
