use bevy::{ecs::prelude::Entity, prelude::Resource};
use std::collections::HashMap;

use rose_data::{NpcId, ZoneId};

#[derive(Hash, PartialEq, Eq)]
struct EventObjectKey {
    event_id: u16,
    map_chunk_x: i32,
    map_chunk_y: i32,
}

struct ZoneData {
    monster_spawns_enabled: bool,
    event_objects: HashMap<EventObjectKey, Entity>,
}

#[derive(Resource)]
pub struct ZoneList {
    // Indexed by zone id; read by every monster spawn point each tick, so a Vec
    // instead of a HashMap
    zones: Vec<Option<ZoneData>>,
    npcs: HashMap<NpcId, Entity>,
}

impl ZoneList {
    pub fn new() -> Self {
        Self {
            zones: Default::default(),
            npcs: Default::default(),
        }
    }

    fn get_zone(&self, zone_id: ZoneId) -> Option<&ZoneData> {
        self.zones
            .get(zone_id.get() as usize)
            .and_then(Option::as_ref)
    }

    fn get_zone_mut(&mut self, zone_id: ZoneId) -> Option<&mut ZoneData> {
        self.zones
            .get_mut(zone_id.get() as usize)
            .and_then(Option::as_mut)
    }

    pub fn add_zone(&mut self, zone_id: ZoneId) {
        let index = zone_id.get() as usize;
        if self.zones.len() <= index {
            self.zones.resize_with(index + 1, || None);
        }
        self.zones[index] = Some(ZoneData {
            monster_spawns_enabled: true,
            event_objects: Default::default(),
        });
    }

    pub fn get_monster_spawns_enabled(&self, zone_id: ZoneId) -> bool {
        self.get_zone(zone_id)
            .map(|zone| zone.monster_spawns_enabled)
            .unwrap_or(false)
    }

    pub fn set_monster_spawns_enabled(&mut self, zone_id: ZoneId, enabled: bool) -> bool {
        if let Some(zone) = self.get_zone_mut(zone_id) {
            zone.monster_spawns_enabled = enabled;
            true
        } else {
            false
        }
    }

    pub fn add_event_object(
        &mut self,
        zone_id: ZoneId,
        event_id: u16,
        map_chunk_x: i32,
        map_chunk_y: i32,
        entity: Entity,
    ) {
        if let Some(zone) = self.get_zone_mut(zone_id) {
            zone.event_objects.insert(
                EventObjectKey {
                    event_id,
                    map_chunk_x,
                    map_chunk_y,
                },
                entity,
            );
        }
    }

    pub fn find_event_object(
        &self,
        zone_id: ZoneId,
        event_id: u16,
        map_chunk_x: i32,
        map_chunk_y: i32,
    ) -> Option<Entity> {
        self.get_zone(zone_id).and_then(|zone| {
            zone.event_objects
                .get(&EventObjectKey {
                    event_id,
                    map_chunk_x,
                    map_chunk_y,
                })
                .cloned()
        })
    }

    pub fn add_npc(&mut self, npc_id: NpcId, entity: Entity) {
        self.npcs.insert(npc_id, entity);
    }

    pub fn find_npc(&self, npc_id: NpcId) -> Option<Entity> {
        self.npcs.get(&npc_id).cloned()
    }
}
