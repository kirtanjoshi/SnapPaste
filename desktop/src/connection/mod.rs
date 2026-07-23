mod server;

pub use server::{read_packet, write_packet};

use std::fmt;
use std::net::{TcpListener, TcpStream};
use std::time::Duration;
use std::thread;
use crate::adb::{ADBManager, DeviceStatus};
use crate::transfer::ImageTransferService;
use crate::protocol::{Packet, PacketType};



/// States of the SnapPaste connection state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    DeviceFound,
    AdbReady,
    ForwardReady,
    TcpConnected,
    Authenticated,
    Idle,
    ReceivingImage,
    UpdatingClipboard,
}

impl fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Events that trigger state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionEvent {
    /// Device connected and detected via ADB.
    DeviceDetected,
    /// ADB handshake or authorization succeeded.
    AdbAuthorized,
    /// ADB port forward command executed successfully.
    ForwardEstablished,
    /// TCP socket client connected.
    TcpClientConnected,
    /// Authentication/Pairing succeeded.
    AuthSuccess,
    /// Transition from authenticated state to waiting state.
    EnterIdle,
    /// START_IMAGE packet received.
    StartImageReceived,
    /// END_IMAGE or CANCEL packet received / transfer finished.
    TransferFinished,
    /// Clipboard operation completed or failed/busy recovery completed.
    ClipboardUpdated,
    
    // --- Recovery Edges ---
    /// USB unplugged / device disconnected at any point.
    UsbUnplugged,
    /// ADB server crashed or restart needed.
    AdbServerDeath,
    /// TCP connection timeout or PING-PONG heartbeat failure.
    TcpTimeout,
    /// Pairing token rejected or authentication failed.
    AuthFailure,
    /// Windows clipboard lock or write error.
    ClipboardBusy,
    /// Image transfer interrupted or cancelled.
    StreamCancelled,
}

impl fmt::Display for ConnectionEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Manages the state machine transitions.
pub struct ConnectionManager {
    state: ConnectionState,
}

impl ConnectionManager {
    /// Creates a new `ConnectionManager` initialized to `Disconnected`.
    pub fn new() -> Self {
        Self {
            state: ConnectionState::Disconnected,
        }
    }

    /// Gets the current state of the connection.
    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// Explicit transition table that maps `(current_state, event) -> next_state`.
    /// Returns `Ok(next_state)` on valid transitions, or `Err` if invalid.
    pub fn transition(&mut self, event: ConnectionEvent) -> Result<ConnectionState, String> {
        let current = self.state;

        // Global override: USB Unplugged transitions back to Disconnected from any state
        if event == ConnectionEvent::UsbUnplugged {
            let next = ConnectionState::Disconnected;
            if current != next {
                log::info!("[State Transition] {} --({})--> {}", current, event, next);
                self.state = next;
            }
            return Ok(self.state);
        }

        // Global override: ADB Server Death forces re-negotiation from ADB_READY
        if event == ConnectionEvent::AdbServerDeath {
            let next = ConnectionState::AdbReady;
            log::info!("[State Transition] {} --({})--> {}", current, event, next);
            self.state = next;
            return Ok(self.state);
        }

        // Global override: TCP Timeout/Heartbeat failure transitions back to TCP_CONNECTED for retry
        if event == ConnectionEvent::TcpTimeout {
            let next = ConnectionState::ForwardReady; // Listen/accept again
            log::info!("[State Transition] {} --({})--> {}", current, event, next);
            self.state = next;
            return Ok(self.state);
        }

        // Explicit transition table lookup
        let next = match (current, event) {
            // Normal Flow
            (ConnectionState::Disconnected, ConnectionEvent::DeviceDetected) => ConnectionState::DeviceFound,
            (ConnectionState::DeviceFound, ConnectionEvent::AdbAuthorized) => ConnectionState::AdbReady,
            (ConnectionState::AdbReady, ConnectionEvent::ForwardEstablished) => ConnectionState::ForwardReady,
            (ConnectionState::ForwardReady, ConnectionEvent::TcpClientConnected) => ConnectionState::TcpConnected,
            (ConnectionState::TcpConnected, ConnectionEvent::AuthSuccess) => ConnectionState::Authenticated,
            (ConnectionState::Authenticated, ConnectionEvent::EnterIdle) => ConnectionState::Idle,
            (ConnectionState::Idle, ConnectionEvent::StartImageReceived) => ConnectionState::ReceivingImage,
            (ConnectionState::ReceivingImage, ConnectionEvent::TransferFinished) => ConnectionState::UpdatingClipboard,
            (ConnectionState::UpdatingClipboard, ConnectionEvent::ClipboardUpdated) => ConnectionState::Idle,

            // State Machine Recovery Edges (per Architecture.md §6)
            // Auth failure -> DISCONNECTED (do not retry with same token silently)
            (ConnectionState::TcpConnected, ConnectionEvent::AuthFailure) => ConnectionState::Disconnected,

            // Clipboard busy/unavailable -> log + drop image, return to IDLE (never crash)
            (ConnectionState::UpdatingClipboard, ConnectionEvent::ClipboardBusy) => ConnectionState::Idle,

            // Stream cancelled mid-transfer -> discard partial buffer, return to IDLE, await next START_IMAGE
            (ConnectionState::ReceivingImage, ConnectionEvent::StreamCancelled) => ConnectionState::Idle,

            // Allowed idle/receiving cancellations
            (ConnectionState::Idle, ConnectionEvent::StreamCancelled) => ConnectionState::Idle,

            // Invalid transitions
            _ => {
                return Err(format!(
                    "Invalid transition attempt from state {} with event {}",
                    current, event
                ));
            }
        };

        log::info!("[State Transition] {} --({})--> {}", current, event, next);
        self.state = next;
        Ok(next)
    }
}


