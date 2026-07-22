#![windows_subsystem = "windows"]

//! SnapPaste Desktop — main entry point.

mod adb;
mod clipboard;
mod connection;
mod logging;
mod protocol;
mod settings;
mod transfer;
mod tray;

use clipboard::ClipboardService;
use connection::ConnectionRunner;
use logging::RotatingLogger;
use settings::SettingsService;
use transfer::ImageTransferService;
use tray::{TrayEvent, TrayService};

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;


fn main() {
    // 1. Change current working directory to the executable's parent directory
    // This prevents permission crashes when launched by Windows Startup in C:\Windows\system32
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let _ = std::env::set_current_dir(exe_dir);
        }
    }

    let start_time = std::time::Instant::now();

    // 1. Initialize rotating logger
    if let Err(e) = RotatingLogger::init(log::LevelFilter::Info) {
        eprintln!("Failed to initialize logging: {}", e);
    }

    log::info!("==========================================");
    log::info!("Starting SnapPaste Desktop utility...");

    // 2. Load Settings
    let settings_service = SettingsService::new();
    let mut settings = settings_service.load();
    log::info!("Settings loaded: {:?}", settings);

    // Synchronize Windows Startup Registry
    let _ = set_startup_registration(settings.auto_start);

    // 3. Initialize ClipboardService (spawns background HWND_MESSAGE owner thread)
    let clipboard_service = match ClipboardService::new() {
        Ok(service) => Arc::new(service),
        Err(e) => {
            log::error!("CRITICAL: ClipboardService initialization failed: {}", e);
            return;
        }
    };

    // 4. Initialize Active/Paused State and ImageTransferService
    let is_active = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let transfer_service = ImageTransferService::new(Arc::clone(&clipboard_service), Arc::clone(&is_active));

    // 5. Initialize Tray System (passing main thread ID so it can wake the Win32 loop)
    let main_thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
    let (tray_tx, tray_rx) = mpsc::channel();
    let tray_service = match TrayService::new(tray_tx, settings.auto_start, main_thread_id) {
        Ok(t) => Arc::new(t),
        Err(e) => {
            log::error!("CRITICAL: TrayService initialization failed: {}", e);
            return;
        }
    };

    // 6. Spawn ConnectionRunner on a dedicated thread to avoid blocking the UI thread
    let mut runner = ConnectionRunner::new(9999, 9999, transfer_service);
    
    // Flag to control runner thread termination
    let is_running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let is_running_runner = Arc::clone(&is_running);

    let connection_thread = thread::Builder::new()
        .name("ConnectionRunnerThread".to_string())
        .spawn(move || {
            log::info!("Connection worker thread started.");
            while is_running_runner.load(std::sync::atomic::Ordering::Relaxed) {
                // Execute a single step (handles device checks, listens, accepts, reads, transfers)
                runner.step();
                thread::sleep(Duration::from_millis(50));
            }
            log::info!("Connection worker thread exiting.");
        });

    let elapsed = start_time.elapsed().as_millis();
    log::info!("[Performance] Cold startup completed in {} ms.", elapsed);

    // 7. Main Loop (UI thread: pumps standard Win32 window message loop to keep tray icon responsive)
    unsafe {
        let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        while windows::Win32::UI::WindowsAndMessaging::GetMessageW(
            &mut msg,
            windows::Win32::Foundation::HWND::default(),
            0,
            0,
        )
        .as_bool()
        {
            // Process any pending tray menu events
            while let Ok(event) = tray_rx.try_recv() {
                match event {
                    TrayEvent::ToggleActive => {
                        let active = !is_active.load(std::sync::atomic::Ordering::Relaxed);
                        is_active.store(active, std::sync::atomic::Ordering::Relaxed);
                        log::info!("Toggled Active state to: {}", active);
                        tray_service.set_active_checked(active);
                    }
                    TrayEvent::ToggleAutoStart => {
                        settings.auto_start = !settings.auto_start;
                        log::info!("Toggled Auto-start to {}", settings.auto_start);
                        let _ = settings_service.save(&settings);
                        tray_service.set_auto_start_checked(settings.auto_start);
                        let _ = set_startup_registration(settings.auto_start);
                    }
                    TrayEvent::OpenLogs => {
                        log::info!("Opening logs file...");
                        // Copy to a temp view file to prevent Notepad from locking the active log file
                        if std::fs::copy("app.log", "app_view.log").is_ok() {
                            let mut cmd = Command::new("notepad.exe");
                            cmd.creation_flags(0x08000000);
                            let _ = cmd.arg("app_view.log").spawn();
                        } else {
                            let mut cmd = Command::new("notepad.exe");
                            cmd.creation_flags(0x08000000);
                            let _ = cmd.arg("app.log").spawn();
                        }
                    }
                    TrayEvent::Exit => {
                        log::info!("Tray Exit clicked. Shutting down gracefully...");
                        
                        // Stop runner thread
                        is_running.store(false, std::sync::atomic::Ordering::Relaxed);

                        // Stop ClipboardService hidden window/thread
                        clipboard_service.shutdown();

                        // Exit process
                        std::process::exit(0);
                    }
                }
            }

            windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
            windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
        }
    }
}

/// Helper function to configure or remove Windows Startup Registry registration.
fn set_startup_registration(enabled: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Failed to get current executable path: {}", e))?;
    let exe_str = exe_path.to_str()
        .ok_or_else(|| "Invalid UTF-8 path".to_string())?;

    let cmd_str = if enabled {
        format!(
            "Set-ItemProperty -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run' -Name 'SnapPaste' -Value '\"{}\"'",
            exe_str
        )
    } else {
        "Remove-ItemProperty -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run' -Name 'SnapPaste' -ErrorAction SilentlyContinue".to_string()
    };

    let status = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .arg("-Command")
        .arg(&cmd_str)
        .status()
        .map_err(|e| format!("Failed to run powershell: {}", e))?;

    if status.success() {
        log::info!("Successfully synchronized Windows Startup Registry (Enabled: {}).", enabled);
        Ok(())
    } else {
        Err("Registry update command failed".to_string())
    }
}
