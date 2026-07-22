use std::ptr;
use std::sync::mpsc::{self, Sender};
use std::thread;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage,
    PostThreadMessageW, RegisterClassExW, TranslateMessage, MSG, WNDCLASSEXW, CS_HREDRAW, CS_VREDRAW,
    CW_USEDEFAULT, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_USER,
};

/// Custom format name for raw PNG bytes in Windows Clipboard.
const PNG_FORMAT_NAME: PCWSTR = w!("PNG");

/// Custom window message to signal new clipboard content.
const WM_USER_COPY_PNG: u32 = WM_USER + 1;

/// Command messages sent to the ClipboardService thread.
pub enum ClipboardCmd {
    /// Copy raw PNG bytes to the clipboard.
    CopyPng(Vec<u8>),
    /// Destroy the hidden window and terminate the thread.
    Shutdown,
}

/// Owner and proxy to update the Windows clipboard.
pub struct ClipboardService {
    tx: Sender<ClipboardCmd>,
    thread_id: u32,
}

impl ClipboardService {
    /// Spawns a new hidden thread solely to manage the message-only window and clipboard operations.
    pub fn new() -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let (setup_tx, setup_rx) = mpsc::channel::<u32>();
        
        thread::Builder::new()
            .name("ClipboardOwnerThread".to_string())
            .spawn(move || {
                let hwnd = match create_hidden_window() {
                    Ok(h) => h,
                    Err(e) => {
                        log::error!("Failed to create message-only window: {}", e);
                        return;
                    }
                };

                let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
                if let Err(e) = setup_tx.send(thread_id) {
                    log::error!("Failed to send setup info: {}", e);
                    return;
                }

                log::info!("Message-only window successfully created: {:?} (Thread ID: {})", hwnd, thread_id);

                unsafe {
                    let mut msg = MSG::default();
                    // Process message loop: blocks efficiently until a message is posted to this thread
                    while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
                        if msg.message == WM_USER_COPY_PNG {
                            // Process any pending commands in the channel
                            while let Ok(cmd) = rx.try_recv() {
                                match cmd {
                                    ClipboardCmd::CopyPng(bytes) => {
                                        let start_time = std::time::Instant::now();
                                        if let Err(e) = set_png_to_clipboard(hwnd, &bytes) {
                                            log::error!("Clipboard copy failed: {}", e);
                                        } else {
                                            let elapsed = start_time.elapsed().as_millis();
                                            log::info!(
                                                "[Performance] Clipboard updated successfully in {} ms (Size: {} bytes).",
                                                elapsed,
                                                bytes.len()
                                            );
                                        }
                                    }
                                    ClipboardCmd::Shutdown => {
                                        log::info!("Shutting down Clipboard Service.");
                                        PostQuitMessage(0);
                                    }
                                }
                            }
                        }

                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                log::info!("Clipboard owner thread exited.");
            })
            .map_err(|e| format!("Failed to spawn clipboard thread: {}", e))?;

        let thread_id = setup_rx
            .recv()
            .map_err(|e| format!("Failed to receive clipboard thread setup info: {}", e))?;

        Ok(Self { tx, thread_id })
    }

