use bevy::math::UVec2;
use bevy::prelude::Resource;
use rose_data::ZoneId;

use crate::game::{
    components::{ClientEntity, ClientEntityId},
    messages::server::ServerMessage,
};

pub struct GlobalMessage {
    pub message: ServerMessage,
}

pub struct ZoneMessage {
    pub zone_id: ZoneId,
    pub message: ServerMessage,
}

pub struct EntityMessage {
    pub zone_id: ZoneId,
    pub entity_id: ClientEntityId,
    pub message: ServerMessage,
}

pub struct SectorMessage {
    pub zone_id: ZoneId,
    pub sector: UVec2,
    pub message: ServerMessage,
}

#[derive(Default, Resource)]
pub struct ServerMessages {
    pub pending_global_messages: Vec<GlobalMessage>,
    pub pending_zone_messages: Vec<ZoneMessage>,
    pub pending_entity_messages: Vec<EntityMessage>,
    pub pending_sector_messages: Vec<SectorMessage>,
}

#[allow(dead_code)]
impl ServerMessages {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn send_global_message(&mut self, message: ServerMessage) {
        self.pending_global_messages.push(GlobalMessage { message });
    }

    pub fn send_zone_message(&mut self, zone_id: ZoneId, message: ServerMessage) {
        self.pending_zone_messages
            .push(ZoneMessage { zone_id, message });
    }

    pub fn send_entity_message(&mut self, entity: &ClientEntity, message: ServerMessage) {
        self.pending_entity_messages.push(EntityMessage {
            zone_id: entity.zone_id,
            entity_id: entity.id,
            message,
        });
    }

    /// Send a message to all entities in a specific sector and its adjacent sectors (3x3 area)
    pub fn send_sector_message(&mut self, zone_id: ZoneId, sector: UVec2, message: ServerMessage) {
        self.pending_sector_messages.push(SectorMessage {
            zone_id,
            sector,
            message,
        });
    }
}
