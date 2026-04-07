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
        }
    }
}
