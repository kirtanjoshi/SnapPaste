package com.snappaste.app.observer

import android.net.Uri
import java.util.concurrent.atomic.AtomicReference

/**
 * Thread-safe queue-of-1 holding the latest detected screenshot [Uri].
 *
 * ## Design (per Architecture.md §4)
 *
 * Exactly one in-flight "latest screenshot" slot. A new screenshot arriving
 * replaces the queued one atomically. This is safe to call from the observer
 * thread while a hypothetical consumer is reading — [AtomicReference] provides
 * lock-free compare-and-swap semantics, so neither side ever blocks.
 *
 * When a real network consumer is wired up in a later milestone, the consumer
 * will call [take] to atomically retrieve-and-clear the slot; if it gets a
 * non-null [Uri], it owns it. If a newer screenshot arrives mid-send, the
 * consumer will be notified via the [onReplaced] callback (set by the network
 * layer) so it can cancel the in-flight transfer via the wire protocol.
 *
 * @param onEnqueued Called (on the caller's thread) whenever a new [Uri] is
 *   placed into the slot. Defaults to no-op; wired up by the service layer
 *   for logging / triggering the send pipeline.
 */
class ScreenshotQueue(
    private val onEnqueued: (Uri) -> Unit = {},
) {
    private val slot = AtomicReference<Uri?>(null)

    /**
     * Atomically replace whatever is in the slot with [uri].
     *
     * @return the previous [Uri] that was displaced (null if the slot was
     *   empty). Callers can use this to log "superseded" events.
     */
    fun offer(uri: Uri): Uri? {
        val previous = slot.getAndSet(uri)
        onEnqueued(uri)
        return previous
    }

    /**
     * Atomically retrieve and clear the slot.
     *
     * @return the [Uri] if one was queued, or null if the slot was empty.
     */
    fun take(): Uri? = slot.getAndSet(null)

    /**
     * Peek at the current value without clearing it.
     * Useful for diagnostics / tests only — production code should use [take].
     */
    fun peek(): Uri? = slot.get()

    /**
     * Clear the slot without returning its contents.
     */
    fun clear() {
        slot.set(null)
    }
}
