//! Physical input: two buttons and an LIS3DH accelerometer for tilt gestures.

use anyhow::{anyhow, Result};
use esp_idf_svc::hal::gpio::{Input as GpioInput, PinDriver};
use esp_idf_svc::hal::i2c::I2cDriver;
use lis3dh::accelerometer::Accelerometer;
use lis3dh::{Lis3dh, Lis3dhI2C};

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

/// Acceleration in g, as last read from the accelerometer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Accel {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// All inputs, polled once per frame.
pub struct Input<'d> {
    up: Button<'d>,
    down: Button<'d>,
    gestures: GestureDetector<'d>,
}

impl<'d> Input<'d> {
    pub fn new(
        up: PinDriver<'d, GpioInput>,
        down: PinDriver<'d, GpioInput>,
        i2c: I2cDriver<'d>,
    ) -> Result<Self> {
        Ok(Self {
            up: Button::new(up),
            down: Button::new(down),
            gestures: GestureDetector::new(i2c)?,
        })
    }

    /// Poll every input, calling `on_event` for each event that occurred.
    pub fn poll(&mut self, mut on_event: impl FnMut(InputEvent)) {
        if self.up.is_clicked() {
            on_event(InputEvent::Next);
        }
        if self.down.is_clicked() {
            on_event(InputEvent::Previous);
        }
        if let Some(event) = self.gestures.poll() {
            on_event(event);
        }
    }

    /// The most recent accelerometer reading.
    pub const fn accel(&self) -> Accel {
        self.gestures.last_accel
    }
}

// ============================================================================
// Buttons
// ============================================================================

/// Debounce period, in frames (~240ms at 60fps)
const DEBOUNCE_FRAMES: u32 = 15;

/// An active-low push button with a pull-up.
struct Button<'d> {
    pin: PinDriver<'d, GpioInput>,
    was_released: bool,
    debounce_frames: u32,
}

impl<'d> Button<'d> {
    const fn new(pin: PinDriver<'d, GpioInput>) -> Self {
        Self {
            pin,
            was_released: true,
            debounce_frames: 0,
        }
    }

    /// Call every frame. True on the frame the button is pressed down.
    fn is_clicked(&mut self) -> bool {
        let released = self.pin.is_high(); // active-low
        let clicked =
            !released && self.was_released && self.debounce_frames == 0;
        if clicked {
            self.debounce_frames = DEBOUNCE_FRAMES;
        }
        self.debounce_frames = self.debounce_frames.saturating_sub(1);
        self.was_released = released;
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum TiltState {
    Flat,
    TiltedRight,
    TiltedLeft,
}

struct GestureDetector<'d> {
    accelerometer: Lis3dh<Lis3dhI2C<I2cDriver<'d>>>,
    state: TiltState,
    last_accel: Accel,
}

impl<'d> GestureDetector<'d> {
    fn new(i2c: I2cDriver<'d>) -> Result<Self> {
        let mut accelerometer =
            Lis3dh::new_i2c(i2c, lis3dh::SlaveAddr::Alternate).map_err(
                |e| anyhow!("Failed to initialize LIS3DH accelerometer: {e:?}"),
            )?;
        accelerometer
            .set_range(lis3dh::Range::G2)
            .map_err(|e| anyhow!("Failed to set LIS3DH range: {e:?}"))?;

        Ok(Self {
            accelerometer,
            state: TiltState::Flat,
            last_accel: Accel::default(),
        })
    }

    /// Positive x is a tilt to the left, negative to the right.
    fn poll(&mut self) -> Option<InputEvent> {
        let a = self.accelerometer.accel_norm().ok()?;
        self.last_accel = Accel {
            x: a.x,
            y: a.y,
            z: a.z,
        };

        // A swipe fires once per tilt; returning to flat re-arms it.
        if a.x > SWIPE_THRESHOLD && self.state != TiltState::TiltedLeft {
            self.state = TiltState::TiltedLeft;
            return Some(InputEvent::Previous);
        }
        if a.x < -SWIPE_THRESHOLD && self.state != TiltState::TiltedRight {
            self.state = TiltState::TiltedRight;
            return Some(InputEvent::Next);
        }
        if a.x.abs() < FLAT_THRESHOLD {
            self.state = TiltState::Flat;
        }

        // While flat-ish, report partial tilts for on-screen feedback
        (self.state == TiltState::Flat && a.x.abs() > TILT_FEEDBACK_THRESHOLD)
            .then(|| InputEvent::Tilt(-a.x.signum()))
    }
}
