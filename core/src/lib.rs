//! Hardware-independent logic of Matrix Hub.
//!
//! Everything here builds and runs on the host, so it can be unit tested;
//! the firmware crate wires it to the ESP32-S3 hardware.

pub mod apps;
pub mod buffer;
pub mod config;
pub mod device_config;
pub mod fonts;
pub mod input;
pub mod net;
pub mod proto;
pub mod util;
