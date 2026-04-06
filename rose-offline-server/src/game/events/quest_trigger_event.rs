use bevy::{ecs::prelude::Entity, prelude::{Event, Message}};

use rose_data::QuestTriggerHash;

#[derive(Message)]
pub struct QuestTriggerEvent {
    pub trigger_entity: Entity,
    pub trigger_hash: QuestTriggerHash,
}
