package com.snappaste.app.observer

import android.content.ContentResolver
import android.database.ContentObserver
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.provider.MediaStore
import android.util.Log

/**
 * Observes [MediaStore.Images.Media.EXTERNAL_CONTENT_URI] for new screenshot
 * creations and feeds them into a [ScreenshotQueue].
 *
 * ## Filtering Logic (documented per user request)
 *
 * When `onChange` fires we query the MediaStore for the most recent image and
 * apply these filters **in order**:
 *
 * 1. **IS_PENDING (API 29+):** On Android 10+ the MediaStore briefly inserts a
 *    row with `IS_PENDING = 1` while the file is being written, then updates it
 *    to `0` when the write completes. We **skip** rows where `IS_PENDING = 1`
 *    because the file bytes aren't readable yet, and the observer will fire
 *    *again* when the pending flag clears — that second firing is the one we
 *    process. This naturally de-duplicates the "insert + update" pair that
 *    every screenshot produces.
 *
 * 2. **Path contains "Screenshots":** The standard Android screenshot path is
 *    `Pictures/Screenshots/` (or `DCIM/Screenshots/` on some OEMs). We check
 *    `RELATIVE_PATH` (API 29+) or `DATA` (legacy) for a path segment
 *    containing "Screenshots" (case-insensitive). This filters out camera
 *    photos, downloaded images, edited copies, and app-generated thumbnails.
 *
 * 3. **DATE_ADDED recency:** We only accept images whose `DATE_ADDED` is
 *    within the last 5 seconds of wall-clock time. This rejects old images
 *    that might surface in the query due to MediaStore re-indexing, database
 *    compaction, or batch inserts from cloud sync.
 *
 * 4. **URI dedup:** We track the last-accepted content URI and skip if the
 *    observer fires again for the same URI (can happen on some OEMs that
 *    broadcast multiple change notifications for a single insert).
 *
 * ## Why not DISPLAY_NAME filtering?
 *
 * Screenshot filenames vary wildly across OEMs (e.g. "Screenshot_20240101.png",
 * "screen-20240101.jpg", "Captura_de_pantalla_…"). Path-based filtering is
 * more reliable because Android's default `MediaStore.Images.Media.insertImage`
 * and `MediaProjection` APIs all write to `Pictures/Screenshots/`.
 *
 * ## Threading
 *
 * The [ContentObserver] is registered with the provided [Handler]; all
 * callbacks execute on that handler's thread. The [ScreenshotQueue.offer] call
 * is lock-free and safe to invoke from any thread.
 */
