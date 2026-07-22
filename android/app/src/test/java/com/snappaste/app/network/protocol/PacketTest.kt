package com.snappaste.app.network.protocol

import org.junit.Assert.*
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Unit tests for the SnapPaste wire protocol (Kotlin / Android side).
 *
 * Test matrix mirrors the Rust (desktop) side so both implementations can be
 * verified independently against the same spec.
 */
class PacketTest {

    // ── Helpers ──────────────────────────────────────────────────────────

    /** All packet types in the protocol, used for exhaustive iteration. */
    private val allTypes = PacketType.entries

    /** Build a test packet with the given type and an optional payload. */
    private fun makePacket(type: PacketType, payload: ByteArray = ByteArray(0)): Packet =
        Packet.create(type = type, flags = 0, imageId = 42, sequence = 7, payload = payload)

    /** Assert that encoding then decoding a packet produces an equal packet. */
    private fun assertRoundTrip(original: Packet) {
        val encoded = encode(original)
        val decoded = decode(encoded)
        assertEquals(original, decoded)
    }

    // ═════════════════════════════════════════════════════════════════════
    // Round-trip tests: encode → decode for every packet type
    // ═════════════════════════════════════════════════════════════════════

    @Test
    fun `round trip HELLO`() = assertRoundTrip(makePacket(PacketType.HELLO))

    @Test
    fun `round trip AUTH`() =
        assertRoundTrip(makePacket(PacketType.AUTH, "token123".toByteArray()))

    @Test
    fun `round trip NEW_SCREENSHOT`() =
        assertRoundTrip(makePacket(PacketType.NEW_SCREENSHOT))

