use bevy::prelude::Resource;

#[derive(Resource, Clone)]
pub struct GameConfig {
    pub enable_npc_spawns: bool,
    pub enable_monster_spawns: bool,
    pub spawn_bots_on_startup: bool,
    pub startup_bot_count: u32,
    pub startup_bot_zone: u32,
    pub startup_bot_level_min: u32,
    pub startup_bot_level_max: u32,
    /// Number of bots spawned per zone when `bot_spawn_all_zones` is enabled.
    pub startup_bots_per_zone: u32,
    /// Spawn bots across every zone in the game, classified as towns or fields.
    pub bot_spawn_all_zones: bool,
    /// Hard cap on the total number of bots spawned at startup.
    pub bot_total_cap: u32,
    /// Route long bot/monster travel orders around baked zone blockers
    /// (IFO walls + cnst/deco props). Disable to restore direct walking.
    pub enable_bot_navigation: bool,
}

impl GameConfig {
    pub fn default() -> Self {
        Self {
            enable_monster_spawns: true,
            enable_npc_spawns: true,
            spawn_bots_on_startup: true,
            startup_bot_count: 10,
            startup_bot_zone: 1,
            startup_bot_level_min: 1,
            startup_bot_level_max: 10,
            startup_bots_per_zone: 20,
            bot_spawn_all_zones: true,
            bot_total_cap: 700,
            enable_bot_navigation: true,
        }
    }
}
