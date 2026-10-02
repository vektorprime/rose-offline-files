use async_trait::async_trait;
use thiserror::Error;

use rose_game_common::messages::{client::ClientMessage, server::ServerMessage};
use rose_network_common::{Connection, PacketCodec};

use crate::game::messages::control::ClientType;

pub struct Client<'a> {
    pub entity: bevy::ecs::prelude::Entity,
    pub connection: Connection<'a>,
    pub client_message_tx: crossbeam_channel::Sender<ClientMessage>,
    pub server_message_rx: tokio::sync::mpsc::UnboundedReceiver<ServerMessage>,
}

#[derive(Debug, Error)]
pub enum ProtocolServerError {
    #[error("server initiated disconnect")]
    ServerInitiatedDisconnect,
}

#[async_trait]
pub trait ProtocolServer {
    async fn run_client(&mut self, client: &mut Client) -> Result<(), anyhow::Error>;
}

pub struct Protocol {
    pub client_type: ClientType,
    pub packet_codec: Box<dyn PacketCodec + Send + Sync>,
    pub create_server: fn() -> Box<dyn ProtocolServer + Send + Sync>,
}

pub mod server;

/// Most server messages written before one flush. Bounds how long a burst of
/// queued messages can delay reading from the client.
pub const MAX_SERVER_MESSAGES_PER_FLUSH: usize = 256;

#[macro_export]
macro_rules! implement_protocol_server {
    ( $x:ident ) => {
        #[async_trait]
        impl ProtocolServer for $x {
            async fn run_client(&mut self, client: &mut Client) -> Result<(), anyhow::Error> {
                loop {
                    tokio::select! {
                        packet = client.connection.read_packet() => {
                            match packet {
                                Ok(packet) => {
                                    match self.handle_packet(client, &packet).await {
                                        Ok(_) => {},
                                        Err(error) => {
                                            log::warn!("RECV [{:03X}] {:02x?}", packet.command, &packet.data[..]);
                                            return Err(error);
                                        },
                                    }
                                },
                                Err(error) => {
                                    return Err(error);
                                }
                            }
                        },
                        server_message = client.server_message_rx.recv() => {
                            if let Some(message) = server_message {
                                // Write everything already queued (usually a whole game
                                // tick's worth) and flush once, instead of one flush and
                                // send syscall per packet. Bytes and order are unchanged.
                                let mut result = self.handle_server_message(client, message).await;
                                let mut message_count = 1;
                                while result.is_ok()
                                    && message_count < $crate::protocol::MAX_SERVER_MESSAGES_PER_FLUSH
                                {
                                    let Ok(message) = client.server_message_rx.try_recv() else {
                                        break;
                                    };
                                    result = self.handle_server_message(client, message).await;
                                    message_count += 1;
                                }

                                // Flush even when a handler failed: e.g. LogoutSuccess
                                // writes its reply and then requests the disconnect.
                                let flush_result = client.connection.flush().await;
                                result?;
                                flush_result?;
                            } else {
                                return Err(ProtocolServerError::ServerInitiatedDisconnect.into());
                            }
                        }
                    };
                }
            }
        }
    };
}
