//! Panel geometry and network constants.

/// Panel size in pixels
pub const WIDTH: u32 = 128;
pub const HEIGHT: u32 = 32;

/// IP of the setup access point (esp-idf-svc's default AP subnet). The
/// captive portal's DNS answers and HTTP redirects point here.
pub const AP_IP: [u8; 4] = [192, 168, 71, 1];
