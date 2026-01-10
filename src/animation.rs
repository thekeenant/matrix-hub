//! Shared animation timing helpers.
//!
//! Embedded animation loops often want to:
//! - compute `dt` once per iteration
//! - advance multiple independent "phases" with `value += rate_per_sec * dt`
//!
//! This module centralizes the dt conversion and integration so individual
//! animations don't duplicate the same math.

use embassy_time::Duration;

/// A shared animated scalar value (phase/position/etc.).
///
/// The units are defined by the caller (pixels, frames, degrees...).
#[derive(Debug, Clone, Copy, Default)]
pub struct AnimValue {
    pub value: f32,
}

impl AnimValue {
    #[inline]
    pub const fn new(value: f32) -> Self {
        Self { value }
    }

    /// Advance this value by `rate_per_sec` for the given time step.
    #[inline]
    pub fn advance(&mut self, rate_per_sec: f32, step: AnimationStep) {
        self.value += rate_per_sec * step.dt_secs;
    }

    /// Advance and wrap the value to stay within [0, max).
    #[inline]
    pub fn advance_periodic(&mut self, rate_per_sec: f32, step: AnimationStep, max: f32) {
        self.advance(rate_per_sec, step);
        self.value = self.value % max;
    }

    /// Wrap the value to stay within [0, max).
    #[inline]
    pub fn wrap(&mut self, max: f32) {
        self.value = self.value % max;
    }

    /// Clamp the value between min and max.
    #[inline]
    pub fn clamp(&mut self, min: f32, max: f32) {
        if self.value < min {
            self.value = min;
        } else if self.value > max {
            self.value = max;
        }
    }

    #[inline]
    pub fn as_u32(&self) -> u32 {
        self.value as u32
    }
}

/// Per-loop timing information for advancing animations.
#[derive(Debug, Clone, Copy)]
pub struct AnimationStep {
    /// Delta time in seconds.
    pub dt_secs: f32,
}

impl AnimationStep {
    pub fn from_duration(dt: Duration) -> Self {
        let dt_secs = (dt.as_micros() as f32) / 1_000_000.0;
        Self { dt_secs }
    }
}
