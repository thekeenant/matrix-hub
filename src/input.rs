//! Physical input: two buttons and an LIS3DH accelerometer. Interpretation
//! (debouncing, gestures) lives in `matrix_hub_core::input`.

use anyhow::{anyhow, Result};
use esp_idf_svc::hal::gpio::{Input as GpioInput, PinDriver};
use esp_idf_svc::hal::i2c::I2cDriver;
use lis3dh::accelerometer::Accelerometer;
use lis3dh::{Lis3dh, Lis3dhI2C};
use matrix_hub_core::input::{Accel, Debouncer, InputEvent, TiltTracker};

/// All inputs, polled once per frame.
pub struct Input<'d> {
    up: Button<'d>,
    down: Button<'d>,
    accelerometer: Lis3dh<Lis3dhI2C<I2cDriver<'d>>>,
    tilt: TiltTracker,
    last_accel: Accel,
}

impl<'d> Input<'d> {
    pub fn new(
        up: PinDriver<'d, GpioInput>,
        down: PinDriver<'d, GpioInput>,
        i2c: I2cDriver<'d>,
    ) -> Result<Self> {
        let mut accelerometer =
            Lis3dh::new_i2c(i2c, lis3dh::SlaveAddr::Alternate).map_err(
                |e| anyhow!("Failed to initialize LIS3DH accelerometer: {e:?}"),
            )?;
        accelerometer
            .set_range(lis3dh::Range::G2)
            .map_err(|e| anyhow!("Failed to set LIS3DH range: {e:?}"))?;

        Ok(Self {
            up: Button::new(up),
            down: Button::new(down),
            accelerometer,
            tilt: TiltTracker::new(),
            last_accel: Accel::default(),
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
        if let Ok(a) = self.accelerometer.accel_norm() {
            self.last_accel = Accel {
                x: a.x,
                y: a.y,
                z: a.z,
            };
            if let Some(event) = self.tilt.update(self.last_accel) {
                on_event(event);
            }
        }
    }

    /// The most recent accelerometer reading.
    pub const fn accel(&self) -> Accel {
        self.last_accel
    }
}

/// An active-low push button with a pull-up.
struct Button<'d> {
    pin: PinDriver<'d, GpioInput>,
    debouncer: Debouncer,
}

impl<'d> Button<'d> {
    const fn new(pin: PinDriver<'d, GpioInput>) -> Self {
        Self {
            pin,
            debouncer: Debouncer::new(),
        }
    }

    /// Call every frame. True on the frame the button is pressed down.
    fn is_clicked(&mut self) -> bool {
        self.debouncer.update(self.pin.is_low())
    }
}
