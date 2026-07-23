package com.snappaste.app.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * BroadcastReceiver to automatically start SnapPasteService when the device boots
 * or when it is plugged into a power source (USB/AC), and stop it when unplugged.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val action = intent.action
        Log.i("BootReceiver", "Received broadcast action: $action")
        if (action == Intent.ACTION_BOOT_COMPLETED || 
            action == "android.intent.action.QUICKBOOT_POWERON" ||
            action == "com.htc.intent.action.QUICKBOOT_POWERON" ||
            action == Intent.ACTION_POWER_CONNECTED
        ) {
            Log.i("BootReceiver", "Starting SnapPaste service automatically.")
            SnapPasteService.start(context)
        } else if (action == Intent.ACTION_POWER_DISCONNECTED) {
            Log.i("BootReceiver", "Power disconnected, stopping SnapPaste service.")
            SnapPasteService.stop(context)
        }
    }
}
