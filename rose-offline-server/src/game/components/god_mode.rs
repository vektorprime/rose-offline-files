use bevy::prelude::Component;

/// Component that marks an entity as having god mode (invincibility).
/// Entities with this component will not take damage.
#[derive(Component, Default)]
pub struct GodMode;
