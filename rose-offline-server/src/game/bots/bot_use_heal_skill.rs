use bevy::{
    prelude::{Commands, Component, Entity, Query, Res, With},
    time::Time,
};
use big_brain::prelude::{ActionBuilder, ActionState, Actor, Score, ScorerBuilder};
use rose_data::{SkillTargetFilter};

use crate::game::{
    bundles::{skill_target_filter_allows, skill_use_requirements_met},
    components::{
        AbilityValues, ClanMembership, ClientEntity, Command, CommandData, Cooldowns, Equipment,
        ExperiencePoints, HealthPoints, Inventory, ManaPoints, MoveMode, NextCommand,
        Party, PartyMembership, SkillList, Stamina, StatusEffects, Team,
    },
    GameData,
};

use super::{actors_with_skill_in_progress, BotQueryFilterAlive, BotQueryFilterAliveNoTarget};

#[derive(Clone, Component, Debug, ScorerBuilder)]
pub struct ShouldUseHealSkill {
    pub score: f32,
    pub min_health_percent: f32,
    pub party_min_health_percent: f32,
}

#[derive(Debug, Clone, Component, ActionBuilder)]
pub struct UseHealSkill {
    pub min_health_percent: f32,
    pub party_min_health_percent: f32,
}

/// Whether `skill_data` can heal `heal_target` (`None` = ourself). Self-only skills
/// only heal ourselves; a party member must also pass the target filter
/// command_system checks, since a cast it refuses stops the bot.
#[allow(clippy::too_many_arguments)]
fn heal_skill_can_target(
    caster_entity: Entity,
    caster_team: &Team,
    caster_clan: Option<&ClanMembership>,
    caster_party: Option<&PartyMembership>,
    heal_target: Option<Entity>,
    skill_data: &rose_data::SkillData,
    query_heal_target: &Query<(
        &ClientEntity,
        &HealthPoints,
        &Team,
        Option<&ClanMembership>,
        Option<&PartyMembership>,
    )>,
) -> bool {
    let Some(target_entity) = heal_target else {
        return matches!(
            skill_data.target_filter,
            SkillTargetFilter::OnlySelf | SkillTargetFilter::Allied | SkillTargetFilter::Group
        );
    };

    if !matches!(
        skill_data.target_filter,
        SkillTargetFilter::Allied | SkillTargetFilter::Group
    ) {
        return false;
    }

    query_heal_target.get(target_entity).is_ok_and(
        |(target_client_entity, target_health_points, target_team, target_clan, target_party)| {
            skill_target_filter_allows(
                caster_entity,
                caster_team,
                caster_clan,
                caster_party,
                target_entity,
                target_team,
                target_clan,
                target_party,
                target_health_points.hp,
                target_client_entity,
                skill_data,
            )
        },
    )
}

