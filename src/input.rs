use esp_idf_svc::hal::gpio::{Input, PinDriver};

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
