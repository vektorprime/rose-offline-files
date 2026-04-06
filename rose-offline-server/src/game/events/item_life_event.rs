use bevy::prelude::{Entity, Event, Message};

use rose_game_common::data::Damage;

#[derive(Message)]
pub enum ItemLifeEvent {
    DecreaseWeaponLife { entity: Entity },
    DecreaseArmourLife { entity: Entity, damage: Damage },
    DecreaseVehicleEngineLife { entity: Entity, amount: Option<u16> },
}
