//! Flash busy coordinator task.
//!
//! This task monitors FLASH_BUSY and coordinates pausing other tasks on the AppCpu.

use core::sync::atomic::Ordering;

use esp_hal::ram;
use log::info;

use crate::nvs::{CORE_0_WRITING_TO_FLASH, CORE_1_PAUSED};

/// Task that monitors FLASH_BUSY and sets the ready flag
#[embassy_executor::task]
#[ram]
pub async fn wait_for_flash_busy_task() {
    loop {
        // Wait for FLASH_BUSY to be set
        loop {
            if CORE_0_WRITING_TO_FLASH.load(Ordering::Acquire) {
                break;
            }
            embassy_futures::yield_now().await;
        }

        // Send the ready signal to core 0 to allow it to write to flash.
        CORE_1_PAUSED.store(true, Ordering::Release);

        // Pause Core 1 until FLASH_BUSY is cleared.
        info!("Core 1 pausing for flash write");
        while CORE_0_WRITING_TO_FLASH.load(Ordering::Acquire) {
            core::hint::spin_loop();
        }
        info!("Core 1 resuming after flash write");

        // Clear the ready signal to prevent core 0 from writing to flash until
        // we are ready again.
        CORE_1_PAUSED.store(false, Ordering::Release);
    }
}
