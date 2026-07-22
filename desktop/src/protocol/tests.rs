//! Unit tests for the SnapPaste wire protocol.
//!
//! Test matrix mirrors the Android (Kotlin) side so both implementations can be
//! verified independently against the same spec.

use super::error::{ProtocolError, MAX_PAYLOAD_SIZE};
use super::packet::{decode, encode, Packet, PacketType, HEADER_SIZE, MAGIC, PROTOCOL_VERSION};

// ─── Helpers ────────────────────────────────────────────────────────────────

/// All packet types in the protocol, used for exhaustive iteration.
const ALL_TYPES: [PacketType; 12] = [
    PacketType::Hello,
    PacketType::Auth,
    PacketType::NewScreenshot,
    PacketType::StartImage,
    PacketType::Chunk,
    PacketType::Cancel,
    PacketType::EndImage,
    PacketType::Ack,
    PacketType::Ping,
    PacketType::Pong,
    PacketType::Disconnect,
    PacketType::Error,
];

/// Build a test packet with the given type and an optional payload.
fn make_packet(ptype: PacketType, payload: Vec<u8>) -> Packet {
    Packet::new(ptype, 0x0000, 42, 7, payload)
}

// ═══════════════════════════════════════════════════════════════════════════
// Round-trip tests: encode → decode for every packet type
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn round_trip_hello() {
    let pkt = make_packet(PacketType::Hello, vec![]);
    assert_eq!(decode(&encode(&pkt)).unwrap(), pkt);
}

#[test]
fn round_trip_auth() {
    let pkt = make_packet(PacketType::Auth, b"token123".to_vec());
    assert_eq!(decode(&encode(&pkt)).unwrap(), pkt);
}

#[test]
fn round_trip_new_screenshot() {
    let pkt = make_packet(PacketType::NewScreenshot, vec![]);
    assert_eq!(decode(&encode(&pkt)).unwrap(), pkt);
}

#[test]
fn round_trip_start_image() {
    // Payload: 4 bytes width + 4 bytes height (LE) as a minimal metadata example
    let mut meta = Vec::new();
    meta.extend_from_slice(&1080u32.to_le_bytes());
    meta.extend_from_slice(&1920u32.to_le_bytes());
    let pkt = Packet::new(PacketType::StartImage, 0, 1, 0, meta);
    assert_eq!(decode(&encode(&pkt)).unwrap(), pkt);
}

