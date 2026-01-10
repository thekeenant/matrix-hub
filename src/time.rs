//! Time management module.
//!
//! Provides time-related functions and state storage for the system.
//! Maintains system time using boot offset calculations.

use core::sync::atomic::Ordering;

use chrono::{DateTime, TimeZone, Utc};
use embassy_time::{Duration, Instant, Timer};
use portable_atomic::AtomicU64;

/// Unix timestamp at system boot (microseconds)
static BOOT_UNIX_TIME_US: AtomicU64 = AtomicU64::new(0);
/// Monotonic time at boot (microseconds)
static BOOT_MONOTONIC_TIME_US: AtomicU64 = AtomicU64::new(0);

/// Try to get the current time if it has been synchronized.
/// Returns None if time hasn't been set yet.
fn try_current_time() -> Option<DateTime<Utc>> {
    let boot_unix = BOOT_UNIX_TIME_US.load(Ordering::Relaxed);
    if boot_unix == 0 {
        return None;
    }
    let boot_mono = BOOT_MONOTONIC_TIME_US.load(Ordering::Relaxed);
    let now_us = Instant::now().as_micros();
    let elapsed = now_us.saturating_sub(boot_mono);
    let timestamp_us = boot_unix + elapsed;
    Some(
        Utc.timestamp_micros(timestamp_us as i64)
            .single()
            .expect("Invalid timestamp"),
    )
}

/// Get the current time, waiting until it has been synchronized if necessary.
pub async fn current_time_blocking() -> DateTime<Utc> {
    loop {
        if let Some(time) = try_current_time() {
            return time;
        }
        Timer::after(Duration::from_millis(100)).await;
    }
}

/// Store the boot time offset for time calculations.
/// Called by SNTP task when time is synchronized.
pub fn set_boot_time(unix_us: u64, monotonic_us: u64) {
    BOOT_UNIX_TIME_US.store(unix_us, Ordering::Relaxed);
    BOOT_MONOTONIC_TIME_US.store(monotonic_us, Ordering::Relaxed);
}
