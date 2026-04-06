//! Context Builder for LLM Bot Decision Making
//!
//! This module provides the context builder that gathers bot state and nearby entities
//! for the LLM to make informed decisions. It queries the ECS world to build a comprehensive
//! context structure.

use serde::Serialize;
use uuid::Uuid;

use super::event_types::LlmEvent;

// ============================================================================
// Context Structures
// ============================================================================

/// Context about the assigned player for the bot.
#[derive(Debug, Clone, Serialize)]
pub struct AssignedPlayerContext {
    /// Name of the assigned player
    pub name: String,
    /// Health percentage (0-100)
    pub health_percent: u8,
    /// Distance from the bot
    pub distance: f32,
    /// Whether the player is in combat
    pub is_in_combat: bool,
}

/// Context about a nearby monster/entity.
#[derive(Debug, Clone, Serialize)]
pub struct NearbyEntityContext {
    /// Entity ID in the game world
    pub entity_id: u32,
    /// Name of the monster/entity
    pub name: String,
    /// Level of the monster
    pub level: u16,
    /// Distance from the bot
    pub distance: f32,
    /// Health percentage (0-100), if known
    pub health_percent: Option<u8>,
}

/// Context about a nearby item on the ground.
#[derive(Debug, Clone, Serialize)]
pub struct NearbyItemContext {
    /// Entity ID in the game world
    pub entity_id: u32,
    /// Name of the item
    pub name: String,
    /// Distance from the bot
    pub distance: f32,
}

/// Context about a nearby player.
#[derive(Debug, Clone, Serialize)]
pub struct NearbyPlayerContext {
    /// Entity ID in the game world
    pub entity_id: u32,
    /// Name of the player
    pub name: String,
    /// Distance from the bot
    pub distance: f32,
    /// Level of the player
    pub level: u16,
}

/// Skill information for the bot context.
#[derive(Debug, Clone, Serialize)]
pub struct SkillContext {
    /// Skill ID
    pub skill_id: u16,
    /// Skill name
    pub name: String,
    /// Skill level
    pub level: u16,
}

/// Complete context for an LLM-controlled bot.
///
/// This structure contains all the information the LLM needs to make
/// decisions about bot actions.
#[derive(Debug, Clone, Serialize)]
pub struct LlmContext {
    /// Unique bot ID
    pub bot_id: Uuid,
    /// Bot character name
    pub bot_name: String,
    /// Bot level
    pub bot_level: u16,
    /// Bot health percentage (0-100)
    pub health_percent: u8,
    /// Bot mana percentage (0-100)
    pub mana_percent: u8,
    /// Bot position (x, y, z)
    pub position: (f32, f32, f32),
    /// Current zone name
    pub zone_name: String,
    /// Current behavior mode
    pub behavior_mode: String,
    /// Whether the bot is currently in combat
    pub is_in_combat: bool,
    /// Whether the bot is dead
    pub is_dead: bool,
    /// Whether the bot is sitting
    pub is_sitting: bool,
    /// Information about the assigned player (if any)
    pub assigned_player: Option<AssignedPlayerContext>,
    /// Nearby monsters/hostile entities
    pub nearby_monsters: Vec<NearbyEntityContext>,
    /// Nearby items on the ground
    pub nearby_items: Vec<NearbyItemContext>,
    /// Nearby players (excluding assigned player)
    pub nearby_players: Vec<NearbyPlayerContext>,
    /// Available skills
    pub available_skills: Vec<SkillContext>,
    /// Recent events relevant to the bot
    pub recent_events: Vec<String>,
}

