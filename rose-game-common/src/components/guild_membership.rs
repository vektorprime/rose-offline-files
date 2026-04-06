use bevy::{ecs::prelude::Component, reflect::Reflect};
use serde::{Deserialize, Serialize};

#[derive(Default, Component, Clone, Debug, Deserialize, Serialize, Reflect)]
pub struct GuildMembership {
    pub guild_number: u32,
    pub score: i32,
    pub position: u8,
}

impl GuildMembership {
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if the character is not in a guild
    pub fn is_none(&self) -> bool {
        self.guild_number == 0
    }
}
