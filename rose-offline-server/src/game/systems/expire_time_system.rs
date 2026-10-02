use bevy::{
    ecs::prelude::{Commands, Entity, Query, ResMut},
};

use std::time::{Duration, Instant};

use crate::game::{
    components::{
        ClientEntity, ClientEntitySector, Command, EntityExpireTime, MotionData, Owner,
        OwnerExpireTime, PartyOwner, Position,
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
        Option<&MotionData>,
    )>,
    owner_expire_time_query: Query<(Entity, &OwnerExpireTime, Option<&PartyOwner>)>,
    mut client_entity_list: ResMut<ClientEntityList>,
) {
    let now = Instant::now();

    entity_expire_time_query.iter().for_each(
        |(
            entity,
            entity_expire_time,
            position,
            client_entity,
            client_entity_sector,
            command,
            motion_data,
        )| {
            if now >= entity_expire_time.when {
                match command {
                    Some(command) => {
                        // Kill it once, with a finite death duration (as damage_system does) so
                        // the death completes and the entity is removed; re-inserting a Die
                        // every tick would restart it forever.
                        if !command.is_dead() {
                            let die_duration = motion_data
                                .and_then(|motion_data| motion_data.get_die())
                                .map_or(Duration::from_secs(1), |die| die.duration);
                            commands.entity(entity).insert(Command::with_die(
                                None,
                                None,
                                Some(die_duration),
                            ));
                        }
                    }
                    None => {
                        // Only the zone slot needs releasing: removing the client entity
                        // components first is redundant as the entity is despawned.
                        if let (Some(position), Some(client_entity), Some(client_entity_sector)) =
                            (position, client_entity, client_entity_sector)
                        {
                            if let Some(zone) = client_entity_list.get_zone_mut(position.zone_id) {
                                zone.leave_zone(entity, client_entity, client_entity_sector);
                            }
                        }
                        commands.entity(entity).despawn();
                    }
                }
            }
        },
    );

    owner_expire_time_query.iter().for_each(|(entity, owner_expire_time, party_owner)| {
        if now >= owner_expire_time.when {
            if let Ok(mut entity_commands) = commands.get_entity(entity) {
                // Remove the expire time with the owner so this is not repeated every tick.
                entity_commands.remove::<(Owner, OwnerExpireTime)>();
                // Only remove PartyOwner if it exists (not all items with Owner have PartyOwner)
                if party_owner.is_some() {
                    entity_commands.remove::<PartyOwner>();
                }
            }
        }
    });
}
