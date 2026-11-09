//! Input interpretation: button debouncing and tilt gestures, driven by raw
//! readings so it's independent of the GPIO and accelerometer drivers.

/// Something the user did, as far as navigation is concerned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    /// Up button, or a full tilt to the right
    Next,
    /// Down button, or a full tilt to the left
    Previous,
    /// Partial tilt while flat-ish: positive is right, negative left
    Tilt(f32),
}

/// Acceleration in g. Positive x is a tilt to the left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Accel {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

// ============================================================================
// Buttons
// ============================================================================

/// Debounce period, in frames (~240ms at 60fps)
const DEBOUNCE_FRAMES: u32 = 15;

/// Turns per-frame button readings into clean "clicked" edges.
#[derive(Debug)]
pub struct Debouncer {
    was_pressed: bool,
    lockout_frames: u32,
}

impl Default for Debouncer {
    fn default() -> Self {
        Self::new()
    }
}

impl Debouncer {
    pub const fn new() -> Self {
        Self {
            was_pressed: false,
            lockout_frames: 0,
        }
    }

    /// Feed this frame's reading. True on the frame the button goes down,
    /// unless it was already clicked within the debounce period.
    pub fn update(&mut self, pressed: bool) -> bool {
        let clicked = pressed && !self.was_pressed && self.lockout_frames == 0;
        if clicked {
            self.lockout_frames = DEBOUNCE_FRAMES;
        }
        self.lockout_frames = self.lockout_frames.saturating_sub(1);
        self.was_pressed = pressed;
        clicked
    }
}

// ============================================================================
// Tilt gestures
// ============================================================================

/// |x| (in g) beyond which a tilt counts as a swipe
const SWIPE_THRESHOLD: f32 = 0.8;
/// |x| below which the device counts as flat again, re-arming swipes
const FLAT_THRESHOLD: f32 = 0.4;
/// |x| beyond which a flat-ish device reports analog tilt feedback
const TILT_FEEDBACK_THRESHOLD: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TiltState {
    Flat,
    TiltedRight,
    TiltedLeft,
}

/// Turns accelerometer readings into swipe and tilt events. A full tilt
/// fires one swipe; the device must return to flat before the next.
#[derive(Debug)]
pub struct TiltTracker {
    state: TiltState,
}

impl Default for TiltTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TiltTracker {
    pub const fn new() -> Self {
        Self {
            state: TiltState::Flat,
        }
    }

    pub fn update(&mut self, accel: Accel) -> Option<InputEvent> {
        let x = accel.x;
        if x > SWIPE_THRESHOLD && self.state != TiltState::TiltedLeft {
            self.state = TiltState::TiltedLeft;
            return Some(InputEvent::Previous);
        }
        if x < -SWIPE_THRESHOLD && self.state != TiltState::TiltedRight {
            self.state = TiltState::TiltedRight;
            return Some(InputEvent::Next);
        }
        if x.abs() < FLAT_THRESHOLD {
            self.state = TiltState::Flat;
        }

        // While flat-ish, report partial tilts for on-screen feedback
        (self.state == TiltState::Flat && x.abs() > TILT_FEEDBACK_THRESHOLD)
            .then(|| InputEvent::Tilt(-x.signum()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use googletest::prelude::*;

    fn clicks(debouncer: &mut Debouncer, readings: &[bool]) -> Vec<bool> {
        readings.iter().map(|&r| debouncer.update(r)).collect()
    }

    #[gtest]
    fn a_press_clicks_once_on_the_down_edge() {
        let mut d = Debouncer::default();
        expect_that!(
            clicks(&mut d, &[false, true, true, true, false]),
            elements_are![
                eq(&false),
                eq(&true),
                eq(&false),
                eq(&false),
                eq(&false)
            ]
        );
    }

    #[gtest]
    fn bounces_within_the_debounce_period_are_ignored() {
        let mut d = Debouncer::new();
        expect_true!(d.update(true));
        // Contact bounce: released and pressed again a few frames later
        expect_that!(clicks(&mut d, &[false, true, false]), each(eq(&false)));
        // After the lockout, a new press clicks again
        clicks(&mut d, &[false; DEBOUNCE_FRAMES as usize]);
        expect_true!(d.update(true));
    }

    fn tilt(x: f32) -> Accel {
        Accel {
            x,
            ..Accel::default()
        }
    }

    #[gtest]
    fn full_tilts_swipe_once_until_flat_again() {
        let mut t = TiltTracker::default();
        expect_that!(t.update(tilt(-0.9)), some(eq(InputEvent::Next)));
        expect_that!(t.update(tilt(-0.95)), none(), "still tilted: no repeat");
        expect_that!(t.update(tilt(-0.6)), none(), "not flat yet: no feedback");
        expect_that!(t.update(tilt(0.0)), none());
        expect_that!(t.update(tilt(-0.9)), some(eq(InputEvent::Next)));
    }

    #[gtest]
    fn tilting_left_swipes_to_the_previous_app() {
        let mut t = TiltTracker::new();
        expect_that!(t.update(tilt(0.9)), some(eq(InputEvent::Previous)));
        expect_that!(t.update(tilt(0.85)), none());
    }

    #[gtest]
    fn swinging_straight_across_swipes_both_ways() {
        let mut t = TiltTracker::new();
        expect_that!(t.update(tilt(0.9)), some(eq(InputEvent::Previous)));
        expect_that!(t.update(tilt(-0.9)), some(eq(InputEvent::Next)));
    }

    #[gtest]
    fn partial_tilts_report_direction_feedback_while_flat() {
        let mut t = TiltTracker::new();
        expect_that!(t.update(tilt(0.6)), some(eq(InputEvent::Tilt(-1.0))));
        expect_that!(t.update(tilt(-0.6)), some(eq(InputEvent::Tilt(1.0))));
        expect_that!(
            t.update(tilt(0.45)),
            none(),
            "below the feedback threshold"
        );
    }
}
