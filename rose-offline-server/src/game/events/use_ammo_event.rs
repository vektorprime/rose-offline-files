use bevy::prelude::{Entity, Event, Message};

use rose_data::AmmoIndex;

#[derive(Message)]
pub struct UseAmmoEvent {
    pub entity: Entity,
    pub ammo_index: AmmoIndex,
    pub quantity: usize,
}
