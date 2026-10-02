use bevy::{
    prelude::{Commands, Component, Entity, Query, Res, With, Without},
    time::Time,
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rand::Rng;
use rose_data::{SkillTargetFilter, SkillType};

use crate::game::{
    bundles::skill_use_requirements_met,
    components::{
        AbilityValues, ClientEntity, ClientEntityType, ClanMembership, Command,
        CommandData, Cooldowns, Dead, Equipment, ExperiencePoints, HealthPoints, Inventory,
        ManaPoints, MoveMode, NextCommand, PartyMembership, SkillList, Stamina, Team,
    },
    GameData,
};

use super::{actors_with_skill_in_progress, BotCombatTarget, BotQueryFilterAlive};

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldUseAttackSkill {
    pub score: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct UseAttackSkill;

// Check if skill can target entity - takes all data directly without QueryData.
// Same as command_system's filter (bundles::skill_target_filter_allows) except that
// Allied is limited to our own team, so a chosen target always passes the server check.
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
        SkillTargetFilter::Allied => target_is_alive && caster_team.id == target_team.id,
        SkillTargetFilter::Monster => {
            target_is_alive
                && matches!(target_client_entity.entity_type, ClientEntityType::Monster)
        }
        SkillTargetFilter::Enemy => {
            target_is_alive
                && target_team.id != Team::DEFAULT_NPC_TEAM_ID
                && caster_team.id != target_team.id
        }
        SkillTargetFilter::EnemyCharacter => {
            target_is_alive
                && caster_team.id != target_team.id
                && matches!(target_client_entity.entity_type, ClientEntityType::Character)
        }
        SkillTargetFilter::Character => {
            target_is_alive
                && matches!(target_client_entity.entity_type, ClientEntityType::Character)
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
                && matches!(target_client_entity.entity_type, ClientEntityType::Character)
        }
        SkillTargetFilter::EnemyMonster => {
            target_is_alive
                && caster_team.id != target_team.id
                && matches!(target_client_entity.entity_type, ClientEntityType::Monster)
        }
    }
}

pub fn score_should_use_attack_skill(
    mut query: Query<(&ShouldUseAttackSkill, &Actor, &mut Score)>,
    query_combat_target: Query<&BotCombatTarget, BotQueryFilterAlive>,
    query_skill_list: Query<&SkillList, BotQueryFilterAlive>,
    query_caster: Query<(
        Entity,
        &AbilityValues,
        &ClientEntity,
        &HealthPoints,
        &MoveMode,
        &Team,
        Option<&ManaPoints>,
        Option<&Cooldowns>,
        Option<&Equipment>,
        Option<&ExperiencePoints>,
        Option<&Inventory>,
        Option<&PartyMembership>,
        Option<&ClanMembership>,
        Option<&Stamina>,
    ), BotQueryFilterAlive>,
    query_alive: Query<(), BotQueryFilterAlive>,
    query_skill_actions: Query<(&Actor, &ActionState), With<UseAttackSkill>>,
    query_command: Query<(&Command, &NextCommand)>,
    query_target: Query<(
        Entity,
        &ClientEntity,
        &HealthPoints,
        &Team,
        Option<&ClanMembership>,
        Option<&PartyMembership>,
    )>,
    game_data: Res<GameData>,
    time: Res<Time>,
) {
    let now = std::time::Instant::now();
    let _ = time.elapsed();
    let mut rng = rand::thread_rng();
    let casting_actors = actors_with_skill_in_progress(&query_skill_actions, &query_command);

    for (scorer, &Actor(entity), mut score) in query.iter_mut() {
        score.set(0.0);

        if !query_alive.contains(entity) {
            continue;
        }

        // Keep the score while our cast is in progress so the thinker does
        // not cancel it (and overwrite the queued cast) mid-way.
        if casting_actors.contains(&entity) {
            score.set(scorer.score);
            continue;
        }

        // Get combat target
        let Ok(bot_combat_target) = query_combat_target.get(entity) else {
            continue;
        };

        // Get caster data
        let Ok((caster_entity, caster_ability_values, caster_client_entity, caster_hp,
                caster_move_mode, caster_team, caster_mana, caster_cooldowns, caster_equipment,
                caster_xp, caster_inventory, caster_party, caster_clan, caster_stamina)) =
            query_caster.get(entity) else {
            continue;
        };

        // Check mana
        let Some(mana_points) = caster_mana else {
            continue;
        };
        if (mana_points.mp as f32 / caster_ability_values.get_max_mana() as f32) < 0.5 {
            continue;
        }

        // Get skill list
        let Ok(skill_list) = query_skill_list.get(entity) else {
            continue;
        };
        let Some(active_skill_page) = skill_list.pages.get(1) else {
            continue;
        };

        // Get target data
        let Ok((target_entity, target_client_entity, target_hp, target_team,
                target_clan, target_party)) =
            query_target.get(bot_combat_target.entity) else {
            continue;
        };
        if target_hp.hp < 250 {
            continue;
        }

        if rng.gen_range(0..=100) < 95 {
            continue;
        }

        // Check each skill
        for skill_id in active_skill_page.skills.iter().filter_map(|x| x.as_ref()) {
            let Some(skill_data) = game_data.skills.get_skill(*skill_id) else {
                continue;
            };

            // Same use requirements command_system enforces: a cast it refuses stops the bot.
            if !skill_use_requirements_met(
                now,
                &game_data,
                caster_client_entity,
                caster_ability_values,
                caster_hp,
                caster_move_mode,
                caster_cooldowns,
                caster_equipment,
                caster_xp,
                caster_inventory,
                caster_mana,
                caster_stamina,
                skill_data,
            ) {
                continue;
            }

            // Check target filter
            if !check_skill_target_filter(
                caster_entity,
                caster_team,
                caster_clan,
                caster_party,
                target_entity,
                target_team,
                target_clan,
                target_party,
                target_hp.hp,
                target_client_entity,
                skill_data,
            ) {
                continue;
            }

            score.set(scorer.score);
            break;
        }
    }
}

