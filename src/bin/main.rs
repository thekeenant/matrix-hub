#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig},
};
use esp_println::println;
use log::info;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: embassy_executor::Spawner) {
    println!("Matrix Hub Test - Starting up");
    esp_println::logger::init_logger(log::LevelFilter::Info);
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    info!("Testing HUB75 matrix with direct GPIO control...");
    
    let mut r1 = Output::new(peripherals.GPIO42, Level::Low, OutputConfig::default());
    let mut g1 = Output::new(peripherals.GPIO40, Level::Low, OutputConfig::default());
    let mut b1 = Output::new(peripherals.GPIO41, Level::Low, OutputConfig::default());
    let mut r2 = Output::new(peripherals.GPIO38, Level::Low, OutputConfig::default());
    let mut g2 = Output::new(peripherals.GPIO37, Level::Low, OutputConfig::default());
    let mut b2 = Output::new(peripherals.GPIO39, Level::Low, OutputConfig::default());
    let mut addr0 = Output::new(peripherals.GPIO45, Level::Low, OutputConfig::default());
    let mut addr1 = Output::new(peripherals.GPIO36, Level::Low, OutputConfig::default());
    let mut addr2 = Output::new(peripherals.GPIO48, Level::Low, OutputConfig::default());
    let mut addr3 = Output::new(peripherals.GPIO35, Level::Low, OutputConfig::default());
    let mut addr4 = Output::new(peripherals.GPIO21, Level::Low, OutputConfig::default());
    let mut blank = Output::new(peripherals.GPIO14, Level::High, OutputConfig::default());
    let mut clock = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());
    let mut latch = Output::new(peripherals.GPIO47, Level::Low, OutputConfig::default());

    info!("Starting continuous refresh loop - moving wave pattern");
    
    let mut frame = 0u32;
    loop {
        for row in 0..16 {
            // Set address lines
            addr0.set_level(if row & 0x01 != 0 { Level::High } else { Level::Low });
            addr1.set_level(if row & 0x02 != 0 { Level::High } else { Level::Low });
            addr2.set_level(if row & 0x04 != 0 { Level::High } else { Level::Low });
            addr3.set_level(if row & 0x08 != 0 { Level::High } else { Level::Low });
            addr4.set_level(if row & 0x10 != 0 { Level::High } else { Level::Low });

            // Shift out 64 pixels for this row (covers 128 pixels total with upper/lower halves)
            for col in 0..64 {
                // Create wavy pattern using sine approximation
                // Upper half (rows 0-15)
                let wave_y1 = 8 + (((col * 4 + frame) % 64) as i32 - 32).abs() / 4;
                let is_on_r1 = (row as i32) == (wave_y1 / 2);
                let is_on_g1 = (row as i32) == ((wave_y1 / 2) + 1) % 16;
                let is_on_b1 = (row as i32) == ((wave_y1 / 2) + 2) % 16;
                
                // Lower half (rows 16-31)
                let wave_y2 = 8 + (((col * 4 + frame + 32) % 64) as i32 - 32).abs() / 4;
                let is_on_r2 = (row as i32) == (wave_y2 / 2);
                let is_on_g2 = (row as i32) == ((wave_y2 / 2) + 1) % 16;
                let is_on_b2 = (row as i32) == ((wave_y2 / 2) + 2) % 16;

                r1.set_level(if is_on_r1 { Level::High } else { Level::Low });
                g1.set_level(if is_on_g1 { Level::High } else { Level::Low });
                b1.set_level(if is_on_b1 { Level::High } else { Level::Low });
                r2.set_level(if is_on_r2 { Level::High } else { Level::Low });
                g2.set_level(if is_on_g2 { Level::High } else { Level::Low });
                b2.set_level(if is_on_b2 { Level::High } else { Level::Low });

                // Clock pulse
                clock.set_high();
                clock.set_low();
            }

            // Latch the data
            latch.set_high();
            latch.set_low();

            // Enable output briefly
            blank.set_low();
            for _ in 0..50 { core::hint::spin_loop(); }
            blank.set_high();
        }
        
        frame = frame.wrapping_add(1);
    }
}
