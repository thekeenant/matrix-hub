//! Display rendering task.
//!
//! Handles framebuffer rendering by calling the active app's render() method.

extern crate alloc;

use alloc::sync::Arc;
use core::{
    cell::RefCell,
    sync::atomic::{AtomicU32, Ordering},
};

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::Duration;
use esp_println as _;
use log::{error, info, warn};
use rhai::Engine;

use crate::{
    apps::{App, RenderContext},
    metrics::RateCounter,
    state::SharedMatrixHubState,
    tasks::{FrameBufferExchange, hub75::FrameBuffer},
};

/// Display rendering task
///
/// This task runs at 60Hz and calls the active app's render() method.
#[embassy_executor::task]
pub async fn display_task(
    rendered_buffer: &'static FrameBufferExchange<FrameBuffer>,
    free_buffer: &'static FrameBufferExchange<FrameBuffer>,
    frame_buffer: &'static mut FrameBuffer,
    apps: &'static Mutex<CriticalSectionRawMutex, alloc::vec::Vec<Arc<dyn App>>>,
    matrix_hub_state: SharedMatrixHubState,
    frames_per_second: Arc<AtomicU32>,
    ticks_per_second: AtomicU32,
    engine: &'static Mutex<CriticalSectionRawMutex, &'static Engine>,
) {
    display_task_impl(
        rendered_buffer,
        free_buffer,
        frame_buffer,
        apps,
        matrix_hub_state,
        frames_per_second,
        ticks_per_second,
        engine,
    )
    .await
    .expect("Display task failed");
}

async fn display_task_impl(
    rendered_buffer: &'static FrameBufferExchange<FrameBuffer>,
    free_buffer: &'static FrameBufferExchange<FrameBuffer>,
    mut frame_buffer: &'static mut FrameBuffer,
    apps: &'static Mutex<CriticalSectionRawMutex, alloc::vec::Vec<Arc<dyn App>>>,
    matrix_hub_state: SharedMatrixHubState,
    frames_per_second: Arc<AtomicU32>,
    ticks_per_second: AtomicU32,
    engine: &'static Mutex<CriticalSectionRawMutex, &'static Engine>,
) -> anyhow::Result<()> {
    info!("display_task: starting!");

    let engine_ref = engine;

    let mut tps_counter = RateCounter::init(Duration::from_secs(1));

    loop {
        frame_buffer.erase();
        let apps_guard = apps.lock().await;
        if !apps_guard.is_empty() {
            let mut state = matrix_hub_state.lock().await;
            let system_info = state.system_info.get_or_insert_default();
            system_info.fps = frames_per_second.load(Ordering::Relaxed);
            system_info.tps = ticks_per_second.load(Ordering::Relaxed);

            // Find the current app by AppId
            let current_app = system_info
                .current_app_id
                .and_then(|app_id| apps_guard.iter().find(|a| a.id() == app_id))
                .or_else(|| apps_guard.first());

            match current_app {
                Some(app) => {
                    let render_ctx = RenderContext {
                        state: RefCell::new(&mut *state),
                        display: RefCell::new(&mut *frame_buffer),
                        engine: engine_ref,
                    };
                    if let Err(e) = app.render(&render_ctx).await {
                        error!("Error rendering app '{:?}': {:?}", app.id(), e);
                    }
                }
                None => {
                    warn!("No app available to render");
                }
            }
        }
        rendered_buffer.signal(frame_buffer);
        frame_buffer = free_buffer.wait().await;
        tps_counter.increment(&ticks_per_second);
    }
}