#[test]
fn round_trip_chunk() {
    let pkt = Packet::new(PacketType::Chunk, 0, 1, 1, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(decode(&encode(&pkt)).unwrap(), pkt);
}

#[test]
fn round_trip_cancel() {
    let pkt = Packet::new(PacketType::Cancel, 0, 1, 2, vec![]);
    assert_eq!(decode(&encode(&pkt)).unwrap(), pkt);
}

#[test]
fn round_trip_end_image() {
    let pkt = Packet::new(PacketType::EndImage, 0, 1, 3, vec![]);
    assert_eq!(decode(&encode(&pkt)).unwrap(), pkt);
}

#[test]
fn round_trip_ack() {
    let pkt = make_packet(PacketType::Ack, vec![]);
    assert_eq!(decode(&encode(&pkt)).unwrap(), pkt);
}

#[test]
fn round_trip_ping() {
    let pkt = make_packet(PacketType::Ping, vec![]);
    assert_eq!(decode(&encode(&pkt)).unwrap(), pkt);
}

#[test]
fn round_trip_pong() {
    let pkt = make_packet(PacketType::Pong, vec![]);
    assert_eq!(decode(&encode(&pkt)).unwrap(), pkt);
}

#[test]
fn round_trip_disconnect() {
    let pkt = make_packet(PacketType::Disconnect, vec![]);
    assert_eq!(decode(&encode(&pkt)).unwrap(), pkt);
}

#[test]
fn round_trip_error() {
    let pkt = make_packet(PacketType::Error, b"something went wrong".to_vec());
    assert_eq!(decode(&encode(&pkt)).unwrap(), pkt);
}

/// Exhaustive: ensure every variant survives a round-trip.
#[test]
fn round_trip_all_types_exhaustive() {
    for (i, &ptype) in ALL_TYPES.iter().enumerate() {
        let pkt = Packet::new(ptype, 0, i as u32, i as u32, vec![i as u8; i]);
        let encoded = encode(&pkt);
        let decoded = decode(&encoded).unwrap_or_else(|e| {
            panic!("decode failed for {:?}: {}", ptype, e);
        });
        assert_eq!(decoded, pkt, "round-trip mismatch for {:?}", ptype);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Header field tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn header_size_is_20_bytes() {
    let pkt = make_packet(PacketType::Ping, vec![]);
    assert_eq!(encode(&pkt).len(), HEADER_SIZE);
}

#[test]
fn magic_bytes_at_offset_0() {
    let pkt = make_packet(PacketType::Ping, vec![]);
    let buf = encode(&pkt);
    assert_eq!(&buf[0..4], &MAGIC);
}

#[test]
fn version_at_offset_4() {
    let pkt = make_packet(PacketType::Ping, vec![]);
    let buf = encode(&pkt);
    assert_eq!(buf[4], PROTOCOL_VERSION);
}

#[test]
fn flags_preserved() {
    let pkt = Packet::new(PacketType::Hello, 0xABCD, 0, 0, vec![]);
    let decoded = decode(&encode(&pkt)).unwrap();
    assert_eq!(decoded.flags, 0xABCD);
}

#[test]
fn image_id_preserved() {
    let pkt = Packet::new(PacketType::StartImage, 0, 0xDEAD_BEEF, 0, vec![]);
    let decoded = decode(&encode(&pkt)).unwrap();
    assert_eq!(decoded.image_id, 0xDEAD_BEEF);
}

#[test]
fn sequence_preserved() {
    let pkt = Packet::new(PacketType::Chunk, 0, 0, 0xCAFE_BABE, vec![0x01]);
    let decoded = decode(&encode(&pkt)).unwrap();
    assert_eq!(decoded.sequence, 0xCAFE_BABE);
}

#[test]
fn payload_length_field_matches() {
    let payload = vec![0xAA; 256];
    let pkt = make_packet(PacketType::Chunk, payload.clone());
    let buf = encode(&pkt);
    let length_field = u32::from_le_bytes(buf[16..20].try_into().unwrap());
    assert_eq!(length_field, 256);
    assert_eq!(buf.len(), HEADER_SIZE + 256);
}

// ═══════════════════════════════════════════════════════════════════════════
// Decode error tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn reject_malformed_magic() {
    let pkt = make_packet(PacketType::Ping, vec![]);
    let mut buf = encode(&pkt);
    buf[0] = 0x00; // corrupt first magic byte
    let err = decode(&buf).unwrap_err();
    assert!(
        matches!(err, ProtocolError::InvalidMagic { .. }),
        "expected InvalidMagic, got {:?}",
        err
    );
}

#[test]
fn reject_completely_wrong_magic() {
    let pkt = make_packet(PacketType::Ping, vec![]);
    let mut buf = encode(&pkt);
    buf[0..4].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    let err = decode(&buf).unwrap_err();
    assert!(matches!(err, ProtocolError::InvalidMagic { found } if found == [0xFF; 4]));
}

#[test]
fn reject_unsupported_version() {
    let pkt = make_packet(PacketType::Ping, vec![]);
    let mut buf = encode(&pkt);
    buf[4] = 99;
    let err = decode(&buf).unwrap_err();
    assert!(
        matches!(err, ProtocolError::UnsupportedVersion { found: 99, expected: 1 }),
        "expected UnsupportedVersion, got {:?}",
        err
    );
}

#[test]
fn reject_unknown_type() {
    let pkt = make_packet(PacketType::Ping, vec![]);
    let mut buf = encode(&pkt);
    buf[5] = 0x99; // no such type
    let err = decode(&buf).unwrap_err();
    assert!(
        matches!(err, ProtocolError::UnknownPacketType { type_byte: 0x99 }),
        "expected UnknownPacketType, got {:?}",
        err
    );
}

#[test]
fn reject_truncated_header() {
    let buf = vec![0x53, 0x4E, 0x50]; // only 3 bytes
    let err = decode(&buf).unwrap_err();
    assert!(
        matches!(err, ProtocolError::InsufficientHeader { expected: 20, actual: 3 }),
        "expected InsufficientHeader, got {:?}",
        err
    );
}

#[test]
fn reject_empty_buffer() {
    let err = decode(&[]).unwrap_err();
    assert!(matches!(err, ProtocolError::InsufficientHeader { expected: 20, actual: 0 }));
}

#[test]
fn reject_truncated_payload() {
    let pkt = make_packet(PacketType::Auth, b"secret_token".to_vec());
    let buf = encode(&pkt);
    // Chop off the last 5 bytes of payload
    let truncated = &buf[..buf.len() - 5];
    let err = decode(truncated).unwrap_err();
    assert!(
        matches!(err, ProtocolError::TruncatedPayload { .. }),
        "expected TruncatedPayload, got {:?}",
        err
    );
}

#[test]
fn reject_oversized_length_no_panic_no_alloc() {
    let pkt = make_packet(PacketType::Chunk, vec![0x01]);
    let mut buf = encode(&pkt);
    // Set the length field to MAX_PAYLOAD_SIZE + 1
    let oversized = MAX_PAYLOAD_SIZE + 1;
    buf[16..20].copy_from_slice(&oversized.to_le_bytes());
    let err = decode(&buf).unwrap_err();
    assert!(
        matches!(err, ProtocolError::PayloadTooLarge { declared, max }
            if declared == oversized && max == MAX_PAYLOAD_SIZE),
        "expected PayloadTooLarge, got {:?}",
        err
    );
}

#[test]
fn reject_u32_max_length_safely() {
    let pkt = make_packet(PacketType::Chunk, vec![]);
    let mut buf = encode(&pkt);
    // Set the length field to u32::MAX — must not panic or try to allocate 4 GiB.
    buf[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    let err = decode(&buf).unwrap_err();
    assert!(
        matches!(err, ProtocolError::PayloadTooLarge { .. }),
        "expected PayloadTooLarge, got {:?}",
        err
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PacketType byte conversion
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn packet_type_round_trip() {
    for &ptype in &ALL_TYPES {
        let byte = ptype.to_byte();
        let back = PacketType::from_byte(byte).unwrap();
        assert_eq!(back, ptype, "type byte round-trip failed for {:?}", ptype);
    }
}

#[test]
fn packet_type_from_invalid_byte_returns_none() {
    // 0x00, 0x04, 0x50, 0xFE are not assigned to any type
    for byte in [0x00, 0x04, 0x50, 0xFE] {
        assert!(
            PacketType::from_byte(byte).is_none(),
            "0x{:02X} should not map to a PacketType",
            byte
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Edge cases
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn decode_ignores_trailing_bytes() {
    // Extra bytes after a valid packet should be ignored (they belong to
    // the next packet in a stream).
    let pkt = make_packet(PacketType::Pong, vec![]);
    let mut buf = encode(&pkt);
    buf.extend_from_slice(&[0xFF; 100]); // trailing garbage
    let decoded = decode(&buf).unwrap();
    assert_eq!(decoded, pkt);
}

#[test]
fn large_payload_round_trip() {
    // 64 KiB payload — realistic chunk size.
    let payload = vec![0x42; 65536];
    let pkt = Packet::new(PacketType::Chunk, 0, 5, 100, payload);
    let decoded = decode(&encode(&pkt)).unwrap();
    assert_eq!(decoded, pkt);
}

#[test]
fn zero_image_id_and_sequence() {
    let pkt = Packet::new(PacketType::Hello, 0, 0, 0, vec![]);
    let decoded = decode(&encode(&pkt)).unwrap();
    assert_eq!(decoded.image_id, 0);
    assert_eq!(decoded.sequence, 0);
}

#[test]
fn max_valid_image_id_and_sequence() {
    let pkt = Packet::new(PacketType::Ack, 0xFFFF, u32::MAX, u32::MAX, vec![]);
    let decoded = decode(&encode(&pkt)).unwrap();
    assert_eq!(decoded.image_id, u32::MAX);
    assert_eq!(decoded.sequence, u32::MAX);
    assert_eq!(decoded.flags, 0xFFFF);
}
