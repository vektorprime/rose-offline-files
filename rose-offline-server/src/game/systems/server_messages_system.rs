use std::collections::HashMap;

use bevy::{
    ecs::prelude::{Local, Query, ResMut},
    math::UVec2,
};
use rose_data::ZoneId;

use crate::game::{
    components::{ClientEntity, ClientEntitySector, ClientEntityVisibility, GameClient, Position},
    messages::server::ServerMessage,
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

struct MessageRecipient<'a> {
    game_client: &'a GameClient,
    client_entity: Option<&'a ClientEntity>,
    sector: UVec2,
    visibility: &'a ClientEntityVisibility,
}

/// Sends `message` to the listed recipients: a clone to each but the last, which
/// receives the original.
fn send_to_recipients(recipients: &[MessageRecipient], indices: &[usize], message: ServerMessage) {
    let Some((&last, others)) = indices.split_last() else {
        return;
    };

    for &index in others {
        recipients[index]
            .game_client
            .server_message_tx
            .send(message.clone())
            .ok();
    }
    recipients[last]
        .game_client
        .server_message_tx
        .send(message)
        .ok();
}

pub fn server_messages_system(
    query: Query<(
        &GameClient,
        Option<&ClientEntity>,
        &Position,
        &ClientEntitySector,
        &ClientEntityVisibility,
    )>,
    mut server_messages: ResMut<ServerMessages>,
    mut recipient_indices: Local<Vec<usize>>,
) {
    if server_messages.pending_global_messages.is_empty()
        && server_messages.pending_zone_messages.is_empty()
        && server_messages.pending_sector_messages.is_empty()
        && server_messages.pending_entity_messages.is_empty()
    {
        return;
    }

    // Collect the clients once (in query order), indexed by zone, then deliver
    // message by message. Every client still receives exactly the messages and
    // order of a per-client scan: global, zone, sector, then entity messages,
    // each in queue order.
    let mut recipients = Vec::new();
    let mut zone_recipients: HashMap<ZoneId, Vec<usize>> = HashMap::new();
    for (game_client, client_entity, position, client_entity_sector, visibility) in query.iter() {
        zone_recipients
            .entry(position.zone_id)
            .or_default()
            .push(recipients.len());
        recipients.push(MessageRecipient {
            game_client,
            client_entity,
            sector: client_entity_sector.sector,
            visibility,
        });
    }

    let server_messages = server_messages.as_mut();
    let recipient_indices = &mut *recipient_indices;

    recipient_indices.clear();
    recipient_indices.extend(0..recipients.len());
    for message in server_messages.pending_global_messages.drain(..) {
        send_to_recipients(&recipients, recipient_indices, message.message);
    }

    for message in server_messages.pending_zone_messages.drain(..) {
        if let Some(indices) = zone_recipients.get(&message.zone_id) {
            send_to_recipients(&recipients, indices, message.message);
        }
    }

    for message in server_messages.pending_sector_messages.drain(..) {
        let Some(indices) = zone_recipients.get(&message.zone_id) else {
            continue;
        };

        recipient_indices.clear();
        recipient_indices.extend(
            indices
                .iter()
                .copied()
                .filter(|&index| is_sector_in_range(message.sector, recipients[index].sector)),
        );
        send_to_recipients(&recipients, recipient_indices, message.message);
    }

    for message in server_messages.pending_entity_messages.drain(..) {
        let Some(indices) = zone_recipients.get(&message.zone_id) else {
            continue;
        };

        recipient_indices.clear();
        recipient_indices.extend(indices.iter().copied().filter(|&index| {
            let recipient = &recipients[index];
            let is_self_target = recipient
                .client_entity
                .map(|client_entity| client_entity.id == message.entity_id)
                .unwrap_or(false);

            is_self_target
                || recipient
                    .visibility
                    .get(message.entity_id.0)
                    .map_or(false, |b| *b)
        }));
        send_to_recipients(&recipients, recipient_indices, message.message);
    }
}
