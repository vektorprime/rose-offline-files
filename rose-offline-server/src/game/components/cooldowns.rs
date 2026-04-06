use std::{collections::HashMap, time::Instant};

use bevy::prelude::Component;

use rose_data::SkillId;

const MAX_SKILL_COOLDOWN_GROUPS: usize = 16;
const MAX_ITEM_COOLDOWN_TYPES: usize = 32;

#[derive(Default, Component)]
pub struct Cooldowns {
    pub skill: HashMap<SkillId, Instant>,
    pub skill_global: Option<Instant>,
    pub skill_group: [Option<Instant>; MAX_SKILL_COOLDOWN_GROUPS],
    /// Item cooldown types, indexed by cooldown_type_id from ConsumableItemData
    pub item: [Option<Instant>; MAX_ITEM_COOLDOWN_TYPES],
}

impl Cooldowns {
    /// Check if an item with the given cooldown type is on cooldown
    pub fn is_item_on_cooldown(&self, cooldown_type_id: usize, now: Instant) -> bool {
        if cooldown_type_id == 0 {
            // cooldown_type_id of 0 means no cooldown
            return false;
        }
        
        self.item
            .get(cooldown_type_id)
            .and_then(|cooldown| *cooldown)
            .map_or(false, |cooldown_finished| now < cooldown_finished)
    }
    
    /// Set an item cooldown
    pub fn set_item_cooldown(&mut self, cooldown_type_id: usize, duration: std::time::Duration) {
        if cooldown_type_id == 0 || cooldown_type_id >= self.item.len() {
            return;
        }
        
        self.item[cooldown_type_id] = Some(Instant::now() + duration);
    }
}