/// Runs the main connection lifecycle loop based on the state machine.
pub struct ConnectionRunner {
    adb: ADBManager,
    state_manager: ConnectionManager,
    local_port: u16,
    transfer_service: ImageTransferService,
}

impl ConnectionRunner {
    /// Creates a new `ConnectionRunner`.
    pub fn new(local_port: u16, device_port: u16, transfer_service: ImageTransferService) -> Self {
        Self {
            adb: ADBManager::new(local_port, device_port),
            state_manager: ConnectionManager::new(),
            local_port,
            transfer_service,
        }
    }

    /// Returns the current state.
    pub fn current_state(&self) -> ConnectionState {
        self.state_manager.state()
    }

    /// Single step of the connection runner. Run this in a loop.
    pub fn step(&mut self) {
        match self.state_manager.state() {
            ConnectionState::Disconnected => {
                log::info!("Checking for connected Android devices...");
                let (status, serial) = self.adb.check_device();
                match status {
                    DeviceStatus::Device | DeviceStatus::Unauthorized => {
                        log::info!("Device found: {:?}", serial);
                        let _ = self.state_manager.transition(ConnectionEvent::DeviceDetected);
                    }
                    DeviceStatus::None => {
                        // Sleep and retry detection
                        thread::sleep(Duration::from_millis(1000));
                    }
                }
            }

            ConnectionState::DeviceFound => {
                let (status, _) = self.adb.check_device();
                match status {
                    DeviceStatus::Device => {
                        log::info!("Device authorized. Performing auto-setup (granting permissions & starting service)...");
                        self.adb.grant_required_permissions();
                        self.adb.start_service();
                        let _ = self.state_manager.transition(ConnectionEvent::AdbAuthorized);
                    }
                    DeviceStatus::Unauthorized => {
                        log::warn!("Device connected but unauthorized. Please accept verification dialog on your phone.");
                        thread::sleep(Duration::from_millis(2000));
                    }
                    DeviceStatus::None => {
                        let _ = self.state_manager.transition(ConnectionEvent::UsbUnplugged);
                    }
                }
            }

            ConnectionState::AdbReady => {
                match self.adb.setup_forward() {
                    Ok(_) => {
                        log::info!("ADB Port forward setup successfully.");
                        let _ = self.state_manager.transition(ConnectionEvent::ForwardEstablished);
                    }
                    Err(e) => {
                        log::error!("Failed to establish port forward: {}. Retrying...", e);
                        let _ = self.state_manager.transition(ConnectionEvent::AdbServerDeath);
                        thread::sleep(Duration::from_millis(2000));
                    }
                }
            }

            ConnectionState::ForwardReady => {
                // Check if device is still attached
                let (status, _) = self.adb.check_device();
                if status == DeviceStatus::None {
                    log::warn!("Device disconnected while in ForwardReady.");
                    let _ = self.state_manager.transition(ConnectionEvent::UsbUnplugged);
                    self.adb.clear_forward();
                    return;
                }

                // Start raw TCP Server listener on local port
                let addr = format!("127.0.0.1:{}", self.local_port);
                log::info!("Listening for TCP connections on {}...", addr);
                
                let listener = match TcpListener::bind(&addr) {
                    Ok(l) => l,
                    Err(e) => {
                        log::error!("Failed to bind TCP listener to {}: {}. Retrying...", addr, e);
                        thread::sleep(Duration::from_millis(2000));
                        return;
                    }
                };

                // Set listener read/accept timeout so we check device connection periodically
                if let Err(e) = listener.set_nonblocking(true) {
                    log::warn!("Failed to set listener non-blocking: {}", e);
                }

                // Poll for connection
                loop {
                    // Quick check if device was disconnected
                    let (status, _) = self.adb.check_device();
                    if status == DeviceStatus::None {
                        log::warn!("Device disconnected while waiting for TCP client.");
                        let _ = self.state_manager.transition(ConnectionEvent::UsbUnplugged);
                        self.adb.clear_forward();
                        break;
                    }

                    match listener.accept() {
                        Ok((stream, client_addr)) => {
                            log::info!("Accepted connection from client: {}", client_addr);
                            let _ = self.state_manager.transition(ConnectionEvent::TcpClientConnected);
                            
                            // Process the client connection
                            self.handle_client(stream);
                            break;
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            // No connection pending, sleep a bit and loop
                            thread::sleep(Duration::from_millis(200));
                        }
                        Err(e) => {
                            log::error!("Accept failed: {}", e);
                            break;
                        }
                    }
                }
            }

            // In these active connection states, the handle_client function handles the loop
            ConnectionState::TcpConnected |
            ConnectionState::Authenticated |
            ConnectionState::Idle |
            ConnectionState::ReceivingImage |
            ConnectionState::UpdatingClipboard => {
                // If we get here directly, reset to ForwardReady to rebuild connection
                let _ = self.state_manager.transition(ConnectionEvent::TcpTimeout);
            }
        }
    }


