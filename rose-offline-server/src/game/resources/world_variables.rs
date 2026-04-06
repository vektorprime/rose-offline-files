use bevy::prelude::Resource;

const MAX_WORLD_VARIABLES: usize = 100;
const MAX_ECONOMY_VARIABLES: usize = 100;

/// World-scoped variables that persist across all zones and NPCs.
/// These can be read and modified by AI conditions and actions.
#[derive(Resource)]
pub struct WorldVariables {
    pub variables: Vec<i32>,
}

impl WorldVariables {
    pub fn new() -> Self {
        Self {
            variables: vec![0; MAX_WORLD_VARIABLES],
        }
    }

    pub fn get(&self, index: usize) -> i32 {
        self.variables.get(index).copied().unwrap_or(0)
    }

    pub fn set(&mut self, index: usize, value: i32) {
        if index < self.variables.len() {
            self.variables[index] = value;
        }
    }

    pub fn add(&mut self, index: usize, value: i32) {
        if index < self.variables.len() {
            self.variables[index] = i32::min(self.variables[index] + value, 500);
        }
    }

    pub fn subtract(&mut self, index: usize, value: i32) {
        if index < self.variables.len() {
            self.variables[index] = i32::max(self.variables[index] - value, 0);
        }
    }
}

impl Default for WorldVariables {
    fn default() -> Self {
        Self::new()
    }
}

/// Economy-scoped variables for tracking economic state across the game world.
/// These can be used for dynamic pricing, supply/demand, quest economies, etc.
#[derive(Resource)]
pub struct EconomyVariables {
    pub variables: Vec<i32>,
}

impl EconomyVariables {
    pub fn new() -> Self {
        Self {
            variables: vec![0; MAX_ECONOMY_VARIABLES],
        }
    }

    pub fn get(&self, index: usize) -> i32 {
        self.variables.get(index).copied().unwrap_or(0)
    }

    pub fn set(&mut self, index: usize, value: i32) {
        if index < self.variables.len() {
            self.variables[index] = value;
        }
    }

    pub fn add(&mut self, index: usize, value: i32) {
        if index < self.variables.len() {
            self.variables[index] = i32::min(self.variables[index] + value, 500);
        }
    }

    pub fn subtract(&mut self, index: usize, value: i32) {
        if index < self.variables.len() {
            self.variables[index] = i32::max(self.variables[index] - value, 0);
        }
    }
}

impl Default for EconomyVariables {
    fn default() -> Self {
        Self::new()
    }
}
