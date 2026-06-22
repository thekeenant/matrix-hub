use anyhow::{Context, Result};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::{PinDriver, Pull};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::EspWifi;
use log::info;
use std::sync::mpsc;

mod apps;
mod buffer;
mod config;
mod display;
mod fonts;
mod input;
mod network;
pub mod proto;
pub mod storage;
mod task;

use apps::manager::AppManager;
use apps::mta::MtaApp;

use apps::particles::ParticleApp;
use apps::plasma::PlasmaApp;
use buffer::Framebuffer;
use config::*;
use display::MatrixDisplay;
use input::Button;

use embedded_graphics::prelude::*;

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("Starting Matrix Hub v1 with Error Bubbling...");

    // Pre-allocate the background worker thread stack (16KB) early
    // to avoid OOM fragmentation failures later when rotating apps
    crate::task::init();

    let peripherals = Peripherals::take()
        .map_err(|_| anyhow::anyhow!("Failed to take peripherals"))?;
    let sysloop = EspSystemEventLoop::take()
        .map_err(|_| anyhow::anyhow!("Failed to take sysloop"))?;
    let nvs = EspDefaultNvsPartition::take()
        .map_err(|_| anyhow::anyhow!("Failed to take nvs"))?;

    crate::storage::init_config(nvs.clone());

    let btn_driver = PinDriver::input(peripherals.pins.gpio6, Pull::Up)?;
    let mut btn_up = Button::new(btn_driver);

    let btn_down_driver = PinDriver::input(peripherals.pins.gpio7, Pull::Up)?;
    let mut btn_down = Button::new(btn_down_driver);

    let (tx, rx) = mpsc::sync_channel::<Box<Framebuffer>>(2);
    let (return_tx, return_rx) = mpsc::sync_channel::<Box<Framebuffer>>(2);
    let display_return_tx = return_tx.clone();

    // We use a channel to block the main thread until the display is fully initialized
    let (display_ready_tx, display_ready_rx) = mpsc::sync_channel::<()>(1);

    // =========================================================================
    // Core 1: The Display Actor
    // =========================================================================
    let core1_config =
        esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration {
            pin_to_core: Some(esp_idf_svc::hal::cpu::Core::Core1),
            stack_size: 8192,
            priority: 20, // VERY high priority so display never stutters
            ..Default::default()
        };
    core1_config
        .set()
        .map_err(|e| anyhow::anyhow!("Failed to set core 1 config: {:?}", e))?;

    std::thread::spawn(move || {
        info!(
            "Display Actor started on {:?}",
            esp_idf_svc::hal::cpu::core()
        );

        // If the hardware fails to initialize, we WANT a panic so the ESP32 bootloops!
        let mut display = MatrixDisplay::new(display::MatrixConfig {
            width: u16::try_from(WIDTH as i32).unwrap_or(0),
            height: u16::try_from(HEIGHT as i32).unwrap_or(0),
            r1: PIN_R1,
            g1: PIN_G1,
            b1: PIN_B1,
            r2: PIN_R2,
            g2: PIN_G2,
            b2: PIN_B2,
            a: PIN_A,
            b: PIN_B,
            c: PIN_C,
            d: PIN_D,
            e: PIN_E,
            lat: PIN_LAT,
            oe: PIN_OE,
            clk: PIN_CLK,
        })
        .unwrap_or_else(|_| {
            panic!("CRITICAL: Failed to initialize Matrix Display!")
        });

        info!("Display hardware ready!");

        // Signal the main thread that DMA memory is safely allocated!
        let _ = display_ready_tx.send(());

        let mut current_brightness = crate::storage::global_config()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .brightness as u8;
        display.set_brightness(current_brightness);

        while let Ok(framebuffer) = rx.recv() {
            let new_brightness = crate::storage::global_config()
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .brightness as u8;
            if new_brightness != current_brightness {
                current_brightness = new_brightness;
                display.set_brightness(current_brightness);
            }

            display.clear_display();

            let width = WIDTH as usize;
            let iter = framebuffer.pixels.iter().enumerate().filter_map(
                |(i, &color)| {
                    if color.r() == 0 && color.g() == 0 && color.b() == 0 {
                        None
                    } else {
                        let x = (i % width) as i32;
                        let y = (i / width) as i32;
                        Some(Pixel(Point::new(x, y), color))
                    }
                },
            );

            let _ = display.draw_iter(iter);
            display.flip();
            let _ = display_return_tx.try_send(framebuffer);
        }
    });

    // Reset thread config back to default BEFORE waiting, just in case
    esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration::default()
        .set()
        .map_err(|e| {
            anyhow::anyhow!("Failed to reset thread config: {:?}", e)
        })?;

    // Block until the display has successfully grabbed its 114KB of contiguous DMA memory
    info!("Waiting for Display to allocate DMA memory...");
    display_ready_rx.recv().unwrap_or_else(|_| {
        panic!("Display thread crashed before signaling ready!")
    });

    // =========================================================================
    // Core 0: WiFi Initialization (Delayed until after Display)
    // =========================================================================
    info!("Display initialized. Now starting WiFi...");
    let mut wifi = EspWifi::new(peripherals.modem, sysloop, Some(nvs.clone()))
        .map_err(|e| anyhow::anyhow!("Failed to create EspWifi: {:?}", e))?;

    network::wifi::connect_wifi(&mut wifi)
        .context("Failed to connect to WiFi")?;

    let (wifi_tx, wifi_rx) = std::sync::mpsc::channel();

    // Start Captive Portal DNS Server
    network::dns::start_dns_server();

    let _http_server = network::server::start_server(wifi_tx)
        .context("Failed to start HTTP server")?;

    // =========================================================================
    // Core 0: The Logic/Async Actor
    // =========================================================================

    let i2c = peripherals.i2c0;
    let sda = peripherals.pins.gpio16;
    let scl = peripherals.pins.gpio17;
    let i2c_config = esp_idf_svc::hal::i2c::I2cConfig::new()
        .baudrate(esp_idf_svc::hal::units::FromValueType::kHz(400).into());
    let i2c_driver =
        esp_idf_svc::hal::i2c::I2cDriver::new(i2c, sda, scl, &i2c_config)
            .unwrap_or_else(|e| {
                panic!("CRITICAL: Failed to create I2C driver! {:?}", e)
            });

    let mut gesture_detector = Some(
        input::GestureDetector::new(i2c_driver)
            .unwrap_or_else(|e| panic!("CRITICAL: Failed to initialize LIS3DH Accelerometer! Check I2C address or wiring. {:?}", e))
    );

    // Boost the main task's priority so it preempts background network tasks,
    // avoiding the memory overhead of spawning an extra thread.
    #[allow(
        unsafe_code,
        reason = "ESP-IDF FFI required to change task priority"
    )]
    unsafe {
        esp_idf_svc::sys::vTaskPrioritySet(std::ptr::null_mut(), 15);
    }

    let _ = return_tx.send(Box::new(Framebuffer::new()));
    let _ = return_tx.send(Box::new(Framebuffer::new()));

    block_on(async {
        info!("Logic Actor started on {:?}", esp_idf_svc::hal::cpu::core());

        let mut app_manager = AppManager::new(vec![
            || Box::new(PlasmaApp::new()),
            || Box::new(ParticleApp::new()),
            || Box::new(MtaApp::new()),
            || Box::new(crate::apps::settings::SettingsApp::new()),
        ]);

        let mut is_connected = false;
        let mut ip_str: Option<String> = None;
        let mut wifi_check_countdown = 0u32;
        let mut wifi_reconnect_countdown: Option<u32> = None;

        loop {
            if btn_up.is_clicked() {
                app_manager.next_app();
            }
            if btn_down.is_clicked() {
                app_manager.previous_app();
            }

            if let Some(gd) = &mut gesture_detector {
                match gd.poll() {
                    input::GestureEvent::SwipeRight => app_manager.next_app(),
                    input::GestureEvent::SwipeLeft => {
                        app_manager.previous_app()
                    }
                    input::GestureEvent::Tilting(tilt) => {
                        // Smoothly ease the tilt value in AppManager
                        app_manager.current_tilt =
                            app_manager.current_tilt * 0.5 + tilt * 0.5;
                    }
                    input::GestureEvent::None => {}
                }
            }

            // Check for new credentials from HTTP server
            if let Ok((_new_ssid, _new_pass)) = wifi_rx.try_recv() {
                log::info!("Received new credentials! Scheduling reconnect in 1s to allow HTTP response to flush...");
                wifi_reconnect_countdown = Some(60); // 60 frames = ~1 second
            }

            if let Some(count) = wifi_reconnect_countdown {
                if count == 0 {
                    log::info!("Reconnecting WiFi now...");
                    let _ = network::wifi::connect_wifi(&mut wifi);
                    is_connected = false;
                    ip_str = None;
                    wifi_reconnect_countdown = None;
                } else {
                    wifi_reconnect_countdown = Some(count - 1);
                }
            }

            // Re-check WiFi IP status once per second (~every 60 frames) to avoid
            // contending with the background HTTP thread on the ESP-IDF netif lock.
            if wifi_check_countdown == 0 {
                let sta_result = wifi.sta_netif().get_ip_info();
                match sta_result {
                    Ok(info) if info.ip.to_string() != "0.0.0.0" => {
                        // Switch to Client mode to drop the AP once connected
                        if !is_connected {
                            if let Ok(
                                esp_idf_svc::wifi::Configuration::Mixed(
                                    client_cfg,
                                    _,
                                ),
                            ) = wifi.get_configuration()
                            {
                                let _ = wifi.set_configuration(
                                    &esp_idf_svc::wifi::Configuration::Client(
                                        client_cfg,
                                    ),
                                );
                                log::info!(
                                    "Switched to Client-only mode, AP dropped."
                                );
                            }
                        }
                        is_connected = true;
                        ip_str = Some(info.ip.to_string());
                    }
                    _ => {
                        is_connected = false;
                        if let Ok(ap_info) = wifi.ap_netif().get_ip_info() {
                            ip_str = Some(format!("AP: {}", ap_info.ip));
                        } else {
                            ip_str = None;
                        }
                    }
                }
                wifi_check_countdown = 60;
            }
            wifi_check_countdown -= 1;

            let mut fb = return_rx
                .try_recv()
                .unwrap_or_else(|_| Box::new(Framebuffer::new()));
            fb.clear();

            let accel_data = gesture_detector.as_ref().map(|gd| gd.last_accel);
            app_manager.update(16.0, is_connected, ip_str.clone(), accel_data);
            app_manager.draw(&mut fb, is_connected);

            // Using try_send is inherently safe, it just drops the frame if the receiver is full
            let _ = tx.try_send(fb);

            embassy_time::Timer::after_millis(16).await;
        }
    });

    Ok(())
}