pub fn action_use_attack_skill(
    mut commands: Commands,
    mut query: Query<(&Actor, &mut ActionState), With<UseAttackSkill>>,
    query_combat_target: Query<&BotCombatTarget, BotQueryFilterAlive>,
    query_skill_list: Query<&SkillList, BotQueryFilterAlive>,
    query_caster: Query<(
        Entity,
        &AbilityValues,
        &ClientEntity,
        &HealthPoints,
        &MoveMode,
        &Team,
        Option<&ManaPoints>,
        Option<&Cooldowns>,
        Option<&Equipment>,
        Option<&ExperiencePoints>,
        Option<&Inventory>,
        Option<&PartyMembership>,
        Option<&ClanMembership>,
        Option<&Stamina>,
    ), BotQueryFilterAlive>,
    query_target: Query<(
        Entity,
        &ClientEntity,
        &HealthPoints,
        &Team,
        Option<&ClanMembership>,
        Option<&PartyMembership>,
    )>,
    query_command: Query<(&Command, &NextCommand)>,
    game_data: Res<GameData>,
    time: Res<Time>,
) {
    let now = std::time::Instant::now();
    let _ = time.elapsed();

    for (&Actor(entity), mut state) in query.iter_mut() {
        match *state {
            ActionState::Requested => {
                // Get combat target
                let Ok(bot_combat_target) = query_combat_target.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                // Get caster data
                let Ok((caster_entity, caster_ability_values, caster_client_entity, caster_hp,
                        caster_move_mode, caster_team, caster_mana, caster_cooldowns,
                        caster_equipment, caster_xp, caster_inventory, caster_party,
                        caster_clan, caster_stamina)) =
                    query_caster.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                // Get skill list
                let Ok(skill_list) = query_skill_list.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };
                let Some(active_skill_page) = skill_list.pages.get(1) else {
                    *state = ActionState::Failure;
                    continue;
                };

                // Get target data
                let Ok((target_entity, target_client_entity, target_hp, target_team,
                        target_clan, target_party)) =
                    query_target.get(bot_combat_target.entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                *state = ActionState::Failure;

                // Check each skill
                for skill_id in active_skill_page.skills.iter().filter_map(|x| x.as_ref()) {
                    let Some(skill_data) = game_data.skills.get_skill(*skill_id) else {
                        continue;
                    };

                    // Same use requirements command_system enforces: a cast it refuses
                    // stops the bot.
                    if !skill_use_requirements_met(
                        now,
                        &game_data,
                        caster_client_entity,
                        caster_ability_values,
                        caster_hp,
                        caster_move_mode,
                        caster_cooldowns,
                        caster_equipment,
                        caster_xp,
                        caster_inventory,
                        caster_mana,
                        caster_stamina,
                        skill_data,
                    ) {
                        continue;
                    }

                    // Check target filter
                    if !check_skill_target_filter(
                        caster_entity,
                        caster_team,
                        caster_clan,
                        caster_party,
                        target_entity,
                        target_team,
                        target_clan,
                        target_party,
                        target_hp.hp,
                        target_client_entity,
                        skill_data,
                    ) {
                        continue;
                    }

                    commands.entity(entity).insert(
                        NextCommand::with_cast_skill_target_entity(
                            *skill_id,
                            bot_combat_target.entity,
                            None,
                        ),
                    );
                    *state = ActionState::Executing;
                    break;
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
