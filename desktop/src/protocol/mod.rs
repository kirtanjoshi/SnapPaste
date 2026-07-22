//! Wire protocol module for SnapPaste.
//!
//! Implements the custom binary framing protocol defined in Architecture.md §5.
//! This module handles packet encoding/decoding only — no socket I/O.

mod packet;
mod error;

pub use packet::{decode, encode, Packet, PacketType, HEADER_SIZE, MAGIC, PROTOCOL_VERSION};
pub use error::ProtocolError;

#[cfg(test)]
mod tests;
