//! Performance metrics tracking for the display system.

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use embassy_time::{Duration, Instant};

/// Shared metrics for measuring rendering performance. This
/// cannot belong inside the app state, since it is mutated by
/// the rendering tasks (hub75, display).
pub struct RenderMetrics {
    /// The current frames per second (FPS) measured. Written
    /// by the hub75 task and consumed by the display task.
    pub frames_per_second: Arc<AtomicU32>,
    /// The current ticks per second (TPS) measured. Written
    /// by the display task and can be consumed by the display task,
    /// so it does not need to be in an Arc.
    pub ticks_per_second: AtomicU32,
}

impl RenderMetrics {
    /// Create new render metrics with zeroed atomics.
    pub fn new() -> Self {
        Self {
            frames_per_second: Arc::new(AtomicU32::new(0)),
            ticks_per_second: AtomicU32::new(0),
        }
    }
}

impl Default for RenderMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Reusable counter for tracking rates (FPS, TPS, etc.)
pub struct RateCounter {
    count: u32,
    start: Instant,
    interval: Duration,
}

impl RateCounter {
    /// Create a new rate counter with the given update interval starting now.
    pub fn init(interval: Duration) -> Self {
        Self {
            count: 0,
            start: Instant::now(),
            interval,
        }
    }

    /// Increment the counter and optionally update the atomic if interval elapsed
    /// Returns Some(rate) if the interval elapsed and the atomic was updated
    pub fn increment(&mut self, atomic: &AtomicU32) -> Option<u32> {
        self.count += 1;
        if self.start.elapsed() > self.interval {
            let rate = self.count;
            atomic.store(rate, Ordering::Relaxed);
            self.count = 0;
            self.start = Instant::now();
            Some(rate)
        } else {
            None
        }
    }
}