    /// Submits PNG bytes to be copied onto the clipboard.
    pub fn copy_png(&self, bytes: Vec<u8>) {
        if self.tx.send(ClipboardCmd::CopyPng(bytes)).is_ok() {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_USER_COPY_PNG, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// Shutdown the service.
    pub fn shutdown(&self) {
        if self.tx.send(ClipboardCmd::Shutdown).is_ok() {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_USER_COPY_PNG, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// Registers window class and creates a message-only HWND.
fn create_hidden_window() -> Result<HWND, String> {
    unsafe {
        let class_name = w!("SnapPasteClipboardOwner");
        let handle = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        let wnd_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(clipboard_wnd_proc),
            hInstance: windows::Win32::Foundation::HINSTANCE(handle.0),
            lpszClassName: class_name,
            ..Default::default()
        };

        if RegisterClassExW(&wnd_class) == 0 {
            return Err("RegisterClassExW failed".to_string());
        }

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("SnapPasteHiddenWindow"),
            WINDOW_STYLE::default(),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            HWND_MESSAGE,
            None,
            wnd_class.hInstance,
            None,
        ).map_err(|e| format!("CreateWindowExW failed: {}", e))?;

        if hwnd.0.is_null() {
            Err("CreateWindowExW returned null".to_string())
        } else {
            Ok(hwnd)
        }
    }
}

/// Window procedure for the hidden window.
unsafe extern "system" fn clipboard_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Standard Windows clipboard format for Device Independent Bitmaps.
const CF_DIB: u32 = 8;

#[repr(C, packed)]
struct BitmapInfoHeader {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_x_pels_per_meter: i32,
    bi_y_pels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

/// Direct copy of raw PNG bytes and CF_DIB bitmap into the clipboard.
fn set_png_to_clipboard(owner: HWND, png_bytes: &[u8]) -> Result<(), String> {
    // 1. Decode PNG to BGRA pixels for standard CF_DIB format
    let img = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png)
        .map_err(|e| format!("Failed to decode PNG: {}", e))?;
    let rgba = img.to_rgba8();
    let width = img.width() as i32;
    let height = img.height() as i32;
    let rgba_raw = rgba.as_raw();
    let row_stride = (width * 4) as usize;
    let mut bgra = vec![0u8; rgba_raw.len()];
    
    // Copy pixels from source to destination, swapping R/B channels and flipping rows vertically
    for y in 0..height {
        let src_y = y as usize;
        let dest_y = (height - 1 - y) as usize;
        let src_offset = src_y * row_stride;
        let dest_offset = dest_y * row_stride;
        
        for x in (0..row_stride).step_by(4) {
            bgra[dest_offset + x] = rgba_raw[src_offset + x + 2];     // B
            bgra[dest_offset + x + 1] = rgba_raw[src_offset + x + 1]; // G
            bgra[dest_offset + x + 2] = rgba_raw[src_offset + x];     // R
            bgra[dest_offset + x + 3] = rgba_raw[src_offset + x + 3]; // A
        }
    }

    unsafe {
        // Register PNG clipboard format name
        let png_format = RegisterClipboardFormatW(PNG_FORMAT_NAME);
        if png_format == 0 {
            return Err("Failed to register PNG clipboard format".to_string());
        }

        // Open Clipboard
        if OpenClipboard(owner).is_err() {
            return Err("Failed to OpenClipboard. Clipboard might be locked by another process (Clipboard Busy recovery).".to_string());
        }

        // Empty Clipboard
        if EmptyClipboard().is_err() {
            let _ = CloseClipboard();
            return Err("Failed to EmptyClipboard".to_string());
        }

        // 2. Set Custom "PNG" Format (for transparent alpha in supporting apps)
        let h_png_mem = GlobalAlloc(GMEM_MOVEABLE, png_bytes.len());
        if h_png_mem.is_ok() {
            let h_png_mem = h_png_mem.unwrap();
            let dest = GlobalLock(h_png_mem);
            if !dest.is_null() {
                ptr::copy_nonoverlapping(png_bytes.as_ptr(), dest as *mut u8, png_bytes.len());
                let _ = GlobalUnlock(h_png_mem);
                let handle = windows::Win32::Foundation::HANDLE(h_png_mem.0);
                if SetClipboardData(png_format, handle).is_err() {
                    log::warn!("Failed to set custom PNG clipboard format");
                }
            }
        }

        // 3. Set Standard "CF_DIB" Format (for standard compatibility & Clipboard History)
        let header = BitmapInfoHeader {
            bi_size: std::mem::size_of::<BitmapInfoHeader>() as u32,
            bi_width: width,
            bi_height: height, // Positive indicates bottom-up rows (standard for Windows shell)
            bi_planes: 1,
            bi_bit_count: 32,
            bi_compression: 0, // BI_RGB (uncompressed)
            bi_size_image: bgra.len() as u32,
            bi_x_pels_per_meter: 0,
            bi_y_pels_per_meter: 0,
            bi_clr_used: 0,
            bi_clr_important: 0,
        };

        let dib_size = std::mem::size_of::<BitmapInfoHeader>() + bgra.len();
        let h_dib_mem = GlobalAlloc(GMEM_MOVEABLE, dib_size);
        if h_dib_mem.is_ok() {
            let h_dib_mem = h_dib_mem.unwrap();
            let dest = GlobalLock(h_dib_mem);
            if !dest.is_null() {
                // Copy header
                ptr::copy_nonoverlapping(
                    &header as *const BitmapInfoHeader as *const u8,
                    dest as *mut u8,
                    std::mem::size_of::<BitmapInfoHeader>(),
                );
                // Copy pixels
                ptr::copy_nonoverlapping(
                    bgra.as_ptr(),
                    (dest as usize + std::mem::size_of::<BitmapInfoHeader>()) as *mut u8,
                    bgra.len(),
                );
                let _ = GlobalUnlock(h_dib_mem);
                let handle = windows::Win32::Foundation::HANDLE(h_dib_mem.0);
                if SetClipboardData(CF_DIB, handle).is_err() {
                    log::warn!("Failed to set standard CF_DIB clipboard format");
                }
            }
        }

        // Close clipboard
        if CloseClipboard().is_err() {
            return Err("Failed to CloseClipboard".to_string());
        }

        Ok(())
    }
}

