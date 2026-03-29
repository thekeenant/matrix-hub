#![no_std]
#![no_main]
#![allow(clippy::uninlined_format_args)]
#![allow(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};

use embassy_executor::Spawner;
use embassy_net::DhcpConfig;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    gpio::{Input, InputConfig, Pin, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    interrupt::{Priority, software::SoftwareInterruptControl},
    rng::Rng,
    system::CpuControl,
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_radio::wifi::WifiMode;
use esp_rtos::embassy::{Executor, InterruptExecutor};
use log::info;
use matrix_hub::{
    app_rotation::AppRotationSignal,
    apps::App,
    metrics::RenderMetrics,
    mk_static,
    nvs::{CORE_1_PAUSED, Kvs},
    proto::app_state::{
        AppId, AppRotationConfig, Config, KeyValueStorage, MatrixHubState, MtaConfig,
        StationConfig, WifiConfig, app_id, key_value_storage,
    },
    tasks::{
        FrameBufferExchange,
        accelerometer::accelerometer_task,
        app_controller::app_controller_task,
        button_monitor::button_monitor_task,
        config_save::config_save_task,
        display::display_task,
        hub75::{FrameBuffer, Hub75Brightness, Hub75Peripherals, hub75_task},
        sntp::sntp_task,
        wait_for_flash_busy::wait_for_flash_busy_task,
        wifi_connection::wifi_connection_task,
        wifi_net::wifi_net_task,
    },
};
use reqwless::client::HttpClient;
use static_cell::StaticCell;

unsafe extern "C" {
    static _stack_end_cpu0: u32;
    static _stack_start_cpu0: u32;
}

static WRITE_BUFFER: StaticCell<FrameBuffer> = StaticCell::new();
static READ_BUFFER: StaticCell<FrameBuffer> = StaticCell::new();

esp_bootloader_esp_idf::esp_app_desc!();
#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger(log::LevelFilter::Info);
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    info!("Main starting!");
    info!("main: stack size:  {}", unsafe {
        core::ptr::addr_of!(_stack_start_cpu0).offset_from(core::ptr::addr_of!(_stack_end_cpu0))
    });

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram);

    let sw_ints = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let software_interrupt = sw_ints.software_interrupt2;
    let timg0 = TimerGroup::new(peripherals.TIMG0);

    info!("init embassy");
    esp_rtos::start(timg0.timer0);

    info!("init radio");
    let radio_init = mk_static!(
        esp_radio::Controller<'static>,
        esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller")
    );
    let (mut wifi_controller, interfaces) =
        esp_radio::wifi::new(radio_init, peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");
    wifi_controller
        .set_mode(WifiMode::Sta)
        .expect("Failed to set Wi-Fi mode");
    wifi_controller
        .start()
        .expect("Failed to start Wi-Fi controller");

    info!("init shared http client, network stack, and wifi runner");
    let (http_client, network_stack, runner) = {
        let rng = Rng::new();
        let net_seed = rng.random() as u64 | ((rng.random() as u64) << 32);
        let tls_seed = rng.random() as u64 | ((rng.random() as u64) << 32);
        let dhcp_config = DhcpConfig::default();
        let config = embassy_net::Config::dhcpv4(dhcp_config);
        let (stack, runner) = embassy_net::new(
            interfaces.sta,
            config,
            mk_static!(
                embassy_net::StackResources<8>,
                embassy_net::StackResources::<8>::new()
            ),
            net_seed,
        );
        let stack = mk_static!(embassy_net::Stack<'static>, stack);
        let dns = mk_static!(
            embassy_net::dns::DnsSocket<'static>,
            embassy_net::dns::DnsSocket::new(*stack)
        );
        let tcp_state = mk_static!(embassy_net::tcp::client::TcpClientState<2, 4096, 4096>, embassy_net::tcp::client::TcpClientState::<2, 4096, 4096>::new());
        let tcp = mk_static!(
            embassy_net::tcp::client::TcpClient<'static, 2, 4096, 4096>,
            embassy_net::tcp::client::TcpClient::new(*stack, tcp_state)
        );
        let rx_buffer = Box::leak(alloc::vec![0u8; 64 * 1024].into_boxed_slice());
        let tx_buffer = Box::leak(alloc::vec![0u8; 64 * 1024].into_boxed_slice());
        let tls = reqwless::client::TlsConfig::new(
            tls_seed,
            rx_buffer,
            tx_buffer,
            reqwless::client::TlsVerify::None,
        );
        (
            Arc::new(Mutex::new(HttpClient::new_with_tls(tcp, dns, tls))),
            *stack,
            runner,
        )
    };

    info!("init matrix hub state");
    let matrix_hub_state = Arc::new(Mutex::new(MatrixHubState::default()));

    info!("init wifi connection task");
    spawner
        .spawn(wifi_connection_task(
            wifi_controller,
            matrix_hub_state.clone(),
        ))
        .expect("Failed to spawn wifi_connection_task");

    info!("init wifi net task");
    spawner
        .spawn(wifi_net_task(runner))
        .expect("Failed to spawn wifi_net_task");

    info!("init sntp task");
    spawner
        .spawn(sntp_task(network_stack, http_client.clone()))
        .expect("Failed to spawn sntp_task");

    info!("init app rotation signal");
    let app_rotation_signal = mk_static!(AppRotationSignal, embassy_sync::signal::Signal::new());

    info!("init http server task");
    spawner
        .spawn(matrix_hub::tasks::http_server::http_server_task(
            network_stack,
            matrix_hub_state.clone(),
            app_rotation_signal,
        ))
        .expect("Failed to spawn http_server_task");

    info!("init framebuffer exchange");
    static RENDERED_BUFFER: FrameBufferExchange<FrameBuffer> = FrameBufferExchange::new();
    static FREE_BUFFER: FrameBufferExchange<FrameBuffer> = FrameBufferExchange::new();

    info!("init framebuffers");
    let write_buffer = {
        let buffer = WRITE_BUFFER.init(unsafe { core::mem::zeroed() });
        buffer.format();
        buffer
    };
    let read_buffer = {
        let buffer = READ_BUFFER.init(unsafe { core::mem::zeroed() });
        buffer.format();
        buffer
    };
    info!("read_buffer addr: {:x}", read_buffer as *const _ as usize);

    info!("init KVS");
    let default_config = Config {
        wifi: Some(WifiConfig {
            ssid: env!("WIFI_SSID").into(),
            password: env!("WIFI_PASSWORD").into(),
        }),
        mta: Some(MtaConfig {
            stations: alloc::vec![
                StationConfig {
                    route: String::from("L"),
                    station_id: String::from("L10"),
                },
                StationConfig {
                    route: String::from("G"),
                    station_id: String::from("G29"),
                },
            ],
        }),
        app_rotation: Some(AppRotationConfig {
            enabled_apps: alloc::vec![
                AppId {
                    id: Some(app_id::Id::Mta(app_id::Mta {}))
                },
                AppId {
                    id: Some(app_id::Id::Plasma(app_id::Plasma {}))
                },
                AppId {
                    id: Some(app_id::Id::Sandbox(app_id::Sandbox {}))
                },
            ],
        }),
    };
    let fallback_kvs = {
        KeyValueStorage {
            entries: alloc::vec![key_value_storage::Entry {
                key: key_value_storage::Key::Config as i32,
                value: Some(key_value_storage::Value {
                    value_oneof: Some(key_value_storage::value::ValueOneof::Config(
                        default_config.clone()
                    )),
                }),
            }],
        }
    };
    let kvs = Arc::new(Mutex::<CriticalSectionRawMutex, _>::new(Kvs::open(
        esp_storage::FlashStorage::new(peripherals.FLASH),
        CpuControl::new(peripherals.CPU_CTRL),
        fallback_kvs,
    )));

    info!("init matrix hub state config from KVS");
    {
        let kvs_lock = kvs.lock().await;
        // This is probably horribly written and not efficient, but it works.
        // We want to fall back to the default config, no matter what.
        let config = kvs_lock
            .get(key_value_storage::Key::Config)
            .map(|v| v.value_oneof.clone())
            .flatten()
            .map(|value_oneof| match value_oneof {
                key_value_storage::value::ValueOneof::Config(c) => Some(c),
                _ => None,
            })
            .flatten()
            .unwrap_or(default_config.clone());
        matrix_hub_state.lock().await.config = Some(config);
        info!("Loaded config from KVS into matrix hub state");
    }

    info!("init config save task");
    spawner
        .spawn(config_save_task(matrix_hub_state.clone(), kvs.clone()))
        .expect("Failed to spawn config_save_task");

    info!("init apps vec");
    let apps: &'static Mutex<CriticalSectionRawMutex, Vec<Arc<dyn App>>> = mk_static!(
        Mutex<CriticalSectionRawMutex, Vec<Arc<dyn App>>>,
        Mutex::new(Vec::new())
    );

    info!("init I2C for accelerometer");
    let i2c = I2c::new(
        peripherals.I2C0,
        I2cConfig::default().with_frequency(Rate::from_hz(100_000)),
    )
    .expect("Failed to initialize I2C")
    .with_sda(peripherals.GPIO16)
    .with_scl(peripherals.GPIO17)
    .into_async();

    info!("init LIS3DH accelerometer");
    spawner
        .spawn(accelerometer_task(i2c, matrix_hub_state.clone()))
        .expect("Failed to spawn accelerometer_task");

    info!("init metrics");
    let render_metrics = RenderMetrics::new();

    info!("init buttons");
    let button_up = Input::new(
        peripherals.GPIO6,
        InputConfig::default().with_pull(Pull::Up),
    );
    let button_down = Input::new(
        peripherals.GPIO7,
        InputConfig::default().with_pull(Pull::Up),
    );
    spawner
        .spawn(button_monitor_task(
            button_up,
            button_down,
            app_rotation_signal,
        ))
        .expect("Failed to spawn button_monitor_task");

    info!("init hub75 brightness");
    let hub75_target_hz: Arc<Hub75Brightness> = Arc::new(Hub75Brightness::new(1.0)); // 50% brightness

    let hub75_peripherals = Hub75Peripherals {
        lcd_cam: peripherals.LCD_CAM,
        dma_channel: peripherals.DMA_CH0,
        red1: peripherals.GPIO42.degrade(),
        grn1: peripherals.GPIO40.degrade(),
        blu1: peripherals.GPIO41.degrade(),
        red2: peripherals.GPIO38.degrade(),
        grn2: peripherals.GPIO37.degrade(),
        blu2: peripherals.GPIO39.degrade(),
        addr0: peripherals.GPIO45.degrade(),
        addr1: peripherals.GPIO36.degrade(),
        addr2: peripherals.GPIO48.degrade(),
        addr3: peripherals.GPIO35.degrade(),
        addr4: peripherals.GPIO21.degrade(),
        blank: peripherals.GPIO14.degrade(),
        clock: peripherals.GPIO2.degrade(),
        latch: peripherals.GPIO47.degrade(),
    };
    let engine: &'static mut rhai::Engine = mk_static!(rhai::Engine, rhai::Engine::new_raw());
    engine.register_fn("print", |s: &str| {
        info!("[rhai] {}", s);
    });

    // Register framebuffer functions for fast direct access from scripts
    engine.register_fn("set_pixel", |x: i64, y: i64, color: i64| {
        matrix_hub::apps::framebuffer_api::set_pixel(x, y, color);
    });
    engine.register_fn("clear", |color: i64| {
        matrix_hub::apps::framebuffer_api::clear(color);
    });
    engine.register_fn("fb_width", || matrix_hub::apps::framebuffer_api::width());
    engine.register_fn("fb_height", || matrix_hub::apps::framebuffer_api::height());
    let engine_ref: &'static Mutex<CriticalSectionRawMutex, &'static rhai::Engine> = mk_static!(
        Mutex<CriticalSectionRawMutex, &'static rhai::Engine>,
        Mutex::new(engine)
    );

    let cpu_ctrl = unsafe { esp_hal::peripherals::CPU_CTRL::<'static>::steal() };
    esp_rtos::start_second_core(
        cpu_ctrl,
        sw_ints.software_interrupt0,
        sw_ints.software_interrupt1,
        mk_static!(
            esp_hal::system::Stack<{ 8 * 1024 }>,
            esp_hal::system::Stack::new()
        ),
        {
            CORE_1_PAUSED.store(false, core::sync::atomic::Ordering::Release);

            let matrix_hub_state = matrix_hub_state.clone();
            let hub75_target_hz = hub75_target_hz.clone();
            let engine_ref = engine_ref;
            move || {
                // High priority (+interrupt) Hub75 task for high FPS.
                let high_prio_executor = mk_static!(
                    InterruptExecutor<2>,
                    InterruptExecutor::new(software_interrupt)
                );
                let high_prio_spawner = high_prio_executor.start(Priority::Priority3);
                info!("init hub75 task");
                high_prio_spawner
                    .spawn(hub75_task(
                        hub75_peripherals,
                        &RENDERED_BUFFER,
                        &FREE_BUFFER,
                        read_buffer,
                        render_metrics.frames_per_second.clone(),
                        hub75_target_hz.clone(),
                    ))
                    .expect("Failed to spawn hub75_task");
                let low_prio_executor = mk_static!(Executor, Executor::new());
                info!("init display task");

                low_prio_executor.run(|spawner| {
                    spawner
                        .spawn(display_task(
                            &RENDERED_BUFFER,
                            &FREE_BUFFER,
                            write_buffer,
                            apps,
                            matrix_hub_state.clone(),
                            render_metrics.frames_per_second.clone(),
                            render_metrics.ticks_per_second,
                            engine_ref,
                        ))
                        .expect("Failed to spawn display_task");

                    info!("init wait_for_flash_busy task");
                    spawner
                        .spawn(wait_for_flash_busy_task())
                        .expect("Failed to spawn wait_for_flash_busy_task");
                });
            }
        },
    );

    info!("init app controller task");
    spawner
        .spawn(app_controller_task(
            spawner,
            apps,
            matrix_hub_state.clone(),
            http_client.clone(),
            hub75_target_hz.clone(),
            app_rotation_signal,
            engine_ref,
        ))
        .expect("Failed to spawn app_controller_task");

    info!("main: entering idle loop");
    loop {
        Timer::after(Duration::from_secs(3)).await;
    }
}