impl LlmContext {
    /// Creates a new empty context with the given bot ID.
    pub fn empty(bot_id: Uuid) -> Self {
        Self {
            bot_id,
            bot_name: String::new(),
            bot_level: 1,
            health_percent: 100,
            mana_percent: 100,
            position: (0.0, 0.0, 0.0),
            zone_name: String::new(),
            behavior_mode: "defensive".to_string(),
            is_in_combat: false,
            is_dead: false,
            is_sitting: false,
            assigned_player: None,
            nearby_monsters: Vec::new(),
            nearby_items: Vec::new(),
            nearby_players: Vec::new(),
            available_skills: Vec::new(),
            recent_events: Vec::new(),
        }
    }

    /// Formats the context as a human-readable summary for LLM consumption.
    ///
    /// This provides a concise text representation of the bot's current state
    /// and surroundings, suitable for inclusion in LLM prompts.
    pub fn format_context_summary(&self) -> String {
        let mut summary = String::new();

        // Bot status
        summary.push_str(&format!(
            "## Your Status\n\
             - Name: {} (Level {})\n\
             - Health: {}%, Mana: {}%\n\
             - Position: ({:.1}, {:.1}, {:.1}) in {}\n\
             - Behavior Mode: {}\n\
             - Combat: {}\n\
             - State: {}\n",
            self.bot_name,
            self.bot_level,
            self.health_percent,
            self.mana_percent,
            self.position.0,
            self.position.1,
            self.position.2,
            self.zone_name,
            self.behavior_mode,
            if self.is_in_combat { "In combat" } else { "Not in combat" },
            if self.is_dead {
                "Dead"
            } else if self.is_sitting {
                "Sitting"
            } else {
                "Standing"
            }
        ));

        // Assigned player
        if let Some(ref player) = self.assigned_player {
            summary.push_str(&format!(
                "\n## Assigned Player\n\
                 - Name: {}\n\
                 - Health: {}%\n\
                 - Distance: {:.1}\n\
                 - Combat: {}\n",
                player.name,
                player.health_percent,
                player.distance,
                if player.is_in_combat { "In combat" } else { "Not in combat" }
            ));
        }

        // Nearby monsters
        if !self.nearby_monsters.is_empty() {
            summary.push_str("\n## Nearby Monsters\n");
            for monster in &self.nearby_monsters {
                let hp_info = monster
                    .health_percent
                    .map(|h| format!(" HP: {}%", h))
                    .unwrap_or_default();
                summary.push_str(&format!(
                    "- {} (Lv.{}) at distance {:.1} [ID: {}]{}\n",
                    monster.name, monster.level, monster.distance, monster.entity_id, hp_info
                ));
            }
        }

        // Nearby items
        if !self.nearby_items.is_empty() {
            summary.push_str("\n## Nearby Items\n");
            for item in &self.nearby_items {
                summary.push_str(&format!(
                    "- {} at distance {:.1} [ID: {}]\n",
                    item.name, item.distance, item.entity_id
                ));
            }
        }

        // Nearby players
        if !self.nearby_players.is_empty() {
            summary.push_str("\n## Nearby Players\n");
            for player in &self.nearby_players {
                summary.push_str(&format!(
                    "- {} (Lv.{}) at distance {:.1}\n",
                    player.name, player.level, player.distance
                ));
            }
        }

        // Available skills
        if !self.available_skills.is_empty() {
            summary.push_str("\n## Available Skills\n");
            for skill in &self.available_skills {
                summary.push_str(&format!(
                    "- {} (ID: {}, Lv.{})\n",
                    skill.name, skill.skill_id, skill.level
                ));
            }
        }

        // Recent events
        if !self.recent_events.is_empty() {
            summary.push_str("\n## Recent Events\n");
            for event in &self.recent_events {
                summary.push_str(&format!("- {}\n", event));
            }
        }

        summary
    }

    /// Converts LLM events to formatted strings for the context.
    pub fn format_events(events: &[LlmEvent]) -> Vec<String> {
        events.iter().map(|e| e.to_string()).collect()
    }
}

// ============================================================================
// Context Builder
// ============================================================================

