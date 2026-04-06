//! LLM Context Query System
//!
//! This module provides the system that queries the ECS world to build context
//! for LLM decision-making. It gathers bot state, nearby entities, and other
//! relevant information.

use std::sync::Arc;

use bevy::prelude::*;
use uuid::Uuid;

use rose_data::{ItemDatabase, NpcDatabase, SkillDatabase, ZoneDatabase};

use super::context_builder::{
    AssignedPlayerContext, LlmContext, LlmContextBuilder, NearbyEntityContext, NearbyItemContext,
    NearbyPlayerContext, SkillContext,
};
use super::event_types::LlmEvent;
use crate::game::components::{
    AbilityValues, CharacterInfo, ClientEntity, Dead, HealthPoints, ItemDrop, Level, LlmBuddyBot,
    ManaPoints, Npc, Position, SkillList,
};

/// Maximum distance to consider entities as "nearby" for context building.
/// Increased by 10x to allow bots to detect entities further away.
const NEARBY_ENTITY_DISTANCE: f32 = 10000.0;

/// System that gathers context for all LLM buddy bots.
///
/// This system queries the ECS world to build comprehensive context for each bot.
/// The context is stored in a resource for use by the feedback system.
#[cfg(feature = "llm-feedback")]
pub fn gather_bot_context_system(
    _bot_query: Query<&LlmBuddyBot>,
) {
    // This system is designed to be called on-demand rather than every frame
    // The actual context building happens in build_bot_context function
}

/// Builds context for a specific bot using pre-queried data.
///
/// This function is designed to work with Query parameters rather than World
/// directly, as Bevy's World doesn't support direct iteration in the same way.
///
/// # Arguments
///
/// * `bot_id` - The UUID of the bot to build context for
/// * `events` - Recent events for this bot
/// * `bot_data` - Pre-queried bot data (entity, components)
/// * `nearby_monsters` - Pre-queried nearby monster data
/// * `nearby_players` - Pre-queried nearby player data
/// * `nearby_items` - Pre-queried nearby item data
///
/// # Returns
///
/// The built LlmContext.
#[cfg(feature = "llm-feedback")]
pub fn build_bot_context_from_data(
    bot_id: Uuid,
    events: &[LlmEvent],
    bot_data: BotData,
    nearby_monsters: Vec<NearbyEntityContext>,
    nearby_players: Vec<NearbyPlayerContext>,
    nearby_items: Vec<NearbyItemContext>,
    skills: Vec<SkillContext>,
) -> LlmContext {
    // Build the context using the builder pattern
    // Use actual bot name from bot_data instead of placeholder
    LlmContextBuilder::new(bot_id)
        .with_bot_info(bot_data.name, bot_data.level)
        .with_vitals(bot_data.health_percent, bot_data.mana_percent)
        .with_position(bot_data.position.0, bot_data.position.1, bot_data.position.2)
        .with_zone(bot_data.zone_name)
        .with_behavior_mode(bot_data.behavior_mode)
        .with_combat_status(bot_data.is_in_combat)
        .with_dead_status(bot_data.is_dead)
        .with_sitting_status(false)
        .with_assigned_player(bot_data.assigned_player)
        .with_nearby_monsters(nearby_monsters)
        .with_nearby_players(nearby_players)
        .with_nearby_items(nearby_items)
        .with_skills(skills)
        .with_llm_events(events)
        .build()
}

/// Pre-extracted bot data for context building.
#[derive(Debug, Clone)]
pub struct BotData {
    pub name: String,
    pub level: u16,
    pub health_percent: u8,
    pub mana_percent: u8,
    pub position: (f32, f32, f32),
    pub zone_name: String,
    pub behavior_mode: String,
    pub is_in_combat: bool,
    pub is_dead: bool,
    pub assigned_player: Option<AssignedPlayerContext>,
}

