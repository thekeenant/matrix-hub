//! App controller task - orchestrates app rotation and lifecycle.
//!
//! This module contains the AppController which manages multiple apps,
//! calling their lifecycle methods (mount, shown, hidden, run) and rotating
//! between them on a schedule. Runs on Core 0.

extern crate alloc;

use alloc::{sync::Arc, vec::Vec};

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use log::info;
use rhai::Engine;

use crate::{
    app_rotation::{AppRotationDirection, AppRotationSignal},
    apps::{
        App, RunContext, app_script::AppScript, mta::MtaApp, plasma::PlasmaApp, sandbox::SandboxApp,
    },
    proto::app_state::{AppId, app_id},
    state::SharedMatrixHubState,
    tasks::hub75::Hub75Brightness,
    wifi::SharedHttpTcpClient,
};

#[embassy_executor::task]
pub async fn app_controller_task(
    spawner: Spawner,
    apps: &'static Mutex<CriticalSectionRawMutex, Vec<Arc<dyn App>>>,
    matrix_hub_state: SharedMatrixHubState,
    http_client: SharedHttpTcpClient,
    hub75_brightness: Arc<Hub75Brightness>,
    rotation_signal: &'static AppRotationSignal,
    engine: &'static Mutex<CriticalSectionRawMutex, &'static Engine>,
) {
    app_controller_impl(
        spawner,
        apps,
        matrix_hub_state,
        http_client,
        hub75_brightness,
        rotation_signal,
        engine,
    )
    .await
    .expect("App controller failed");
}

async fn app_controller_impl(
    spawner: Spawner,
    apps: &'static Mutex<CriticalSectionRawMutex, Vec<Arc<dyn App>>>,
    matrix_hub_state: SharedMatrixHubState,
    http_client: SharedHttpTcpClient,
    _hub75_brightness: Arc<Hub75Brightness>,
    rotation_signal: &'static AppRotationSignal,
    engine: &'static Mutex<CriticalSectionRawMutex, &'static Engine>,
) -> anyhow::Result<()> {
    info!("AppController: starting");

    let mut last_enabled_apps: Vec<AppId> = Vec::new();

    loop {
        // Check if enabled apps have changed and rebuild if necessary
        let current_enabled_apps: Vec<AppId> = {
            let state = matrix_hub_state.lock().await;
            state
                .config
                .as_ref()
                .and_then(|c| c.app_rotation.as_ref())
                .map(|ar| {
                    ar.enabled_apps
                        .iter()
                        .filter_map(|e| e.id.clone().map(|id| AppId { id: Some(id) }))
                        .collect()
                })
                .unwrap_or_default()
        };

        if current_enabled_apps != last_enabled_apps {
            info!("Enabled apps changed, rebuilding apps list");
            let state = matrix_hub_state.lock().await;
            let enabled_apps = state
                .config
                .as_ref()
                .and_then(|c| c.app_rotation.as_ref())
                .map(|ar| ar.enabled_apps.as_slice())
                .unwrap_or(&[]);

            let mut apps_guard = apps.lock().await;
            apps_guard.clear();

            for enabled_id in enabled_apps {
                let app: Arc<dyn App> = match &enabled_id.id {
                    Some(app_id::Id::Mta(_)) => {
                        Arc::new(MtaApp::build(&matrix_hub_state, enabled_id.clone()))
                    }
                    Some(app_id::Id::Plasma(_)) => {
                        Arc::new(PlasmaApp::build(&matrix_hub_state, enabled_id.clone()))
                    }
                    Some(app_id::Id::Sandbox(_)) => {
                        Arc::new(SandboxApp::build(&matrix_hub_state, enabled_id.clone()))
                    }
                    Some(app_id::Id::AppScript(_)) => {
                        Arc::new(AppScript::build(&matrix_hub_state, enabled_id.clone()))
                    }
                    _ => {
                        info!("Warning: unknown app ID, skipping");
                        continue;
                    }
                };
                apps_guard.push(app);
            }

            if !apps_guard.is_empty() {
                info!("Built {} apps from config", apps_guard.len());
                // Mount all built apps
                for app in apps_guard.iter() {
                    let run_ctx = RunContext {
                        spawner,
                        http_client: core::cell::RefCell::new(http_client.clone()),
                        matrix_state: matrix_hub_state.clone(),
                        engine,
                    };
                    app.mount(&run_ctx).await?;
                }
                // Set initial app
                drop(state);
                drop(apps_guard);
                let mut state = matrix_hub_state.lock().await;
                let apps_guard = apps.lock().await;
                state.system_info.get_or_insert_default().current_app_id = Some(apps_guard[0].id());
            }

            last_enabled_apps = current_enabled_apps;
        }

        let (current_index, num_apps, current_app) = {
            let apps = apps.lock().await;
            if apps.is_empty() {
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
            // Find current app from stored ID
            let current_index = {
                let state = matrix_hub_state.lock().await;
                state
                    .system_info
                    .as_ref()
                    .and_then(|si| si.current_app_id.as_ref())
                    .and_then(|id| apps.iter().position(|a| a.id() == *id))
                    .unwrap_or(0)
            };
            (current_index, apps.len(), apps[current_index].clone())
        };

        let run_ctx = RunContext {
            spawner,
            http_client: core::cell::RefCell::new(http_client.clone()),
            matrix_state: matrix_hub_state.clone(),
            engine,
        };
        let app_future = current_app.run(&run_ctx);
        let rotation_future = rotation_signal.wait();

        let app_id = current_app.id();
        let direction = match select(app_future, rotation_future).await {
            Either::First(_) => {
                info!("App {:?} completed its run() early", app_id);
                AppRotationDirection::Next
            }
            Either::Second(dir) => {
                info!(
                    "Rotating from app {:?} due to rotation signal: {:?}",
                    app_id, dir
                );
                dir
            }
        };

        // Rotate to next or previous app based on direction
        let next_index = match direction {
            crate::app_rotation::AppRotationDirection::Next => (current_index + 1) % num_apps,
            crate::app_rotation::AppRotationDirection::Prev => {
                if current_index == 0 {
                    num_apps - 1
                } else {
                    current_index - 1
                }
            }
        };
        {
            let apps_guard = apps.lock().await;
            let next_app_id = apps_guard[next_index].id();
            let mut state = matrix_hub_state.lock().await;
            state.system_info.get_or_insert_default().current_app_id = Some(next_app_id.clone());
            info!(
                "Showing app {} ({:?}), current_app_id updated to {:?}",
                next_index,
                next_app_id,
                state
                    .system_info
                    .as_ref()
                    .and_then(|si| si.current_app_id.as_ref())
            );
        }
    }
}
