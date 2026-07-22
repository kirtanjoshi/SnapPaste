package com.snappaste.app.network.protocol

/**
 * Wire protocol error types for the SnapPaste protocol.
 *
 * Every decode failure mode is modeled as a distinct subclass so callers can
 * pattern-match and handle each case (logging, connection teardown, etc.).
 */
sealed class ProtocolError(message: String) : Exception(message) {

    /** The magic bytes at the start of the packet do not match [MAGIC]. */
    data class InvalidMagic(val found: ByteArray) :
        ProtocolError("invalid magic bytes: ${found.joinToString(" ") { "0x%02X".format(it) }}") {

        override fun equals(other: Any?): Boolean =
            other is InvalidMagic && found.contentEquals(other.found)

        override fun hashCode(): Int = found.contentHashCode()
    }

    /** The protocol version is not supported. */
    data class UnsupportedVersion(val found: Int, val expected: Int) :
        ProtocolError("unsupported protocol version $found (expected $expected)")

    /** The type byte does not map to any known [PacketType]. */
    data class UnknownPacketType(val typeByte: Int) :
        ProtocolError("unknown packet type: 0x%02X".format(typeByte))

    /** The buffer is too short to contain a full header. */
    data class InsufficientHeader(val expected: Int, val actual: Int) :
        ProtocolError("buffer too short for header: need $expected bytes, got $actual")

    /** The buffer is too short to contain the full payload declared by the Length field. */
    data class TruncatedPayload(val declared: Long, val available: Int) :
        ProtocolError("truncated payload: header declares $declared bytes but only $available available")

    /**
     * The Length field exceeds the maximum allowed payload size.
     * Prevents unbounded allocation from a malicious or corrupt packet.
     */
    data class PayloadTooLarge(val declared: Long, val max: Long) :
        ProtocolError("payload length $declared exceeds maximum $max")
}
