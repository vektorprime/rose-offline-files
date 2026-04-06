use std::time::{Duration, Instant};

use bevy::{ecs::query::QueryData, prelude::Entity};
use rose_data::{
    AbilityType, EquipmentIndex, SkillCooldown, SkillData, SkillTargetFilter, SkillType,
    StatusEffectType, VehiclePartIndex,
};

use crate::game::{
    components::{
        AbilityValues, ClanMembership, ClientEntity, ClientEntityType, Cooldowns, Equipment,
        ExperiencePoints, HealthPoints, Inventory, LlmBuddyBot, ManaPoints, MoveMode, PartyMembership, Stamina,
        StatusEffects, SummonPoints, Team, Weight,
    },
    GameData,
};

pub const GLOBAL_SKILL_COOLDOWN: Duration = Duration::from_millis(250);

#[derive(QueryData)]
pub struct SkillCasterBundle<'w> {
    pub entity: Entity,

    pub ability_values: &'w AbilityValues,
    pub client_entity: &'w ClientEntity,
    pub health_points: &'w HealthPoints,
    pub move_mode: &'w MoveMode,
    pub team: &'w Team,

    pub clan_membership: Option<&'w ClanMembership>,
    pub cooldowns: Option<&'w Cooldowns>,
    pub equipment: Option<&'w Equipment>,
    pub experience_points: Option<&'w ExperiencePoints>,
    pub inventory: Option<&'w Inventory>, // Only for Money
    pub llm_buddy_bot: Option<&'w LlmBuddyBot>,
    pub mana_points: Option<&'w ManaPoints>,
    pub party_membership: Option<&'w PartyMembership>,
    pub stamina: Option<&'w Stamina>,
    pub status_effects: Option<&'w StatusEffects>,
    pub summon_points: Option<&'w SummonPoints>,
    pub weight: Option<&'w Weight>,
}

#[derive(QueryData)]
pub struct SkillTargetBundle<'w> {
    pub entity: Entity,

    pub client_entity: &'w ClientEntity,
    pub health_points: &'w HealthPoints,
    pub team: &'w Team,

    pub clan_membership: Option<&'w ClanMembership>,
    pub party_membership: Option<&'w PartyMembership>,
}

