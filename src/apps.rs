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
}
