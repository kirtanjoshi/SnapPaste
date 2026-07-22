//! System tray service.

use std::sync::mpsc::Sender;
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

/// Events emitted by the system tray interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// Toggle active On/Off state.
    ToggleActive,
    /// Toggle Auto-start preference.
    ToggleAutoStart,
    /// Open the logs file.
    OpenLogs,
    /// Exit the application.
    Exit,
}

/// System tray management service.
pub struct TrayService {
    _tray_icon: TrayIcon,
    auto_start_item: CheckMenuItem,
    active_item: CheckMenuItem,
    status_item: MenuItem,
}

impl TrayService {
    /// Creates and displays the system tray icon.
    pub fn new(event_tx: Sender<TrayEvent>, auto_start_initial: bool, main_thread_id: u32) -> Result<Self, String> {
        let tray_menu = Menu::new();

        // 1. Status Label (disabled item)
        let status_item = MenuItem::with_id(
            "status_label",
            "● SnapPaste: Watching ADB...",
            false,
            None,
        );
        let _ = tray_menu.append(&status_item);
        let _ = tray_menu.append(&PredefinedMenuItem::separator());

        // 2. Active Toggle (On/Off)
        let active_item = CheckMenuItem::with_id(
            "active_toggle",
            "Enabled",
            true,
            true, // initially active (On)
            None,
        );
        let _ = tray_menu.append(&active_item);

        // 3. Options
        let auto_start_item = CheckMenuItem::with_id(
            "auto_start",
            "Start with Windows",
            true,
            auto_start_initial,
            None,
        );
        let _ = tray_menu.append(&auto_start_item);

        let logs_item = MenuItem::with_id("open_logs", "View Application Logs", true, None);
        let _ = tray_menu.append(&logs_item);

        let _ = tray_menu.append(&PredefinedMenuItem::separator());

        // 4. Exit Action
        let exit_item = MenuItem::with_id("exit", "Exit", true, None);
        let _ = tray_menu.append(&exit_item);

        // Generate simple 16x16 pixel layout for the tray icon (Blue icon with a central white dot)
        let icon_width = 16;
        let icon_height = 16;
        let mut rgba = vec![0u8; (icon_width * icon_height * 4) as usize];
        for y in 0..icon_height {
            for x in 0..icon_width {
                let idx = ((y * icon_width + x) * 4) as usize;
                // Draw a simple blue square with a light-blue center
                rgba[idx] = 41;       // R
                rgba[idx + 1] = 121;  // G
                rgba[idx + 2] = 255;  // B
                rgba[idx + 3] = 255;  // A
                if x >= 6 && x <= 9 && y >= 6 && y <= 9 {
                    rgba[idx] = 255;
                    rgba[idx + 1] = 255;
                    rgba[idx + 2] = 255;
                }
            }
        }

        let icon = Icon::from_rgba(rgba, icon_width, icon_height)
            .map_err(|e| format!("Failed to create icon pixels: {}", e))?;

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip("SnapPaste screenshot utility")
            .with_icon(icon)
            .build()
            .map_err(|e| format!("Failed to build tray icon: {}", e))?;

        // Spawn a thread to monitor click events and forward them
        std::thread::Builder::new()
            .name("TrayEventsThread".to_string())
            .spawn(move || {
                let menu_channel = tray_icon::menu::MenuEvent::receiver();
                
                // Monitor both menu events and tray icon events
                let tray_channel = tray_icon::TrayIconEvent::receiver();
                
                loop {
                    // 1. Process Menu Events
                    if let Ok(event) = menu_channel.try_recv() {
                        let id = event.id.0.as_str();
                        let tray_event = match id {
                            "active_toggle" => Some(TrayEvent::ToggleActive),
                            "auto_start" => Some(TrayEvent::ToggleAutoStart),
                            "open_logs" => Some(TrayEvent::OpenLogs),
                            "exit" => Some(TrayEvent::Exit),
                            _ => None,
                        };
                        
                        if let Some(ev) = tray_event {
                            if event_tx.send(ev).is_ok() {
                                // Wake up main thread message loop
                                unsafe {
                                    let _ = windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                                        main_thread_id,
                                        windows::Win32::UI::WindowsAndMessaging::WM_USER + 2,
                                        windows::Win32::Foundation::WPARAM(0),
                                        windows::Win32::Foundation::LPARAM(0),
                                    );
                                }
                            }
                        }
                    }
                    
                    // 2. Process Left Click Tray Icon Events
                    if let Ok(event) = tray_channel.try_recv() {
                        if let tray_icon::TrayIconEvent::Click {
                            button: tray_icon::MouseButton::Left,
                            ..
                        } = event
                        {
                            if event_tx.send(TrayEvent::ToggleActive).is_ok() {
                                // Wake up main thread message loop
                                unsafe {
                                    let _ = windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                                        main_thread_id,
                                        windows::Win32::UI::WindowsAndMessaging::WM_USER + 2,
                                        windows::Win32::Foundation::WPARAM(0),
                                        windows::Win32::Foundation::LPARAM(0),
                                    );
                                }
                            }
                        }
                    }
                    
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            })
            .map_err(|e| format!("Failed to start tray events thread: {}", e))?;

        Ok(Self {
            _tray_icon: tray_icon,
            auto_start_item,
            active_item,
            status_item,
        })
    }

    /// Update the checkbox state for auto_start menu option.
    pub fn set_auto_start_checked(&self, checked: bool) {
        self.auto_start_item.set_checked(checked);
    }

    /// Update the status and checked state for active (On/Off) toggle.
    pub fn set_active_checked(&self, checked: bool) {
        self.active_item.set_checked(checked);
        if checked {
            self.status_item.set_text("● SnapPaste: Watching ADB...");
        } else {
            self.status_item.set_text("○ SnapPaste: Paused");
        }
    }
}
