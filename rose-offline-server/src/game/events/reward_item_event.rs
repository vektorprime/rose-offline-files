use bevy::{ecs::prelude::Entity, prelude::{Event, Message}};

use rose_data::Item;

#[derive(Message)]
pub struct RewardItemEvent {
    pub entity: Entity,
    pub item: Item,
    pub drop_on_full_inventory: bool,
    pub from_item_drop: bool,
}

impl RewardItemEvent {
    pub fn new(entity: Entity, item: Item, drop_on_full_inventory: bool) -> Self {
        Self {
            entity,
            item,
            drop_on_full_inventory,
            from_item_drop: false,
        }
    }
}
