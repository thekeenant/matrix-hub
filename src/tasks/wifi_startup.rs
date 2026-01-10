//! WiFi initialization task.
//!
//! This module handles WiFi initialization and network stack setup.

use alloc::{boxed::Box, sync::Arc};

use embassy_executor::Spawner;
use embassy_net::{
    DhcpConfig, Stack, StackResources,
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use esp_hal::rng::Rng;
use esp_radio::wifi::{ClientConfig, Interfaces, ModeConfig, WifiController};
use log::{info, warn};
use reqwless::client::{HttpClient, TlsConfig};

use super::{wifi_connection::wifi_connection_task, wifi_net::wifi_net_task};
use crate::{mk_static, proto::app_state::MatrixHubState};

extern crate alloc;

pub type SharedMatrixHubState = Arc<Mutex<CriticalSectionRawMutex, MatrixHubState>>;

pub type HttpTcpClient<'a> = HttpClient<'a, TcpClient<'a, 2, 4096, 4096>, DnsSocket<'a>>;
pub type SharedHttpTcpClient = Arc<
    Mutex<
        CriticalSectionRawMutex,
        Option<&'static Mutex<CriticalSectionRawMutex, HttpTcpClient<'static>>>,
    >,
>;
pub type SharedNetworkStack = Arc<Mutex<CriticalSectionRawMutex, Option<&'static Stack<'static>>>>;

/// Main WiFi initialization task
#[embassy_executor::task]
pub async fn wifi_startup_task(
    spawner: Spawner,
    wifi_controller: Box<WifiController<'static>>,
    interfaces: Box<Interfaces<'static>>,
    http_client: &'static SharedHttpTcpClient,
    network_stack: &'static SharedNetworkStack,
    matrix_hub_state: SharedMatrixHubState,
) {
    wifi_startup_task_impl(
        spawner,
        wifi_controller,
        interfaces,
        http_client,
        network_stack,
        matrix_hub_state,
    )
    .await
    .expect("WiFi startup task failed");
}

async fn wifi_startup_task_impl(
    spawner: Spawner,
    mut wifi_controller: Box<WifiController<'static>>,
    interfaces: Box<Interfaces<'static>>,
    http_client: &'static SharedHttpTcpClient,
    network_stack: &'static SharedNetworkStack,
    matrix_hub_state: SharedMatrixHubState,
) -> anyhow::Result<()> {
    Timer::after(Duration::from_secs(2)).await;

    // Get WiFi config from state
    let (ssid, password) = {
        let state = matrix_hub_state.lock().await;
        let wifi_config = state
            .config
            .as_ref()
            .and_then(|c| c.wifi.as_ref())
            .ok_or_else(|| anyhow::anyhow!("WiFi config not found"))?;
        (wifi_config.ssid.clone(), wifi_config.password.clone())
    };

    info!("Configuring WiFi");
    let client_config = ClientConfig::default()
        .with_ssid(ssid.clone().into())
        .with_password(password.clone().into());
    wifi_controller.set_config(&ModeConfig::Client(client_config))?;

    info!("Starting WiFi");
    wifi_controller.start().expect("Failed to start WiFi");

    info!("Connecting to WiFi network '{}'", ssid);
    wifi_controller
        .connect()
        .expect("Failed to connect to WiFi");

    info!("Waiting for WiFi connection (timeout: 3s)...");
    let connection_deadline = embassy_time::Instant::now() + Duration::from_secs(3);

    loop {
        match wifi_controller.is_connected() {
            Ok(true) => {
                info!("WiFi connected!");
                break;
            }
            Ok(false) | Err(_) => {
                if embassy_time::Instant::now() >= connection_deadline {
                    warn!("WiFi connection timeout after 3 seconds, continuing anyway...");
                    break;
                }
                Timer::after(Duration::from_millis(100)).await;
            }
        }
    }

    let rng = Rng::new();
    let net_seed = rng.random() as u64 | ((rng.random() as u64) << 32);
    let tls_seed = rng.random() as u64 | ((rng.random() as u64) << 32);

    let dhcp_config = DhcpConfig::default();
    let config = embassy_net::Config::dhcpv4(dhcp_config);

    // Init network stack
    let (stack, runner) = embassy_net::new(
        interfaces.sta,
        config,
        mk_static!(StackResources<8>, StackResources::<8>::new()),
        net_seed,
    );

    let stack = mk_static!(Stack<'static>, stack);

    spawner.spawn(wifi_net_task(runner))?;

    // Convert String to &'static str for wifi_connection_task
    let ssid_static: &'static str = Box::leak(ssid.into_boxed_str());
    let password_static: &'static str = Box::leak(password.into_boxed_str());
    spawner.spawn(wifi_connection_task(
        wifi_controller,
        ssid_static,
        password_static,
    ))?;

    wait_for_connection(*stack).await;

    let dns = mk_static!(DnsSocket<'static>, DnsSocket::new(*stack));
    let tcp_state =
        mk_static!(TcpClientState<2, 4096, 4096>, TcpClientState::<2, 4096, 4096>::new());
    let tcp = mk_static!(
        TcpClient<'static, 2, 4096, 4096>,
        TcpClient::new(*stack, tcp_state)
    );

    // Allocate TLS buffers on the heap to avoid running out of static memory
    let rx_buffer = Box::leak(alloc::vec![0u8; 64 * 1024].into_boxed_slice());
    let tx_buffer = Box::leak(alloc::vec![0u8; 64 * 1024].into_boxed_slice());

    let tls = TlsConfig::new(
        tls_seed,
        rx_buffer,
        tx_buffer,
        reqwless::client::TlsVerify::None,
    );

    let client = mk_static!(
        Mutex<CriticalSectionRawMutex, HttpTcpClient<'static>>,
        Mutex::new(HttpClient::new_with_tls(tcp, dns, tls))
    );

    // Store HTTP client in shared variable for app_task to use
    {
        let mut http = http_client.lock().await;
        *http = Some(client);
    }

    // Store network stack in shared variable for SNTP task
    {
        let mut net_stack = network_stack.lock().await;
        *net_stack = Some(stack);
    }

    Ok(())
}

async fn wait_for_connection(stack: Stack<'_>) {
    info!("Waiting for link to be up");
    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    info!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            info!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}
