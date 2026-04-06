use bevy::prelude::{Entity, Event, Message};

pub enum RevivePosition {
    CurrentZone,
    SaveZone,
}

#[derive(Message)]
pub struct ReviveEvent {
    pub entity: Entity,
    pub position: RevivePosition,
}
