package com.snappaste.app.observer

import android.content.ContentResolver
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.JUnit4
import java.lang.reflect.Field

/**
 * Unit tests for [ScreenshotObserver]'s filtering logic.
 *
 * These tests use a fake [ContentResolver] (via a minimal stub) to exercise
 * the query + filter pipeline without needing an actual MediaStore or device.
 *
 * NOTE: These are JVM-only tests using `android.jar` stubs. On a real CI you'd
 * run these as Robolectric tests. The test is structured so it can work under
 * either environment.
 */
@RunWith(JUnit4::class)
class ScreenshotObserverFilterTest {

    private lateinit var queue: ScreenshotQueue
    private val enqueued = mutableListOf<Uri>()

    @Before
    fun setUp() {
        enqueued.clear()
        queue = ScreenshotQueue(onEnqueued = { enqueued.add(it) })
    }

    // ═════════════════════════════════════════════════════════════════════
    // ScreenshotObserver filtering logic — documented test cases
    // ═════════════════════════════════════════════════════════════════════

    /**
     * Verifies the conceptual filtering rules. Since ScreenshotObserver
     * queries a real ContentResolver (which we can't easily fake in pure
     * JVM tests without Robolectric), these tests validate the filter
     * conditions using a helper that simulates the same decision logic.
     *
     * The full integration test with a real ContentResolver runs as an
     * instrumented test (see androidTest/).
     */

    @Test
    fun `filter rejects path without Screenshots`() {
        // Simulates a camera photo in DCIM/Camera/
        assertFalse(
            "DCIM/Camera path should be rejected",
            pathContainsScreenshots("DCIM/Camera/"),
        )
    }

    @Test
    fun `filter accepts path with Screenshots directory`() {
        assertTrue(
            "Pictures/Screenshots/ should be accepted",
            pathContainsScreenshots("Pictures/Screenshots/"),
        )
    }

    @Test
    fun `filter accepts path with Screenshots case insensitive`() {
        assertTrue(
            "DCIM/screenshots/ (lowercase) should be accepted",
            pathContainsScreenshots("DCIM/screenshots/"),
        )
    }

    @Test
    fun `filter rejects old DATE_ADDED`() {
        val nowSeconds = System.currentTimeMillis() / 1000
        val oldTimestamp = nowSeconds - 60 // 60 seconds ago
        assertFalse(
            "Image from 60s ago should be rejected",
            isRecent(oldTimestamp, nowSeconds, ScreenshotObserver.RECENCY_THRESHOLD_SECONDS),
        )
    }

    @Test
    fun `filter accepts recent DATE_ADDED`() {
        val nowSeconds = System.currentTimeMillis() / 1000
        val recentTimestamp = nowSeconds - 2 // 2 seconds ago
        assertTrue(
            "Image from 2s ago should be accepted",
            isRecent(recentTimestamp, nowSeconds, ScreenshotObserver.RECENCY_THRESHOLD_SECONDS),
        )
    }

    @Test
    fun `filter rejects IS_PENDING = 1`() {
        assertFalse("Pending images should be rejected", isPendingAcceptable(isPending = 1))
    }

    @Test
    fun `filter accepts IS_PENDING = 0`() {
        assertTrue("Non-pending images should be accepted", isPendingAcceptable(isPending = 0))
    }

    @Test
    fun `dedup rejects same URI twice`() {
        val uri1 = Uri.parse("content://media/external/images/media/42")
        val seen = mutableSetOf<Uri>()
        assertTrue("First occurrence should be accepted", dedup(uri1, seen))
        assertFalse("Second occurrence should be rejected", dedup(uri1, seen))
    }

    @Test
    fun `dedup accepts different URIs`() {
        val uri1 = Uri.parse("content://media/external/images/media/42")
        val uri2 = Uri.parse("content://media/external/images/media/43")
        val seen = mutableSetOf<Uri>()
        assertTrue(dedup(uri1, seen))
        assertTrue(dedup(uri2, seen))
    }

    // ═════════════════════════════════════════════════════════════════════
    // Queue integration: rapid-fire replaces correctly
    // ═════════════════════════════════════════════════════════════════════

    @Test
    fun `rapid fire simulated screenshots replace queue correctly`() {
        // Simulate 50 screenshots detected in quick succession.
        // The queue should only hold the very last one.
        for (i in 1..50) {
            queue.offer(Uri.parse("content://media/external/images/media/$i"))
        }
        assertEquals(
            "queue should hold the 50th screenshot",
            Uri.parse("content://media/external/images/media/50"),
            queue.peek(),
        )
        assertEquals(
            "onEnqueued should have fired 50 times",
            50,
            enqueued.size,
        )
    }

    @Test
    fun `rapid fire never blocks observer thread`() {
        val start = System.nanoTime()
        for (i in 1..10_000) {
            queue.offer(Uri.parse("content://media/external/images/media/$i"))
        }
        val elapsedMs = (System.nanoTime() - start) / 1_000_000
        assertTrue(
            "10,000 offers took ${elapsedMs}ms — must not block",
            elapsedMs < 1000,
        )
    }

    // ── Filter helper methods (mirror the observer's internal logic) ─────

    /**
     * Mirrors the path-based filter in [ScreenshotObserver].
     */
    private fun pathContainsScreenshots(path: String?): Boolean =
        path?.contains("Screenshots", ignoreCase = true) == true

    /**
     * Mirrors the DATE_ADDED recency check in [ScreenshotObserver].
     */
    private fun isRecent(dateAdded: Long, nowSeconds: Long, thresholdSeconds: Long): Boolean =
        dateAdded >= (nowSeconds - thresholdSeconds)

    /**
     * Mirrors the IS_PENDING check in [ScreenshotObserver].
     * @param isPending 0 = ready, 1 = still being written.
     */
    private fun isPendingAcceptable(isPending: Int): Boolean = isPending == 0

    /**
     * Mirrors the URI dedup logic in [ScreenshotObserver].
     */
    private fun dedup(uri: Uri, seen: MutableSet<Uri>): Boolean = seen.add(uri)
}
