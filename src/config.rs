//! Board configuration: pins and compiled-in defaults.

pub use matrix_hub_core::config::{AP_IP, HEIGHT, WIDTH};

/// GPIO numbers of the HUB75 connector.
pub struct PanelPins {
    pub r1: i32,
    pub g1: i32,
    pub b1: i32,
    pub r2: i32,
    pub g2: i32,
    pub b2: i32,
    pub a: i32,
    pub b: i32,
    pub c: i32,
    pub d: i32,
    pub e: i32,
    pub lat: i32,
    pub oe: i32,
    pub clk: i32,
}

/// Adafruit MatrixPortal S3 pinout. G and B are physically swapped on this
/// panel, hence G=40/B=41 and G=37/B=39.
pub const PANEL_PINS: PanelPins = PanelPins {
    r1: 42,
    g1: 40,
    b1: 41,
    r2: 38,
    g2: 37,
    b2: 39,
    a: 45,
    b: 36,
    c: 48,
    d: 35,
    e: 21,
    lat: 47,
    oe: 14,
    clk: 2,
};

// Setup-AP credentials (compiled in from .env)
pub const AP_SSID: &str = dotenvy_macro::dotenv!("AP_SSID");
pub const AP_PASS: &str = dotenvy_macro::dotenv!("AP_PASS");
