use bytes::{Buf, BufMut, Bytes, BytesMut};
use encoding_rs::WINDOWS_1252;
use std::borrow::Cow;
use std::cell::RefCell;
use std::io::Cursor;
use std::num::NonZeroU16;
use std::str;
use thiserror::Error;

/// Initial capacity of a `PacketWriter` buffer.
const PACKET_BUFFER_CAPACITY: usize = 1024;

/// Recycled buffers larger than this are dropped instead of kept in the pool.
const PACKET_BUFFER_MAX_RECYCLE_CAPACITY: usize = 4 * PACKET_BUFFER_CAPACITY;

/// Maximum number of idle buffers kept per thread.
const PACKET_BUFFER_POOL_SIZE: usize = 16;

thread_local! {
    // Packets are built and written by the same network task, so a small
    // per-thread pool lets each `PacketWriter` reuse the buffer of a packet
    // that was already sent instead of allocating 1 KB for every packet.
    static PACKET_BUFFER_POOL: RefCell<Vec<BytesMut>> = const { RefCell::new(Vec::new()) };
}

fn take_packet_buffer() -> BytesMut {
    PACKET_BUFFER_POOL
        .with(|pool| pool.borrow_mut().pop())
        .unwrap_or_else(|| BytesMut::with_capacity(PACKET_BUFFER_CAPACITY))
}

/// Returns the buffer of a sent packet to the pool, if no other handle still
/// references it.
pub(crate) fn recycle_packet_buffer(data: Bytes) {
    let Ok(mut buffer) = data.try_into_mut() else {
        return;
    };

    if buffer.capacity() < PACKET_BUFFER_CAPACITY
        || buffer.capacity() > PACKET_BUFFER_MAX_RECYCLE_CAPACITY
    {
        return;
    }

    buffer.clear();
    PACKET_BUFFER_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool.len() < PACKET_BUFFER_POOL_SIZE {
            pool.push(buffer);
        }
    });
}

#[derive(Debug, Error)]
pub enum PacketError {
    #[error("unexpected end of packet")]
    UnexpectedEof,

    #[error("invalid packet")]
    InvalidPacket,
}

pub trait PacketCodec {
    fn get_seed(&self) -> u32;
    fn decrypt_packet_header(&self, buffer: &mut BytesMut) -> usize;
    fn decrypt_packet_body(&self, buffer: &mut BytesMut) -> bool;
    fn encrypt_packet(&self, buffer: &mut BytesMut);

    /// Largest encodable packet in bytes, including the 6 byte packet header.
    fn max_packet_size(&self) -> usize {
        u16::MAX as usize
    }
}

pub struct Packet {
    pub command: u16,
    pub data: Bytes,
}

impl Packet {
    pub fn with_data(command: u16, data: BytesMut) -> Packet {
        Packet {
            command,
            data: data.freeze(),
        }
    }
}

impl std::fmt::Debug for Packet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Packet")
            .field("command", &format_args!("{:03X}", &self.command))
            .field("data", &format_args!("{:02x?}", &self.data[..]))
            .finish()
    }
}

pub struct PacketReader<'a> {
    cursor: Cursor<&'a [u8]>,
}

impl<'a> From<&'a Packet> for PacketReader<'a> {
    fn from(packet: &'a Packet) -> Self {
        Self {
            cursor: Cursor::new(&packet.data[..]),
        }
    }
}

impl<'a> PacketReader<'a> {
    pub fn read_i8(&mut self) -> Result<i8, PacketError> {
        if self.cursor.remaining() < 1 {
            Err(PacketError::UnexpectedEof)
        } else {
            Ok(self.cursor.get_i8())
        }
    }

    pub fn read_u8(&mut self) -> Result<u8, PacketError> {
        if self.cursor.remaining() < 1 {
            Err(PacketError::UnexpectedEof)
        } else {
            Ok(self.cursor.get_u8())
        }
    }

    pub fn read_i16(&mut self) -> Result<i16, PacketError> {
        if self.cursor.remaining() < 2 {
            Err(PacketError::UnexpectedEof)
        } else {
            Ok(self.cursor.get_i16_le())
        }
    }

    pub fn read_u16(&mut self) -> Result<u16, PacketError> {
        if self.cursor.remaining() < 2 {
            Err(PacketError::UnexpectedEof)
        } else {
            Ok(self.cursor.get_u16_le())
        }
    }

    pub fn read_nonzero_u16(&mut self) -> Result<NonZeroU16, PacketError> {
        NonZeroU16::new(self.read_u16()?).ok_or(PacketError::InvalidPacket)
    }

    pub fn read_i32(&mut self) -> Result<i32, PacketError> {
        if self.cursor.remaining() < 4 {
            Err(PacketError::UnexpectedEof)
        } else {
            Ok(self.cursor.get_i32_le())
        }
    }

