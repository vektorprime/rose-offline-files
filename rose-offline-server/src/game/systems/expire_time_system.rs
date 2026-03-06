use bevy::{
    ecs::prelude::{Commands, Entity, Query, ResMut},
};

use std::time::Instant;

use crate::game::{
    bundles::client_entity_leave_zone,
    components::{
        ClientEntity, ClientEntitySector, Command, EntityExpireTime, Owner, OwnerExpireTime,
        PartyOwner, Position,
    },
    resources::ClientEntityList,
};

/// Removes entities/owners whose expire‑time has passed.
pub fn expire_time_system(
    mut commands: Commands,
    entity_expire_time_query: Query<(
        Entity,
        &EntityExpireTime,
        Option<&Position>,
        Option<&ClientEntity>,
        Option<&ClientEntitySector>,
        Option<&Command>,
    )>,
    owner_expire_time_query: Query<(Entity, &OwnerExpireTime, Option<&PartyOwner>)>,
    mut client_entity_list: ResMut<ClientEntityList>,
) {
    entity_expire_time_query.iter().for_each(
        |(entity, entity_expire_time, position, client_entity, client_entity_sector, command)| {
            if Instant::now() >= entity_expire_time.when {
                match command.is_some() {
                    true => {
                        commands
                            .entity(entity)
                            .insert(Command::with_die(None, None, None));
                    }
                    false => {
                        if let (Some(position), Some(client_entity), Some(client_entity_sector)) =
                            (position, client_entity, client_entity_sector)
                        {
                            client_entity_leave_zone(
                                &mut commands,
                                &mut client_entity_list,
                                entity,
                                client_entity,
                                client_entity_sector,
                                position,
                            );
                        }
                        commands.entity(entity).despawn();
                    }
                }
            }
        },
    );

    owner_expire_time_query.iter().for_each(|(entity, owner_expire_time, party_owner)| {
        if Instant::now() >= owner_expire_time.when {
            if let Ok(mut entity_commands) = commands.get_entity(entity) {
                entity_commands.remove::<Owner>();
                // Only remove PartyOwner if it exists (not all items with Owner have PartyOwner)
                if party_owner.is_some() {
                    entity_commands.remove::<PartyOwner>();
                }
            }
        }
    });
}
