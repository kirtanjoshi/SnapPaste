//! Packet struct and encode/decode logic for the SnapPaste wire protocol.
//!
//! ## Header Layout (20 bytes, little-endian)
//!
//! | Field    | Offset | Size | Description                                  |
//! |----------|--------|------|----------------------------------------------|
//! | Magic    |  0     |  4   | `0x53 0x4E 0x50 0x50` ("SNPP")               |
//! | Version  |  4     |  1   | Protocol version (currently `1`)              |
//! | Type     |  5     |  1   | [`PacketType`] discriminant                   |
//! | Flags    |  6     |  2   | Reserved, little-endian                       |
//! | ImageID  |  8     |  4   | Monotonically increasing per screenshot       |
//! | Sequence | 12     |  4   | Per-connection monotonic packet counter       |
//! | Length   | 16     |  4   | Payload length in bytes                       |
//! | Payload  | 20     |  N   | Variable-length payload                       |

use crate::protocol::error::{ProtocolError, MAX_PAYLOAD_SIZE};

// ── Constants ────────────────────────────────────────────────────────────────

/// Magic bytes identifying a SnapPaste packet: ASCII "SNPP".
pub const MAGIC: [u8; 4] = [0x53, 0x4E, 0x50, 0x50];

/// Current protocol version.
pub const PROTOCOL_VERSION: u8 = 1;

/// Fixed header size in bytes.
pub const HEADER_SIZE: usize = 20;

// ── PacketType ───────────────────────────────────────────────────────────────

/// Exhaustive enumeration of all wire protocol message types.
///
/// Each variant maps to a single byte on the wire. The discriminant values are
/// chosen to leave room for future additions without breaking compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PacketType {
    /// Initial handshake packet sent by either side upon TCP connection.
    Hello        = 0x01,
    /// Authentication / pairing-token exchange.
    Auth         = 0x02,
    /// Notification that a new screenshot is available (Android → Desktop).
    NewScreenshot = 0x03,
    /// Begin a chunked image transfer; carries metadata (dimensions, format).
    StartImage   = 0x10,
    /// A single chunk of image data.
    Chunk        = 0x11,
    /// Cancel the current in-flight image transfer.
    Cancel       = 0x12,
    /// Signals the final chunk has been sent; image is complete.
    EndImage     = 0x13,
    /// Generic acknowledgement.
    Ack          = 0x20,
    /// Keep-alive request.
    Ping         = 0x30,
    /// Keep-alive response.
    Pong         = 0x31,
    /// Graceful disconnect.
    Disconnect   = 0x40,
    /// Error report (human-readable message in payload).
    Error        = 0xFF,
}

impl PacketType {
    /// Try to convert a raw byte into a [`PacketType`].
    pub fn from_byte(b: u8) -> Option<PacketType> {
        match b {
            0x01 => Some(Self::Hello),
            0x02 => Some(Self::Auth),
            0x03 => Some(Self::NewScreenshot),
            0x10 => Some(Self::StartImage),
            0x11 => Some(Self::Chunk),
            0x12 => Some(Self::Cancel),
            0x13 => Some(Self::EndImage),
            0x20 => Some(Self::Ack),
            0x30 => Some(Self::Ping),
            0x31 => Some(Self::Pong),
            0x40 => Some(Self::Disconnect),
            0xFF => Some(Self::Error),
            _    => None,
        }
    }

    /// Return the wire byte for this type.
    pub fn to_byte(self) -> u8 {
        self as u8
    }
}

// ── Packet ───────────────────────────────────────────────────────────────────

/// A fully parsed SnapPaste wire-protocol packet.
///
/// Construct via [`Packet::new`] (for sending) or [`decode`] (from raw bytes).
/// Serialize with [`encode`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    /// Protocol version (should be [`PROTOCOL_VERSION`]).
    pub version: u8,
    /// The type of this packet.
    pub packet_type: PacketType,
    /// Reserved flags (little-endian on the wire).
    pub flags: u16,
    /// Image identifier — monotonically increasing per screenshot.
    /// Required on `StartImage`, `Chunk`, `Cancel`, `EndImage`.
    pub image_id: u32,
    /// Per-connection monotonic packet counter (for debugging/logging).
    pub sequence: u32,
    /// Variable-length payload.
    pub payload: Vec<u8>,
}

