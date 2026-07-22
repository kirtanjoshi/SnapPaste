package com.snappaste.app.network.protocol

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * SnapPaste wire protocol packet.
 *
 * ## Header Layout (20 bytes, little-endian)
 *
 * | Field    | Offset | Size | Description                                  |
 * |----------|--------|------|----------------------------------------------|
 * | Magic    |  0     |  4   | `0x53 0x4E 0x50 0x50` ("SNPP")               |
 * | Version  |  4     |  1   | Protocol version (currently `1`)              |
 * | Type     |  5     |  1   | [PacketType] discriminant                     |
 * | Flags    |  6     |  2   | Reserved, little-endian                       |
 * | ImageID  |  8     |  4   | Monotonically increasing per screenshot       |
 * | Sequence | 12     |  4   | Per-connection monotonic packet counter       |
 * | Length   | 16     |  4   | Payload length in bytes                       |
 * | Payload  | 20     |  N   | Variable-length payload                       |
 *
 * @property version  Protocol version (should be [PROTOCOL_VERSION]).
 * @property type     The packet type.
 * @property flags    Reserved flags.
 * @property imageId  Image identifier — monotonically increasing per screenshot.
 * @property sequence Per-connection monotonic packet counter.
 * @property payload  Variable-length payload bytes.
 */
data class Packet(
    val version: Int,
    val type: PacketType,
    val flags: Int,
    val imageId: Long,
    val sequence: Long,
    val payload: ByteArray,
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is Packet) return false
        return version == other.version &&
                type == other.type &&
                flags == other.flags &&
                imageId == other.imageId &&
                sequence == other.sequence &&
                payload.contentEquals(other.payload)
    }

    override fun hashCode(): Int {
        var result = version
        result = 31 * result + type.hashCode()
        result = 31 * result + flags
        result = 31 * result + imageId.hashCode()
        result = 31 * result + sequence.hashCode()
        result = 31 * result + payload.contentHashCode()
        return result
    }

    override fun toString(): String =
        "Packet(version=$version, type=$type, flags=0x${flags.toString(16)}, " +
                "imageId=$imageId, sequence=$sequence, payloadSize=${payload.size})"

    companion object {
        /**
         * Create a new packet with the current [PROTOCOL_VERSION].
         */
        fun create(
            type: PacketType,
            flags: Int = 0,
            imageId: Long = 0,
            sequence: Long = 0,
            payload: ByteArray = ByteArray(0),
        ): Packet = Packet(
            version = PROTOCOL_VERSION,
            type = type,
            flags = flags,
            imageId = imageId,
            sequence = sequence,
            payload = payload,
        )
    }
}

// ── Constants ────────────────────────────────────────────────────────────────

/** Magic bytes identifying a SnapPaste packet: ASCII "SNPP". */
val MAGIC: ByteArray = byteArrayOf(0x53, 0x4E, 0x50, 0x50)

/** Current protocol version. */
const val PROTOCOL_VERSION: Int = 1

/** Fixed header size in bytes. */
const val HEADER_SIZE: Int = 20

/**
 * Maximum payload size we will accept (16 MiB).
 *
 * A single full-resolution screenshot rarely exceeds 10 MiB as PNG;
 * 16 MiB gives comfortable headroom without allowing a malformed length
 * field to trigger multi-gigabyte allocations.
 */
const val MAX_PAYLOAD_SIZE: Long = 16L * 1024 * 1024

// ── Encode ───────────────────────────────────────────────────────────────────

/**
 * Serialize a [Packet] into a [ByteArray] ready for transmission.
 *
 * The output is `HEADER_SIZE + packet.payload.size` bytes long.
 * All multi-byte fields are written in **little-endian** order per the spec.
 */
fun encode(packet: Packet): ByteArray {
    val total = HEADER_SIZE + packet.payload.size
    val buf = ByteBuffer.allocate(total).order(ByteOrder.LITTLE_ENDIAN)

    // Magic (4 bytes)
    buf.put(MAGIC)
    // Version (1 byte)
    buf.put(packet.version.toByte())
    // Type (1 byte)
    buf.put(packet.type.byte.toByte())
    // Flags (2 bytes, little-endian)
    buf.putShort(packet.flags.toShort())
    // ImageID (4 bytes, little-endian)
    buf.putInt(packet.imageId.toInt())
    // Sequence (4 bytes, little-endian)
    buf.putInt(packet.sequence.toInt())
    // Length (4 bytes, little-endian)
    buf.putInt(packet.payload.size)
    // Payload
    buf.put(packet.payload)

    return buf.array()
}

