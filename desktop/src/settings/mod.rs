//! Settings management for SnapPaste Desktop.
//!
//! Persists application preferences to a local config file.

use std::fs;
use std::path::Path;

/// Application settings.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Launch the app automatically at system login.
    pub auto_start: bool,
    /// Identifier of the last paired Android device.
    pub last_paired_device: String,
    /// Verbosity level of logger (debug, info, warn, error).
    pub log_verbosity: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_start: false,
            last_paired_device: String::new(),
            log_verbosity: "info".to_string(),
        }
    }
}

/// Service to persist and retrieve user configurations.
pub struct SettingsService {
    filepath: String,
}

impl SettingsService {
    /// Creates a new `SettingsService`.
    pub fn new() -> Self {
        Self {
            filepath: "settings.txt".to_string(),
        }
    }

    /// Loads settings from disk. Returns default settings if file doesn't exist or is corrupt.
    pub fn load(&self) -> Settings {
        if !Path::new(&self.filepath).exists() {
            return Settings::default();
        }

        match fs::read_to_string(&self.filepath) {
            Ok(content) => {
                let mut settings = Settings::default();
                for line in content.lines() {
                    let parts: Vec<&str> = line.splitn(2, '=').collect();
                    if parts.len() == 2 {
                        let key = parts[0].trim();
                        let val = parts[1].trim();
                        match key {
                            "auto_start" => settings.auto_start = val.parse().unwrap_or(false),
                            "last_paired_device" => settings.last_paired_device = val.to_string(),
                            "log_verbosity" => settings.log_verbosity = val.to_string(),
                            _ => {}
                        }
                    }
                }
                settings
            }
            Err(e) => {
                log::error!("Failed to read settings file: {}. Using defaults.", e);
                Settings::default()
            }
        }
    }

    /// Saves settings to disk.
    pub fn save(&self, settings: &Settings) -> Result<(), String> {
        let content = format!(
            "auto_start={}\nlast_paired_device={}\nlog_verbosity={}\n",
            settings.auto_start, settings.last_paired_device, settings.log_verbosity
        );
        fs::write(&self.filepath, content).map_err(|e| format!("Failed to write settings: {}", e))
    }
}
