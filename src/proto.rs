#![allow(non_camel_case_types)]
#![allow(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    clippy::allow_attributes_without_reason
)]
pub mod transit_realtime {
    include!(concat!(env!("OUT_DIR"), "/transit_realtime.mod.rs"));
}
pub mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));
}
