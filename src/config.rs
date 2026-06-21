pub const WIDTH: f32 = 128.0;
pub const HEIGHT: f32 = 32.0;

// Adafruit MatrixPortal S3 typical pins
pub const PIN_R1: i32 = 42;
pub const PIN_G1: i32 = 40; // NOTE: G and B are physically swapped on this panel
pub const PIN_B1: i32 = 41;
pub const PIN_R2: i32 = 38;
pub const PIN_G2: i32 = 37; // NOTE: G and B are physically swapped on this panel
pub const PIN_B2: i32 = 39;
pub const PIN_A: i32 = 45;
pub const PIN_B: i32 = 36;
pub const PIN_C: i32 = 48;
pub const PIN_D: i32 = 35;
pub const PIN_E: i32 = 21;
pub const PIN_LAT: i32 = 47;
pub const PIN_OE: i32 = 14;
pub const PIN_CLK: i32 = 2;

// WiFi Credentials (Loaded at compile-time from .env)
pub const AP_SSID: &str = dotenvy_macro::dotenv!("AP_SSID");
pub const AP_PASS: &str = dotenvy_macro::dotenv!("AP_PASS");
