//! Async tasks for the display system.

extern crate alloc;

pub mod accelerometer;
pub mod app_controller;
pub mod button_monitor;
pub mod display;
pub mod http_server;
pub mod hub75;
pub mod sntp;
pub mod wifi_connection;
pub mod wifi_net;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer};
use log::debug;

pub use crate::state::SharedMatrixHubState;

/// Type alias for framebuffer exchange signal
pub type FrameBufferExchange<FB> = Signal<CriticalSectionRawMutex, &'static mut FB>;

/// Rate limiter for maintaining a constant loop frequency
pub struct RateLimiter {
    period: Duration,
    next_deadline: Option<Instant>,
    missed_deadlines: u32,
    log: MissedDeadlineLog,
}

struct MissedDeadlineLog {
    label: &'static str,
    interval: Duration,
    last_log: Option<Instant>,
    last_missed: u32,
}

impl MissedDeadlineLog {
    fn new(label: &'static str, interval: Duration) -> Self {
        Self {
            label,
            interval,
            last_log: None,
            last_missed: 0,
        }
    }

    fn maybe_log(&mut self, missed: u32, period: Duration) {
        let now = Instant::now();
        match self.last_log {
            None => self.last_log = Some(now),
            Some(last) if now - last >= self.interval => {
                if missed != self.last_missed {
                    debug!(
                        "{}: missed deadlines: {} (period {:?})",
                        self.label, missed, period
                    );
                    self.last_missed = missed;
                }
                self.last_log = Some(now);
            }
            _ => {}
        }
    }
}

impl RateLimiter {
    /// Create a new rate limiter for the given target rate in Hz.
    ///
    /// `label` tags the periodic "missed deadlines" logs.
    pub fn new(target_hz: u32, label: &'static str) -> Self {
        let hz = if target_hz == 0 { 1 } else { target_hz };
        let mut period_us = 1_000_000u64 / hz as u64;
        if period_us == 0 {
            period_us = 1;
        }
        let period = Duration::from_micros(period_us);
        Self {
            period,
            next_deadline: None,
            missed_deadlines: 0,
            log: MissedDeadlineLog::new(label, Duration::from_secs(1)),
        }
    }

    pub fn set_deadline_log_interval(&mut self, interval: Duration) {
        self.log.interval = interval;
        self.log.last_log = None;
    }

    pub fn period(&self) -> Duration {
        self.period
    }

    pub fn missed_deadlines(&self) -> u32 {
        self.missed_deadlines
    }

    /// Update the target rate in Hz
    pub fn set_target_hz(&mut self, target_hz: u32) {
        let hz = if target_hz == 0 { 1 } else { target_hz };
        let mut period_us = 1_000_000u64 / hz as u64;
        if period_us == 0 {
            period_us = 1;
        }
        self.period = Duration::from_micros(period_us);
    }

    /// Sleep for the remaining time to maintain the target rate
    pub async fn sleep(&mut self) {
        // Schedule the *next* deadline. Keeping an accumulating deadline reduces drift.
        // Caller only needs to call `sleep_remaining()` once per loop.
        let now = Instant::now();
        let deadline = match self.next_deadline {
            Some(prev) => prev + self.period,
            None => now + self.period,
        };
        self.next_deadline = Some(deadline);

        // Best-effort timing: if the system delays the wakeup (e.g. WiFi/critical sections),
        // we don't try to "make up" the sleep later — we just skip sleeping when late.
        if now < deadline {
            Timer::at(deadline).await;
        } else {
            self.missed_deadlines = self.missed_deadlines.saturating_add(1);
        }

        self.log.maybe_log(self.missed_deadlines, self.period);
    }
}
