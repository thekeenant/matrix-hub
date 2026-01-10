//! Network stack runner task.

use embassy_net::Runner;
use esp_radio::wifi::WifiDevice;

/// Network stack runner task
#[embassy_executor::task]
pub async fn wifi_net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}