    /// Handles communications with the single connected TCP client.
    fn handle_client(&mut self, mut stream: TcpStream) {
        use std::fs;
        use std::time::Instant;

        // Set standard read timeout of 2 seconds for responsive ping/pong heartbeats
        if let Err(e) = stream.set_read_timeout(Some(Duration::from_secs(2))) {
            log::warn!("Failed to set socket read timeout: {}", e);
        }

        // 1. Handshake Phase (HELLO)
        log::info!("Waiting for HELLO packet from Android...");
        match read_packet(&mut stream) {
            Ok(pkt) => {
                if pkt.packet_type == PacketType::Hello {
                    log::info!("Received HELLO packet from Android.");
                } else {
                    log::warn!("Unexpected packet type {:?} instead of HELLO", pkt.packet_type);
                    let _ = self.state_manager.transition(ConnectionEvent::AuthFailure);
                    return;
                }
            }
            Err(e) => {
                log::error!("Error reading initial HELLO packet: {}", e);
                let _ = self.state_manager.transition(ConnectionEvent::TcpTimeout);
                return;
            }
        }

        // 2. Authentication Phase (AUTH)
        log::info!("Waiting for AUTH packet from Android...");
        let token = match read_packet(&mut stream) {
            Ok(pkt) => {
                if pkt.packet_type == PacketType::Auth {
                    String::from_utf8_lossy(&pkt.payload).to_string()
                } else {
                    log::warn!("Unexpected packet type {:?} instead of AUTH", pkt.packet_type);
                    let _ = self.state_manager.transition(ConnectionEvent::AuthFailure);
                    return;
                }
            }
            Err(e) => {
                log::error!("Error reading AUTH packet: {}", e);
                let _ = self.state_manager.transition(ConnectionEvent::TcpTimeout);
                return;
            }
        };

        // Check/Store pairing token
        let token_path = "pairing_token.txt";
        let authenticated = if !fs::metadata(token_path).is_ok() {
            // First connect: save token
            log::info!("No pairing token found on Desktop. Saving received token as new pairing key.");
            if let Err(e) = fs::write(token_path, &token) {
                log::error!("Failed to write pairing_token.txt: {}", e);
                false
            } else {
                true
            }
        } else {
            // Subsequent connect: verify token
            match fs::read_to_string(token_path) {
                Ok(stored_token) => {
                    let match_ok = stored_token.trim() == token.trim();
                    if !match_ok {
                        log::warn!("Pairing token mismatch! Stored: '{}', Received: '{}'", stored_token.trim(), token.trim());
                    }
                    match_ok
                }
                Err(e) => {
                    log::error!("Failed to read pairing_token.txt: {}", e);
                    false
                }
            }
        };

        if authenticated {
            log::info!("Authentication/Pairing successful.");
            let _ = self.state_manager.transition(ConnectionEvent::AuthSuccess);
            let _ = self.state_manager.transition(ConnectionEvent::EnterIdle);
        } else {
            log::error!("Authentication failed. Closing connection.");
            let err_pkt = Packet::new(
                PacketType::Error,
                0,
                0,
                0,
                b"Authentication failed: pairing token mismatch".to_vec()
            );
            let _ = write_packet(&mut stream, &err_pkt);
            let _ = self.state_manager.transition(ConnectionEvent::AuthFailure);
            return;
        }

        // Initialize heartbeat tracking
        let mut last_activity = Instant::now();
        let mut last_ping = Instant::now();

        // 3. Message loop
        loop {
            // Check device status via ADB (USB unplug detection)
            let (status, _) = self.adb.check_device();
            if status == DeviceStatus::None {
                log::warn!("Device disconnected during session (USB unplugged).");
                let _ = self.state_manager.transition(ConnectionEvent::UsbUnplugged);
                self.adb.clear_forward();
                break;
            }

            // Periodic PING: Send a heartbeat ping every 5 seconds
            if last_ping.elapsed() > Duration::from_secs(5) {
                log::debug!("Sending PING heartbeat to Android...");
                let ping = Packet::new(PacketType::Ping, 0, 0, 0, vec![]);
                if let Err(e) = write_packet(&mut stream, &ping) {
                    log::error!("Failed to write heartbeat PING: {}", e);
                    let _ = self.state_manager.transition(ConnectionEvent::TcpTimeout);
                    break;
                }
                last_ping = Instant::now();
            }

            match read_packet(&mut stream) {
                Ok(pkt) => {
                    // Update heartbeat activity timer
                    last_activity = Instant::now();

                    match pkt.packet_type {
                        PacketType::Pong => {
                            log::debug!("Received PONG heartbeat response from Android.");
                        }
                        PacketType::Ping => {
                            log::debug!("Received PING from Android, sending PONG...");
                            let pong = Packet::new(PacketType::Pong, 0, pkt.image_id, pkt.sequence, vec![]);
                            if let Err(e) = write_packet(&mut stream, &pong) {
                                log::error!("Failed to send PONG: {}", e);
                                let _ = self.state_manager.transition(ConnectionEvent::TcpTimeout);
                                break;
                            }
                        }
                        PacketType::NewScreenshot => {
                            log::info!("Received NEW_SCREENSHOT notification for ImageID={}", pkt.image_id);
                        }
                        PacketType::StartImage => {
                            // Read expected size from payload (first 8 bytes as LE u64)
                            let expected_size = if pkt.payload.len() >= 8 {
                                u64::from_le_bytes(pkt.payload[0..8].try_into().unwrap())
                            } else {
                                0
                            };

                            let _ = self.state_manager.transition(ConnectionEvent::StartImageReceived);
                            self.transfer_service.start_image(pkt.image_id, expected_size);
                        }
                        PacketType::Chunk => {
                            self.transfer_service.append_chunk(pkt.image_id, &pkt.payload);
                        }
                        PacketType::Cancel => {
                            self.transfer_service.cancel_transfer(pkt.image_id);
                            let _ = self.state_manager.transition(ConnectionEvent::StreamCancelled);
                        }
                        PacketType::EndImage => {
                            self.transfer_service.finalize_image(pkt.image_id);
                            let _ = self.state_manager.transition(ConnectionEvent::TransferFinished);
                            let _ = self.state_manager.transition(ConnectionEvent::ClipboardUpdated);
                        }
                        PacketType::Disconnect => {
                            log::info!("Client requested clean disconnect.");
                            let _ = self.state_manager.transition(ConnectionEvent::UsbUnplugged);
                            break;
                        }
                        _ => {
                            log::info!("Unhandled packet type: {:?}", pkt.packet_type);
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                    // Check if inactivity has exceeded the 12-second keep-alive window
                    if last_activity.elapsed() > Duration::from_secs(12) {
                        log::warn!("Connection inactive for > 12 seconds. Heartbeat timeout triggered.");
                        let _ = self.state_manager.transition(ConnectionEvent::TcpTimeout);
                        break;
                    }
                }
                Err(e) => {
                    log::error!("Connection error: {}", e);
                    let _ = self.state_manager.transition(ConnectionEvent::TcpTimeout);
                    break;
                }
            }
        }

        // Clean up transfer state on client disconnect
        self.transfer_service.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_happy_path_transitions() {
        let mut manager = ConnectionManager::new();
        assert_eq!(manager.state(), ConnectionState::Disconnected);

        // Disconnected -> DeviceFound
        assert_eq!(
            manager.transition(ConnectionEvent::DeviceDetected).unwrap(),
            ConnectionState::DeviceFound
        );

        // DeviceFound -> AdbReady
        assert_eq!(
            manager.transition(ConnectionEvent::AdbAuthorized).unwrap(),
            ConnectionState::AdbReady
        );

        // AdbReady -> ForwardReady
        assert_eq!(
            manager.transition(ConnectionEvent::ForwardEstablished).unwrap(),
            ConnectionState::ForwardReady
        );

        // ForwardReady -> TcpConnected
        assert_eq!(
            manager.transition(ConnectionEvent::TcpClientConnected).unwrap(),
            ConnectionState::TcpConnected
        );

        // TcpConnected -> Authenticated
        assert_eq!(
            manager.transition(ConnectionEvent::AuthSuccess).unwrap(),
            ConnectionState::Authenticated
        );

        // Authenticated -> Idle
        assert_eq!(
            manager.transition(ConnectionEvent::EnterIdle).unwrap(),
            ConnectionState::Idle
        );

        // Idle -> ReceivingImage
        assert_eq!(
            manager.transition(ConnectionEvent::StartImageReceived).unwrap(),
            ConnectionState::ReceivingImage
        );

        // ReceivingImage -> UpdatingClipboard
        assert_eq!(
            manager.transition(ConnectionEvent::TransferFinished).unwrap(),
            ConnectionState::UpdatingClipboard
        );

        // UpdatingClipboard -> Idle
        assert_eq!(
            manager.transition(ConnectionEvent::ClipboardUpdated).unwrap(),
            ConnectionState::Idle
        );
    }

    #[test]
    fn test_usb_unplugged_from_any_state() {
        let states = vec![
            ConnectionState::Disconnected,
            ConnectionState::DeviceFound,
            ConnectionState::AdbReady,
            ConnectionState::ForwardReady,
            ConnectionState::TcpConnected,
            ConnectionState::Authenticated,
            ConnectionState::Idle,
            ConnectionState::ReceivingImage,
            ConnectionState::UpdatingClipboard,
        ];

        for start_state in states {
            let mut manager = ConnectionManager::new();
            manager.state = start_state;

            let next = manager.transition(ConnectionEvent::UsbUnplugged).unwrap();
            assert_eq!(next, ConnectionState::Disconnected);
        }
    }

    #[test]
    fn test_recovery_edges() {
        // ADB server death recovery
        let mut manager = ConnectionManager::new();
        manager.state = ConnectionState::ForwardReady;
        assert_eq!(
            manager.transition(ConnectionEvent::AdbServerDeath).unwrap(),
            ConnectionState::AdbReady
        );

        // TCP Timeout recovery
        manager.state = ConnectionState::Idle;
        assert_eq!(
            manager.transition(ConnectionEvent::TcpTimeout).unwrap(),
            ConnectionState::ForwardReady
        );

        // Auth failure recovery
        manager.state = ConnectionState::TcpConnected;
        assert_eq!(
            manager.transition(ConnectionEvent::AuthFailure).unwrap(),
            ConnectionState::Disconnected
        );

        // Clipboard busy recovery
        manager.state = ConnectionState::UpdatingClipboard;
        assert_eq!(
            manager.transition(ConnectionEvent::ClipboardBusy).unwrap(),
            ConnectionState::Idle
        );

        // Stream cancellation recovery
        manager.state = ConnectionState::ReceivingImage;
        assert_eq!(
            manager.transition(ConnectionEvent::StreamCancelled).unwrap(),
            ConnectionState::Idle
        );
    }

    #[test]
    fn test_invalid_transitions() {
        let mut manager = ConnectionManager::new();
        // Trying to establish forward when disconnected should fail
        assert!(manager.transition(ConnectionEvent::ForwardEstablished).is_err());
        assert_eq!(manager.state(), ConnectionState::Disconnected);
    }
}