    pub fn read_u32(&mut self) -> Result<u32, PacketError> {
        if self.cursor.remaining() < 4 {
            Err(PacketError::UnexpectedEof)
        } else {
            Ok(self.cursor.get_u32_le())
        }
    }

    pub fn read_i64(&mut self) -> Result<i64, PacketError> {
        if self.cursor.remaining() < 8 {
            Err(PacketError::UnexpectedEof)
        } else {
            Ok(self.cursor.get_i64_le())
        }
    }

    pub fn read_u64(&mut self) -> Result<u64, PacketError> {
        if self.cursor.remaining() < 8 {
            Err(PacketError::UnexpectedEof)
        } else {
            Ok(self.cursor.get_u64_le())
        }
    }

    pub fn read_f32(&mut self) -> Result<f32, PacketError> {
        if self.cursor.remaining() < 4 {
            Err(PacketError::UnexpectedEof)
        } else {
            Ok(self.cursor.get_f32_le())
        }
    }

    pub fn read_fixed_length_bytes(&mut self, length: usize) -> Result<&'a [u8], PacketError> {
        if self.cursor.remaining() < length {
            Err(PacketError::UnexpectedEof)
        } else {
            let start = self.cursor.position() as usize;
            let end = start + length;
            self.cursor.set_position(end as u64);
            Ok(&self.cursor.get_ref()[start..end])
        }
    }

    pub fn read_null_terminated_bytes(&mut self) -> Result<&'a [u8], PacketError> {
        let start = self.cursor.position() as usize;
        let end = self.cursor.get_ref().as_ref().len();

        for i in start..end {
            if self.cursor.get_ref()[i] == 0 {
                self.cursor.set_position((i + 1) as u64);
                return Ok(&self.cursor.get_ref()[start..i]);
            }
        }

        Err(PacketError::UnexpectedEof)
    }

    pub fn read_fixed_length_utf8(&mut self, length: usize) -> Result<&'a str, PacketError> {
        match str::from_utf8(self.read_fixed_length_bytes(length)?) {
            Ok(s) => Ok(s.trim_end_matches(char::from(0))),
            Err(_) => Err(PacketError::InvalidPacket),
        }
    }

    pub fn read_null_terminated_utf8(&mut self) -> Result<&'a str, PacketError> {
        match str::from_utf8(self.read_null_terminated_bytes()?) {
            Ok(s) => Ok(s.trim_end_matches(char::from(0))),
            Err(_) => Err(PacketError::InvalidPacket),
        }
    }

    pub fn read_null_terminated_win1252(&mut self) -> Result<Cow<str>, PacketError> {
        let (decoded, _, _) = WINDOWS_1252.decode(self.read_null_terminated_bytes()?);
        Ok(decoded)
    }
}

pub struct PacketWriter {
    command: u16,
    data: BytesMut,
}

impl PacketWriter {
    pub fn new(command: u16) -> PacketWriter {
        PacketWriter {
            command,
            data: take_packet_buffer(),
        }
    }

    pub fn write_bytes(&mut self, value: &[u8]) {
        self.data.put(value);
    }

    pub fn write_i8(&mut self, value: i8) {
        self.data.put_i8(value);
    }

    pub fn write_u8(&mut self, value: u8) {
        self.data.put_u8(value);
    }

    pub fn write_i16(&mut self, value: i16) {
        self.data.put_i16_le(value);
    }

    pub fn write_u16(&mut self, value: u16) {
        self.data.put_u16_le(value);
    }

    pub fn write_f32(&mut self, value: f32) {
        self.data.put_f32_le(value);
    }

    pub fn write_i32(&mut self, value: i32) {
        self.data.put_i32_le(value);
    }

    pub fn write_u32(&mut self, value: u32) {
        self.data.put_u32_le(value);
    }

    pub fn write_i64(&mut self, value: i64) {
        self.data.put_i64_le(value);
    }

    pub fn write_u64(&mut self, value: u64) {
        self.data.put_u64_le(value);
    }

    pub fn write_null_terminated_utf8(&mut self, value: &str) {
        self.data.put(value.as_bytes());
        self.data.put_u8(0);
    }

    pub fn write_fixed_length_utf8(&mut self, value: &str, length: usize) {
        if value.len() > length {
            self.data.put(&value.as_bytes()[0..length]);
        } else {
            self.data.put(value.as_bytes());
            for _ in value.len()..length {
                self.data.put_u8(0);
            }
        }
    }
}

impl From<PacketWriter> for Packet {
    fn from(writer: PacketWriter) -> Packet {
        Packet::with_data(writer.command, writer.data)
    }
}
