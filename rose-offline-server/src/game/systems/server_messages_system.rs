use bevy::ecs::prelude::{Query, ResMut};

use crate::game::{
    components::{ClientEntitySector, ClientEntityVisibility, GameClient, Position},
    resources::ServerMessages,
};

/// Check if a sector is within range of another sector (adjacent or same sector)
fn is_sector_in_range(source_sector: bevy::math::UVec2, target_sector: bevy::math::UVec2) -> bool {
    let dx = if source_sector.x >= target_sector.x {
        source_sector.x - target_sector.x
    } else {
        target_sector.x - source_sector.x
    };
    let dy = if source_sector.y >= target_sector.y {
        source_sector.y - target_sector.y
    } else {
        target_sector.y - source_sector.y
    };
    // Within 1 sector in both directions (3x3 area centered on source)
    dx <= 1 && dy <= 1
}

pub fn server_messages_system(
    query: Query<(&GameClient, &Position, &ClientEntitySector, &ClientEntityVisibility)>,
    mut server_messages: ResMut<ServerMessages>,
) {
    for (game_client, position, client_entity_sector, client_visibility) in query.iter() {
        for message in server_messages.pending_global_messages.iter() {
            game_client
                .server_message_tx
                .send(message.message.clone())
                .ok();
        }

        for message in server_messages.pending_zone_messages.iter() {
            if position.zone_id == message.zone_id {
                game_client
                    .server_message_tx
                    .send(message.message.clone())
                    .ok();
            }
        }

        for message in server_messages.pending_sector_messages.iter() {
            if position.zone_id == message.zone_id
                && is_sector_in_range(message.sector, client_entity_sector.sector)
            {
                game_client
                    .server_message_tx
                    .send(message.message.clone())
                    .ok();
            }
        }

        for message in server_messages.pending_entity_messages.iter() {
            if position.zone_id == message.zone_id
                && client_visibility
                    .get(message.entity_id.0)
                    .map_or(false, |b| *b)
            {
                game_client
                    .server_message_tx
                    .send(message.message.clone())
                    .ok();
            }
        }
    }

    server_messages.pending_global_messages.clear();
    server_messages.pending_zone_messages.clear();
    server_messages.pending_entity_messages.clear();
    server_messages.pending_sector_messages.clear();
}
