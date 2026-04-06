use bevy::{ecs::prelude::Component, math::Vec3, reflect::Reflect};
use enum_map::Enum;
use serde::{Deserialize, Serialize};

use rose_data::ZoneId;

pub type CharacterUniqueId = u32;

#[derive(Copy, Clone, Debug, Deserialize, Serialize, Enum, PartialEq, Eq, Reflect)]
pub enum CharacterGender {
    Male,
    Female,
}

#[derive(Component, Clone, Debug, Deserialize, Serialize, Reflect)]
pub struct CharacterInfo {
    pub name: String,
    pub gender: CharacterGender,
    pub race: u8,
    pub birth_stone: u8,
    pub job: u16,
    pub face: u8,
    pub hair: u8,
    pub rank: u8,
    pub fame: u8,
    pub fame_b: u16,
    pub fame_g: u16,
    pub revive_zone_id: ZoneId,
    pub revive_position: Vec3,
    pub unique_id: CharacterUniqueId,
    /// Head size for appearance customization (default 100 as per C++ source)
    #[serde(default = "default_head_size")]
    pub head_size: i32,
    /// Body size for appearance customization (default 100 as per C++ source)
    #[serde(default = "default_body_size")]
    pub body_size: i32,
    /// PvP flag state (0 = off, non-zero = on)
    #[serde(default)]
    pub pvp_flag: i32,
    /// Mana save percentage (0-100)
    #[serde(default)]
    pub save_mana: u8,
    /// Drop rate percentage modifier
    #[serde(default)]
    pub drop_rate: i32,
    /// Current planet ID
    #[serde(default)]
    pub current_planet: u32,
}

fn default_head_size() -> i32 {
    100
}

fn default_body_size() -> i32 {
    100
}