fn check_skill_cooldown(
    skill_caster: &SkillCasterBundleItem,
    now: Instant,
    skill_data: &SkillData,
) -> bool {
    let Some(cooldowns) = skill_caster.cooldowns else {
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

fn check_not_disabled(skill_caster: &SkillCasterBundleItem) -> bool {
    // Check if the caster is disabled by status effects (stun, sleep, faint, mute)
    let Some(status_effects) = skill_caster.status_effects else {
        // No status effects component means not disabled
        return true;
    };

    // These status effects prevent skill usage:
    // - Sleep: Character is asleep and cannot act
    // - Fainting: Character has fainted and cannot act
    // - Dumb (Mute/Silence): Character cannot use skills
    let is_disabled = status_effects.active[StatusEffectType::Sleep].is_some()
        || status_effects.active[StatusEffectType::Fainting].is_some()
        || status_effects.active[StatusEffectType::Dumb].is_some();

    !is_disabled
}

fn check_weight(skill_caster: &SkillCasterBundleItem) -> bool {
    // Check if entity is over 110% weight capacity
    // At 110%+ weight, skills cannot be used
    let Some(weight) = skill_caster.weight else {
        // No weight component means no weight restriction
        return true;
    };

    let max_weight = skill_caster.ability_values.max_weight;

    // Allow skills if at or below 110% capacity
    // weight.weight is u32, max_weight is i32 - need to compare carefully
    let current_weight = weight.weight as f32;
    let weight_limit = max_weight as f32 * 1.1;

    current_weight <= weight_limit
}

fn check_move_mode(skill_caster: &SkillCasterBundleItem, _skill_data: &SkillData) -> bool {
    !matches!(skill_caster.move_mode, MoveMode::Drive)
}

pub fn check_skill_target_filter(
    skill_caster: &SkillCasterBundleItem,
    skill_target: &SkillTargetBundleItem,
    skill_data: &SkillData,
) -> bool {
    let target_is_alive = skill_target.health_points.hp > 0;
    let target_is_caster = skill_caster.entity == skill_target.entity;

    match skill_data.target_filter {
        SkillTargetFilter::OnlySelf => target_is_alive && target_is_caster,
        SkillTargetFilter::Group => {
            let caster_party = skill_caster
                .party_membership
                .and_then(|party_membership: &PartyMembership| party_membership.party);
            let target_party = skill_target
                .party_membership
                .and_then(|party_membership: &PartyMembership| party_membership.party);
            target_is_alive
                && (target_is_caster || (caster_party.is_some() && caster_party == target_party))
        }
        SkillTargetFilter::Guild => {
            let caster_clan = skill_caster
                .clan_membership
                .and_then(|clan_membership: &ClanMembership| clan_membership.clan());
            let target_clan = skill_target
                .clan_membership
                .and_then(|clan_membership: &ClanMembership| clan_membership.clan());
            target_is_alive
                && (target_is_caster || (caster_clan.is_some() && caster_clan == target_clan))
        }
        SkillTargetFilter::Allied => {
            target_is_alive && skill_caster.team.id == skill_target.team.id
        }
        SkillTargetFilter::Monster => {
            target_is_alive
                && matches!(
                    skill_target.client_entity.entity_type,
                    ClientEntityType::Monster
                )
        }
        SkillTargetFilter::Enemy => {
            target_is_alive
                && skill_target.team.id != Team::DEFAULT_NPC_TEAM_ID
                && skill_caster.team.id != skill_target.team.id
        }
        SkillTargetFilter::EnemyCharacter => {
            target_is_alive
                && skill_caster.team.id != skill_target.team.id
                && matches!(
                    skill_target.client_entity.entity_type,
                    ClientEntityType::Character
                )
        }
        SkillTargetFilter::Character => {
            target_is_alive
                && matches!(
                    skill_target.client_entity.entity_type,
                    ClientEntityType::Character
                )
        }
        SkillTargetFilter::CharacterOrMonster => {
            target_is_alive
                && matches!(
                    skill_target.client_entity.entity_type,
                    ClientEntityType::Character | ClientEntityType::Monster
                )
        }
        SkillTargetFilter::DeadAlliedCharacter => {
            !target_is_alive
                && !target_is_caster
                && skill_caster.team.id == skill_target.team.id
                && matches!(
                    skill_target.client_entity.entity_type,
                    ClientEntityType::Character
                )
        }
        SkillTargetFilter::EnemyMonster => {
            target_is_alive
                && skill_caster.team.id != skill_target.team.id
                && matches!(
                    skill_target.client_entity.entity_type,
                    ClientEntityType::Monster
                )
        }
    }
}

fn check_summon_points(
    game_data: &GameData,
    skill_caster: &SkillCasterBundleItem,
    skill_data: &SkillData,
) -> bool {
    if matches!(skill_data.skill_type, SkillType::SummonPet) {
        let summon_point_requirement = skill_data
            .summon_npc_id
            .and_then(|npc_id| game_data.npcs.get_npc(npc_id))
            .map_or(0, |npc_data| npc_data.summon_point_requirement);

        if summon_point_requirement > 0 {
            // Check if caster has enough summon points
            let available_points = skill_caster
                .summon_points
                .map_or(0, |sp| sp.points);

            // Return false if not enough summon points
            if available_points < summon_point_requirement as u32 {
                return false;
            }
        }
    }

    true
}

fn check_use_ability_value(skill_caster: &SkillCasterBundleItem, skill_data: &SkillData) -> bool {
    // Bots have unlimited mana - skip mana checks for LLM buddy bots
    let is_bot = skill_caster.llm_buddy_bot.is_some();
    
    for &(use_ability_type, mut use_ability_value) in skill_data.use_ability.iter() {
        if use_ability_type == AbilityType::Mana {
            // Bots have unlimited mana
            if is_bot {
                continue;
            }
            let use_mana_rate = (100 - skill_caster.ability_values.get_save_mana()) as f32 / 100.0;
            use_ability_value = (use_ability_value as f32 * use_mana_rate) as i32;
        }

        let ability_value = match use_ability_type {
            AbilityType::Level => skill_caster.ability_values.level,
            AbilityType::Strength => skill_caster.ability_values.strength,
            AbilityType::Dexterity => skill_caster.ability_values.dexterity,
            AbilityType::Intelligence => skill_caster.ability_values.intelligence,
            AbilityType::Concentration => skill_caster.ability_values.concentration,
            AbilityType::Charm => skill_caster.ability_values.charm,
            AbilityType::Sense => skill_caster.ability_values.sense,
            AbilityType::Health => skill_caster.health_points.hp,
            AbilityType::Mana => skill_caster
                .mana_points
                .map_or(0, |mana_points: &ManaPoints| mana_points.mp),
            AbilityType::Experience => skill_caster
                .experience_points
                .map_or(0, |experience_points: &ExperiencePoints| {
                    experience_points.xp
                })
                .try_into()
                .unwrap_or(i32::MAX),
            AbilityType::Money => skill_caster
                .inventory
                .map_or(0, |inventory: &Inventory| inventory.money.0)
                .try_into()
                .unwrap_or(i32::MAX),
            AbilityType::Stamina => skill_caster
                .stamina
                .map_or(0, |stamina: &Stamina| stamina.stamina)
                .try_into()
                .unwrap_or(i32::MAX),
            AbilityType::Fuel => skill_caster.equipment.map_or(0, |equipment: &Equipment| {
                equipment
                    .get_vehicle_item(VehiclePartIndex::Engine)
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
    skill_caster: &SkillCasterBundleItem,
    skill_data: &SkillData,
) -> bool {
    let Some(equipment) = skill_caster.equipment else {
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

pub fn skill_can_use(
    now: Instant,
    game_data: &GameData,
    skill_caster: &SkillCasterBundleItem,
    skill_data: &SkillData,
) -> bool {
    if !skill_caster.client_entity.is_character() {
        // We only check use requirements for characters
        return true;
    }

    if !check_skill_cooldown(skill_caster, now, skill_data) {
        return false;
    }

    if !check_not_disabled(skill_caster) {
        return false;
    }

    if !check_weight(skill_caster) {
        return false;
    }

    if !check_move_mode(skill_caster, skill_data) {
        return false;
    }

    if !check_summon_points(game_data, skill_caster, skill_data) {
        return false;
    }

    if !check_use_ability_value(skill_caster, skill_data) {
        return false;
    }

    if !check_equipment(game_data, skill_caster, skill_data) {
        return false;
    }

    true
}

pub fn skill_can_target_entity(
    skill_caster: &SkillCasterBundleItem,
    skill_target: &SkillTargetBundleItem,
    skill_data: &SkillData,
) -> bool {
    if !check_skill_target_filter(skill_caster, skill_target, skill_data) {
        return false;
    }

    true
}

pub fn skill_can_target_self(
    skill_caster: &SkillCasterBundleItem,
    skill_data: &SkillData,
) -> bool {
    if !check_skill_target_filter(
        skill_caster,
        &SkillTargetBundleItem {
            entity: skill_caster.entity,
            client_entity: skill_caster.client_entity,
            health_points: skill_caster.health_points,
            clan_membership: skill_caster.clan_membership,
            party_membership: skill_caster.party_membership,
            team: skill_caster.team,
        },
        skill_data,
    ) {
        return false;
    }

    true
}

pub fn skill_can_target_position(skill_data: &SkillData) -> bool {
    matches!(skill_data.skill_type, SkillType::AreaTarget)
}

pub fn skill_use_ability_value(
    skill_caster: &SkillCasterBundleItem,
    skill_data: &SkillData,
) -> bool {
    for &(use_ability_type, mut use_ability_value) in skill_data.use_ability.iter() {
        if use_ability_type == AbilityType::Mana {
            let use_mana_rate = (100 - skill_caster.ability_values.get_save_mana()) as f32 / 100.0;
            use_ability_value = (use_ability_value as f32 * use_mana_rate) as i32;
        }

        let ability_value = match use_ability_type {
            AbilityType::Level => skill_caster.ability_values.level,
            AbilityType::Strength => skill_caster.ability_values.strength,
            AbilityType::Dexterity => skill_caster.ability_values.dexterity,
            AbilityType::Intelligence => skill_caster.ability_values.intelligence,
            AbilityType::Concentration => skill_caster.ability_values.concentration,
            AbilityType::Charm => skill_caster.ability_values.charm,
            AbilityType::Sense => skill_caster.ability_values.sense,
            AbilityType::Health => skill_caster.health_points.hp,
            AbilityType::Mana => skill_caster
                .mana_points
                .map_or(0, |mana_points: &ManaPoints| mana_points.mp),
            AbilityType::Experience => skill_caster
                .experience_points
                .map_or(0, |experience_points: &ExperiencePoints| {
                    experience_points.xp
                })
                .try_into()
                .unwrap_or(i32::MAX),
            AbilityType::Money => skill_caster
                .inventory
                .map_or(0, |inventory: &Inventory| inventory.money.0)
                .try_into()
                .unwrap_or(i32::MAX),
            AbilityType::Stamina => skill_caster
                .stamina
                .map_or(0, |stamina: &Stamina| stamina.stamina)
                .try_into()
                .unwrap_or(i32::MAX),
            AbilityType::Fuel => skill_caster.equipment.map_or(0, |equipment: &Equipment| {
                equipment
                    .get_vehicle_item(VehiclePartIndex::Engine)
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
