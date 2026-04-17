use bevy::{
    ecs::prelude::{Entity, Query, Res, ResMut},
    math::{Vec3, Vec3Swizzles},
    time::Time,
};

use crate::game::{
    components::{ClientEntity, ClientEntitySector, Command, CommandData, MoveSpeed, Position},
    resources::ClientEntityList,
};

/// Server-authoritative position update system.
/// This system moves entities toward their destination at their configured speed.
/// The server is the source of truth for all entity positions.
/// 
/// Position validation:
/// - Movement speed is enforced based on MoveSpeed component
/// - Maximum movement per tick is limited to prevent teleportation
/// - Z coordinate (height) is preserved during horizontal movement
/// - Client collision reports are validated in game_server_system.rs
pub fn update_position_system(
    mut query: Query<(
        Entity,
        Option<&ClientEntity>,
        Option<&mut ClientEntitySector>,
        &MoveSpeed,
        &mut Position,
        &Command,
    )>,
    mut client_entity_list: ResMut<ClientEntityList>,
    time: Res<Time>,
) {
    query.iter_mut().for_each(
        |(entity, client_entity, client_entity_sector, move_speed, mut position, command)| {
            let CommandData::Move { destination, .. } = command.command else {
                return;
            };

            let direction = destination.xy() - position.position.xy();
            let distance_squared = direction.length_squared();

            let old_position = position.position;
            
            // Maximum allowed movement per tick (prevent speed hacking)
            let max_move_per_tick = move_speed.speed * time.delta_secs() * 1.5; // 50% tolerance
            
            if distance_squared == 0.0 {
                // Preserve Z coordinate when setting position to destination
                position.position = Vec3::new(destination.x, destination.y, destination.z);
            } else {
                let move_vector = direction.normalize() * move_speed.speed * time.delta_secs();
                
                // Clamp movement to maximum allowed per tick
                let clamped_move_vector = if move_vector.length_squared() >= distance_squared {
                    // Can reach destination this tick
                    move_vector
                } else if move_vector.length() > max_move_per_tick {
                    // Clamp to max allowed movement
                    move_vector.normalize() * max_move_per_tick
                } else {
                    move_vector
                };
                
                if clamped_move_vector.length_squared() >= distance_squared {
                    // Preserve Z coordinate when reaching destination
                    position.position = Vec3::new(destination.x, destination.y, destination.z);
                } else {
                    // Only update X and Y, preserve Z coordinate
                    position.position.x += clamped_move_vector.x;
                    position.position.y += clamped_move_vector.y;
                    // Z is preserved - no change needed (will be set by MoveCollision from client)
                }
            }

            // Log if position diverged significantly from expected (for debugging)
            let actual_movement = (position.position.xy() - old_position.xy()).length();
            if actual_movement > max_move_per_tick * 1.5 {
                log::warn!(
                    "[POSITION_AUTH] Entity moved {:.2}cm (max allowed: {:.2}cm) - possible desync",
                    actual_movement,
                    max_move_per_tick
                );
            }

            if let (Some(client_entity), Some(mut client_entity_sector)) =
                (client_entity, client_entity_sector)
            {
                if let Some(zone) = client_entity_list.get_zone_mut(position.zone_id) {
                    zone.update_position(
                        entity,
                        client_entity,
                        &mut client_entity_sector,
                        position.position,
                    )
                }
            }
        },
    );
}
