//! Structured logging service with file rotation.

use log::{Level, Metadata, Record, SetLoggerError};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

/// Maximum size of log file before rotation (5 MiB).
const MAX_LOG_SIZE: u64 = 5 * 1024 * 1024;
const LOG_FILE: &str = "app.log";
const BACKUP_LOG_FILE: &str = "app.log.old";

/// Custom thread-safe logger writing to both stderr and rotating log file.
pub struct RotatingLogger {
    file_lock: Mutex<()>,
}

impl RotatingLogger {
    /// Creates a new `RotatingLogger`.
    pub fn new() -> Self {
        Self {
            file_lock: Mutex::new(()),
        }
    }

    /// Initializes this logger globally.
    pub fn init(max_level: log::LevelFilter) -> Result<(), SetLoggerError> {
        let logger = Box::leak(Box::new(RotatingLogger::new()));
        log::set_max_level(max_level);
        log::set_logger(logger)
    }

    /// Performs log file rotation if the size exceeds MAX_LOG_SIZE.
    fn rotate_if_needed(&self) {
        let _lock = self.file_lock.lock().unwrap();
        let path = Path::new(LOG_FILE);
        if path.exists() {
            if let Ok(metadata) = fs::metadata(path) {
                if metadata.len() > MAX_LOG_SIZE {
                    let _ = fs::rename(LOG_FILE, BACKUP_LOG_FILE);
                }
            }
        }
    }

    /// Logs the message to the output file.
    fn log_to_file(&self, message: &str) {
        self.rotate_if_needed();
        let _lock = self.file_lock.lock().unwrap();
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(LOG_FILE)
        {
            let _ = writeln!(file, "{}", message.trim_end());
        }
    }
}

impl log::Log for RotatingLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // Log levels configured globally
        metadata.level() <= Level::Debug
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let timestamp = match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
                Ok(d) => d.as_secs(),
                Err(_) => 0,
            };

            let formatted = format!(
                "[{}] [{}] [{}:{}] {}",
                timestamp,
                record.level(),
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.args()
            );

            // Print to standard console output
            eprintln!("{}", formatted);

            // Log to local file
            self.log_to_file(&formatted);
        }
    }

    fn flush(&self) {}
}
