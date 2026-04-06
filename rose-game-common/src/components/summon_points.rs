use bevy::{ecs::prelude::Component, reflect::Reflect};
use serde::{Deserialize, Serialize};

#[derive(Component, Copy, Clone, Debug, Deserialize, Serialize, Reflect)]
pub struct SummonPoints {
    pub points: u32,
}

impl SummonPoints {
    pub fn new(points: u32) -> Self {
        Self { points }
    }

    pub fn try_spend(&mut self, cost: u32) -> bool {
        if self.points >= cost {
            self.points -= cost;
            true
        } else {
            false
        }
    }

    pub fn add_points(&mut self, amount: u32) {
        self.points = self.points.saturating_add(amount);
    }
}

impl Default for SummonPoints {
    fn default() -> Self {
        Self::new(0)
    }
}
