//! Protocol error types.

use std::fmt;

/// Errors that can occur during packet encoding or decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// The magic bytes at the start of the packet do not match `MAGIC`.
    InvalidMagic { found: [u8; 4] },

    /// The protocol version is not supported.
    UnsupportedVersion { found: u8, expected: u8 },

    /// The type byte does not map to any known `PacketType`.
    UnknownPacketType { type_byte: u8 },

    /// The buffer is too short to contain a full header.
    InsufficientHeader { expected: usize, actual: usize },

    /// The buffer is too short to contain the full payload declared by the
    /// `Length` field.
    TruncatedPayload { declared: u32, available: usize },

    /// The `Length` field exceeds the maximum allowed payload size.
    /// This prevents unbounded allocation from a malicious or corrupt packet.
    PayloadTooLarge { declared: u32, max: u32 },
}

/// Maximum payload size we will accept (16 MiB).
///
/// A single full-resolution screenshot rarely exceeds 10 MiB as PNG;
/// 16 MiB gives comfortable headroom without allowing a malformed length
/// field to trigger multi-gigabyte allocations.
pub const MAX_PAYLOAD_SIZE: u32 = 16 * 1024 * 1024;

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic { found } => {
                write!(f, "invalid magic bytes: {found:02X?}")
            }
            Self::UnsupportedVersion { found, expected } => {
                write!(
                    f,
                    "unsupported protocol version {found} (expected {expected})"
                )
            }
            Self::UnknownPacketType { type_byte } => {
                write!(f, "unknown packet type: 0x{type_byte:02X}")
            }
            Self::InsufficientHeader { expected, actual } => {
                write!(
                    f,
                    "buffer too short for header: need {expected} bytes, got {actual}"
                )
            }
            Self::TruncatedPayload { declared, available } => {
                write!(
                    f,
                    "truncated payload: header declares {declared} bytes \
                     but only {available} available"
                )
            }
            Self::PayloadTooLarge { declared, max } => {
                write!(
                    f,
                    "payload length {declared} exceeds maximum {max}"
                )
            }
        }
    }
}

impl std::error::Error for ProtocolError {}
