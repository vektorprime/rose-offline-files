use bevy::{
    ecs::prelude::{Entity, Query, Res, ResMut},
    math::{Vec3, Vec3Swizzles},
    time::Time,
};

use crate::game::{
    components::{ClientEntity, ClientEntitySector, Command, CommandData, MoveSpeed, Position},
    resources::ClientEntityList,
};

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
            
            if distance_squared == 0.0 {
                // Preserve Z coordinate when setting position to destination
                position.position = Vec3::new(destination.x, destination.y, destination.z);
            } else {
                let move_vector = direction.normalize() * move_speed.speed * time.delta_secs();
                if move_vector.length_squared() >= distance_squared {
                    // Preserve Z coordinate when reaching destination
                    position.position = Vec3::new(destination.x, destination.y, destination.z);
                } else {
                    // Only update X and Y, preserve Z coordinate
                    position.position.x += move_vector.x;
                    position.position.y += move_vector.y;
                    // Z is preserved - no change needed
                }
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
