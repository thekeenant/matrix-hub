//! The off-screen framebuffer apps draw into, and the fixed pool of them that
//! circulates between the logic loop and the display thread.

use crate::config::{HEIGHT, WIDTH};
use embedded_graphics::{pixelcolor::Rgb888, prelude::*};
use std::sync::mpsc::{
    self, Receiver, RecvTimeoutError, SyncSender, TrySendError,
};
use std::time::Duration;

pub struct Framebuffer {
    pub pixels: Vec<Rgb888>,
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Framebuffer {
    pub const LEN: usize = (WIDTH * HEIGHT) as usize;

    /// Index into `pixels` for an in-bounds coordinate
    pub const fn index(x: u32, y: u32) -> usize {
        (y * WIDTH + x) as usize
    }

    pub fn new() -> Self {
        // 128x32 RGB888 is 12KB. ESP-IDF's malloc serves requests at or below
        // CONFIG_SPIRAM_MALLOC_ALWAYSINTERNAL (16KB) from scarce internal RAM
        // first, and only larger ones from PSRAM first. The framebuffer is only
        // touched by the CPU, so over-reserve capacity past that threshold to
        // land it in PSRAM and leave internal RAM for WiFi/lwIP/DMA.
        const PSRAM_THRESHOLD_BYTES: usize = 16 * 1024;
        let capacity =
            Self::LEN.max(PSRAM_THRESHOLD_BYTES / size_of::<Rgb888>() + 1);
        let mut pixels = Vec::with_capacity(capacity);
        pixels.resize(Self::LEN, Rgb888::BLACK);
        Self { pixels }
    }

    pub fn clear(&mut self) {
        self.pixels.fill(Rgb888::BLACK);
    }

    /// Every non-black pixel with its coordinates.
    pub fn lit_pixels(&self) -> impl Iterator<Item = Pixel<Rgb888>> + '_ {
        let width = WIDTH as usize;
        self.pixels
            .iter()
            .enumerate()
            .filter(|(_, color)| **color != Rgb888::BLACK)
            .map(move |(i, &color)| {
                let point = Point::new((i % width) as i32, (i / width) as i32);
                Pixel(point, color)
            })
    }
}

impl DrawTarget for Framebuffer {
    type Color = Rgb888;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels {
            if let (Ok(x), Ok(y)) =
                (u32::try_from(coord.x), u32::try_from(coord.y))
            {
                if x < WIDTH && y < HEIGHT {
                    self.pixels[Self::index(x, y)] = color;
                }
            }
        }
        Ok(())
    }
}

impl OriginDimensions for Framebuffer {
    fn size(&self) -> Size {
        Size::new(WIDTH, HEIGHT)
    }
}

// ============================================================================
// Frame pool
// ============================================================================

/// Create a pool of `size` framebuffers shared by a producer (the logic loop)
/// and a consumer (the display thread).
///
/// Buffers are allocated once, up front: nothing is allocated per frame.
/// Each channel can hold the whole pool, so returning a buffer never fails.
pub fn frame_pool(size: usize) -> (FrameProducer, FrameConsumer) {
    let (free_tx, free_rx) = mpsc::sync_channel(size);
    let (ready_tx, ready_rx) = mpsc::sync_channel(size);
    for _ in 0..size {
        let _ = free_tx.send(Box::new(Framebuffer::new()));
    }
    (
        FrameProducer {
            free_rx,
            free_tx: free_tx.clone(),
            ready_tx,
        },
        FrameConsumer { ready_rx, free_tx },
    )
}

/// The drawing side of the frame pool.
pub struct FrameProducer {
    free_rx: Receiver<Box<Framebuffer>>,
    free_tx: SyncSender<Box<Framebuffer>>,
    ready_tx: SyncSender<Box<Framebuffer>>,
}

impl FrameProducer {
    /// A cleared buffer to draw into, or `None` if the display currently
    /// holds every buffer (then skip drawing this frame).
    pub fn acquire(&self) -> Option<Box<Framebuffer>> {
        let mut frame = self.free_rx.try_recv().ok()?;
        frame.clear();
        Some(frame)
    }

    /// Queue a drawn frame for display. If the display is behind, the frame
    /// is dropped and its buffer goes straight back into the pool.
    pub fn submit(&self, frame: Box<Framebuffer>) {
        if let Err(TrySendError::Full(frame)) = self.ready_tx.try_send(frame) {
            let _ = self.free_tx.try_send(frame);
        }
    }
}

/// The display side of the frame pool.
pub struct FrameConsumer {
    ready_rx: Receiver<Box<Framebuffer>>,
    free_tx: SyncSender<Box<Framebuffer>>,
}

impl FrameConsumer {
    /// Wait up to `timeout` for the next frame to show.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Box<Framebuffer>, RecvTimeoutError> {
        self.ready_rx.recv_timeout(timeout)
    }

    /// Return a shown frame's buffer to the pool.
    pub fn release(&self, frame: Box<Framebuffer>) {
        let _ = self.free_tx.try_send(frame);
    }
}
