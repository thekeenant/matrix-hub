//! HUB75 display hardware driver task.
//!
//! Handles the low-level hardware interaction with the HUB75 LED matrix display
//! using the ESP32-S3's LCD_CAM peripheral and DMA.

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use embassy_time::Duration;
use esp_hal::{gpio::AnyPin, peripherals::LCD_CAM, time::Rate};
use esp_hub75::{
    Hub75, Hub75Pins16,
    framebuffer::{compute_frame_count, compute_rows, plain::DmaFrameBuffer},
};
use log::info;

use crate::{
    metrics::RateCounter,
    tasks::{FrameBufferExchange, RateLimiter},
};

/// Display dimensions and color depth
pub const ROWS: usize = 32;
pub const COLS: usize = 128;
pub const BITS: u8 = 4; // 4-bit color = 16 levels per channel (4k colors)
pub const NROWS: usize = compute_rows(ROWS);
pub const FRAME_COUNT: usize = compute_frame_count(BITS);

/// Hub75 brightness controller
///
/// Manages the target refresh rate (Hz) which controls display brightness.
/// Lower Hz = dimmer, higher Hz = brighter due to the nature of HUB75 displays.
pub struct Hub75Brightness {
    target_hz: AtomicU32,
}

impl Hub75Brightness {
    /// Convert a ratio (0.0 to 1.0) to Hz (80 to 500)
    fn ratio_to_hz(ratio: f32) -> u32 {
        let ratio_clamped = if ratio < 0.0 {
            0.0
        } else if ratio > 1.0 {
            1.0
        } else {
            ratio
        };
        80 + (ratio_clamped * 420.0) as u32
    }

    /// Create a new brightness controller with the given ratio (0.0 to 1.0)
    /// Maps to Hz range: 80 (dim) to 500 (bright)
    pub fn new(ratio: f32) -> Self {
        Self {
            target_hz: AtomicU32::new(Self::ratio_to_hz(ratio)),
        }
    }

    /// Set brightness using a ratio from 0.0 (dimmest) to 1.0 (brightest)
    /// Maps to Hz range: 80 (dim) to 500 (bright)
    pub fn set_ratio(&self, ratio: f32) {
        self.target_hz
            .store(Self::ratio_to_hz(ratio), Ordering::Relaxed);
    }

    /// Get the current target Hz
    pub fn get_hz(&self) -> u32 {
        self.target_hz.load(Ordering::Relaxed)
    }
}

pub type Hub75Async = Hub75<'static, esp_hal::Async>;
pub type FrameBuffer = DmaFrameBuffer<ROWS, COLS, NROWS, BITS, FRAME_COUNT>;

/// Hardware peripherals needed for HUB75 display
pub struct Hub75Peripherals<'d> {
    pub lcd_cam: LCD_CAM<'d>,
    pub dma_channel: esp_hal::peripherals::DMA_CH0<'d>,
    pub red1: AnyPin<'d>,
    pub grn1: AnyPin<'d>,
    pub blu1: AnyPin<'d>,
    pub red2: AnyPin<'d>,
    pub grn2: AnyPin<'d>,
    pub blu2: AnyPin<'d>,
    pub addr0: AnyPin<'d>,
    pub addr1: AnyPin<'d>,
    pub addr2: AnyPin<'d>,
    pub addr3: AnyPin<'d>,
    pub addr4: AnyPin<'d>,
    pub blank: AnyPin<'d>,
    pub clock: AnyPin<'d>,
    pub latch: AnyPin<'d>,
}

/// HUB75 hardware driver task
///
/// This task runs at high priority and handles the DMA transfers to the display.
/// It receives framebuffers from the display task, pushes them to the hardware,
/// and tracks the refresh rate.
#[embassy_executor::task]
pub async fn hub75_task(
    peripherals: Hub75Peripherals<'static>,
    rendered_buffer: &'static FrameBufferExchange<FrameBuffer>,
    free_buffer: &'static FrameBufferExchange<FrameBuffer>,
    frame_buffer: &'static mut FrameBuffer,
    frames_per_second: Arc<AtomicU32>,
    brightness: Arc<Hub75Brightness>,
) {
    hub75_task_impl(
        peripherals,
        rendered_buffer,
        free_buffer,
        frame_buffer,
        frames_per_second,
        brightness,
    )
    .await
    .expect("Hub75 task failed");
}

async fn hub75_task_impl(
    peripherals: Hub75Peripherals<'static>,
    rendered_buffer: &'static FrameBufferExchange<FrameBuffer>,
    free_buffer: &'static FrameBufferExchange<FrameBuffer>,
    frame_buffer: &'static mut FrameBuffer,
    frames_per_second: Arc<AtomicU32>,
    brightness: Arc<Hub75Brightness>,
) -> anyhow::Result<()> {
    info!("hub75_task: starting!");
    let channel = peripherals.dma_channel;
    let (_, tx_descriptors) = esp_hal::dma_descriptors!(0, FrameBuffer::dma_buffer_size_bytes());

    let pins = Hub75Pins16 {
        red1: peripherals.red1,
        grn1: peripherals.grn1,
        blu1: peripherals.blu1,
        red2: peripherals.red2,
        grn2: peripherals.grn2,
        blu2: peripherals.blu2,
        addr0: peripherals.addr0,
        addr1: peripherals.addr1,
        addr2: peripherals.addr2,
        addr3: peripherals.addr3,
        addr4: peripherals.addr4,
        blank: peripherals.blank,
        clock: peripherals.clock,
        latch: peripherals.latch,
    };

    info!("hub75_task: initializing hardware...");
    let mut hub75 = match Hub75Async::new_async(
        peripherals.lcd_cam,
        pins,
        channel,
        tx_descriptors,
        Rate::from_mhz(20),
    ) {
        Ok(h) => {
            info!("hub75_task: hardware initialized successfully");
            h
        }
        Err(e) => {
            info!(
                "hub75_task: hardware init failed (expected in simulation): {:?}",
                e
            );
            // Sleep forever - hardware not available
            loop {
                embassy_time::Timer::after(Duration::from_secs(60)).await;
            }
        }
    };

    let mut fps_counter = RateCounter::init(Duration::from_secs(1));
    let mut rate_limiter = RateLimiter::new(300, "hub75");
    let mut last_target_hz = 0u32;

    // keep the frame buffer in an option so we can swap it
    let mut frame_buffer = Some(frame_buffer);

    loop {
        // if there is a new buffer available, swap it and send the old one
        if rendered_buffer.signaled() {
            let new_fb = rendered_buffer.wait().await;
            if let Some(old_fb) = frame_buffer.replace(new_fb) {
                free_buffer.signal(old_fb);
            }

            // Check target_hz when swapping buffers (simple u32 load, no math)
            let current_target_hz = brightness.get_hz();
            if current_target_hz != last_target_hz {
                rate_limiter.set_target_hz(current_target_hz);
                last_target_hz = current_target_hz;
            }
        }
        if let Some(ref mut fb) = frame_buffer {
            let mut transfer = hub75
                .render(fb)
                .map_err(|(e, _hub75)| anyhow::anyhow!("Failed to start render: {:?}", e))?;
            transfer
                .wait_for_done()
                .await
                .map_err(|e| anyhow::anyhow!("DMA transfer failed: {:?}", e))?;
            let (result, new_hub75) = transfer.wait();
            hub75 = new_hub75;
            result.map_err(|e| anyhow::anyhow!("Transfer failed: {:?}", e))?;
        }

        fps_counter.increment(frames_per_second.as_ref());

        // Rate limiting: sleep remaining time to maintain target FPS
        rate_limiter.sleep().await;
    }
}
