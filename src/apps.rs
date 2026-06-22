pub mod manager;
pub mod mta;

pub mod particles;
pub mod plasma;
pub mod settings;

use crate::buffer::Framebuffer;

pub trait App {
    /// Update the logic/physics of the app
    fn update(&mut self, dt_ms: f32);

    /// Draw the current state directly to the off-screen framebuffer using embedded-graphics
    fn draw(&self, buffer: &mut Framebuffer);

    /// Optional: Update the network status of the app
    fn set_network_status(&mut self, _is_connected: bool, _ip: Option<String>) {
    }

    /// Optional: Provide raw accelerometer data
    fn set_accelerometer(&mut self, _x: f32, _y: f32, _z: f32) {}
}
