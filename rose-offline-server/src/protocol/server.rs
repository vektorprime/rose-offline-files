use bevy::ecs::prelude::Entity;
use lazy_static::__Deref;
use log::{info, warn};
use std::{sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

use crate::{
    game::messages::{
        control::{ClientType, ControlMessage},
        server::ServerMessage,
    },
    protocol::{Client, Connection, Protocol},
};

/// Pause after a failed accept, so a persistent error (e.g. out of file
/// handles) does not spin the listener.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);

/// Sends `RemoveClient` when dropped, so the client entity is always removed
/// (and a game character saved) however the connection task ends, including a
/// panic while handling a packet: tokio drops a panicked task's future.
struct RemoveClientOnDrop {
    control_message_tx: crossbeam_channel::Sender<ControlMessage>,
    client_type: ClientType,
    entity: Entity,
}

impl Drop for RemoveClientOnDrop {
    fn drop(&mut self) {
        self.control_message_tx
            .send(ControlMessage::RemoveClient {
                client_type: self.client_type,
                entity: self.entity,
            })
            .ok();
    }
}

/// Accepts the next connection, logging and retrying on accept errors instead
/// of panicking the listener task.
async fn accept_connection(listener: &TcpListener, server_name: &str) -> TcpStream {
    loop {
        match listener.accept().await {
            Ok((socket, _)) => return socket,
            Err(error) => {
                warn!("{} failed to accept connection: {}", server_name, error);
                // Sleep on the blocking pool: the backoff must not depend on
                // tokio's optional timer feature.
                tokio::task::spawn_blocking(|| std::thread::sleep(ACCEPT_ERROR_BACKOFF))
                    .await
                    .ok();
            }
        }
    }
}

async fn run_connection(
    stream: TcpStream,
    protocol: &Protocol,
    control_message_tx: crossbeam_channel::Sender<ControlMessage>,
) -> Result<(), anyhow::Error> {
    let (client_message_tx, client_message_rx) = crossbeam_channel::unbounded();
    let (server_message_tx, server_message_rx) =
        tokio::sync::mpsc::unbounded_channel::<ServerMessage>();
    let (response_tx, response_rx) = oneshot::channel();

    control_message_tx.send(ControlMessage::AddClient {
        client_type: protocol.client_type,
        server_message_tx,
        client_message_rx,
        response_tx,
    })?;

    let entity = response_rx.await?;
    let remove_client = RemoveClientOnDrop {
        control_message_tx,
        client_type: protocol.client_type,
        entity,
    };
    let mut client = Client {
        entity,
        connection: Connection::new(stream, protocol.packet_codec.deref()),
        client_message_tx,
        server_message_rx,
    };
    let result = (protocol.create_server)().run_client(&mut client).await;

    drop(remove_client);
    client.connection.shutdown().await;
    result
}

pub struct LoginServer {
    listener: TcpListener,
    protocol: Arc<Protocol>,
    control_message_tx: crossbeam_channel::Sender<ControlMessage>,
}

impl LoginServer {
    pub async fn new(
        listener: TcpListener,
        protocol: Arc<Protocol>,
        control_message_tx: crossbeam_channel::Sender<ControlMessage>,
    ) -> Result<LoginServer, anyhow::Error> {
        Ok(LoginServer {
            listener,
            protocol,
            control_message_tx,
        })
    }

    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                _ = async {
                    loop {
                        let socket = accept_connection(&self.listener, "Login Server").await;
                        let protocol = self.protocol.clone();
                        let control_message_tx = self.control_message_tx.clone();
                        tokio::spawn(async move {
                            if let Ok(addr) = socket.peer_addr() {
                                info!("Login Server new connection from: {:?}", addr);
                            }
                            if let Err(err) = run_connection(socket, protocol.deref(), control_message_tx).await {
                                info!("Login Server connection error: {:?}", err);
                            }
                        });
                    }
                } => {},
            };
        }
    }
}

pub struct WorldServer {
    entity: Entity,

    listener: TcpListener,
    protocol: Arc<Protocol>,
    control_message_tx: crossbeam_channel::Sender<ControlMessage>,
}

impl WorldServer {
    pub async fn new(
        name: String,
        listener: TcpListener,
        protocol: Arc<Protocol>,
        control_message_tx: crossbeam_channel::Sender<ControlMessage>,
    ) -> Result<WorldServer, anyhow::Error> {
        let (response_tx, response_rx) = oneshot::channel();
        let local_addr = listener.local_addr().unwrap();
        control_message_tx.send(ControlMessage::AddWorldServer {
            name,
            ip: local_addr.ip().to_string(),
            port: local_addr.port(),
            packet_codec_seed: protocol.packet_codec.get_seed(),
            response_tx,
        })?;
        let entity = response_rx.await?;

        Ok(WorldServer {
            entity,
            listener,
            protocol,
            control_message_tx,
        })
    }

    pub fn get_entity(&self) -> Entity {
        self.entity
    }

    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                _ = async {
                    loop {
                        let socket = accept_connection(&self.listener, "World Server").await;
                        let protocol = self.protocol.clone();
                        let control_message_tx = self.control_message_tx.clone();
                        tokio::spawn(async move {
                            if let Ok(addr) = socket.peer_addr() {
                                info!("World Server new connection from: {:?}", addr);
                            }
                            if let Err(err) = run_connection(socket, protocol.deref(), control_message_tx).await {
                                info!("World Server connection error: {:?}", err);
                            }
                        });
                    }
                } => {},
            };
        }

        // TODO: Allow server to exit gracefully
        #[allow(unreachable_code)]
        {
            self.control_message_tx
                .send(ControlMessage::RemoveServer {
                    entity: self.entity,
                })
                .ok();
        }
    }
}

pub struct GameServer {
    entity: Entity,

    listener: TcpListener,
    protocol: Arc<Protocol>,
    control_message_tx: crossbeam_channel::Sender<ControlMessage>,
}

impl GameServer {
    pub async fn new(
        name: String,
        world_server: Entity,
        listener: TcpListener,
        protocol: Arc<Protocol>,
        control_message_tx: crossbeam_channel::Sender<ControlMessage>,
    ) -> Result<GameServer, anyhow::Error> {
        let (response_tx, response_rx) = oneshot::channel();
        let local_addr = listener.local_addr().unwrap();
        control_message_tx.send(ControlMessage::AddGameServer {
            name,
            world_server,
            ip: local_addr.ip().to_string(),
            port: local_addr.port(),
            packet_codec_seed: protocol.packet_codec.get_seed(),
            response_tx,
        })?;
        let entity = response_rx.await?;

        Ok(GameServer {
            entity,
            listener,
            protocol,
            control_message_tx,
        })
    }

    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                _ = async {
                    loop {
                        let socket = accept_connection(&self.listener, "Game Server").await;
                        let protocol = self.protocol.clone();
                        let control_message_tx = self.control_message_tx.clone();
                        tokio::spawn(async move {
                            if let Ok(addr) = socket.peer_addr() {
                                info!("Game Server connection from: {:?}", addr);
                            }
                            if let Err(err) = run_connection(socket, protocol.deref(), control_message_tx).await {
                                info!("Game Server connection error: {:?}", err);
                            }
                        });
                    }
                } => {},
            };
        }

        // TODO: Allow server to exit gracefully
        #[allow(unreachable_code)]
        {
            self.control_message_tx
                .send(ControlMessage::RemoveServer {
                    entity: self.entity,
                })
                .ok();
        }
    }
}
