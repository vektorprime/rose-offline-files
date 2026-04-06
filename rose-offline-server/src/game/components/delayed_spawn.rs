use bevy::ecs::prelude::Component;
use rose_data::{NpcId, ZoneId};
use std::time::Instant;

use crate::game::components::Team;

/// Component for tracking delayed monster spawns.
/// When an NPC dies and wants to spawn another NPC, this component
/// tracks the spawn to be executed after a delay.
#[derive(Component, Clone)]
pub struct DelayedSpawn {
    pub npc_id: NpcId,
    pub zone_id: ZoneId,
    pub spawn_position: bevy::math::Vec3,
    pub spawn_range: i32,
    pub team: Team,
    pub owner_entity: Option<bevy::ecs::entity::Entity>,
    pub spawn_time: Instant,
}

impl DelayedSpawn {
    pub fn new(
        npc_id: NpcId,
        zone_id: ZoneId,
        spawn_position: bevy::math::Vec3,
        spawn_range: i32,
        team: Team,
        owner_entity: Option<bevy::ecs::entity::Entity>,
        delay_secs: u64,
    ) -> Self {
        Self {
            npc_id,
            zone_id,
            spawn_position,
            spawn_range,
            team,
            owner_entity,
            spawn_time: Instant::now() + std::time::Duration::from_secs(delay_secs),
        }
    }

    pub fn is_ready(&self) -> bool {
        Instant::now() >= self.spawn_time
    }
}