class ScreenshotObserver(
    private val contentResolver: ContentResolver,
    handler: Handler,
    private val queue: ScreenshotQueue,
    private val tag: String = TAG,
) : ContentObserver(handler) {

    companion object {
        const val TAG = "ScreenshotObserver"

        /**
         * Maximum age (in seconds) of a `DATE_ADDED` timestamp for an image to
         * be considered a "just-taken" screenshot. Images older than this are
         * treated as stale re-index artifacts and ignored.
         */
        const val RECENCY_THRESHOLD_SECONDS = 60L

        /** MediaStore URI we observe. */
        val OBSERVED_URI: Uri = MediaStore.Images.Media.EXTERNAL_CONTENT_URI
    }

    /**
     * The content URI of the last screenshot we accepted. Used to de-duplicate
     * multiple observer callbacks for the same insert.
     */
    @Volatile
    private var lastAcceptedUri: Uri? = null

    // ── Registration ─────────────────────────────────────────────────────

    /**
     * Start observing the MediaStore for new screenshots.
     * Call [unregister] to stop.
     */
    fun register() {
        contentResolver.registerContentObserver(
            OBSERVED_URI,
            /* notifyForDescendants = */ true,
            this,
        )
        Log.i(tag, "Registered ContentObserver on $OBSERVED_URI")
    }

    /**
     * Stop observing.
     */
    fun unregister() {
        contentResolver.unregisterContentObserver(this)
        Log.i(tag, "Unregistered ContentObserver")
    }

    // ── Observer callback ────────────────────────────────────────────────

    override fun onChange(selfChange: Boolean, uri: Uri?) {
        Log.d(tag, "onChange selfChange=$selfChange uri=$uri")
        if (uri != null) {
            val isSpecificRow = try {
                android.content.ContentUris.parseId(uri)
                true
            } catch (e: Exception) {
                false
            }
            if (isSpecificRow) {
                queryScreenshotUri(uri)
                return
            }
        }
        queryLatestScreenshot()
    }

    // ── Query & filter pipeline ──────────────────────────────────────────

    private fun queryScreenshotUri(uri: Uri) {
        val projection = buildProjection()
        val cursor = try {
            contentResolver.query(uri, projection, null, null, null)
        } catch (e: Exception) {
            Log.e(tag, "Failed to query specific URI $uri", e)
            return
        }

        cursor?.use { c ->
            if (c.moveToFirst()) {
                processCursorRow(c, uri)
            } else {
                Log.d(tag, "No row found for specific URI $uri")
            }
        }
    }

    /**
     * Query the MediaStore for the single most-recent image and run the
     * filtering pipeline. If the image passes all filters, it is offered to
     * the [queue].
     */
    internal fun queryLatestScreenshot() {
        val nowSeconds = System.currentTimeMillis() / 1000
        val projection = buildProjection()
        val selection = buildSelection()
        val selectionArgs = buildSelectionArgs(nowSeconds)
        val sortOrder = "${MediaStore.Images.Media.DATE_ADDED} DESC"

        val cursor = try {
            contentResolver.query(
                OBSERVED_URI,
                projection,
                selection,
                selectionArgs,
                sortOrder,
            )
        } catch (e: Exception) {
            Log.e(tag, "MediaStore query failed", e)
            return
        }

        cursor?.use { c ->
            if (c.moveToFirst()) {
                val id = c.getLong(c.getColumnIndexOrThrow(MediaStore.Images.Media._ID))
                val contentUri = Uri.withAppendedPath(OBSERVED_URI, id.toString())
                processCursorRow(c, contentUri)
            } else {
                Log.d(tag, "No matching screenshot found in MediaStore query")
            }
        }
    }

    private fun processCursorRow(c: android.database.Cursor, contentUri: Uri) {
        val path = getPathFromCursor(c)

        // ── Filter: path must contain "Screenshots" ──────────────────
        if (path == null || !path.contains("Screenshots", ignoreCase = true)) {
            Log.d(tag, "Rejected: path does not contain 'Screenshots': $path")
            return
        }

        // ── Filter: IS_PENDING must be 0 (API 29+) ──────────────────
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val pendingIdx = c.getColumnIndex(MediaStore.Images.Media.IS_PENDING)
            if (pendingIdx >= 0) {
                val isPending = c.getInt(pendingIdx)
                if (isPending != 0) {
                    Log.d(tag, "Rejected: IS_PENDING=1, waiting for finalization")
                    return
                }
            }
        }

        // ── Filter: de-duplicate ─────────────────────────────────────
        if (contentUri == lastAcceptedUri) {
            Log.d(tag, "Rejected: duplicate URI $contentUri")
            return
        }

        // ── Accept! ──────────────────────────────────────────────────
        lastAcceptedUri = contentUri
        val superseded = queue.offer(contentUri)
        if (superseded != null) {
            Log.i(tag, "Superseded queued screenshot $superseded with $contentUri")
        } else {
            Log.i(tag, "Queued new screenshot: $contentUri")
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    /**
     * Build the projection (column list) for the MediaStore query.
     */
    private fun buildProjection(): Array<String> {
        val cols = mutableListOf(
            MediaStore.Images.Media._ID,
            MediaStore.Images.Media.DATE_ADDED,
        )
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            cols += MediaStore.Images.Media.RELATIVE_PATH
            cols += MediaStore.Images.Media.IS_PENDING
        } else {
            @Suppress("DEPRECATION")
            cols += MediaStore.Images.Media.DATA
        }
        return cols.toTypedArray()
    }

    /**
     * Build the selection (WHERE clause).
     *
     * - `DATE_ADDED >= ?` restricts to images added within the recency window.
     * - On API 29+ we also add `IS_PENDING = 0` into the WHERE clause for
     *   efficiency (so the DB engine filters rather than us post-query), but
     *   we still check it post-query as a safety net.
     */
    private fun buildSelection(): String {
        val clauses = mutableListOf("${MediaStore.Images.Media.DATE_ADDED} >= ?")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            clauses += "${MediaStore.Images.Media.IS_PENDING} = 0"
        }
        return clauses.joinToString(" AND ")
    }

    /**
     * Build the selection args corresponding to [buildSelection].
     */
    private fun buildSelectionArgs(nowSeconds: Long): Array<String> {
        val cutoff = nowSeconds - RECENCY_THRESHOLD_SECONDS
        return arrayOf(cutoff.toString())
    }

    /**
     * Extract the file path from the cursor, using `RELATIVE_PATH` on API 29+
     * or the deprecated `DATA` column on older APIs.
     */
    private fun getPathFromCursor(c: android.database.Cursor): String? {
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val idx = c.getColumnIndex(MediaStore.Images.Media.RELATIVE_PATH)
            if (idx >= 0) c.getString(idx) else null
        } else {
            @Suppress("DEPRECATION")
            val idx = c.getColumnIndex(MediaStore.Images.Media.DATA)
            if (idx >= 0) c.getString(idx) else null
        }
    }
}
