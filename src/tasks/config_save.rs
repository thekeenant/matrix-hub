//! Config save task.
//!
//! This task periodically checks if the config in MatrixHubState has changed
//! and saves it to flash if necessary.

extern crate alloc;

use alloc::sync::Arc;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use log::info;

use crate::{
    nvs::Kvs,
    proto::app_state::{Config, MatrixHubState, key_value_storage},
};

/// Task that monitors config changes and persists them to flash.
#[embassy_executor::task]
pub async fn config_save_task(
    matrix_hub_state: Arc<Mutex<CriticalSectionRawMutex, MatrixHubState>>,
    kvs: Arc<Mutex<CriticalSectionRawMutex, Kvs>>,
) {
    let mut last_saved_config: Option<Config> = None;

    loop {
        Timer::after(Duration::from_millis(300)).await;

        let current_config = {
            let state = matrix_hub_state.lock().await;
            state.config.clone()
        };

        // Check if config has changed
        let should_save = match (&last_saved_config, &current_config) {
            (None, Some(_)) => true,
            (Some(old), Some(new)) => {
                // Simple comparison - if the encoded bytes differ, config changed
                use prost::Message;
                let old_bytes = old.encode_to_vec();
                let new_bytes = new.encode_to_vec();
                old_bytes != new_bytes
            }
            _ => false,
        };

        if should_save {
            if let Some(config) = current_config.clone() {
                let value = key_value_storage::Value {
                    value_oneof: Some(key_value_storage::value::ValueOneof::Config(config.clone())),
                };

                let mut kvs_lock = kvs.lock().await;
                match kvs_lock.set(key_value_storage::Key::Config, value) {
                    Ok(_) => {
                        info!("Config saved to flash");
                        last_saved_config = Some(config);
                    }
                    Err(e) => {
                        info!("Failed to save config to flash: {}", e);
                    }
                }
            }
        }
    }
}