/// Extracts bot data from query results.
#[cfg(feature = "llm-feedback")]
pub fn extract_bot_data(
    bot_entity: Entity,
    bot: &LlmBuddyBot,
    character_info: Option<&CharacterInfo>,
    position: &Position,
    hp: &HealthPoints,
    mp: &ManaPoints,
    level: &Level,
    ability_values: Option<&AbilityValues>,
    zone_database: &Arc<ZoneDatabase>,
    is_dead: bool,
    command: Option<&crate::game::components::Command>,
) -> BotData {
    // Get max HP/MP from ability values if available, otherwise use current as max
    let max_hp = ability_values.map(|av| av.max_health as f32).unwrap_or(hp.hp as f32).max(1.0);
    let max_mp = ability_values.map(|av| av.max_mana as f32).unwrap_or(mp.mp as f32).max(1.0);

    let health_percent = ((hp.hp as f32 / max_hp) * 100.0).min(100.0) as u8;
    let mana_percent = ((mp.mp as f32 / max_mp) * 100.0).min(100.0) as u8;

    // Get bot name from CharacterInfo component
    let name = character_info
        .map(|ci| ci.name.clone())
        .unwrap_or_else(|| format!("Bot_{}", bot.id));

    // Get zone name from ZoneDatabase
    let zone_name = zone_database
        .get_zone(position.zone_id)
        .map(|z| z.name.clone())
        .unwrap_or_else(|| format!("Zone_{}", position.zone_id.0));

    // Determine combat status based on current command
    // Bot is in combat if it has an Attack command or is casting a skill on an enemy
    let is_in_combat = command.map_or(false, |cmd| {
        matches!(cmd.command, crate::game::components::CommandData::Attack { .. }) ||
        matches!(cmd.command, crate::game::components::CommandData::CastSkill {
            skill_target: Some(crate::game::components::CommandCastSkillTarget::Entity(_)),
            ..
        })
    });

    BotData {
        name,
        level: level.level as u16,
        health_percent,
        mana_percent,
        position: (position.position.x, position.position.y, position.position.z),
        zone_name,
        behavior_mode: format!("{:?}", bot.behavior_mode).to_lowercase(),
        is_in_combat,
        is_dead,
        assigned_player: None, // Will be set separately via find_assigned_player
    }
}

/// Extracts nearby monster context from NPC query results.
#[cfg(feature = "llm-feedback")]
pub fn extract_nearby_monster(
    _entity: Entity,
    client_entity: &ClientEntity,
    npc: &Npc,
    position: &Position,
    level: Option<&Level>,
    hp: Option<&HealthPoints>,
    ability_values: Option<&AbilityValues>,
    bot_position: &Position,
    npc_database: &Arc<NpcDatabase>,
) -> Option<NearbyEntityContext> {
    let distance = bot_position.position.distance(position.position);
    if distance > NEARBY_ENTITY_DISTANCE {
        return None;
    }

    let health_percent = hp.and_then(|h| {
        let max_hp = ability_values.map(|av| av.max_health as f32).unwrap_or(100.0).max(1.0);
        Some(((h.hp as f32 / max_hp) * 100.0).min(100.0) as u8)
    });

    // Get actual NPC name from database
    let name = npc_database
        .get_npc(npc.id)
        .map(|npc_data| npc_data.name.clone())
        .unwrap_or_else(|| format!("NPC_{}", npc.id.get()));

    Some(NearbyEntityContext {
        entity_id: client_entity.id.0 as u32,
        name,
        level: level.map(|l| l.level as u16).unwrap_or(1),
        distance,
        health_percent,
    })
}

/// Extracts nearby player context from query results.
#[cfg(feature = "llm-feedback")]
pub fn extract_nearby_player(
    _entity: Entity,
    client_entity: &ClientEntity,
    character_info: Option<&CharacterInfo>,
    position: &Position,
    level: Option<&Level>,
    bot_position: &Position,
) -> Option<NearbyPlayerContext> {
    let distance = bot_position.position.distance(position.position);
    if distance > NEARBY_ENTITY_DISTANCE || distance < 0.1 {
        return None;
    }

    // Get actual player name from CharacterInfo component
    let name = character_info
        .map(|ci| ci.name.clone())
        .unwrap_or_else(|| format!("Player_{}", client_entity.id.0));

    Some(NearbyPlayerContext {
        entity_id: client_entity.id.0 as u32,
        name,
        distance,
        level: level.map(|l| l.level as u16).unwrap_or(1),
    })
}

