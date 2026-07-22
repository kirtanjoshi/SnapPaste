package com.snappaste.app.observer

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.Environment
import android.provider.MediaStore
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Instrumented integration tests for [ScreenshotObserver] and [ScreenshotQueue].
 * Runs on an active Android device or emulator to verify real interaction with
 * the Android MediaStore content observation framework.
 */
@RunWith(AndroidJUnit4::class)
class ScreenshotObserverAndroidTest {

    private lateinit var context: Context
    private lateinit var handlerThread: HandlerThread
    private lateinit var handler: Handler
    private lateinit var queue: ScreenshotQueue
    private lateinit var observer: ScreenshotObserver
    private val insertedUris = mutableListOf<Uri>()

    @Before
    fun setUp() {
        context = InstrumentationRegistry.getInstrumentation().targetContext
        handlerThread = HandlerThread("TestObserverThread").apply { start() }
        handler = Handler(handlerThread.looper)
        queue = ScreenshotQueue()
        observer = ScreenshotObserver(context.contentResolver, handler, queue)
        observer.register()
    }

    @After
    fun tearDown() {
        observer.unregister()
        handlerThread.quitSafely()

        // Clean up any inserted files from the MediaStore to avoid polluting the device
        val resolver = context.contentResolver
        for (uri in insertedUris) {
            try {
                resolver.delete(uri, null, null)
            } catch (e: Exception) {
                // Ignore failure on cleanup
            }
        }
        insertedUris.clear()
    }

    /**
     * Helper to insert a mock screenshot entry into the MediaStore.
     */
    private fun insertMockScreenshot(fileName: String, dateAddedSeconds: Long, isPending: Int = 0): Uri {
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, fileName)
            put(MediaStore.Images.Media.MIME_TYPE, "image/png")
            put(MediaStore.Images.Media.DATE_ADDED, dateAddedSeconds)
            
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                put(MediaStore.Images.Media.RELATIVE_PATH, Environment.DIRECTORY_PICTURES + "/Screenshots")
                put(MediaStore.Images.Media.IS_PENDING, isPending)
            } else {
                // For older APIs, simulate the path via DATA column
                @Suppress("DEPRECATION")
                val mockPath = "/sdcard/" + Environment.DIRECTORY_PICTURES + "/Screenshots/" + fileName
                put(MediaStore.Images.Media.DATA, mockPath)
            }
        }

        val uri = resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)
        assertNotNull("Failed to insert mock screenshot into MediaStore", uri)
        insertedUris.add(uri!!)
        return uri
    }

    @Test
    fun testRealObservationAndFiltering() {
        val nowSeconds = System.currentTimeMillis() / 1000

        // 1. Insert an image that does NOT match the Screenshots folder.
        // It should be filtered out by the path filter.
        val nonScreenshotValues = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, "camera_photo.jpg")
            put(MediaStore.Images.Media.MIME_TYPE, "image/jpeg")
            put(MediaStore.Images.Media.DATE_ADDED, nowSeconds)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                put(MediaStore.Images.Media.RELATIVE_PATH, Environment.DIRECTORY_DCIM + "/Camera")
                put(MediaStore.Images.Media.IS_PENDING, 0)
            } else {
                @Suppress("DEPRECATION")
                put(MediaStore.Images.Media.DATA, "/sdcard/" + Environment.DIRECTORY_DCIM + "/Camera/camera_photo.jpg")
            }
        }
        val nonScreenshotUri = context.contentResolver.insert(
            MediaStore.Images.Media.EXTERNAL_CONTENT_URI, nonScreenshotValues
        )
        assertNotNull(nonScreenshotUri)
        insertedUris.add(nonScreenshotUri!!)

        // Manually trigger query to test the pipeline synchronously/deterministically
        observer.queryLatestScreenshot()
        assertNull("Non-screenshot URI should not be enqueued", queue.peek())

        // 2. Insert a valid screenshot. It should pass the pipeline and be enqueued.
        val screenshotUri = insertMockScreenshot("Screenshot_test_1.png", nowSeconds, isPending = 0)
        
        // Wait a short time for ContentObserver asynchronous callback, or trigger query manually
        // We will trigger query manually to guarantee evaluation, but we also verify it gets caught
        observer.queryLatestScreenshot()
        
        val enqueuedUri = queue.peek()
        assertEquals("Valid screenshot should be enqueued in the queue", screenshotUri, enqueuedUri)
    }

    @Test
    fun testRapidFireSimulatedScreenshots() {
        val nowSeconds = System.currentTimeMillis() / 1000
        val count = 10
        val latch = CountDownLatch(1)
        
        // Setup listener on queue to trace enqueues
        val enqueuedUris = mutableListOf<Uri>()
        queue = ScreenshotQueue(onEnqueued = {
            enqueuedUris.add(it)
            if (enqueuedUris.size == count) {
                latch.countDown()
            }
        })
        
        // Rebuild observer with new queue
        observer.unregister()
        observer = ScreenshotObserver(context.contentResolver, handler, queue)
        observer.register()

        // Write rapid fire screenshots to MediaStore
        val uris = (1..count).map { i ->
            insertMockScreenshot("Screenshot_rapid_$i.png", nowSeconds + i, isPending = 0)
        }

        // Trigger queries. In real scenarios, multiple changes arrive and trigger queries.
        // We run these queries on the handler thread to simulate real execution.
        handler.post {
            for (i in 1..count) {
                observer.queryLatestScreenshot()
            }
        }

        // Wait up to 5 seconds for evaluations to finish on the handler thread.
        latch.await(5, TimeUnit.SECONDS)

        // Verify the queue holds the last inserted screenshot (queue-of-1 behavior)
        val expectedLatest = uris.last()
        val actualLatest = queue.take()
        assertEquals("Queue should hold only the latest screenshot offered", expectedLatest, actualLatest)
        assertNull("Queue should be empty after taking the latest", queue.take())
    }

    @Test
    fun testObserverThreadIsNotBlocked() {
        val start = System.nanoTime()
        val nowSeconds = System.currentTimeMillis() / 1000

        // Offer 500 screenshots rapidly in the background observer handler thread
        val doneLatch = CountDownLatch(1)
        handler.post {
            for (i in 1..500) {
                queue.offer(Uri.parse("content://media/external/images/media/$i"))
            }
            doneLatch.countDown()
        }

        val awaitSuccess = doneLatch.await(2, TimeUnit.SECONDS)
        val end = System.nanoTime()
        val durationMs = (end - start) / 1_000_000

        assertTrue("Handler executions should finish quickly", awaitSuccess)
        assertTrue("Rapid-fire offers must be extremely fast, took ${durationMs}ms", durationMs < 500)
    }
}