/// Find the heal target: ourself if below `min_health_percent`, otherwise the
/// lowest health party member below `party_min_health_percent`.
/// Returns `(hp_ratio, target_entity)` where `target_entity` is `None` for self.
fn find_heal_target(
    entity: Entity,
    health_points: &HealthPoints,
    ability_values: &AbilityValues,
    min_health_percent: f32,
    party_min_health_percent: f32,
    party_membership: Option<&PartyMembership>,
    query_party: &Query<&Party>,
    query_member_health: &Query<(&HealthPoints, &AbilityValues)>,
) -> Option<(f32, Option<Entity>)> {
    let hp_ratio = health_points.hp as f32 / ability_values.get_max_health() as f32;

    if hp_ratio < min_health_percent {
        return Some((hp_ratio, None));
    }

    let party_entity = party_membership.and_then(|membership| membership.party)?;
    let party = query_party.get(party_entity).ok()?;

    let mut lowest: Option<(f32, Entity)> = None;
    for member in party.members.iter() {
        let Some(member_entity) = member.get_entity() else {
            continue;
        };

        if member_entity == entity {
            continue;
        }

        let Ok((member_health, member_ability)) = query_member_health.get(member_entity) else {
            continue;
        };

        // Heal skills (self/allied/group filters) need a living target, and a corpse at
        // 0% would always win the lowest-health pick over living members.
        if member_health.hp <= 0 {
            continue;
        }

        let member_ratio =
            member_health.hp as f32 / member_ability.get_max_health() as f32;

        if member_ratio < party_min_health_percent {
            if lowest.map_or(true, |(lowest_ratio, _)| member_ratio < lowest_ratio) {
                lowest = Some((member_ratio, member_entity));
            }
        }
    }

    lowest.map(|(ratio, member_entity)| (ratio, Some(member_entity)))
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
    query_party: Query<&Party>,
    query_member_health: Query<(&HealthPoints, &AbilityValues)>,
    query_heal_target: Query<(
        &ClientEntity,
        &HealthPoints,
        &Team,
        Option<&ClanMembership>,
        Option<&PartyMembership>,
    )>,
    query_alive_no_target: Query<(), BotQueryFilterAliveNoTarget>,
    query_skill_actions: Query<(&Actor, &ActionState), With<UseHealSkill>>,
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
            team,
            clan_membership,
            cooldowns,
            equipment,
            experience_points,
            inventory,
            mana_points,
            party_membership,
            stamina,
        )) = query_skill_caster.get(entity) else {
            continue;
        };

        let Some((hp_ratio, heal_target)) = find_heal_target(
            entity,
            health_points,
            ability_values,
            scorer.min_health_percent,
            scorer.party_min_health_percent,
            party_membership,
            &query_party,
            &query_member_health,
        ) else {
            continue;
        };

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
            // Same skill choice as action_use_heal_skill, so a high score always has a
            // cast the action will issue.
            if !heal_skill_can_target(
                entity,
                team,
                clan_membership,
                party_membership,
                heal_target,
                skill_data,
                &query_heal_target,
            ) {
                continue;
            }

            // Heuristic: we consider it a heal skill if it's a self/allied skill
            // and we don't have a better way to identify it.
            // In a real scenario, we'd check for HP restoration effects.
            // For now, we'll just check if it's a skill that can be used.
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
                let urgency = 1.0 - hp_ratio;
                score.set(scorer.score * urgency);
                break;
            }
        }
    }
}

pub fn action_use_heal_skill(
    mut commands: Commands,
    mut query: Query<(&UseHealSkill, &Actor, &mut ActionState)>,
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
    query_party: Query<&Party>,
    query_member_health: Query<(&HealthPoints, &AbilityValues)>,
    query_heal_target: Query<(
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

    for (use_heal_skill, &Actor(entity), mut state) in query.iter_mut() {
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
                    clan_membership,
                    cooldowns,
                    equipment,
                    experience_points,
                    inventory,
                    mana_points,
                    party_membership,
                    stamina,
                )) = query_skill_caster.get(entity) else {
                    *state = ActionState::Failure;
                    continue;
                };

                let Some((_, heal_target)) = find_heal_target(
                    entity,
                    health_points,
                    ability_values,
                    use_heal_skill.min_health_percent,
                    use_heal_skill.party_min_health_percent,
                    party_membership,
                    &query_party,
                    &query_member_health,
                ) else {
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
                    // Self-cast skills only help when healing ourselves, allied
                    // skills can heal either.
                    if !heal_skill_can_target(
                        entity,
                        team,
                        clan_membership,
                        party_membership,
                        heal_target,
                        skill_data,
                        &query_heal_target,
                    ) {
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
                        match heal_target {
                            Some(target_entity) => {
                                commands.entity(entity).insert(
                                    NextCommand::with_cast_skill_target_entity(
                                        skill_data.id,
                                        target_entity,
                                        None,
                                    ),
                                );
                            }
                            None => {
                                commands
                                    .entity(entity)
                                    .insert(NextCommand::with_cast_skill_target_self(
                                        skill_data.id,
                                        None,
                                    ));
                            }
                        }
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
