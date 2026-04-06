use bevy::prelude::{Entity, Event, Message};

#[derive(Message)]
pub struct PickupItemEvent {
    pub pickup_entity: Entity,
    pub item_entity: Entity,
}
