use bevy::{
    prelude::{Commands, Component, Entity, Query, Res, With},
    time::Time,
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rose_data::{SkillTargetFilter, SkillType};

use crate::game::{
    bundles::skill_use_requirements_met,
    components::{
        AbilityValues, ClanMembership, ClientEntity, Command, CommandData, Cooldowns, Equipment,
        ExperiencePoints, HealthPoints, Inventory, ManaPoints, MoveMode, NextCommand,
        PartyMembership, SkillList, Stamina, StatusEffects, Team,
    },
    GameData,
};

use super::{actors_with_skill_in_progress, BotQueryFilterAlive, BotQueryFilterAliveNoTarget};

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldUseBuffSkill {
    pub score: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct UseBuffSkill;

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
    query_alive_no_target: Query<(), BotQueryFilterAliveNoTarget>,
    query_skill_actions: Query<(&Actor, &ActionState), With<UseBuffSkill>>,
    query_command: Query<(&Command, &NextCommand)>,
    game_data: Res<GameData>,
    time: Res<Time>,
) {
    let now = std::time::Instant::now();
    let _ = time.elapsed();
    let casting_actors = actors_with_skill_in_progress(&query_skill_actions, &query_command);

    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        if !query_alive_no_target.contains(entity) {
            continue;
        }

        // Keep the score while our cast is in progress so the thinker does
        // not cancel it (and overwrite the queued cast) mid-way.
        if casting_actors.contains(&entity) {
            score.set(scorer.score);
            continue;
        }

        let Ok((
            ability_values,
            client_entity,
            health_points,
            move_mode,
            _team,
            _clan_membership,
            cooldowns,
            equipment,
            experience_points,
            inventory,
            mana_points,
            _party_membership,
            stamina,
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

            // Same use requirements command_system enforces: a cast it refuses stops the bot.
            if skill_use_requirements_met(
                now,
                &game_data,
                client_entity,
                ability_values,
                health_points,
                move_mode,
                cooldowns,
                equipment,
                experience_points,
                inventory,
                Some(mana_points),
                stamina,
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
                    _team,
                    _clan_membership,
                    cooldowns,
                    equipment,
                    experience_points,
                    inventory,
                    mana_points,
                    _party_membership,
                    stamina,
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

                    // Same use requirements command_system enforces: a cast it refuses
                    // stops the bot.
                    if skill_use_requirements_met(
                        now,
                        &game_data,
                        client_entity,
                        ability_values,
                        health_points,
                        move_mode,
                        cooldowns,
                        equipment,
                        experience_points,
                        inventory,
                        mana_points,
                        stamina,
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