/// Extracts nearby item context from query results.
#[cfg(feature = "llm-feedback")]
pub fn extract_nearby_item(
    _entity: Entity,
    client_entity: &ClientEntity,
    item_drop: &ItemDrop,
    position: &Position,
    bot_position: &Position,
    item_database: &Arc<ItemDatabase>,
) -> Option<NearbyItemContext> {
    let distance = bot_position.position.distance(position.position);
    if distance > NEARBY_ENTITY_DISTANCE {
        return None;
    }

    // Get actual item name from ItemDrop and ItemDatabase
    let name = match item_drop.item.as_ref() {
        Some(dropped_item) => {
            match dropped_item {
                rose_game_common::components::DroppedItem::Item(item) => {
                    // Get item reference from the Item enum
                    let item_ref = item.get_item_reference();
                    // Get item data from database and extract name from BaseItemData
                    item_database.get_item(item_ref).map(|item_data| {
                        match item_data {
                            rose_data::ItemData::Face(data) => data.item_data.name.clone(),
                            rose_data::ItemData::Head(data) => data.item_data.name.clone(),
                            rose_data::ItemData::Body(data) => data.item_data.name.clone(),
                            rose_data::ItemData::Hands(data) => data.item_data.name.clone(),
                            rose_data::ItemData::Feet(data) => data.item_data.name.clone(),
                            rose_data::ItemData::Back(data) => data.item_data.name.clone(),
                            rose_data::ItemData::Jewellery(data) => data.item_data.name.clone(),
                            rose_data::ItemData::Weapon(data) => data.item_data.name.clone(),
                            rose_data::ItemData::SubWeapon(data) => data.item_data.name.clone(),
                            rose_data::ItemData::Consumable(data) => data.item_data.name.clone(),
                            rose_data::ItemData::Gem(data) => data.item_data.name.clone(),
                            rose_data::ItemData::Material(data) => data.item_data.name.clone(),
                            rose_data::ItemData::Quest(data) => data.item_data.name.clone(),
                            rose_data::ItemData::Vehicle(data) => data.item_data.name.clone(),
                        }
                    }).unwrap_or_else(|| "Unknown Item".to_string())
                }
                rose_game_common::components::DroppedItem::Money(money) => {
                    format!("{} Zulie", money.0)
                }
            }
        }
        None => "Unknown Item".to_string(),
    };

    Some(NearbyItemContext {
        entity_id: client_entity.id.0 as u32,
        name,
        distance,
    })
}

/// Extracts skills from a skill list.
#[cfg(feature = "llm-feedback")]
pub fn extract_skills(skill_list: &SkillList, skill_database: &Arc<SkillDatabase>) -> Vec<SkillContext> {
    let mut skills = Vec::new();

    for page in &skill_list.pages {
        for skill_id_opt in &page.skills {
            if let Some(skill_id) = skill_id_opt {
                // Get actual skill name and level from database
                let (name, level) = skill_database
                    .get_skill(*skill_id)
                    .map(|skill_data| (skill_data.name.clone(), skill_data.level as u16))
                    .unwrap_or_else(|| (format!("Skill_{}", skill_id.get()), 1));

                skills.push(SkillContext {
                    skill_id: skill_id.get(),
                    name,
                    level,
                });
            }
        }
    }

    // Limit to reasonable number
    skills.truncate(20);
    skills
}

/// Builds context for a specific bot by querying the ECS world.
///
/// This function gathers all relevant information about a bot and its surroundings
/// to provide the LLM with the context needed for decision-making.
///
/// # Arguments
///
/// * `bot_id` - The UUID of the bot to build context for
/// * `world` - The ECS world to query
/// * `events` - Recent events for this bot
///
/// # Returns
///
/// The built LlmContext, or None if the bot doesn't exist.
///
/// # Note
///
/// This function returns an empty context as a fallback. The primary way to build
/// context is through system queries using the extract_* functions above.
#[cfg(feature = "llm-feedback")]
pub fn build_bot_context(
    bot_id: Uuid,
    _world: &World,
    events: &[LlmEvent],
) -> Option<LlmContext> {
    // This is a fallback implementation that returns a basic context
    // In practice, context should be built using system queries
    log::warn!(
        "Using fallback context builder for bot {}. Consider using system queries instead.",
        bot_id
    );

    Some(
        LlmContextBuilder::new(bot_id)
            .with_bot_info(format!("Bot_{}", bot_id), 1)
            .with_vitals(100, 100)
            .with_position(0.0, 0.0, 0.0)
            .with_zone("Unknown".to_string())
            .with_behavior_mode("defensive".to_string())
            .with_llm_events(events)
            .build(),
    )
}

/// Quick context builder for simple queries.
///
/// This function provides a simplified context for cases where
/// full context building is not needed.
#[cfg(feature = "llm-feedback")]
pub fn build_quick_context(bot_id: Uuid, _world: &World) -> Option<LlmContext> {
    Some(
        LlmContextBuilder::new(bot_id)
            .with_bot_info(format!("Bot_{}", bot_id), 1)
            .with_vitals(100, 100)
            .with_position(0.0, 0.0, 0.0)
            .with_zone("Unknown".to_string())
            .with_behavior_mode("defensive".to_string())
            .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_context() {
        let bot_id = Uuid::new_v4();
        let context = LlmContext::empty(bot_id);

        assert_eq!(context.bot_id, bot_id);
        assert!(context.nearby_monsters.is_empty());
        assert!(context.nearby_items.is_empty());
        assert!(context.nearby_players.is_empty());
    }
}
