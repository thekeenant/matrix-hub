//! Fast direct framebuffer API for Rhai scripts.
//!
//! This uses a temporary global pointer (stored as usize) set by the
//! `render()` path while the real `&mut FrameBuffer` is borrowed. Script
//! functions use this pointer to perform direct, in-place writes with
//! minimal overhead.

extern crate alloc;

use core::sync::atomic::{AtomicUsize, Ordering};

use embedded_graphics::{geometry::Point, pixelcolor::Rgb888, prelude::*};
use log::warn;

use crate::tasks::hub75::{COLS, FrameBuffer, ROWS};

static FRAMEBUFFER_PTR: AtomicUsize = AtomicUsize::new(0);

/// Set from Rust before running the script `render()` function.
pub fn set_current_framebuffer(ptr: usize) {
    FRAMEBUFFER_PTR.store(ptr, Ordering::Relaxed);
}

/// Clear after the script returns.
pub fn clear_current_framebuffer() {
    FRAMEBUFFER_PTR.store(0, Ordering::Relaxed);
}

fn with_fb<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut FrameBuffer) -> R,
{
    let p = FRAMEBUFFER_PTR.load(Ordering::Relaxed);
    if p == 0 {
        warn!("framebuffer_api: no framebuffer set");
        return None;
    }
    unsafe {
        let fb = &mut *(p as *mut FrameBuffer);
        Some(f(fb))
    }
}

/// Set a pixel by packed 0xRRGGBB color.
pub fn set_pixel(x: i64, y: i64, color: i64) {
    with_fb(|fb| {
        let r = ((color >> 16) & 0xFF) as u8;
        let g = ((color >> 8) & 0xFF) as u8;
        let b = (color & 0xFF) as u8;
        let col = Rgb888::new(r, g, b);
        let _ = Pixel(Point::new(x as i32, y as i32), col).draw(fb);
    });
}

/// Clear the framebuffer to a color (0xRRGGBB).
pub fn clear(color: i64) {
    with_fb(|fb| {
        let r = ((color >> 16) & 0xFF) as u8;
        let g = ((color >> 8) & 0xFF) as u8;
        let b = (color & 0xFF) as u8;
        let col = Rgb888::new(r, g, b);
        let _ = fb.clear(col);
    });
}

/// Framebuffer width in pixels.
pub fn width() -> i64 {
    COLS as i64
}

/// Framebuffer height in pixels.
pub fn height() -> i64 {
    ROWS as i64
}