impl Packet {
    /// Create a new packet with the current protocol version.
    pub fn new(
        packet_type: PacketType,
        flags: u16,
        image_id: u32,
        sequence: u32,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            packet_type,
            flags,
            image_id,
            sequence,
            payload,
        }
    }
}

// ── Encode ───────────────────────────────────────────────────────────────────

/// Serialize a [`Packet`] into a `Vec<u8>` ready for transmission.
///
/// The output is `HEADER_SIZE + packet.payload.len()` bytes long.
/// All multi-byte fields are written in **little-endian** order per the spec.
pub fn encode(packet: &Packet) -> Vec<u8> {
    let payload_len = packet.payload.len() as u32;
    let total = HEADER_SIZE + packet.payload.len();
    let mut buf = Vec::with_capacity(total);

    // Magic (4 bytes)
    buf.extend_from_slice(&MAGIC);
    // Version (1 byte)
    buf.push(packet.version);
    // Type (1 byte)
    buf.push(packet.packet_type.to_byte());
    // Flags (2 bytes, little-endian)
    buf.extend_from_slice(&packet.flags.to_le_bytes());
    // ImageID (4 bytes, little-endian)
    buf.extend_from_slice(&packet.image_id.to_le_bytes());
    // Sequence (4 bytes, little-endian)
    buf.extend_from_slice(&packet.sequence.to_le_bytes());
    // Length (4 bytes, little-endian)
    buf.extend_from_slice(&payload_len.to_le_bytes());
    // Payload
    buf.extend_from_slice(&packet.payload);

    buf
}

// ── Decode ───────────────────────────────────────────────────────────────────

/// Deserialize a [`Packet`] from raw bytes.
///
/// # Errors
///
/// Returns [`ProtocolError`] if:
/// - The buffer is shorter than [`HEADER_SIZE`] bytes.
/// - The magic bytes do not match [`MAGIC`].
/// - The version byte is not [`PROTOCOL_VERSION`].
/// - The type byte does not correspond to a known [`PacketType`].
/// - The declared payload length exceeds [`MAX_PAYLOAD_SIZE`].
/// - The buffer does not contain enough bytes for the declared payload.
pub fn decode(buf: &[u8]) -> Result<Packet, ProtocolError> {
    // ── Header length check ──────────────────────────────────────────────
    if buf.len() < HEADER_SIZE {
        return Err(ProtocolError::InsufficientHeader {
            expected: HEADER_SIZE,
            actual: buf.len(),
        });
    }

    // ── Magic ────────────────────────────────────────────────────────────
    let magic: [u8; 4] = buf[0..4].try_into().unwrap();
    if magic != MAGIC {
        return Err(ProtocolError::InvalidMagic { found: magic });
    }

    // ── Version ──────────────────────────────────────────────────────────
    let version = buf[4];
    if version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion {
            found: version,
            expected: PROTOCOL_VERSION,
        });
    }

    // ── Type ─────────────────────────────────────────────────────────────
    let type_byte = buf[5];
    let packet_type = PacketType::from_byte(type_byte).ok_or(
        ProtocolError::UnknownPacketType { type_byte },
    )?;

    // ── Flags ────────────────────────────────────────────────────────────
    let flags = u16::from_le_bytes(buf[6..8].try_into().unwrap());

    // ── ImageID ──────────────────────────────────────────────────────────
    let image_id = u32::from_le_bytes(buf[8..12].try_into().unwrap());

    // ── Sequence ─────────────────────────────────────────────────────────
    let sequence = u32::from_le_bytes(buf[12..16].try_into().unwrap());

    // ── Length ────────────────────────────────────────────────────────────
    let payload_len = u32::from_le_bytes(buf[16..20].try_into().unwrap());

    // Guard: reject oversized length before allocating.
    if payload_len > MAX_PAYLOAD_SIZE {
        return Err(ProtocolError::PayloadTooLarge {
            declared: payload_len,
            max: MAX_PAYLOAD_SIZE,
        });
    }

    // Guard: reject truncated payload.
    let available = buf.len() - HEADER_SIZE;
    if (payload_len as usize) > available {
        return Err(ProtocolError::TruncatedPayload {
            declared: payload_len,
            available,
        });
    }

    // ── Payload ──────────────────────────────────────────────────────────
    let payload = buf[HEADER_SIZE..HEADER_SIZE + payload_len as usize].to_vec();

    Ok(Packet {
        version,
        packet_type,
        flags,
        image_id,
        sequence,
        payload,
    })
}
