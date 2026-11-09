//! HUB75 panel driver binding and the display thread that feeds it.

use anyhow::{anyhow, Context, Result};
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;
use esp_idf_svc::sys::hub75::{
    hub75_c_begin, hub75_c_clear, hub75_c_create, hub75_c_draw_pixel,
    hub75_c_flip_buffer, hub75_c_set_brightness, hub75_handle_t,
};
use log::info;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::config::{PanelPins, HEIGHT, PANEL_PINS, WIDTH};
use crate::diag::{self, Crumb, Stage};
use matrix_hub_core::buffer::FrameConsumer;

/// Start the display thread on core 1 and wait until the panel (and its DMA
/// buffers) are initialised. Frames arrive through `frames`.
///
/// Panics if the panel can't be initialised: without a display the device is
/// useless, and a panic reboots it.
pub fn spawn(frames: FrameConsumer) -> Result<()> {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<()>(1);

    // Pin to core 1 at high priority so drawing never stutters. Core 0 runs
    // WiFi, networking, and the app logic.
    ThreadSpawnConfiguration {
        pin_to_core: Some(Core::Core1),
        priority: 20,
        ..Default::default()
    }
    .set()
    .context("Failed to configure display thread")?;

    // Explicit stack size: ThreadSpawnConfiguration's stack_size is ignored
    // by std::thread, and the 8KB used here is plenty (~3KB measured).
    let spawned = std::thread::Builder::new()
        .stack_size(8192)
        .spawn(move || run(&frames, &ready_tx));

    ThreadSpawnConfiguration::default()
        .set()
        .context("Failed to reset thread config")?;
    spawned.context("Failed to spawn display thread")?;

    info!("Waiting for display to allocate DMA memory...");
    ready_rx
        .recv()
        .map_err(|_| anyhow!("Display thread exited before becoming ready"))
}

fn run(frames: &FrameConsumer, ready: &mpsc::SyncSender<()>) {
    info!(
        "Display thread started on {:?}",
        esp_idf_svc::hal::cpu::core()
    );

    let mut display =
        MatrixDisplay::new(WIDTH as u16, HEIGHT as u16, &PANEL_PINS)
            .unwrap_or_else(|e| {
                panic!("Failed to initialize matrix display: {e}")
            });
    info!("Display hardware ready");
    let _ = ready.send(());

    let mut brightness = crate::storage::brightness();
    display.set_brightness(brightness);

    // If this thread ever stops looping (e.g. stuck inside the driver), the
    // watchdog panics, saves a coredump, and reboots.
    diag::watch_current_thread();

    loop {
        diag::feed_watchdog();
        diag::DISPLAY.enter(Stage::DisplayWait);

        // Wake up periodically even without frames so the watchdog stays fed;
        // a stalled main loop is caught by its own watchdog subscription.
        let frame = match frames.recv_timeout(Duration::from_secs(1)) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };

        diag::DISPLAY.enter(Stage::DisplayBrightness);
        let wanted = crate::storage::brightness();
        if wanted != brightness {
            brightness = wanted;
            display.set_brightness(brightness);
        }

        diag::DISPLAY.enter(Stage::DisplayClear);
        display.clear();

        // The panel was just cleared, so only lit pixels need drawing
        diag::DISPLAY.enter(Stage::DisplayDraw);
        let _ = display.draw_iter(frame.lit_pixels());

        diag::DISPLAY.enter(Stage::DisplayFlip);
        display.flip();
        diag::inc(Crumb::DisplayFrames);

        frames.release(frame);
    }
}

/// Safe handle to the C++ HUB75 driver (double-buffered).
pub struct MatrixDisplay {
    handle: hub75_handle_t,
    width: u16,
    height: u16,
}

impl MatrixDisplay {
    pub fn new(
        width: u16,
        height: u16,
        pins: &PanelPins,
    ) -> Result<Self, &'static str> {
        #[allow(
            unsafe_code,
            reason = "Calling C FFI for Hub75 display creation"
        )]
        // SAFETY: plain-value arguments; returns null on allocation failure.
        let handle = unsafe {
            hub75_c_create(
                width, height, pins.r1, pins.g1, pins.b1, pins.r2, pins.g2,
                pins.b2, pins.a, pins.b, pins.c, pins.d, pins.e, pins.lat,
                pins.oe, pins.clk,
            )
        };
        if handle.is_null() {
            return Err("Failed to allocate Hub75 display");
        }

        #[allow(unsafe_code, reason = "Calling C FFI to begin Hub75 display")]
        // SAFETY: `handle` was just returned non-null by hub75_c_create.
        if !unsafe { hub75_c_begin(handle) } {
            return Err("Failed to initialize Hub75 display hardware");
        }

        Ok(Self {
            handle,
            width,
            height,
        })
    }

    /// Show the back buffer (swaps at the next frame boundary).
    pub fn flip(&self) {
        #[allow(unsafe_code, reason = "Calling C FFI to flip Hub75 buffer")]
        // SAFETY: `handle` is valid for the lifetime of `self`.
        unsafe {
            hub75_c_flip_buffer(self.handle);
        }
    }

    /// Clear the back buffer.
    pub fn clear(&self) {
        #[allow(unsafe_code, reason = "Calling C FFI to clear Hub75 buffer")]
        // SAFETY: `handle` is valid for the lifetime of `self`.
        unsafe {
            hub75_c_clear(self.handle);
        }
    }

    pub fn set_brightness(&self, brightness: u8) {
        #[allow(unsafe_code, reason = "Calling C FFI for Hub75 brightness")]
        // SAFETY: `handle` is valid for the lifetime of `self`.
        unsafe {
            hub75_c_set_brightness(self.handle, brightness);
        }
    }
}

impl DrawTarget for MatrixDisplay {
    type Color = Rgb888;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels {
            let (Ok(x), Ok(y)) =
                (u16::try_from(coord.x), u16::try_from(coord.y))
            else {
                continue;
            };
            if x < self.width && y < self.height {
                #[allow(unsafe_code, reason = "Calling C FFI to draw a pixel")]
                // SAFETY: `handle` is valid and (x, y) is in bounds.
                unsafe {
                    hub75_c_draw_pixel(
                        self.handle,
                        x,
                        y,
                        color.r(),
                        color.g(),
                        color.b(),
                    );
                }
            }
        }
        Ok(())
    }
}

impl OriginDimensions for MatrixDisplay {
    fn size(&self) -> Size {
        Size::new(u32::from(self.width), u32::from(self.height))
    }
}
