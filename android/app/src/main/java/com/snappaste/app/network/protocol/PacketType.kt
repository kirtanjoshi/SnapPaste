package com.snappaste.app.network.protocol

/**
 * Exhaustive enumeration of all wire protocol message types.
 *
 * Each variant maps to a single byte on the wire. Discriminant values are
 * intentionally spaced to leave room for future additions without breaking
 * compatibility.
 *
 * @property byte The on-wire byte value for this type.
 */
enum class PacketType(val byte: Int) {
    /** Initial handshake packet sent by either side upon TCP connection. */
    HELLO(0x01),
    /** Authentication / pairing-token exchange. */
    AUTH(0x02),
    /** Notification that a new screenshot is available (Android → Desktop). */
    NEW_SCREENSHOT(0x03),
    /** Begin a chunked image transfer; carries metadata (dimensions, format). */
    START_IMAGE(0x10),
    /** A single chunk of image data. */
    CHUNK(0x11),
    /** Cancel the current in-flight image transfer. */
    CANCEL(0x12),
    /** Signals the final chunk has been sent; image is complete. */
    END_IMAGE(0x13),
    /** Generic acknowledgement. */
    ACK(0x20),
    /** Keep-alive request. */
    PING(0x30),
    /** Keep-alive response. */
    PONG(0x31),
    /** Graceful disconnect. */
    DISCONNECT(0x40),
    /** Error report (human-readable message in payload). */
    ERROR(0xFF);

    companion object {
        private val byteMap: Map<Int, PacketType> = entries.associateBy { it.byte }

        /**
         * Look up a [PacketType] by its wire byte value.
         * @return the matching type, or `null` if the byte is not assigned.
         */
        fun fromByte(b: Int): PacketType? = byteMap[b and 0xFF]
    }
}
