use bevy::prelude::{Commands, MessageWriter, Res, ResMut};

use crate::game::{
    components::{GameClient, LoginClient, ServerInfo, WorldClient},
    events::SaveEvent,
    messages::control::{ClientType, ControlMessage},
    resources::{ControlChannel, GameServer, LoginTokens, ServerList, WorldServer},
};

pub fn control_server_system(
    mut commands: Commands,
    channel: Res<ControlChannel>,
    mut login_tokens: ResMut<LoginTokens>,
    mut server_list: ResMut<ServerList>,
    mut save_events: MessageWriter<SaveEvent>,
) {
    while let Ok(message) = channel.control_rx.try_recv() {
        match message {
            ControlMessage::AddClient {
                client_type,
                client_message_rx,
                server_message_tx,
                response_tx,
            } => {
                let entity = match client_type {
                    ClientType::Login => commands
                        .spawn(LoginClient::new(client_message_rx, server_message_tx))
                        .id(),
                    ClientType::World => commands
                        .spawn(WorldClient::new(client_message_rx, server_message_tx))
                        .id(),
                    ClientType::Game => commands
                        .spawn(GameClient::new(client_message_rx, server_message_tx))
                        .id(),
                };
                response_tx.send(entity).unwrap();
            }
            ControlMessage::RemoveClient {
                client_type,
                entity,
            } => match client_type {
                ClientType::Login => {
                    // Tokens stay until claimed by the world server or abandoned,
                    // see LoginTokens::remove_abandoned_tokens.
                    login_tokens.remove_login_client(entity);

                    commands.entity(entity).despawn();
                }
                ClientType::World => {
                    // Only this client's own token may be cleared / removed.
                    login_tokens.remove_world_client(entity);

                    commands.entity(entity).despawn();
                }
                ClientType::Game => {
                    login_tokens.remove_game_client(entity);

                    // Let the save system handle despawning the entity
                    save_events.write(SaveEvent::Character {
                        entity,
                        remove_after_save: true,
                    });
                    commands.entity(entity).remove::<GameClient>();
                }
            },
            ControlMessage::AddWorldServer {
                name,
                ip,
                port,
                packet_codec_seed,
                response_tx,
            } => {
                let entity = commands
                    .spawn(ServerInfo {
                        name: name.clone(),
                        ip: ip.clone(),
                        port,
                        packet_codec_seed,
                    })
                    .id();
                server_list.world_servers.push(WorldServer {
                    entity,
                    name,
                    ip,
                    port,
                    packet_codec_seed,
                    channels: Vec::new(),
                });
                response_tx.send(entity).unwrap();
            }
            ControlMessage::AddGameServer {
                world_server,
                name,
                ip,
                port,
                packet_codec_seed,
                response_tx,
            } => {
                let entity = commands
                    .spawn(ServerInfo {
                        name: name.clone(),
                        ip: ip.clone(),
                        port,
                        packet_codec_seed,
                    })
                    .id();
                let world_server = server_list
                    .world_servers
                    .iter_mut()
                    .find(|s| s.entity == world_server)
                    .unwrap();
                world_server.channels.push(GameServer {
                    entity,
                    name,
                    ip,
                    port,
                    packet_codec_seed,
                });
                response_tx.send(entity).unwrap();
            }
            ControlMessage::RemoveServer { entity } => {
                commands.entity(entity).despawn();
            }
        }
    }
}
