use bevy::{
    prelude::{Commands, Component, Entity, Query, Res, With},
    time::Time,
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rose_data::{SkillTargetFilter};

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
pub struct ShouldUseHealSkill {
    pub score: f32,
    pub min_health_percent: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct UseHealSkill;

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
        return true;
    }

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

    if matches!(move_mode, MoveMode::Drive) {
        return false;
    }

    if skill_data.use_ability.iter().any(|(ability_type, _)| {
        *ability_type == rose_data::AbilityType::Mana && mana_points.is_none()
    }) {
        return false;
    }

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

pub fn score_should_use_heal_skill(
    mut query: Query<(&ShouldUseHealSkill, &Actor, &mut Score)>,
    query_skill_list: Query<&SkillList, BotQueryFilterAliveNoTarget>,
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
    query_is_using_skill: Query<Option<&UseHealSkill>, BotQueryFilterAliveNoTarget>,
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

        let current_hp = health_points.hp as f32;
        let max_hp = ability_values.get_max_health() as f32;
        let hp_ratio = current_hp / max_hp;

        if hp_ratio >= scorer.min_health_percent {
            continue;
        }

        let Ok(skill_list) = query_skill_list.get(entity) else {
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
            if !matches!(
                skill_data.target_filter,
                SkillTargetFilter::OnlySelf
                    | SkillTargetFilter::Allied
                    | SkillTargetFilter::Group
            ) {
                continue;
            }

            // Heuristic: we consider it a heal skill if it's a self/allied skill 
            // and we don't have a better way to identify it. 
            // In a real scenario, we'd check for HP restoration effects.
            // For now, we'll just check if it's a skill that can be used.
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
                let urgency = 1.0 - hp_ratio;
                score.set(scorer.score * urgency);
                break;
            }
        }
    }
}

pub fn action_use_heal_skill(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<UseHealSkill>>,
    query_skill_list: Query<&SkillList, BotQueryFilterAlive>,
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
                    if !matches!(
                        skill_data.target_filter,
                        SkillTargetFilter::OnlySelf
                            | SkillTargetFilter::Allied
                            | SkillTargetFilter::Group
                    ) {
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
