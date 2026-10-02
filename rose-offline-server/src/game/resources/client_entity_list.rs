use bevy::ecs::{change_detection::Mut, prelude::Entity};
use bevy::math::{UVec2, Vec2, Vec3, Vec3Swizzles};
use bevy::prelude::Resource;
use bitvec::prelude::*;

use rose_data::{ZoneData, ZoneDatabase, ZoneId};

use crate::game::components::{ClientEntity, ClientEntityId, ClientEntitySector, ClientEntityType};

const MAX_CLIENT_ENTITY_ID: usize = 4096;

pub type ClientEntitySet = BitArr!(for MAX_CLIENT_ENTITY_ID);

#[derive(Clone, Default)]
struct ClientEntityZoneSector {
    // The list of entities currently inside this sector
    entities: ClientEntitySet,

    // The list of entities visible from this sector, this is a union of the entities of all adjacent sectors
    visible_entities: ClientEntitySet,
}

#[allow(dead_code)]
impl ClientEntityZoneSector {
    pub fn get_visible_entities(&self) -> &ClientEntitySet {
        &self.visible_entities
    }

    fn join_sector(&mut self, id: ClientEntityId) {
        self.entities.set(id.0, true);
    }

    fn leave_sector(&mut self, id: ClientEntityId) {
        self.entities.set(id.0, false);
    }

    fn add_visible_entity(&mut self, id: ClientEntityId) {
        self.visible_entities.set(id.0, true);
    }

    fn remove_visible_entity(&mut self, id: ClientEntityId) {
        self.visible_entities.set(id.0, false);
    }
}

pub struct ClientEntityZone {
    // Current zone id
    zone_id: ZoneId,

    // The size (width and height) of a sector
    sector_size: f32,

    // Distance from middle of sector before leaving sector
    sector_leave_distance_squared: f32,

    // X, Y position of the first sector
    sector_base_position: Vec2,

    // Number of sectors in X and Y direction
    sector_count: UVec2,

    // The list of sectors
    sectors: Vec<ClientEntityZoneSector>,

    // The list of entities currently inside this zone
    entities: Vec<Option<(Entity, ClientEntity, Vec3)>>,

    // Bit set for every id whose `entities` slot is Some, plus id 0 which is never handed out,
    // so the lowest free id is the first zero bit
    occupied_ids: ClientEntitySet,

    // The list of entities leaving the zone, this is so we can process any
    // visibility changes before freeing the entity id
    leaving_entities: Vec<ClientEntityId>,
}

impl ClientEntityZone {
    pub fn new(zone_info: &ZoneData) -> Self {
        let sector_size = zone_info.sector_size as f32;
        let sector_limit = (sector_size / 2.0) + (sector_size * 0.2);

        let mut occupied_ids = ClientEntitySet::default();
        occupied_ids.set(0, true);

        Self {
            zone_id: zone_info.id,
            sector_size,
            sector_count: UVec2::new(zone_info.num_sectors_x, zone_info.num_sectors_y),
            sector_base_position: zone_info.sectors_base_position,
            sector_leave_distance_squared: sector_limit * sector_limit,
            sectors: vec![
                Default::default();
                (zone_info.num_sectors_x * zone_info.num_sectors_y) as usize
            ],
            entities: vec![None; MAX_CLIENT_ENTITY_ID],
            occupied_ids,
            leaving_entities: Vec::new(),
        }
    }

    pub fn calculate_sector(&self, position: Vec2) -> UVec2 {
        let sector = (position - self.sector_base_position) / self.sector_size;
        UVec2::new(
            u32::min(
                i32::max(0i32, sector[0] as i32) as u32,
                self.sector_count.x - 1,
            ),
            u32::min(
                i32::max(0i32, sector[1] as i32) as u32,
                self.sector_count.y - 1,
            ),
        )
    }

    fn calculate_sector_midpoint(&self, sector: UVec2) -> Vec2 {
        self.sector_base_position
            + Vec2::new(sector[0] as f32 + 0.5, sector[1] as f32 + 0.5) * self.sector_size
    }

    fn get_sector(&self, sector: UVec2) -> &ClientEntityZoneSector {
        &self.sectors[sector[0] as usize + (sector[1] * self.sector_count.x) as usize]
    }

    fn get_sector_mut(&mut self, sector: UVec2) -> &mut ClientEntityZoneSector {
        &mut self.sectors[sector[0] as usize + (sector[1] * self.sector_count.x) as usize]
    }