/// Builder for creating LlmContext from ECS queries.
///
/// This struct provides a fluent interface for building context,
/// allowing incremental construction from various ECS queries.
pub struct LlmContextBuilder {
    context: LlmContext,
}

impl LlmContextBuilder {
    /// Creates a new context builder for the given bot ID.
    pub fn new(bot_id: Uuid) -> Self {
        Self {
            context: LlmContext::empty(bot_id),
        }
    }

    /// Sets the bot's basic information.
    pub fn with_bot_info(mut self, name: String, level: u16) -> Self {
        self.context.bot_name = name;
        self.context.bot_level = level;
        self
    }

    /// Sets the bot's vital statistics.
    pub fn with_vitals(mut self, health_percent: u8, mana_percent: u8) -> Self {
        self.context.health_percent = health_percent;
        self.context.mana_percent = mana_percent;
        self
    }

    /// Sets the bot's position.
    pub fn with_position(mut self, x: f32, y: f32, z: f32) -> Self {
        self.context.position = (x, y, z);
        self
    }

    /// Sets the zone name.
    pub fn with_zone(mut self, zone_name: String) -> Self {
        self.context.zone_name = zone_name;
        self
    }

    /// Sets the behavior mode.
    pub fn with_behavior_mode(mut self, mode: String) -> Self {
        self.context.behavior_mode = mode;
        self
    }

    /// Sets combat status.
    pub fn with_combat_status(mut self, is_in_combat: bool) -> Self {
        self.context.is_in_combat = is_in_combat;
        self
    }

    /// Sets death status.
    pub fn with_dead_status(mut self, is_dead: bool) -> Self {
        self.context.is_dead = is_dead;
        self
    }

    /// Sets sitting status.
    pub fn with_sitting_status(mut self, is_sitting: bool) -> Self {
        self.context.is_sitting = is_sitting;
        self
    }

    /// Sets the assigned player context.
    pub fn with_assigned_player(mut self, player: Option<AssignedPlayerContext>) -> Self {
        self.context.assigned_player = player;
        self
    }

    /// Adds a nearby monster.
    pub fn add_nearby_monster(mut self, monster: NearbyEntityContext) -> Self {
        self.context.nearby_monsters.push(monster);
        self
    }

    /// Sets all nearby monsters.
    pub fn with_nearby_monsters(mut self, monsters: Vec<NearbyEntityContext>) -> Self {
        self.context.nearby_monsters = monsters;
        self
    }

    /// Adds a nearby item.
    pub fn add_nearby_item(mut self, item: NearbyItemContext) -> Self {
        self.context.nearby_items.push(item);
        self
    }

    /// Sets all nearby items.
    pub fn with_nearby_items(mut self, items: Vec<NearbyItemContext>) -> Self {
        self.context.nearby_items = items;
        self
    }

    /// Adds a nearby player.
    pub fn add_nearby_player(mut self, player: NearbyPlayerContext) -> Self {
        self.context.nearby_players.push(player);
        self
    }

    /// Sets all nearby players.
    pub fn with_nearby_players(mut self, players: Vec<NearbyPlayerContext>) -> Self {
        self.context.nearby_players = players;
        self
    }

    /// Adds an available skill.
    pub fn add_skill(mut self, skill: SkillContext) -> Self {
        self.context.available_skills.push(skill);
        self
    }

    /// Sets all available skills.
    pub fn with_skills(mut self, skills: Vec<SkillContext>) -> Self {
        self.context.available_skills = skills;
        self
    }

    /// Adds a recent event.
    pub fn add_event(mut self, event: String) -> Self {
        self.context.recent_events.push(event);
        self
    }

    /// Sets all recent events.
    pub fn with_events(mut self, events: Vec<String>) -> Self {
        self.context.recent_events = events;
        self
    }

    /// Sets recent events from LlmEvent types.
    pub fn with_llm_events(mut self, events: &[LlmEvent]) -> Self {
        self.context.recent_events = LlmContext::format_events(events);
        self
    }

