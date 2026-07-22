# SnapPaste — End-to-End Reliability Verification Guide

This document describes how to execute the reliability tests and verify the robust recovery behaviors implemented in the SnapPaste connection and transmission layers.

---

## 1. Rapid-Fire Screenshot Test (Queue-of-1 & Cancel Verification)

This test validates that when multiple screenshots are taken in quick succession, older in-flight transfers are cancelled immediately, resources are freed, and only the latest screenshot is written to the Windows clipboard.

### Automated Simulation via ADB

You can run this shell script/command to simulate 5 screenshots arriving within 500ms:

```bash
# Execute this in a loop to write 5 mock screenshots to the device MediaStore
for i in {1..5}; do
  adb shell content insert --uri content://media/external/images/media \
    --bind name:s:"Screenshot_rapid_$i.png" \
    --bind mime_type:s:"image/png" \
    --bind relative_path:s:"Pictures/Screenshots" \
    --bind is_pending:i:0
  sleep 0.1
done
```

### Expected Log Output

#### Android Service Logs (`adb logcat -s SnapPasteService TcpClient ScreenshotObserver`)
```text
I/ScreenshotObserver: Queued new screenshot: content://media/external/images/media/1001
I/TcpClient: Starting transmission of screenshot: URI=content://media/.../1001, ImageID=1
I/ScreenshotObserver: Superseded queued screenshot content://media/.../1001 with content://media/.../1002
I/TcpClient: New screenshot arrived during transmission. Cancelling active transfer.
I/TcpClient: Screenshot transfer cancelled.
I/TcpClient: Sent CANCEL packet for previous transfer.
I/TcpClient: Starting transmission of screenshot: URI=content://media/.../1002, ImageID=2
...
I/TcpClient: Successfully completed transmission for ImageID=5
```

#### Desktop Utility Logs (`app.log`)
```text
[INFO] [connection:293] START_IMAGE: ImageID=1, expected size=854932 bytes
[INFO] [connection:329] Received CANCEL for ImageID=1
[INFO] [connection:293] START_IMAGE: ImageID=2, expected size=854932 bytes
[INFO] [connection:329] Received CANCEL for ImageID=2
...
[INFO] [connection:335] Received END_IMAGE for ImageID=5. Written=854932 bytes.
[INFO] [clipboard:48] [Performance] Clipboard updated successfully in 12 ms (Size: 854932 bytes).
```

---

## 2. State Machine Recovery Edges Verification

### A. USB Unplug/Replug Recovery
1. Start the Desktop application (status: `Watching ADB...`).
2. Connect your Android phone via USB.
3. Observe connection states: `Disconnected -> DeviceFound -> AdbReady -> ForwardReady -> TcpConnected -> Authenticated -> Idle`.
4. While connected, **unplug the USB cable**.
5. **Expected result:**
   - Desktop transitions to `Disconnected` immediately.
   - Android client transitions to `DISCONNECTED` and starts reconnection retries.
6. Replug the USB cable.
7. **Expected result:** State machine automatically re-establishes the connection back to `Idle` in <2 seconds.

### B. ADB Server Crash Recovery
1. While connected and in `Idle` state, force-kill the local ADB server on your PC:
   ```bash
   adb kill-server
   ```
2. **Expected result:**
   - Desktop runner detects the ADB failure, removes the forward configuration, logs the server death, transitions to `AdbReady`, and waits to re-verify the device.
   - Connection is cleanly closed, and rebuilt as soon as the ADB server restarts.

### C. TCP Heartbeat & Keep-Alive Timeout
1. Disable mobile data/Wifi and mock a silent disconnect (e.g. disable ADB networking or simulate network drops).
2. The Desktop sends a `PING` every 5 seconds.
3. If the Android client is disconnected silently and does not respond with `PONG`:
   - After **12 seconds** of inactivity, the Desktop triggers `TcpTimeout` and closes the socket.
   - It transitions to `ForwardReady` to wait for a new connection.
   - Android detects read failure, transitions to `DISCONNECTED`, and enters the retry loop.

### D. Clipboard Lock & Busy Recovery
1. Open another program on Windows that continuously locks the clipboard (or open clipboard and keep it open).
2. Send a screenshot from Android.
3. **Expected result:**
   - `ClipboardService` logs `Clipboard copy failed: Failed to OpenClipboard.`
   - Desktop application does **not** crash. It drops the image, logs the failure, and returns to `Idle` state safely.

---

## 3. Performance Budget Targets Checklist

Verify these log metrics in `app.log`:
- **Cold Startup Time:** Target `<300ms` (Logged as `[Performance] Cold startup completed in X ms`).
- **Clipboard Update Time:** Target `<20ms` (Logged as `[Performance] Clipboard updated successfully in X ms`).
- **Idle RAM footprint:** `<15MB` (No memory leaks are present as memory is cleaned on cancel).