    pub fn get_sector_visible_entities(&self, sector: UVec2) -> &ClientEntitySet {
        self.get_sector(sector).get_visible_entities()
    }

    pub fn get_entity(&self, id: ClientEntityId) -> Option<&(Entity, ClientEntity, Vec3)> {
        self.entities[id.0].as_ref()
    }

    /// Returns an iterator over all entities in this zone with their type
    pub fn iter_entities(&self) -> impl Iterator<Item = (Entity, ClientEntityType)> + '_ {
        self.entities
            .iter()
            .filter_map(|entry| entry.as_ref())
            .map(|(entity, client_entity, _)| (*entity, client_entity.entity_type))
    }

    fn for_each_visible_sector<F>(&mut self, sector: UVec2, mut f: F)
    where
        F: FnMut(&mut ClientEntityZoneSector),
    {
        // 7x7 sector window (radius 3) so monsters spawn roughly twice as far away.
        let min_sector_x = sector.x.saturating_sub(3);
        let max_sector_x = u32::min(sector.x + 3, self.sector_count.x - 1);
        let min_sector_y = sector.y.saturating_sub(3);
        let max_sector_y = u32::min(sector.y + 3, self.sector_count.y - 1);

        for x in min_sector_x..=max_sector_x {
            for y in min_sector_y..=max_sector_y {
                f(self.get_sector_mut(UVec2::new(x, y)))
            }
        }
    }

    fn join_sector(&mut self, sector: UVec2, id: ClientEntityId) {
        // Join the sector
        self.get_sector_mut(sector).join_sector(id);

        // Join the visible set of adjacent sectors
        self.for_each_visible_sector(sector, |zone_sector| zone_sector.add_visible_entity(id));
    }

    fn leave_sector(&mut self, sector: UVec2, id: ClientEntityId) {
        // Leave the sector
        self.get_sector_mut(sector).leave_sector(id);

        // Leave the visible set of adjacent sectors
        self.for_each_visible_sector(sector, |zone_sector| zone_sector.remove_visible_entity(id));
    }

    pub fn join_zone(
        &mut self,
        entity_type: ClientEntityType,
        entity: Entity,
        position: Vec3,
    ) -> Option<(ClientEntity, ClientEntitySector)> {
        let sector = self.calculate_sector(position.xy());

        // Allocate the lowest free entity id (id 0 is invalid and always marked occupied)
        let free_index = self.occupied_ids[..MAX_CLIENT_ENTITY_ID].first_zero()?;
        let client_entity_id = ClientEntityId(free_index);
        let client_entity = ClientEntity::new(entity_type, client_entity_id, self.zone_id);
        let client_entity_sector = ClientEntitySector::new(sector);

        // Join zone
        self.entities[free_index] = Some((entity, client_entity.clone(), position));
        self.occupied_ids.set(free_index, true);

        // Join sector
        self.join_sector(sector, client_entity_id);

        Some((client_entity, client_entity_sector))
    }

    pub fn leave_zone(
        &mut self,
        entity: Entity,
        client_entity: &ClientEntity,
        client_entity_sector: &ClientEntitySector,
    ) {
        // Validate entity list
        assert_eq!(
            self.entities[client_entity.id.0].as_ref().map(|x| &x.0),
            Some(&entity)
        );

        // Leave sector
        self.leave_sector(client_entity_sector.sector, client_entity.id);

        // Set as leaving zone
        self.leaving_entities.push(client_entity.id);
    }

    pub fn update_position(
        &mut self,
        entity: Entity,
        client_entity: &ClientEntity,
        client_entity_sector: &mut Mut<ClientEntitySector>,
        position: Vec3,
    ) {
        // Validate entity list
        assert_eq!(
            self.entities[client_entity.id.0].as_ref().map(|x| &x.0),
            Some(&entity)
        );

        // Update sector
        let midpoint = self.calculate_sector_midpoint(client_entity_sector.sector);
        if position.xy().distance_squared(midpoint) > self.sector_leave_distance_squared {
            let previous_sector = client_entity_sector.sector;
            let new_sector = self.calculate_sector(position.xy());
            self.leave_sector(previous_sector, client_entity.id);
            self.join_sector(new_sector, client_entity.id);
            client_entity_sector.sector = new_sector;
        }

        // Update the entity data storage
        self.entities[client_entity.id.0] = Some((entity, client_entity.clone(), position));
    }

    pub fn process_zone_leavers(&mut self) {
        // Free the entity id
        for id in self.leaving_entities.iter() {
            self.entities[id.0] = None;
            self.occupied_ids.set(id.0, false);
        }

        self.leaving_entities.clear();
    }

    pub fn iter_entities_within_distance(
        &self,
        origin: Vec2,
        distance: f32,
    ) -> ClientEntityZoneEntityIterator<'_, '_> {
        let min_sector = self.calculate_sector(origin - Vec2::new(distance, distance));
        let max_sector = self.calculate_sector(origin + Vec2::new(distance, distance));

        ClientEntityZoneEntityIterator {
            zone: self,
            min_sector,
            max_sector,
            current_sector: min_sector,
            current_iter: self.get_sector(min_sector).entities.iter_ones(),
            origin,
            max_distance_squared: distance * distance,
            filter_entity_type: None,
        }
    }

    pub fn iter_entity_type_within_distance<'a>(
        &self,
        origin: Vec2,
        distance: f32,
        entity_type: &'a [ClientEntityType],
    ) -> ClientEntityZoneEntityIterator<'_, 'a> {
        let min_sector = self.calculate_sector(origin - Vec2::new(distance, distance));
        let max_sector = self.calculate_sector(origin + Vec2::new(distance, distance));

        ClientEntityZoneEntityIterator {
            zone: self,
            min_sector,
            max_sector,
            current_sector: min_sector,
            current_iter: self.get_sector(min_sector).entities.iter_ones(),
            origin,
            max_distance_squared: distance * distance,
            filter_entity_type: Some(entity_type),
        }
    }
}

