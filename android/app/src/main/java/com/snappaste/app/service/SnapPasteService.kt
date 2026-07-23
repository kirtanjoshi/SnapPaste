package com.snappaste.app.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.util.Log
import com.snappaste.app.network.TcpClient
import com.snappaste.app.network.ConnectionState
import com.snappaste.app.observer.ScreenshotObserver
import com.snappaste.app.observer.ScreenshotQueue
import com.snappaste.app.pairing.PairingManager
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel

/**
 * Foreground service that keeps the screenshot observer alive.
 *
 * ## Lifecycle (per Architecture.md §3)
 *
 * - Uses `foregroundServiceType="dataSync"` (required on Android 14+).
 * - Starts a dedicated [HandlerThread] for the [ScreenshotObserver] so the
 *   main thread is never blocked by MediaStore queries.
 * - On [onStartCommand]: promotes to foreground, registers the observer.
 * - On [onDestroy]: unregisters the observer, quits the handler thread, clears
 *   the queue, cancels network client coroutines — no orphaned threads or leaked observers.
 */
class SnapPasteService : Service() {

    companion object {
        const val TAG = "SnapPasteService"

        const val NOTIFICATION_CHANNEL_ID = "snappaste_service_channel"
        const val NOTIFICATION_CHANNEL_NAME = "SnapPaste Service"
        const val NOTIFICATION_ID = 1

        /** Intent action to stop the service from the notification. */
        const val ACTION_STOP = "com.snappaste.app.action.STOP_SERVICE"

        // ── Convenience starters ─────────────────────────────────────────

        fun start(context: Context) {
            val intent = Intent(context, SnapPasteService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, SnapPasteService::class.java))
        }
    }

    private lateinit var observerThread: HandlerThread
    private lateinit var observer: ScreenshotObserver
    private lateinit var queue: ScreenshotQueue

    private val serviceJob = SupervisorJob()
    private val serviceScope = CoroutineScope(Dispatchers.Default + serviceJob)

    private lateinit var pairingManager: PairingManager
    private lateinit var tcpClient: TcpClient

    // ── Service lifecycle ────────────────────────────────────────────────

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "Service created")

        pairingManager = PairingManager(applicationContext)
        tcpClient = TcpClient(contentResolver, pairingManager).apply {
            onStateChanged = { state ->
                val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
                nm.notify(NOTIFICATION_ID, buildNotification(state))
            }
        }
        
        // Start TCP connection loop
        tcpClient.start(serviceScope)

        // ── Queue wired to TCP client ────────────────────────────────────
        queue = ScreenshotQueue(
            onEnqueued = { uri ->
                Log.i(TAG, "Queue enqueued new screenshot: $uri, offering to TCP client")
                tcpClient.offerScreenshot(uri)
            },
        )

        // ── Background thread for the observer ──────────────────────────
        observerThread = HandlerThread("ScreenshotObserverThread").also { it.start() }
        val handler = Handler(observerThread.looper)

        observer = ScreenshotObserver(
            contentResolver = contentResolver,
            handler = handler,
            queue = queue,
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Handle stop action from notification
        if (intent?.action == ACTION_STOP) {
            Log.i(TAG, "Stop action received, stopping service")
            stopSelf()
            return START_NOT_STICKY
        }

        promoteToForeground()
        observer.register()

        Log.i(TAG, "Service started, observer registered")

        // If the system kills us, restart automatically.
        return START_STICKY
    }

    override fun onDestroy() {
        Log.i(TAG, "Service destroying — cleaning up")

        observer.unregister()
        observerThread.quitSafely()
        queue.clear()
        
        // Cancel all network coroutines
        serviceScope.cancel("Service destroyed")

        super.onDestroy()
        Log.i(TAG, "Service destroyed")
    }

    override fun onBind(intent: Intent?): IBinder? = null // Not a bound service.

    // ── Foreground notification ──────────────────────────────────────────

    /**
     * Promote this service to the foreground with a persistent notification.
     *
     * On Android 14+ (API 34) we must specify the `foregroundServiceType`
     * parameter in [startForeground] to match the manifest declaration.
     */
    private fun promoteToForeground(state: ConnectionState = ConnectionState.DISCONNECTED) {
        createNotificationChannel()

        val notification = buildNotification(state)

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            // Android 14+: must specify foregroundServiceType at runtime.
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                NOTIFICATION_CHANNEL_ID,
                NOTIFICATION_CHANNEL_NAME,
                NotificationManager.IMPORTANCE_LOW,  // silent, no sound/vibration
            ).apply {
                description = "Keeps SnapPaste running to detect screenshots"
                setShowBadge(false)
            }
            val nm = getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(channel)
        }
    }

    private fun buildNotification(state: ConnectionState = ConnectionState.DISCONNECTED): Notification {
        // Stop action PendingIntent
        val stopIntent = Intent(this, SnapPasteService::class.java).apply {
            action = ACTION_STOP
        }
        val stopPendingIntent = PendingIntent.getService(
            this, 0, stopIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

        val statusText = when (state) {
            ConnectionState.DISCONNECTED -> "Waiting for PC connection…"
            ConnectionState.TCP_CONNECTED -> "Connecting to PC…"
            ConnectionState.AUTHENTICATED -> "Authenticating…"
            ConnectionState.IDLE -> "Active • Connected to PC"
            ConnectionState.SENDING_IMAGE -> "Beaming screenshot…"
        }

        return Notification.Builder(this, NOTIFICATION_CHANNEL_ID)
            .setContentTitle("SnapPaste")
            .setContentText(statusText)
            .setSmallIcon(android.R.drawable.ic_menu_camera)  // placeholder icon
            .setOngoing(true)
            .addAction(
                Notification.Action.Builder(
                    null, "Stop", stopPendingIntent,
                ).build()
            )
            .build()
    }
}
