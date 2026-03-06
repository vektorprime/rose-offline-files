use bevy::prelude::Component;

/// Component that marks an entity as being in ghost mode.
/// Entities with this component:
/// - Have no collision with other entities
/// - Are invisible to monsters (monsters won't aggro)
#[derive(Component, Default)]
pub struct GhostMode;
