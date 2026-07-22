package com.snappaste.app.network

import android.content.ContentResolver
import android.net.Uri
import android.util.Log
import com.snappaste.app.network.protocol.Packet
import com.snappaste.app.network.protocol.PacketType
import com.snappaste.app.network.protocol.encode
import com.snappaste.app.pairing.PairingManager
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import java.io.InputStream
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

import kotlin.coroutines.coroutineContext

/**
 * Handles TCP client connection, authentication/pairing, and screenshot transmission.
 *
 * ## Design (per Architecture.md §4, §5, §7)
 *
 * - Attempts connection to localhost:9999 in a loop.
 * - Performs HELLO and AUTH handshake using the token from [PairingManager].
 * - Listens for new screenshot Uris via a coroutine [Channel].
 * - Streams screenshot files in 64 KiB chunks.
 * - If a new screenshot is enqueued mid-transfer, it immediately cancels the current
 *   transfer by sending a [PacketType.CANCEL] packet, aborts reading, and begins the new one.
 */
class TcpClient(
    private val contentResolver: ContentResolver,
    private val pairingManager: PairingManager,
    private val host: String = "127.0.0.1",
    private val port: Int = 9999
) {
    companion object {
        private const val TAG = "TcpClient"
        private const val CHUNK_SIZE = 64 * 1024 // 64 KiB chunks
    }

    private val connectionManager = ConnectionManager()
    private val uriChannel = Channel<Uri>(Channel.CONFLATED)
    
    private val nextImageId = AtomicLong(1)
    private val nextSequence = AtomicLong(1)

    // Tracks if an active transfer is in progress
    private var activeTransferJob: Job? = null
    private val isTransferring = AtomicBoolean(false)

    private var scope: CoroutineScope? = null

    /**
     * Start the TCP client background connection loop.
     */
    fun start(parentScope: CoroutineScope) {
        scope = parentScope
        parentScope.launch(Dispatchers.IO) {
            connectionLoop()
        }
    }

    /**
     * Push a new screenshot Uri to be sent.
     */
    fun offerScreenshot(uri: Uri) {
        Log.i(TAG, "Offered screenshot URI to send queue: $uri")
        uriChannel.trySend(uri)
    }

    private suspend fun connectionLoop() {
        while (coroutineContext.isActive) {
            Log.i(TAG, "Connecting to SnapPaste Desktop at $host:$port...")
            var socket: Socket? = null
            try {
                socket = Socket()
                socket.keepAlive = true
                socket.soTimeout = 10000 // 10 seconds timeout
                socket.connect(InetSocketAddress(host, port), 3000)
                Log.i(TAG, "TCP connected to desktop server.")
                connectionManager.transition(ConnectionEvent.TCP_CONNECTED)

                val outputStream = socket.getOutputStream()
                val inputStream = socket.getInputStream()

                // Perform Handshake
                if (performHandshake(outputStream)) {
                    connectionManager.transition(ConnectionEvent.AUTH_SUCCESS)
                    connectionManager.transition(ConnectionEvent.ENTER_IDLE)
                    
                    // Connected & Authenticated successfully. Run transmission loop.
                    runTransmissionLoop(socket, outputStream, inputStream)
                } else {
                    Log.w(TAG, "Handshake failed.")
                    connectionManager.transition(ConnectionEvent.AUTH_FAILED)
                }
            } catch (e: Exception) {
                Log.w(TAG, "Socket error / disconnect: ${e.message}")
                connectionManager.transition(ConnectionEvent.DISCONNECT_OR_ERROR)
            } finally {
                isTransferring.set(false)
                activeTransferJob?.cancel()
                try {
                    socket?.close()
                } catch (e: Exception) {
                    // Ignore close failures
                }
                Log.i(TAG, "Connection closed. Retrying in 2 seconds...")
                delay(2000)
            }
        }
    }

    private fun performHandshake(out: OutputStream): Boolean {
        return try {
            // 1. Send HELLO
            val hello = Packet.create(PacketType.HELLO, sequence = nextSequence.getAndIncrement())
            out.write(encode(hello))
            out.flush()

            // 2. Send AUTH with stored/generated pairing token
            val token = pairingManager.getOrGenerateToken()
            val auth = Packet.create(
                PacketType.AUTH,
                sequence = nextSequence.getAndIncrement(),
                payload = token.toByteArray(Charsets.UTF_8)
            )
            out.write(encode(auth))
            out.flush()

            Log.i(TAG, "Sent HELLO and AUTH packets. Handshake completed.")
            true
        } catch (e: Exception) {
            Log.e(TAG, "Error performing handshake", e)
            false
        }
    }

    private suspend fun runTransmissionLoop(
        socket: Socket,
        out: OutputStream,
        inStream: InputStream
    ) = coroutineScope {
        
        // Spawn a reader job to detect remote closes, process heartbeats, or disconnects
        val readerJob = launch(Dispatchers.IO) {
            try {
                while (isActive) {
                    val pkt = com.snappaste.app.network.protocol.readPacket(inStream)
                    Log.d(TAG, "Received packet from desktop: ${pkt.type}")
                    when (pkt.type) {
                        PacketType.PING -> {
                            Log.d(TAG, "Received PING, sending PONG...")
                            val pong = Packet.create(
                                PacketType.PONG,
                                imageId = pkt.imageId,
                                sequence = nextSequence.getAndIncrement()
                            )
                            out.write(encode(pong))
                            out.flush()
                        }
                        PacketType.ERROR -> {
                            val msg = String(pkt.payload, Charsets.UTF_8)
                            Log.e(TAG, "Desktop error packet: $msg")
                        }
                        else -> {
                            Log.d(TAG, "Unhandled remote packet type: ${pkt.type}")
                        }
                    }
                }
            } catch (e: Exception) {
                Log.d(TAG, "Socket read loop ended: ${e.message}")
            } finally {
                // Terminate transmission loop by cancelling scope
                this@coroutineScope.cancel("Reader thread closed")
            }
        }

        try {
            while (isActive) {
                // Wait for the next screenshot URI from the conflated queue
                val uri = uriChannel.receive()
                
                // If we are currently transferring, cancel it before starting new one
                if (isTransferring.get()) {
                    Log.i(TAG, "New screenshot arrived during transmission. Cancelling active transfer.")
                    activeTransferJob?.cancelAndJoin()
                }

                // Start the transfer job
                activeTransferJob = launch(Dispatchers.IO) {
                    try {
                        isTransferring.set(true)
                        sendScreenshot(uri, out)
                    } catch (e: CancellationException) {
                        Log.i(TAG, "Screenshot transfer cancelled.")
                        // Send CANCEL packet to remote side to let it discard current buffer
                        sendCancelPacket(out)
                    } catch (e: Exception) {
                        Log.e(TAG, "Error sending screenshot", e)
                        throw e // bubble up to trigger reconnect
                    } finally {
                        isTransferring.set(false)
                    }
                }
            }
        } finally {
            readerJob.cancel()
        }
    }

    private suspend fun sendScreenshot(uri: Uri, out: OutputStream) {
        connectionManager.transition(ConnectionEvent.START_SEND)
        
        val imageId = nextImageId.getAndIncrement()
        Log.i(TAG, "Starting transmission of screenshot: URI=$uri, ImageID=$imageId")

        // 1. Notify that a new screenshot is detected
        val notifyPkt = Packet.create(
            PacketType.NEW_SCREENSHOT,
            imageId = imageId,
            sequence = nextSequence.getAndIncrement()
        )
        out.write(encode(notifyPkt))
        out.flush()

        // 2. Open file stream and count size
        val pfd = contentResolver.openFileDescriptor(uri, "r") ?: throw Exception("Failed to open file descriptor")
        val fileSize = pfd.statSize
        pfd.close()

        val inputStream = contentResolver.openInputStream(uri) ?: throw Exception("Failed to open input stream")
        
        inputStream.use { stream ->
            // 3. Send START_IMAGE with file size metadata
            // Length metadata is sent as 8-byte long inside payload
            val meta = java.nio.ByteBuffer.allocate(8)
                .order(java.nio.ByteOrder.LITTLE_ENDIAN)
                .putLong(fileSize)
                .array()

            val startPkt = Packet.create(
                PacketType.START_IMAGE,
                imageId = imageId,
                sequence = nextSequence.getAndIncrement(),
                payload = meta
            )
            out.write(encode(startPkt))
            out.flush()

            // 4. Stream data in 64 KiB chunks
            val buffer = ByteArray(CHUNK_SIZE)
            var bytesRead: Int
            
            while (stream.read(buffer).also { bytesRead = it } != -1) {
                // Yield thread to verify if coroutine was cancelled (e.g. by newer screenshot arrival)
                currentCoroutineContext().ensureActive()

                val chunkData = if (bytesRead == CHUNK_SIZE) {
                    buffer
                } else {
                    buffer.copyOf(bytesRead)
                }

                val chunkPkt = Packet.create(
                    PacketType.CHUNK,
                    imageId = imageId,
                    sequence = nextSequence.getAndIncrement(),
                    payload = chunkData
                )
                out.write(encode(chunkPkt))
                out.flush()
            }

            // 5. Send END_IMAGE
            val endPkt = Packet.create(
                PacketType.END_IMAGE,
                imageId = imageId,
                sequence = nextSequence.getAndIncrement()
            )
            out.write(encode(endPkt))
            out.flush()

            Log.i(TAG, "Successfully completed transmission for ImageID=$imageId")
            connectionManager.transition(ConnectionEvent.SEND_COMPLETE)
        }
    }

    private fun sendCancelPacket(out: OutputStream) {
        try {
            val cancelPkt = Packet.create(
                PacketType.CANCEL,
                imageId = nextImageId.get() - 1, // cancel the active one
                sequence = nextSequence.getAndIncrement()
            )
            out.write(encode(cancelPkt))
            out.flush()
            Log.i(TAG, "Sent CANCEL packet for previous transfer.")
            connectionManager.transition(ConnectionEvent.SEND_COMPLETE)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to send CANCEL packet", e)
        }
    }
}