// ── Decode ───────────────────────────────────────────────────────────────────

/**
 * Deserialize a [Packet] from raw bytes.
 *
 * @param data The raw byte buffer to decode from.
 * @return The decoded [Packet].
 * @throws ProtocolError if the data is malformed in any way.
 */
fun decode(data: ByteArray): Packet {
    // ── Header length check ──────────────────────────────────────────────
    if (data.size < HEADER_SIZE) {
        throw ProtocolError.InsufficientHeader(
            expected = HEADER_SIZE,
            actual = data.size,
        )
    }

    val buf = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN)

    // ── Magic ────────────────────────────────────────────────────────────
    val magic = ByteArray(4)
    buf.get(magic)
    if (!magic.contentEquals(MAGIC)) {
        throw ProtocolError.InvalidMagic(found = magic)
    }

    // ── Version ──────────────────────────────────────────────────────────
    val version = buf.get().toInt() and 0xFF
    if (version != PROTOCOL_VERSION) {
        throw ProtocolError.UnsupportedVersion(
            found = version,
            expected = PROTOCOL_VERSION,
        )
    }

    // ── Type ─────────────────────────────────────────────────────────────
    val typeByte = buf.get().toInt() and 0xFF
    val packetType = PacketType.fromByte(typeByte)
        ?: throw ProtocolError.UnknownPacketType(typeByte = typeByte)

    // ── Flags ────────────────────────────────────────────────────────────
    val flags = buf.getShort().toInt() and 0xFFFF

    // ── ImageID ──────────────────────────────────────────────────────────
    val imageId = buf.getInt().toLong() and 0xFFFFFFFFL

    // ── Sequence ─────────────────────────────────────────────────────────
    val sequence = buf.getInt().toLong() and 0xFFFFFFFFL

    // ── Length ────────────────────────────────────────────────────────────
    val payloadLen = buf.getInt().toLong() and 0xFFFFFFFFL

    // Guard: reject oversized length before allocating.
    if (payloadLen > MAX_PAYLOAD_SIZE) {
        throw ProtocolError.PayloadTooLarge(
            declared = payloadLen,
            max = MAX_PAYLOAD_SIZE,
        )
    }

    // Guard: reject truncated payload.
    val available = data.size - HEADER_SIZE
    if (payloadLen > available) {
        throw ProtocolError.TruncatedPayload(
            declared = payloadLen,
            available = available,
        )
    }

    // ── Payload ──────────────────────────────────────────────────────────
    val payload = ByteArray(payloadLen.toInt())
    buf.get(payload)

    return Packet(
        version = version,
        type = packetType,
        flags = flags,
        imageId = imageId,
        sequence = sequence,
        payload = payload,
    )
}

/**
 * Safely reads a parsed [Packet] from an [java.io.InputStream].
 *
 * Implements safety limits on payload size to prevent OOM errors.
 */
fun readPacket(inputStream: java.io.InputStream): Packet {
    val headerBuf = ByteArray(HEADER_SIZE)
    readFully(inputStream, headerBuf)

    // Extract payload length from header bytes 16..20 (little-endian)
    val buf = ByteBuffer.wrap(headerBuf).order(ByteOrder.LITTLE_ENDIAN)
    buf.position(16)
    val payloadLen = buf.getInt().toLong() and 0xFFFFFFFFL

    if (payloadLen > MAX_PAYLOAD_SIZE) {
        throw ProtocolError.PayloadTooLarge(declared = payloadLen, max = MAX_PAYLOAD_SIZE)
    }

    val payloadBuf = ByteArray(payloadLen.toInt())
    if (payloadLen > 0) {
        readFully(inputStream, payloadBuf)
    }

    val fullData = ByteArray(HEADER_SIZE + payloadLen.toInt())
    System.arraycopy(headerBuf, 0, fullData, 0, HEADER_SIZE)
    System.arraycopy(payloadBuf, 0, fullData, HEADER_SIZE, payloadLen.toInt())

    return decode(fullData)
}

/**
 * Helper to block and fill a buffer completely, throwing EOFException if the stream closes early.
 */
private fun readFully(inputStream: java.io.InputStream, buffer: ByteArray) {
    var bytesRead = 0
    val size = buffer.size
    while (bytesRead < size) {
        val read = inputStream.read(buffer, bytesRead, size - bytesRead)
        if (read == -1) {
            throw java.io.EOFException("Premature end of stream while reading packet bytes")
        }
        bytesRead += read
    }
}

