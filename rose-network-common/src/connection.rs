use bytes::{Buf, BufMut, Bytes, BytesMut};
use log::{error, trace, warn};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
    net::TcpStream,
};

use crate::{packet::recycle_packet_buffer, Packet, PacketCodec};

/// Size of the plain packet header: u16 size, u16 command, u16 reserved.
const PACKET_HEADER_SIZE: usize = 6;

#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error("connection lost")]
    ConnectionLost,

    #[error("failed to decrypt packet header")]
    DecryptHeaderFailed,

    #[error("failed to decrypt packet body")]
    DecryptBodyFailed,
}

pub struct Connection<'a> {
    stream: BufWriter<TcpStream>,
    buffer: BytesMut,
    /// Frame length of a packet whose header has already been decrypted in place
    /// at the front of `buffer`. Kept on the connection (not in a local) because
    /// `read_packet` runs inside `tokio::select!`: when another branch wins, the
    /// cancelled call must resume this frame rather than decrypt the header twice.
    pending_frame_length: Option<usize>,
    /// Reused encode buffer so sending a packet does not allocate.
    write_buffer: BytesMut,
    packet_codec: &'a (dyn PacketCodec + Send + Sync),
}

impl<'a> Connection<'a> {
    pub fn new(socket: TcpStream, packet_codec: &'a (dyn PacketCodec + Send + Sync)) -> Self {
        // Game traffic is small latency-sensitive packets: without TCP_NODELAY,
        // Nagle holds them back until the peer's (delayed) ACK arrives.
        if let Err(error) = socket.set_nodelay(true) {
            warn!("Failed to set TCP_NODELAY: {}", error);
        }

        Self {
            stream: BufWriter::new(socket),
            buffer: BytesMut::with_capacity(4 * 1024),
            pending_frame_length: None,
            write_buffer: BytesMut::with_capacity(1024),
            packet_codec,
        }
    }

    pub async fn shutdown(&mut self) {
        let _ = self.stream.shutdown().await;
    }

    /// Reads the next packet.
    ///
    /// Cancel safe: `read_buf` is cancel safe and all partial-frame state (the
    /// buffered bytes and the decrypted header length) lives in `self`, so a call
    /// dropped by `tokio::select!` loses nothing and the next call continues
    /// the same frame.
    pub async fn read_packet(&mut self) -> Result<Packet, anyhow::Error> {
        loop {
            let read_length = self.pending_frame_length.unwrap_or(PACKET_HEADER_SIZE);
            while self.buffer.len() < read_length {
                match self.stream.read_buf(&mut self.buffer).await {
                    // BytesMut always has spare capacity, so reading 0 bytes means
                    // EOF. Checking for an empty buffer instead would spin forever
                    // when the peer disconnects mid-frame.
                    Ok(0) | Err(_) => {
                        return Err(ConnectionError::ConnectionLost.into());
                    }
                    Ok(_) => {}
                }
            }

            let Some(read_length) = self.pending_frame_length else {
                let frame_length = self.packet_codec.decrypt_packet_header(&mut self.buffer);
                if frame_length == 0 {
                    return Err(ConnectionError::DecryptHeaderFailed.into());
                }
                self.pending_frame_length = Some(frame_length);
                continue;
            };
            self.pending_frame_length = None;

            if self.packet_codec.decrypt_packet_body(&mut self.buffer) {
                // Read packet into size, command, data
                let size = self.buffer.get_u16_le() as usize;
                let command = self.buffer.get_u16_le();

                if size < 6 || size > read_length {
                    // Malformed frame: the frame length is still known from the
                    // header, so drop the bad frame and resync to the next one.
                    self.buffer.advance(read_length.saturating_sub(4));
                    return Err(ConnectionError::DecryptBodyFailed.into());
                }

                self.buffer.advance(2);
                let data: Bytes = self.buffer.split_to(size - 6).into();

                // Packets can contain random amount of padding at end
                self.buffer.advance(read_length - size);

                trace!(target: "packets", "RECV [{:03X}] {:02x?}", command, &data[..]);
                return Ok(Packet { command, data });
            } else {
                // Checksum mismatch: the frame length is still known from the
                // header, so drop the bad frame and resync to the next one.
                self.buffer.advance(read_length);
                return Err(ConnectionError::DecryptBodyFailed.into());
            }
        }
    }

    /// Encodes and writes a packet into the connection's write buffer without
    /// flushing it. Call [`Connection::flush`] after writing a batch of packets.
    ///
    /// A packet too large for the codec's frame header cannot be encoded; it is
    /// dropped with an error log instead of failing the connection.
    pub async fn write_packet_buffered(&mut self, packet: Packet) -> Result<(), anyhow::Error> {
        trace!(target: "packets", "SEND [{:03X}] {:02x?}", packet.command, &packet.data[..]);

        let size = packet.data.len() + PACKET_HEADER_SIZE;
        let max_packet_size = self.packet_codec.max_packet_size();
        if size > max_packet_size {
            error!(
                "Dropping packet [{:03X}] of {} bytes, larger than the maximum packet size of {} bytes",
                packet.command, size, max_packet_size
            );
            return Ok(());
        }

        self.write_buffer.clear();
        self.write_buffer.reserve(size);
        self.write_buffer.put_u16_le(size as u16);
        self.write_buffer.put_u16_le(packet.command);
        self.write_buffer.put_u16_le(0);
        self.write_buffer.put_slice(&packet.data);
        self.packet_codec.encrypt_packet(&mut self.write_buffer);
        recycle_packet_buffer(packet.data);

        self.stream
            .write_all(&self.write_buffer)
            .await
            .map_err(|_| ConnectionError::ConnectionLost)?;

        Ok(())
    }

    /// Flushes all packets written with [`Connection::write_packet_buffered`].
    pub async fn flush(&mut self) -> Result<(), anyhow::Error> {
        self.stream
            .flush()
            .await
            .map_err(|_| ConnectionError::ConnectionLost)?;

        Ok(())
    }

    /// Writes a single packet and flushes it immediately.
    pub async fn write_packet(&mut self, packet: Packet) -> Result<(), anyhow::Error> {
        self.write_packet_buffered(packet).await?;
        self.flush().await
    }
}