    /// Builds and returns the final context.
    pub fn build(self) -> LlmContext {
        self.context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_context() {
        let bot_id = Uuid::new_v4();
        let context = LlmContext::empty(bot_id);

        assert_eq!(context.bot_id, bot_id);
        assert_eq!(context.bot_level, 1);
        assert_eq!(context.health_percent, 100);
        assert_eq!(context.mana_percent, 100);
        assert!(context.bot_name.is_empty());
        assert!(context.nearby_monsters.is_empty());
        assert!(context.nearby_items.is_empty());
    }

    #[test]
    fn test_context_builder() {
        let bot_id = Uuid::new_v4();
        let context = LlmContextBuilder::new(bot_id)
            .with_bot_info("TestBot".to_string(), 50)
            .with_vitals(75, 50)
            .with_position(100.0, 200.0, 300.0)
            .with_zone("Adventure Plains".to_string())
            .with_behavior_mode("aggressive".to_string())
            .with_combat_status(true)
            .build();

        assert_eq!(context.bot_name, "TestBot");
        assert_eq!(context.bot_level, 50);
        assert_eq!(context.health_percent, 75);
        assert_eq!(context.mana_percent, 50);
        assert_eq!(context.position, (100.0, 200.0, 300.0));
        assert_eq!(context.zone_name, "Adventure Plains");
        assert_eq!(context.behavior_mode, "aggressive");
        assert!(context.is_in_combat);
    }

    #[test]
    fn test_context_with_nearby_entities() {
        let bot_id = Uuid::new_v4();
        let context = LlmContextBuilder::new(bot_id)
            .with_bot_info("TestBot".to_string(), 10)
            .add_nearby_monster(NearbyEntityContext {
                entity_id: 100,
                name: "Jelly Bean".to_string(),
                level: 5,
                distance: 150.0,
                health_percent: Some(100),
            })
            .add_nearby_item(NearbyItemContext {
                entity_id: 200,
                name: "Health Potion".to_string(),
                distance: 50.0,
            })
            .build();

        assert_eq!(context.nearby_monsters.len(), 1);
        assert_eq!(context.nearby_monsters[0].name, "Jelly Bean");
        assert_eq!(context.nearby_items.len(), 1);
        assert_eq!(context.nearby_items[0].name, "Health Potion");
    }

    #[test]
    fn test_format_context_summary() {
        let bot_id = Uuid::new_v4();
        let context = LlmContextBuilder::new(bot_id)
            .with_bot_info("HelperBot".to_string(), 30)
            .with_vitals(80, 60)
            .with_position(100.0, 0.0, 200.0)
            .with_zone("Zant".to_string())
            .with_behavior_mode("defensive".to_string())
            .with_assigned_player(Some(AssignedPlayerContext {
                name: "Player1".to_string(),
                health_percent: 90,
                distance: 200.0,
                is_in_combat: false,
            }))
            .add_nearby_monster(NearbyEntityContext {
                entity_id: 50,
                name: "Slime".to_string(),
                level: 10,
                distance: 300.0,
                health_percent: None,
            })
            .build();

        let summary = context.format_context_summary();

        assert!(summary.contains("HelperBot"));
        assert!(summary.contains("Level 30"));
        assert!(summary.contains("Health: 80%"));
        assert!(summary.contains("Player1"));
        assert!(summary.contains("Slime"));
    }

    #[test]
    fn test_context_serialization() {
        let bot_id = Uuid::new_v4();
        let context = LlmContextBuilder::new(bot_id)
            .with_bot_info("TestBot".to_string(), 20)
            .with_vitals(100, 80)
            .build();

        // Should serialize to JSON without errors
        let json = serde_json::to_string(&context);
        assert!(json.is_ok());

        // Should contain expected fields
        let json_str = json.unwrap();
        assert!(json_str.contains("TestBot"));
        assert!(json_str.contains("health_percent"));
    }
}
