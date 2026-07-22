PROJECT: SnapPaste — instant Android screenshot → Windows clipboard utility.

GOAL
Screenshot on Android → binary streamed over a private TCP tunnel (via ADB port
forward) → Windows clipboard updated → user presses Ctrl+V anywhere.
Target end-to-end latency: <150ms, hard ceiling 300ms.

NON-NEGOTIABLE ARCHITECTURE DECISIONS (do not revisit or "improve" these
without being asked — they were already decided after review):

1. TRANSPORT: raw TCP socket over `adb forward`. No WebSocket, no HTTP,
   no Electron/Tauri, no polling anywhere.

2. SCREENSHOT DETECTION (Android): MediaStore ContentObserver on
   MediaStore.Images.Media.EXTERNAL_CONTENT_URI is PRIMARY.
   FileObserver is an optional OEM-specific fallback only, never primary,
   because scoped storage on Android 10+ makes raw FileObserver unreliable
   without MANAGE_EXTERNAL_STORAGE.

3. ANDROID SERVICE: Foreground Service with
   android:foregroundServiceType="dataSync" (required on Android 14+ or the
   service throws at runtime).

4. QUEUE POLICY: exactly one in-flight "latest screenshot" slot per side.
   New screenshot arriving replaces the queued one. If a send is already
   in-flight when a newer screenshot arrives, the in-flight send is
   cancelled via the wire protocol, not just replaced client-side. Use an
   AtomicReference/MutableStateFlow-safe structure on Android, not a plain
   var, since replace can race with an active read.

5. WIRE PROTOCOL: custom binary framing over the raw TCP socket.
   Every packet header (little-endian):
     Magic       4 bytes   (constant, e.g. 0x53 0x4E 0x50 0x50 "SNPP")
     Version     1 byte
     Type        1 byte    (HELLO, AUTH, NEW_SCREENSHOT, START_IMAGE,
                             CHUNK, CANCEL, END_IMAGE, ACK, PING, PONG,
                             DISCONNECT, ERROR)
     Flags       2 bytes
     ImageID     4 bytes   (monotonically increasing per screenshot;
                             REQUIRED on START_IMAGE/CHUNK/CANCEL/END_IMAGE
                             so the receiver can discard chunks belonging
                             to a cancelled/superseded image)
     Sequence    4 bytes   (per-connection monotonic packet counter, for
                             debugging/logging, not reassembly)
     Length      4 bytes   (payload length)
     Payload     N bytes
   Image transfer is chunked: START_IMAGE -> CHUNK* -> END_IMAGE.
   Receiver drops any CHUNK/END_IMAGE whose ImageID != currently active ID.

6. STATE MACHINE (both sides mirror this, model as an explicit enum +
   transition table, NOT a linear happy-path diagram):
     DISCONNECTED -> DEVICE_FOUND -> ADB_READY -> FORWARD_READY ->
     TCP_CONNECTED -> AUTHENTICATED -> IDLE -> RECEIVING_IMAGE ->
     UPDATING_CLIPBOARD -> back to IDLE
   Required recovery edges (every one of these must have an explicit
   documented transition, not be left implicit):
     - USB unplugged (from any state)         -> DISCONNECTED, retry loop
     - ADB restart / adb server death          -> ADB_READY re-negotiation
     - TCP timeout / PING-PONG failure          -> TCP_CONNECTED retry
     - Auth failure                             -> DISCONNECTED (do not retry
                                                    with same token silently)
     - Clipboard busy/unavailable               -> log + drop image, return
                                                    to IDLE (never crash)
     - Stream cancelled mid-transfer            -> discard partial buffer,
                                                    return to IDLE, await
                                                    next START_IMAGE

7. AUTH: one-time pairing token generated on first connect, stored in
   EncryptedSharedPreferences (Android) and a local config file (Desktop).
   AUTH packet required before any NEW_SCREENSHOT/IMAGE traffic is accepted.

8. CLIPBOARD (Windows/Rust): must use a hidden message-only window
   (HWND_MESSAGE) created at startup solely to own OpenClipboard/
   SetClipboardData calls. Copy PNG bytes directly, preserve transparency,
   no Base64, no unnecessary BMP conversion.

9. ADB MANAGEMENT: no repeated shelling out to adb.exe during steady-state
   operation. A dedicated ADBManager component owns: device detection,
   issuing `adb forward`, and re-issuing it automatically on device
   reconnect. All application traffic after that flows over the raw TCP
   socket, never adb shell.

TECH STACK (locked):
  Desktop: Rust, windows-rs, tray-icon, raw std::net TCP, no async runtime
           unless profiling proves it's needed.
  Android: Kotlin, MediaStore ContentObserver, Foreground Service
           (dataSync), Kotlin Coroutines, java.net.Socket / custom TCP
           client (no OkHttp/WebSocket libraries).

FOLDER STRUCTURE (do not restructure without asking):

Desktop (Rust)
  src/
    adb/            -> ADBManager: device detection, forward, reconnect
    connection/      -> ConnectionManager + state machine
    protocol/         -> packet encode/decode, framing
    transfer/          -> ImageTransferService (chunk reassembly, cancel)
    clipboard/          -> ClipboardService + HWND_MESSAGE owner
    pairing/            -> PairingManager, token storage
    tray/                -> TrayService, menu, icons
    settings/             -> SettingsService (auto-start, last device, log level)
    logging/               -> structured logger, rotation
    config/                 -> app config load/save

Android (Kotlin)
  app/
    service/          -> ForegroundService entrypoint
    observer/          -> MediaStore ContentObserver, screenshot queue
    network/            -> ConnectionManager, protocol, TCP client
    pairing/              -> PairingManager, EncryptedSharedPreferences
    settings/              -> user-facing settings
    storage/                -> minimal local persistence (paired device, token)
    utils/                    -> logging, coroutine dispatchers, extensions

CODING STANDARDS:
  - SOLID, clean architecture, one responsibility per module/service listed
    above. No god-classes.
  - Dependency injection where it reduces coupling; don't force DI for
    trivial leaf classes.
  - Comprehensive doc comments on public APIs.
  - Unit tests for: packet encode/decode round-trip, state machine
    transitions, queue replace-under-race behavior, clipboard service
    (mockable Win32 calls).
  - All errors are handled and logged; NONE may crash the app or service.
    Errors to explicitly handle: ADB unavailable, device disconnected,
    screenshot read failure, clipboard unavailable, socket timeout,
    permission denied, auth failure.
  - Graceful shutdown: sockets closed, foreground service stopped cleanly,
    hidden window destroyed, no orphaned threads.

PERFORMANCE BUDGETS (do not regress these when adding features):
  Idle CPU <1%, Idle RAM <15MB (desktop), clipboard update <20ms,
  connection recovery <2s, cold startup <300ms, end-to-end latency
  target <150ms / ceiling 300ms.

WHEN IMPLEMENTING ANY MILESTONE BELOW: follow this document's decisions
exactly. If you believe a decision here is wrong, stop and ask before
deviating — do not silently "improve" the architecture mid-milestone.