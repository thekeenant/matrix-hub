//! WiFi connection management task.

extern crate alloc;

use alloc::string::String;

use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{ClientConfig, ModeConfig, WifiController, WifiStaState};
use log::info;

use crate::state::SharedMatrixHubState;

/// WiFi connection management task
#[embassy_executor::task]
pub async fn wifi_connection_task(
    controller: WifiController<'static>,
    matrix_hub_state: SharedMatrixHubState,
) {
    wifi_connection_task_impl(controller, matrix_hub_state)
        .await
        .expect("Connection task failed");
}

async fn wifi_connection_task_impl(
    mut controller: WifiController<'static>,
    matrix_hub_state: SharedMatrixHubState,
) -> anyhow::Result<()> {
    info!("start connection task");
    info!("Device capabilities: {:?}", controller.capabilities());
    let mut current_ssid = String::new();
    let mut current_password = String::new();
    loop {
        // Read current WiFi config from state
        let (ssid, password) = {
            let state = matrix_hub_state.lock().await;
            state
                .config
                .as_ref()
                .and_then(|c| c.wifi.as_ref())
                .map(|w| (w.ssid.clone(), w.password.clone()))
                .unwrap_or_default()
        };

        // Skip if no SSID is configured
        if ssid.is_empty() {
            Timer::after(Duration::from_micros(100)).await;
            continue;
        }

        // Check if credentials have changed or if disconnected
        let credentials_changed = ssid != current_ssid || password != current_password;
        let is_connected = matches!(esp_radio::wifi::sta_state(), WifiStaState::Connected);

        // Nothing to do if already connected and credentials haven't changed
        if is_connected && !credentials_changed {
            Timer::after(Duration::from_micros(100)).await;
            continue;
        }

        // If credentials changed, disconnect and update config
        if credentials_changed {
            info!("WiFi credentials changed to '{}'", ssid);
            current_ssid = ssid.clone();
            current_password = password.clone();

            if is_connected {
                info!("Disconnecting from current network...");
                controller.disconnect_async().await?;
            }

            // Set new configuration
            let client_config = ModeConfig::Client(
                ClientConfig::default()
                    .with_ssid(current_ssid.as_str().into())
                    .with_password(current_password.as_str().into())
                    .with_auth_method(if current_password.is_empty() {
                        esp_radio::wifi::AuthMethod::None
                    } else {
                        esp_radio::wifi::AuthMethod::Wpa2Personal
                    }),
            );
            controller.set_config(&client_config)?;
        }

        // Attempt to connect
        let timeout_duration = Duration::from_secs(10);
        info!(
            "Wifi connecting to '{}' with timeout {} seconds",
            current_ssid,
            timeout_duration.as_secs()
        );
        let connect_future = controller.connect_async();
        let timeout_future = Timer::after(timeout_duration);
        match select(connect_future, timeout_future).await {
            Either::First(Ok(_)) => info!("WiFi connected!"),
            Either::First(Err(e)) => {
                info!("Failed to connect to Wifi: {:?}", e);
                Timer::after(Duration::from_millis(5000)).await;
            }
            Either::Second(_) => {
                info!("WiFi connection timed out after 10 seconds");
                Timer::after(Duration::from_millis(5000)).await;
            }
        }
    }
}
