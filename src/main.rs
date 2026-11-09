//! Matrix Hub: an ESP32-S3 smart display for a HUB75 LED matrix.
//!
//! Threads:
//! - **Display** (core 1, high priority): pushes finished frames to the panel.
//! - **Main loop** (core 0): input, WiFi supervision, app logic, drawing.
//! - **Background worker**: slow periodic jobs such as feed fetches.
//! - ESP-IDF's own WiFi/network tasks, plus the HTTP and DNS servers.

use anyhow::{Context, Result};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::{PinDriver, Pull};
use esp_idf_svc::hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::units::FromValueType;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use log::info;
use std::sync::mpsc;
use std::time::{Duration, Instant};

mod config;
mod diag;
mod display;
mod input;
mod mta;
mod network;
mod platform;
mod storage;
mod task;

use matrix_hub_core::apps::manager::AppManager;
use matrix_hub_core::apps::{self as apps, Frame};
use matrix_hub_core::buffer::{self as buffer, FrameProducer};
use matrix_hub_core::util::Interval;

use diag::{Crumb, Stage};
use input::Input;
use network::wifi::WifiManager;

/// Framebuffers circulating between the main loop and the display
const FRAMEBUFFER_POOL: usize = 3;
/// Target frame period. Sleeps round up to the 10ms FreeRTOS tick, so the
/// loop runs at ~50 fps; apps use the measured frame time.
const FRAME_PERIOD: Duration = Duration::from_millis(16);
/// Longest frame time passed to apps
const MAX_FRAME_DT: Duration = Duration::from_millis(100);
/// Priority of the main task: above network tasks (5), below WiFi (23)
const MAIN_TASK_PRIORITY: u32 = 15;

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    info!("Starting Matrix Hub");

    // Capture breadcrumbs left by the previous boot before anything else runs
    diag::init_boot();

    // Start the background worker early, while memory is unfragmented
    task::init();

    let peripherals =
        Peripherals::take().context("Failed to take peripherals")?;
    let sysloop =
        EspSystemEventLoop::take().context("Failed to take event loop")?;
    let nvs = EspDefaultNvsPartition::take().context("Failed to take NVS")?;

    storage::init_config(nvs.clone());

    // The display grabs its DMA memory before WiFi can claim internal RAM
    let (frames, display_frames) = buffer::frame_pool(FRAMEBUFFER_POOL);
    display::spawn(display_frames)?;

    let mut wifi = WifiManager::start(peripherals.modem, sysloop, nvs)?;
    let (credentials_tx, credentials_rx) = mpsc::channel();
    network::dns::start_dns_server();
    let _http_server = network::server::start_server(credentials_tx)
        .context("Failed to start HTTP server")?;

    let pins = peripherals.pins;
    let i2c = I2cDriver::new(
        peripherals.i2c0,
        pins.gpio16,
        pins.gpio17,
        &I2cConfig::new().baudrate(400.kHz().into()),
    )
    .context("Failed to create I2C driver")?;
    let mut input = Input::new(
        PinDriver::input(pins.gpio6, Pull::Up)?,
        PinDriver::input(pins.gpio7, Pull::Up)?,
        i2c,
    )
    .context("Failed to initialize input (check the LIS3DH wiring)")?;

    // Preempt background network tasks
    platform::set_current_task_priority(MAIN_TASK_PRIORITY);

    let mut apps = AppManager::new(vec![
        || Box::new(apps::mta::MtaApp::new(mta::start_fetching)),
        || Box::new(apps::plasma::PlasmaApp::new()),
        || Box::new(apps::particles::ParticleApp::new()),
        || Box::new(apps::settings::SettingsApp::new(config::AP_SSID)),
    ]);

    info!("Main loop started on {:?}", esp_idf_svc::hal::cpu::core());

    // If this loop ever stops iterating for CONFIG_ESP_TASK_WDT_TIMEOUT_S,
    // the watchdog panics, saves a coredump, and reboots.
    diag::watch_current_thread();

    // A plain sleep paces frames. (This used to await an embassy-time timer,
    // whose esp-idf-svc 0.52 driver arms a ~u64::MAX timeout whenever its
    // queue empties, which overflows once uptime passes 24h: the esp_timer
    // task then spins forever and starves this loop.)
    let mut every_second = Interval::new(Duration::from_secs(1));
    let mut last_frame = Instant::now();
    loop {
        diag::feed_watchdog();
        diag::inc(Crumb::MainFrames);
        diag::maybe_test_hang();

        diag::MAIN.enter(Stage::Input);
        input.poll(|event| apps.handle(event));

        diag::MAIN.enter(Stage::WifiCheck);
        if credentials_rx.try_recv().is_ok() {
            wifi.credentials_changed();
        }
        wifi.poll();
        if every_second.tick() {
            diag::tick_1s();
            wifi.check();
        }

        // Real elapsed time, capped so a long stall (e.g. a WiFi reconfigure)
        // doesn't make animations jump
        let now = Instant::now();
        let dt = now.duration_since(last_frame).min(MAX_FRAME_DT);
        last_frame = now;

        diag::MAIN.enter(Stage::Update);
        apps.update(&Frame {
            dt_ms: dt.as_secs_f32() * 1000.0,
            network: wifi.status(),
            accel: input.accel(),
        });

        render(&apps, &frames);

        diag::MAIN.enter(Stage::Sleep);
        std::thread::sleep(FRAME_PERIOD.saturating_sub(now.elapsed()));
    }
}

/// Draw the current frame into a free buffer and hand it to the display.
fn render(apps: &AppManager, frames: &FrameProducer) {
    // If the display holds every buffer, skip this frame
    let Some(mut frame) = frames.acquire() else {
        return;
    };
    diag::MAIN.enter(Stage::Draw);
    apps.draw(&mut frame);
    diag::MAIN.enter(Stage::Send);
    frames.submit(frame);
}
