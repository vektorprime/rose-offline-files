use bevy::prelude::{Entity, Event};
use rose_data::ZoneId;
use crate::game::components::ChatType;

#[derive(Event, Debug, Clone)]
pub struct ChatMessageEvent {
    pub sender_entity: Entity,
    pub sender_name: String,
    pub zone_id: ZoneId,
    pub message: String,
    pub chat_type: ChatType,
}
