package com.snappaste.app.observer

import android.net.Uri
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.CyclicBarrier
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/**
 * Unit tests for [ScreenshotQueue].
 *
 * These are pure JVM tests (no Android framework needed) verifying the
 * lock-free queue-of-1 semantics, including thread-safety under contention.
 */
class ScreenshotQueueTest {

    private lateinit var queue: ScreenshotQueue

    private fun uri(id: Int): Uri = Uri.parse("content://media/external/images/media/$id")

    @Before
    fun setUp() {
        queue = ScreenshotQueue()
    }

    // ═════════════════════════════════════════════════════════════════════
    // Basic operations
    // ═════════════════════════════════════════════════════════════════════

    @Test
    fun `empty queue returns null on take`() {
        assertNull(queue.take())
    }

    @Test
    fun `empty queue returns null on peek`() {
        assertNull(queue.peek())
    }

    @Test
    fun `offer and take single item`() {
        val u = uri(1)
        val previous = queue.offer(u)
        assertNull("first offer should return null (slot was empty)", previous)
        assertEquals(u, queue.take())
        assertNull("take should clear the slot", queue.take())
    }

    @Test
    fun `offer replaces previous and returns it`() {
        val u1 = uri(1)
        val u2 = uri(2)
        queue.offer(u1)
        val superseded = queue.offer(u2)
        assertEquals("should return the superseded URI", u1, superseded)
        assertEquals("slot should hold the newest URI", u2, queue.take())
    }

    @Test
    fun `peek does not consume`() {
        queue.offer(uri(1))
        assertEquals(uri(1), queue.peek())
        assertEquals(uri(1), queue.peek())
        assertEquals(uri(1), queue.take())
    }

    @Test
    fun `clear empties the slot`() {
        queue.offer(uri(1))
        queue.clear()
        assertNull(queue.take())
    }

    // ═════════════════════════════════════════════════════════════════════
    // Queue-of-1 replacement semantics
    // ═════════════════════════════════════════════════════════════════════

    @Test
    fun `rapid fire offers keep only the latest`() {
        // Simulate 100 screenshots arriving in quick succession.
        for (i in 1..100) {
            queue.offer(uri(i))
        }
        assertEquals("only the last URI should survive", uri(100), queue.take())
        assertNull("slot should be empty after take", queue.take())
    }

    @Test
    fun `onEnqueued callback fires for every offer`() {
        val received = mutableListOf<Uri>()
        val q = ScreenshotQueue(onEnqueued = { received.add(it) })

        q.offer(uri(1))
        q.offer(uri(2))
        q.offer(uri(3))

        assertEquals(3, received.size)
        assertEquals(listOf(uri(1), uri(2), uri(3)), received)
    }

    // ═════════════════════════════════════════════════════════════════════
    // Thread safety: concurrent offer + take never blocks
    // ═════════════════════════════════════════════════════════════════════

    @Test
    fun `concurrent offers and takes are safe`() {
        val iterations = 10_000
        val writerCount = 4
        val readerCount = 2

        val callbackCount = AtomicInteger(0)
        val q = ScreenshotQueue(onEnqueued = { callbackCount.incrementAndGet() })

        val barrier = CyclicBarrier(writerCount + readerCount)
        val latch = CountDownLatch(writerCount + readerCount)
        val errors = CopyOnWriteArrayList<Throwable>()

        // Writers: rapid-fire offer
        repeat(writerCount) { writerId ->
            Thread {
                try {
                    barrier.await() // synchronize start
                    for (i in 0 until iterations) {
                        q.offer(uri(writerId * iterations + i))
                    }
                } catch (t: Throwable) {
                    errors.add(t)
                } finally {
                    latch.countDown()
                }
            }.start()
        }

        // Readers: rapid-fire take
        val takenUris = CopyOnWriteArrayList<Uri>()
        repeat(readerCount) {
            Thread {
                try {
                    barrier.await()
                    for (i in 0 until iterations) {
                        val u = q.take()
                        if (u != null) takenUris.add(u)
                    }
                } catch (t: Throwable) {
                    errors.add(t)
                } finally {
                    latch.countDown()
                }
            }.start()
        }

        assertTrue("threads should finish within 10s", latch.await(10, TimeUnit.SECONDS))
        assertTrue("no exceptions during concurrent access: $errors", errors.isEmpty())
        assertEquals(
            "callback should have fired once per offer",
            writerCount * iterations,
            callbackCount.get(),
        )
    }

    @Test
    fun `offer never blocks the calling thread`() {
        // Measure that 10,000 rapid-fire offers complete in well under 1 second.
        // This proves the observer thread will never stall.
        val q = ScreenshotQueue()
        val start = System.nanoTime()
        for (i in 1..10_000) {
            q.offer(uri(i))
        }
        val elapsedMs = (System.nanoTime() - start) / 1_000_000
        assertTrue(
            "10,000 offers took ${elapsedMs}ms — should be < 1000ms",
            elapsedMs < 1000,
        )
    }
}
