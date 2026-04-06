use bevy::{ecs::prelude::Entity, prelude::{Event, Message}};

#[derive(Message)]
pub enum SaveEvent {
    Character {
        entity: Entity,
        remove_after_save: bool,
    },
}
