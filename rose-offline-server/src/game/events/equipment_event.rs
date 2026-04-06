use bevy::prelude::{Entity, Event, Message};

use rose_data::{AmmoIndex, EquipmentIndex, VehiclePartIndex};
use rose_game_common::components::ItemSlot;

#[derive(Message, Copy, Clone, Debug)]
pub enum EquipmentEvent {
    ChangeEquipment {
        entity: Entity,
        equipment_index: EquipmentIndex,
        item_slot: Option<ItemSlot>,
    },
    ChangeAmmo {
        entity: Entity,
        ammo_index: AmmoIndex,
        item_slot: Option<ItemSlot>,
    },
    ChangeVehiclePart {
        entity: Entity,
        vehicle_part_index: VehiclePartIndex,
        item_slot: Option<ItemSlot>,
    },
}
