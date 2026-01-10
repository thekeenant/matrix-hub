//! Accelerometer reading task for LIS3DH sensor

extern crate alloc;

use embassy_time::{Duration, Timer};
use esp_hal::{Async, i2c::master::I2c};
use lis3dh_async::{Lis3dh, Range, SlaveAddr};
use log::info;

use crate::state::SharedMatrixHubState;

/// Task to read LIS3DH accelerometer and update sandbox state
#[embassy_executor::task]
pub async fn accelerometer_task(i2c: I2c<'static, Async>, state: SharedMatrixHubState) {
    info!("Initializing LIS3DH accelerometer...");
    let mut lis3dh = match Lis3dh::new_i2c(i2c, SlaveAddr::Alternate).await {
        Ok(mut accel) => {
            info!("LIS3DH found at {:?}", SlaveAddr::Alternate);
            if let Err(e) = accel.set_range(Range::G2).await {
                info!("Failed to set accelerometer range: {:?}", e);
            }
            info!("LIS3DH initialized successfully!");
            accel
        }
        Err(e) => {
            info!("ERROR: Failed to initialize LIS3DH at 0x19: {:?}", e);
            info!("Check I2C wiring: SDA=GPIO16, SCL=GPIO17 (STEMMA QT)");

            // Just set default gravity and return - don't simulate
            loop {
                Timer::after(Duration::from_secs(5)).await;
                info!("LIS3DH still not available - gravity defaulting to down");
            }
        }
    };

    // Read accelerometer continuously
    loop {
        Timer::after(Duration::from_millis(50)).await;

        if let Ok(accel) = lis3dh.accel_raw().await {
            // accel_raw returns i16 values
            // At ±2g range, normalize to approximately -1.0 to 1.0 range
            // Full scale at ±2g is about ±16384
            let raw_x = accel.x as f32 / 16384.0;
            let raw_y = accel.y as f32 / 16384.0;
            let raw_z = accel.z as f32 / 16384.0;

            // Remap axes for MatrixPortal S3 orientation
            let mut state = state.lock().await;
            let accel = state
                .system_info
                .get_or_insert_default()
                .accelerometer
                .get_or_insert_default();
            accel.accel_x = -raw_x; // Flip X for correct left/right orientation
            accel.accel_y = raw_y;
            accel.accel_z = raw_z;
        }
    }
}