    @Test
    fun `round trip START_IMAGE`() {
        // Payload: 4 bytes width + 4 bytes height (LE) as minimal metadata
        val meta = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN)
            .putInt(1080).putInt(1920).array()
        assertRoundTrip(Packet.create(PacketType.START_IMAGE, imageId = 1, payload = meta))
    }

    @Test
    fun `round trip CHUNK`() = assertRoundTrip(
        Packet.create(PacketType.CHUNK, imageId = 1, sequence = 1,
            payload = byteArrayOf(0xDE.toByte(), 0xAD.toByte(), 0xBE.toByte(), 0xEF.toByte()))
    )

    @Test
    fun `round trip CANCEL`() = assertRoundTrip(
        Packet.create(PacketType.CANCEL, imageId = 1, sequence = 2)
    )

    @Test
    fun `round trip END_IMAGE`() = assertRoundTrip(
        Packet.create(PacketType.END_IMAGE, imageId = 1, sequence = 3)
    )

    @Test
    fun `round trip ACK`() = assertRoundTrip(makePacket(PacketType.ACK))

    @Test
    fun `round trip PING`() = assertRoundTrip(makePacket(PacketType.PING))

    @Test
    fun `round trip PONG`() = assertRoundTrip(makePacket(PacketType.PONG))

    @Test
    fun `round trip DISCONNECT`() = assertRoundTrip(makePacket(PacketType.DISCONNECT))

    @Test
    fun `round trip ERROR`() =
        assertRoundTrip(makePacket(PacketType.ERROR, "something went wrong".toByteArray()))

    /** Exhaustive: ensure every variant survives a round-trip. */
    @Test
    fun `round trip all types exhaustive`() {
        for ((i, type) in allTypes.withIndex()) {
            val pkt = Packet.create(
                type = type,
                imageId = i.toLong(),
                sequence = i.toLong(),
                payload = ByteArray(i) { i.toByte() },
            )
            val decoded = decode(encode(pkt))
            assertEquals("round-trip mismatch for $type", pkt, decoded)
        }
    }

    // ═════════════════════════════════════════════════════════════════════
    // Header field tests
    // ═════════════════════════════════════════════════════════════════════

    @Test
    fun `header size is 20 bytes`() {
        val pkt = makePacket(PacketType.PING)
        assertEquals(HEADER_SIZE, encode(pkt).size)
    }

    @Test
    fun `magic bytes at offset 0`() {
        val buf = encode(makePacket(PacketType.PING))
        assertArrayEquals(MAGIC, buf.sliceArray(0 until 4))
    }

    @Test
    fun `version at offset 4`() {
        val buf = encode(makePacket(PacketType.PING))
        assertEquals(PROTOCOL_VERSION, buf[4].toInt() and 0xFF)
    }

    @Test
    fun `flags preserved`() {
        val pkt = Packet.create(PacketType.HELLO, flags = 0xABCD)
        val decoded = decode(encode(pkt))
        assertEquals(0xABCD, decoded.flags)
    }

    @Test
    fun `image id preserved`() {
        val pkt = Packet.create(PacketType.START_IMAGE, imageId = 0xDEADBEEFL)
        val decoded = decode(encode(pkt))
        assertEquals(0xDEADBEEFL, decoded.imageId)
    }

    @Test
    fun `sequence preserved`() {
        val pkt = Packet.create(
            PacketType.CHUNK, sequence = 0xCAFEBABEL,
            payload = byteArrayOf(0x01)
        )
        val decoded = decode(encode(pkt))
        assertEquals(0xCAFEBABEL, decoded.sequence)
    }

    @Test
    fun `payload length field matches`() {
        val payload = ByteArray(256) { 0xAA.toByte() }
        val pkt = makePacket(PacketType.CHUNK, payload)
        val buf = encode(pkt)
        val lengthField = ByteBuffer.wrap(buf, 16, 4)
            .order(ByteOrder.LITTLE_ENDIAN).int.toLong() and 0xFFFFFFFFL
        assertEquals(256L, lengthField)
        assertEquals(HEADER_SIZE + 256, buf.size)
    }

    // ═════════════════════════════════════════════════════════════════════
    // Decode error tests
    // ═════════════════════════════════════════════════════════════════════

    @Test
    fun `reject malformed magic`() {
        val buf = encode(makePacket(PacketType.PING))
        buf[0] = 0x00 // corrupt first magic byte
        val err = assertThrows(ProtocolError.InvalidMagic::class.java) { decode(buf) }
        assertEquals(0x00.toByte(), err.found[0])
    }

    @Test
    fun `reject completely wrong magic`() {
        val buf = encode(makePacket(PacketType.PING))
        buf[0] = 0xFF.toByte()
        buf[1] = 0xFF.toByte()
        buf[2] = 0xFF.toByte()
        buf[3] = 0xFF.toByte()
        val err = assertThrows(ProtocolError.InvalidMagic::class.java) { decode(buf) }
        assertArrayEquals(byteArrayOf(-1, -1, -1, -1), err.found)
    }

    @Test
    fun `reject unsupported version`() {
        val buf = encode(makePacket(PacketType.PING))
        buf[4] = 99
        val err = assertThrows(ProtocolError.UnsupportedVersion::class.java) { decode(buf) }
        assertEquals(99, err.found)
        assertEquals(1, err.expected)
    }

    @Test
    fun `reject unknown type`() {
        val buf = encode(makePacket(PacketType.PING))
        buf[5] = 0x99.toByte()
        val err = assertThrows(ProtocolError.UnknownPacketType::class.java) { decode(buf) }
        assertEquals(0x99, err.typeByte)
    }

    @Test
    fun `reject truncated header`() {
        val buf = byteArrayOf(0x53, 0x4E, 0x50) // only 3 bytes
        val err = assertThrows(ProtocolError.InsufficientHeader::class.java) { decode(buf) }
        assertEquals(20, err.expected)
        assertEquals(3, err.actual)
    }

    @Test
    fun `reject empty buffer`() {
        val err = assertThrows(ProtocolError.InsufficientHeader::class.java) { decode(ByteArray(0)) }
        assertEquals(20, err.expected)
        assertEquals(0, err.actual)
    }

    @Test
    fun `reject truncated payload`() {
        val pkt = makePacket(PacketType.AUTH, "secret_token".toByteArray())
        val full = encode(pkt)
        val truncated = full.sliceArray(0 until full.size - 5)
        assertThrows(ProtocolError.TruncatedPayload::class.java) { decode(truncated) }
    }

    @Test
    fun `reject oversized length no panic no alloc`() {
        val pkt = makePacket(PacketType.CHUNK, byteArrayOf(0x01))
        val buf = encode(pkt)
        // Set the length field to MAX_PAYLOAD_SIZE + 1
        val oversized = (MAX_PAYLOAD_SIZE + 1).toInt()
        ByteBuffer.wrap(buf, 16, 4).order(ByteOrder.LITTLE_ENDIAN).putInt(oversized)
        val err = assertThrows(ProtocolError.PayloadTooLarge::class.java) { decode(buf) }
        assertEquals(MAX_PAYLOAD_SIZE + 1, err.declared)
        assertEquals(MAX_PAYLOAD_SIZE, err.max)
    }

    @Test
    fun `reject u32 max length safely`() {
        val pkt = makePacket(PacketType.CHUNK)
        val buf = encode(pkt)
        // Set the length field to 0xFFFFFFFF — must not panic or try to allocate 4 GiB.
        ByteBuffer.wrap(buf, 16, 4).order(ByteOrder.LITTLE_ENDIAN).putInt(-1) // -1 == 0xFFFFFFFF unsigned
        val err = assertThrows(ProtocolError.PayloadTooLarge::class.java) { decode(buf) }
        assertEquals(0xFFFFFFFFL, err.declared)
    }

    // ═════════════════════════════════════════════════════════════════════
    // PacketType byte conversion
    // ═════════════════════════════════════════════════════════════════════

    @Test
    fun `packet type round trip`() {
        for (type in allTypes) {
            val byte = type.byte
            val back = PacketType.fromByte(byte)
            assertEquals("type byte round-trip failed for $type", type, back)
        }
    }

    @Test
    fun `packet type from invalid byte returns null`() {
        for (byte in listOf(0x00, 0x04, 0x50, 0xFE)) {
            assertNull("0x${byte.toString(16)} should not map to a PacketType",
                PacketType.fromByte(byte))
        }
    }

    // ═════════════════════════════════════════════════════════════════════
    // Edge cases
    // ═════════════════════════════════════════════════════════════════════

    @Test
    fun `decode ignores trailing bytes`() {
        val pkt = makePacket(PacketType.PONG)
        val encoded = encode(pkt)
        val withTrailing = encoded + ByteArray(100) { 0xFF.toByte() }
        val decoded = decode(withTrailing)
        assertEquals(pkt, decoded)
    }

    @Test
    fun `large payload round trip`() {
        // 64 KiB payload — realistic chunk size.
        val payload = ByteArray(65536) { 0x42 }
        val pkt = Packet.create(PacketType.CHUNK, imageId = 5, sequence = 100, payload = payload)
        assertRoundTrip(pkt)
    }

    @Test
    fun `zero image id and sequence`() {
        val pkt = Packet.create(PacketType.HELLO, imageId = 0, sequence = 0)
        val decoded = decode(encode(pkt))
        assertEquals(0L, decoded.imageId)
        assertEquals(0L, decoded.sequence)
    }

    @Test
    fun `max valid image id and sequence`() {
        val pkt = Packet.create(
            PacketType.ACK,
            flags = 0xFFFF,
            imageId = 0xFFFFFFFFL,
            sequence = 0xFFFFFFFFL,
        )
        val decoded = decode(encode(pkt))
        assertEquals(0xFFFFFFFFL, decoded.imageId)
        assertEquals(0xFFFFFFFFL, decoded.sequence)
        assertEquals(0xFFFF, decoded.flags)
    }
}
