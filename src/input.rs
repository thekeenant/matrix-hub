use esp_idf_svc::hal::gpio::{Input, PinDriver};
use lis3dh::accelerometer::Accelerometer;

pub struct Button<'a> {
    pin: PinDriver<'a, Input>,
    last_state: bool,
    debounce_timer: u32,
}

impl<'a> Button<'a> {
    pub const fn new(pin: PinDriver<'a, Input>) -> Self {
        Self {
            pin,
            last_state: true, // true because active-low
            debounce_timer: 0,
        }
    }

    /// Call this every frame to check if the button was just clicked
    pub fn is_clicked(&mut self) -> bool {
        let state = self.pin.is_high(); // true if NOT pressed (pull-up)
        let mut clicked = false;

        // Trigger on falling edge (pressed)
        if !state && self.last_state && self.debounce_timer == 0 {
            clicked = true;
            self.debounce_timer = 15; // roughly 240ms debounce at 60fps
        }

        if self.debounce_timer > 0 {
            self.debounce_timer -= 1;
        }

        self.last_state = state;
        clicked
    }
}

#[derive(PartialEq)]
pub enum GestureState {
    Flat,
    TiltedRight,
    TiltedLeft,
}

pub enum GestureEvent {
    None,
    SwipeRight,
    SwipeLeft,
    Tilting(f32),
}

pub struct GestureDetector<'a> {
    pub accelerometer:
        lis3dh::Lis3dh<lis3dh::Lis3dhI2C<esp_idf_svc::hal::i2c::I2cDriver<'a>>>,
    pub state: GestureState,
    pub last_accel: (f32, f32, f32),
}

impl<'a> GestureDetector<'a> {
    pub fn new(
        i2c: esp_idf_svc::hal::i2c::I2cDriver<'a>,
    ) -> Result<Self, anyhow::Error> {
        let mut accelerometer =
            lis3dh::Lis3dh::new_i2c(i2c, lis3dh::SlaveAddr::Alternate)
                .map_err(|_| {
                    anyhow::anyhow!("Failed to initialize LIS3DH accelerometer")
                })?;

        // Initial setup
        accelerometer
            .set_range(lis3dh::Range::G2)
            .map_err(|_| anyhow::anyhow!("Failed to set LIS3DH range"))?;

        Ok(Self {
            accelerometer,
            state: GestureState::Flat,
            last_accel: (0.0, 0.0, 0.0),
        })
    }

    pub fn poll(&mut self) -> GestureEvent {
        if let Ok(accel) = self.accelerometer.accel_norm() {
            self.last_accel = (accel.x, accel.y, accel.z);
            // From user calibration:
            // Flat: X ~ 0.0
            // Tilt Left: X > 0.3 (Positive)
            // Tilt Right: X < -0.3 (Negative)
            let threshold = 0.8;
            let reset_threshold = 0.4;

            if accel.x > threshold && self.state != GestureState::TiltedLeft {
                self.state = GestureState::TiltedLeft;
                return GestureEvent::SwipeLeft;
            } else if accel.x < -threshold
                && self.state != GestureState::TiltedRight
            {
                self.state = GestureState::TiltedRight;
                return GestureEvent::SwipeRight;
            } else if accel.x.abs() < reset_threshold {
                self.state = GestureState::Flat;
            }

            // If we are flat, we report the analog tilt
            let analog_start = 0.5;
            if self.state == GestureState::Flat && accel.x.abs() > analog_start
            {
                let sign = accel.x.signum();
                // Invert sign so Tilting > 0 means Right, Tilting < 0 means Left
                return GestureEvent::Tilting(-sign);
            }
        }
        GestureEvent::None
    }
}
