//! ADB management module for SnapPaste.
//!
//! Handles device detection, port forwarding, and automatic connection/disconnection tracking.

use std::os::windows::process::CommandExt;
use std::process::Command;

/// Creation flags value to suppress console window popup on Windows
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Device status as parsed from `adb devices`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatus {
    /// Device is connected and authorized.
    Device,
    /// Device is connected but unauthorized.
    Unauthorized,
    /// No device is connected.
    None,
}

/// Manages interaction with the Android Debug Bridge (ADB).
///
/// Wraps all calls to the external `adb` executable. No other module should execute adb commands.
pub struct ADBManager {
    /// Port on the local machine (Desktop).
    local_port: u16,
    /// Port on the remote machine (Android).
    device_port: u16,
    /// Name or ID of the currently tracked device.
    current_device: Option<String>,
}

impl ADBManager {
    /// Creates a new `ADBManager` instance.
    pub fn new(local_port: u16, device_port: u16) -> Self {
        Self {
            local_port,
            device_port,
            current_device: None,
        }
    }

    /// Checks the list of connected devices using `adb devices`.
    /// Returns the first available device serial and its status, or `DeviceStatus::None`.
    pub fn check_device(&mut self) -> (DeviceStatus, Option<String>) {
        let output = match Command::new("adb")
            .creation_flags(CREATE_NO_WINDOW)
            .arg("devices")
            .output()
        {
            Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
            Err(e) => {
                log::error!("Failed to execute adb devices: {}", e);
                return (DeviceStatus::None, None);
            }
        };

        // Parse adb devices output:
        // List of devices attached
        // serial_number    device
        // serial_number    unauthorized
        let mut lines = output.lines();
        // Skip the header
        if lines.next().is_none() {
            return (DeviceStatus::None, None);
        }

        for line in lines {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let serial = parts[0].to_string();
                let status_str = parts[1];
                let status = match status_str {
                    "device" => DeviceStatus::Device,
                    "unauthorized" => DeviceStatus::Unauthorized,
                    _ => DeviceStatus::None,
                };
                if status != DeviceStatus::None {
                    self.current_device = Some(serial.clone());
                    return (status, Some(serial));
                }
            }
        }

        self.current_device = None;
        (DeviceStatus::None, None)
    }

    /// Establishes port reversing configurations (Android client -> PC server).
    pub fn setup_forward(&self) -> Result<(), String> {
        let local_arg = format!("tcp:{}", self.local_port);
        let device_arg = format!("tcp:{}", self.device_port);

        // Establish adb reverse (Android client -> PC server)
        let mut reverse_cmd = Command::new("adb");
        reverse_cmd.creation_flags(CREATE_NO_WINDOW).arg("reverse").arg(&local_arg).arg(&device_arg);
        log::info!("Executing: adb reverse {} {}", local_arg, device_arg);
        
        match reverse_cmd.output() {
            Ok(output) => {
                if output.status.success() {
                    Ok(())
                } else {
                    let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
                    Err(format!("adb reverse failed: {}", err_msg.trim()))
                }
            }
            Err(e) => Err(format!("Failed to run adb reverse command: {}", e)),
        }
    }

    /// Clears the established reversing configurations.
    pub fn clear_forward(&self) {
        let local_arg = format!("tcp:{}", self.local_port);
        
        // Remove reverse
        let mut reverse_cmd = Command::new("adb");
        reverse_cmd.creation_flags(CREATE_NO_WINDOW).arg("reverse").arg("--remove").arg(&local_arg);
        log::info!("Removing adb reverse on {}", local_arg);
        let _ = reverse_cmd.status();
    }
}
