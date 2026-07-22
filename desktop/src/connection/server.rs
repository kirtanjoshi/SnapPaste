//! TCP Server implementation for Desktop SnapPaste.
//!
//! Listens on a local port, processes incoming packets, and manages the lifecycle.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use crate::protocol::{decode, encode, Packet, HEADER_SIZE};

/// Reads a single parsed `Packet` from a raw TCP stream.
///
/// Ensures safe limits on length allocation to avoid out-of-memory errors.
pub fn read_packet(stream: &mut TcpStream) -> io::Result<Packet> {
    // 1. Read header
    let mut header = [0u8; HEADER_SIZE];
    stream.read_exact(&mut header)?;

    // 2. Parse payload length from header bytes 16..20 (little-endian)
    let payload_len = u32::from_le_bytes(header[16..20].try_into().unwrap());
    
    // Check limit
    const MAX_LIMIT: u32 = 16 * 1024 * 1024; // 16 MiB
    if payload_len > MAX_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Payload length {} exceeds limit {}", payload_len, MAX_LIMIT),
        ));
    }

    // 3. Read payload
    let mut payload = vec![0u8; payload_len as usize];
    if payload_len > 0 {
        stream.read_exact(&mut payload)?;
    }

    // 4. Decode the combined buffer
    let mut full_packet = Vec::with_capacity(HEADER_SIZE + payload.len());
    full_packet.extend_from_slice(&header);
    full_packet.extend_from_slice(&payload);

    decode(&full_packet)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Decode error: {}", e)))
}

/// Writes a single `Packet` into a raw TCP stream.
pub fn write_packet(stream: &mut TcpStream, packet: &Packet) -> io::Result<()> {
    let bytes = encode(packet);
    stream.write_all(&bytes)
}
