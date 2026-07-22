//! Image transfer service for reassembling chunks.

use crate::clipboard::ClipboardService;
use std::sync::Arc;

/// Service responsible for gathering image chunks and handing them to the clipboard.
pub struct ImageTransferService {
    clipboard_service: Arc<ClipboardService>,
    active_image_id: Option<u32>,
    buffer: Vec<u8>,
    is_active: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ImageTransferService {
    /// Creates a new `ImageTransferService`.
    pub fn new(
        clipboard_service: Arc<ClipboardService>,
        is_active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            clipboard_service,
            active_image_id: None,
            buffer: Vec::new(),
            is_active,
        }
    }

    /// Handles a START_IMAGE notification.
    pub fn start_image(&mut self, image_id: u32, expected_size: u64) {
        log::info!("ImageTransfer: Start receiving ImageID={}, expected size={} bytes", image_id, expected_size);
        self.active_image_id = Some(image_id);
        self.buffer.clear();
        if expected_size > 0 {
            self.buffer.reserve(expected_size as usize);
        }
    }

    /// Appends incoming CHUNK data if it matches the current in-flight transfer.
    pub fn append_chunk(&mut self, image_id: u32, chunk_data: &[u8]) {
        if self.active_image_id == Some(image_id) {
            self.buffer.extend_from_slice(chunk_data);
        } else {
            log::warn!("Discarding chunk for ImageID={} (active: {:?})", image_id, self.active_image_id);
        }
    }

    /// Discards any partial buffer and cancels the active transfer.
    pub fn cancel_transfer(&mut self, image_id: u32) {
        if self.active_image_id == Some(image_id) {
            log::info!("ImageTransfer: Cancelled transfer for ImageID={}", image_id);
            self.active_image_id = None;
            self.buffer.clear();
            self.buffer.shrink_to_fit();
        }
    }

    /// Finalizes the image transfer and submits the buffer to the ClipboardService.
    pub fn finalize_image(&mut self, image_id: u32) {
        if self.active_image_id == Some(image_id) {
            log::info!("ImageTransfer: Completed receiver of ImageID={}, total size={} bytes", image_id, self.buffer.len());
            let completed_image = std::mem::take(&mut self.buffer);

            // If the service is paused (Off), discard the incoming image and return
            if !self.is_active.load(std::sync::atomic::Ordering::Relaxed) {
                log::info!("SnapPaste is Paused (Off). Discarding screenshot transfer.");
                self.active_image_id = None;
                return;
            }

            // Save a physical copy to the PC's Pictures/SnapPaste directory
            if let Ok(profile_dir) = std::env::var("USERPROFILE") {
                let mut path = std::path::PathBuf::from(profile_dir);
                path.push("Pictures");
                path.push("SnapPaste");
                if let Err(e) = std::fs::create_dir_all(&path) {
                    log::error!("Failed to create local SnapPaste Pictures directory: {}", e);
                } else {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    path.push(format!("screenshot_{}.png", ts));
                    if let Err(e) = std::fs::write(&path, &completed_image) {
                        log::error!("Failed to write screenshot file copy to disk: {}", e);
                    } else {
                        log::info!("Successfully saved image backup to: {:?}", path);
                    }
                }
            }

            self.clipboard_service.copy_png(completed_image);
            self.active_image_id = None;
        } else {
            log::warn!("Discarding END_IMAGE for ImageID={} (active: {:?})", image_id, self.active_image_id);
        }
    }

    /// Resets the internal state on connection drops.
    pub fn reset(&mut self) {
        self.active_image_id = None;
        self.buffer.clear();
        self.buffer.shrink_to_fit();
    }
}
