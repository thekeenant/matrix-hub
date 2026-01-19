//! Button monitoring task
//!
//! Monitors the UP (GPIO6) and DOWN (GPIO7) buttons and sends a signal when either is pressed.
//! Uses debouncing to avoid multiple triggers.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use esp_hal::gpio::Input;
use log::info;

use crate::app_rotation::{AppRotationDirection, AppRotationSignal};

pub type ButtonPressSignal = Channel<CriticalSectionRawMutex, (), 1>;

#[embassy_executor::task]
pub async fn button_monitor_task(
    mut button_up: Input<'static>,
    mut button_down: Input<'static>,
    rotation_signal: &'static AppRotationSignal,
) {
    use embassy_futures::select::Either;
    const DEBOUNCE_MS: u64 = 200;

    loop {
        let which = embassy_futures::select::select(
            button_up.wait_for_falling_edge(),
            button_down.wait_for_falling_edge(),
        )
        .await;

        let direction = match which {
            Either::First(_) => {
                info!("UP button pressed!");
                AppRotationDirection::Prev
            }
            Either::Second(_) => {
                info!("DOWN button pressed!");
                AppRotationDirection::Next
            }
        };

        rotation_signal.signal(direction);

        Timer::after(Duration::from_millis(DEBOUNCE_MS)).await;

        while button_up.is_low() || button_down.is_low() {
            Timer::after(Duration::from_millis(10)).await;
        }
    }
}
