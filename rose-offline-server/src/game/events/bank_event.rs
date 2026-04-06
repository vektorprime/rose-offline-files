use bevy::prelude::{Entity, Message};

use rose_data::Item;

use crate::game::components::ItemSlot;

#[derive(Message)]
pub enum BankEvent {
    Open {
        entity: Entity,
    },
    DepositItem {
        entity: Entity,
        item_slot: ItemSlot,
        item: Item,
        is_premium: bool,
    },
    WithdrawItem {
        entity: Entity,
        bank_slot: usize,
        item: Item,
        is_premium: bool,
    },
}