pub struct ClientEntityZoneEntityIterator<'a, 'b> {
    zone: &'a ClientEntityZone,
    min_sector: UVec2,
    max_sector: UVec2,
    current_sector: UVec2,
    current_iter: bitvec::slice::IterOnes<'a, usize, Lsb0>,
    origin: Vec2,
    max_distance_squared: f32,
    filter_entity_type: Option<&'b [ClientEntityType]>,
}

impl<'a, 'b> Iterator for ClientEntityZoneEntityIterator<'a, 'b> {
    type Item = (Entity, Vec3);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(index) = self.current_iter.next() {
                if let Some((entity, client_entity, position)) = self.zone.entities[index].as_ref()
                {
                    if !self
                        .filter_entity_type
                        .as_ref()
                        .map_or(true, |x| x.contains(&client_entity.entity_type))
                    {
                        continue;
                    }

                    let distance_squared = self.origin.distance_squared(position.xy());
                    if distance_squared <= self.max_distance_squared {
                        break Some((*entity, *position));
                    } else {
                        continue;
                    }
                }
            }

            self.current_sector.x += 1;

            if self.current_sector.x > self.max_sector.x {
                self.current_sector.x = self.min_sector.x;
                self.current_sector.y += 1;
            }

            if self.current_sector.y > self.max_sector.y {
                break None;
            }

            self.current_iter = self
                .zone
                .get_sector(self.current_sector)
                .entities
                .iter_ones();
        }
    }
}

#[derive(Resource)]
pub struct ClientEntityList {
    // Indexed by zone id; looked up on every position update, so a Vec instead of a HashMap
    zones: Vec<Option<ClientEntityZone>>,
}

impl ClientEntityList {
    pub fn new(zone_database: &ZoneDatabase) -> Self {
        let mut zones: Vec<Option<ClientEntityZone>> = Vec::new();
        for zone in zone_database.iter() {
            let index = zone.id.get() as usize;
            if zones.len() <= index {
                zones.resize_with(index + 1, || None);
            }
            zones[index] = Some(ClientEntityZone::new(zone));
        }
        Self { zones }
    }

    pub fn get_zone(&self, zone_id: ZoneId) -> Option<&ClientEntityZone> {
        self.zones
            .get(zone_id.get() as usize)
            .and_then(Option::as_ref)
    }

    pub fn get_zone_mut(&mut self, zone_id: ZoneId) -> Option<&mut ClientEntityZone> {
        self.zones
            .get_mut(zone_id.get() as usize)
            .and_then(Option::as_mut)
    }

    pub fn process_zone_leavers(&mut self) {
        self.zones
            .iter_mut()
            .flatten()
            .for_each(ClientEntityZone::process_zone_leavers);
    }
}
