//! App controller task - orchestrates app rotation and lifecycle.
//!
//! This module contains the AppController which manages multiple apps,
//! calling their lifecycle methods (mount, shown, hidden, run) and rotating
//! between them on a schedule. Runs on Core 0.

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer};
use esp_hal::gpio::Input;
use esp_radio::wifi::{Interfaces, WifiController};
use log::info;

use crate::{
    apps::App,
    proto::app_state::MatrixHubState,
    tasks::{
        hub75::Hub75Brightness,
        sntp::{current_time_blocking, sntp_task},
        wifi_startup::{SharedHttpTcpClient, SharedNetworkStack, wifi_startup_task},
    },
};

extern crate alloc;
use alloc::{boxed::Box, sync::Arc, vec::Vec};

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};

pub type SharedMatrixHubState = Arc<Mutex<CriticalSectionRawMutex, MatrixHubState>>;
pub type ButtonPressSignal = Channel<CriticalSectionRawMutex, (), 1>;

#[embassy_executor::task]
pub async fn app_controller_task(
    spawner: Spawner,
    wifi_controller: Box<WifiController<'static>>,
    interfaces: Box<Interfaces<'static>>,
    apps: &'static Vec<Arc<dyn App>>,
    matrix_hub_state: SharedMatrixHubState,
    http_client: &'static SharedHttpTcpClient,
    network_stack: &'static SharedNetworkStack,
    hub75_brightness: Arc<Hub75Brightness>,
    button_press_signal: &'static ButtonPressSignal,
) {
    app_controller_impl(
        spawner,
        wifi_controller,
        interfaces,
        apps,
        matrix_hub_state,
        http_client,
        network_stack,
        hub75_brightness,
        button_press_signal,
    )
    .await
    .expect("App controller failed");
}

async fn app_controller_impl(
    spawner: Spawner,
    wifi_controller: Box<WifiController<'static>>,
    interfaces: Box<Interfaces<'static>>,
    apps: &'static Vec<Arc<dyn App>>,
    matrix_hub_state: SharedMatrixHubState,
    http_client: &'static SharedHttpTcpClient,
    network_stack: &'static SharedNetworkStack,
    _hub75_brightness: Arc<Hub75Brightness>,
    button_press_signal: &'static ButtonPressSignal,
) -> anyhow::Result<()> {
    info!("AppController: starting with {} apps", apps.len());

    {
        let mut state = matrix_hub_state.lock().await;
        state.system_info.get_or_insert(Default::default());
        state.plasma.get_or_insert(Default::default());
        state.mta.get_or_insert(Default::default());
        state.clock.get_or_insert(Default::default());
        if state.sandbox.is_none() {
            use alloc::vec::Vec;

            use crate::{
                apps::sandbox::MAX_PARTICLES,
                proto::app_state::{Particle, SandboxAppState},
            };

            let mut particles = Vec::new();
            for _ in 0..MAX_PARTICLES {
                particles.push(Particle {
                    active: false,
                    x: 0.0,
                    y: 0.0,
                    vx: 0.0,
                    vy: 0.0,
                    lifetime: 0,
                    color_r: 0,
                    color_g: 0,
                    color_b: 0,
                });
            }

            state.sandbox = Some(SandboxAppState {
                accel_x: 0.0,
                accel_y: 1.0,
                accel_z: 0.0,
                particles,
                spawn_counter: 0,
            });
        }
    }

    spawner.spawn(wifi_startup_task(
        spawner,
        wifi_controller,
        interfaces,
        http_client,
        network_stack,
        matrix_hub_state.clone(),
    ))?;

    spawner.spawn(sntp_task(network_stack))?;

    for app in apps.iter() {
        app.mount(spawner, http_client.clone()).await?;
    }

    let _ = current_time_blocking().await;

    let mut current_index = 0;

    if !apps.is_empty() {
        apps[current_index].shown();
        let mut state = matrix_hub_state.lock().await;
        if let Some(ref mut system_info) = state.system_info {
            system_info.current_app_index = current_index as u32;
        }
    }

    loop {
        if apps.is_empty() {
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }

        let current_app = apps[current_index].clone();

        let app_future = current_app.run();
        let button_future = button_press_signal.receive();

        match select(app_future, button_future).await {
            Either::First(_) => {
                info!("App {} completed its run() early", current_index);
            }
            Either::Second(_) => {
                info!("Rotating from app {} (button press)", current_index);
            }
        }

        current_app.hidden();

        current_index = (current_index + 1) % apps.len();
        {
            let mut state = matrix_hub_state.lock().await;
            if let Some(ref mut system_info) = state.system_info {
                system_info.current_app_index = current_index as u32;
            }
        }

        apps[current_index].shown();
        info!("Showing app {}", current_index);
    }
}

/// Button monitoring task
///
/// Monitors the UP (GPIO6) and DOWN (GPIO7) buttons and sends a signal when either is pressed.
/// Uses debouncing to avoid multiple triggers.
#[embassy_executor::task]
pub async fn button_monitor_task(
    mut button_up: Input<'static>,
    mut button_down: Input<'static>,
    signal: &'static ButtonPressSignal,
) {
    use embassy_futures::select::Either;
    const DEBOUNCE_MS: u64 = 200;

    loop {
        let which = embassy_futures::select::select(
            button_up.wait_for_falling_edge(),
            button_down.wait_for_falling_edge(),
        )
        .await;

        match which {
            Either::First(_) => info!("UP button pressed!"),
            Either::Second(_) => info!("DOWN button pressed!"),
        }

        signal.send(()).await;

        Timer::after(Duration::from_millis(DEBOUNCE_MS)).await;

        while button_up.is_low() || button_down.is_low() {
            Timer::after(Duration::from_millis(10)).await;
        }
    }
}
