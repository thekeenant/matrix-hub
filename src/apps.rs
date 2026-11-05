pub mod manager;
pub mod mta;
pub mod particles;
pub mod plasma;
pub mod settings;

use crate::buffer::Framebuffer;
use crate::input::Accel;
use crate::network::NetworkStatus;

/// Everything an app may need to know to advance one frame.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// Time since the previous frame, in milliseconds
    pub dt_ms: f32,
    pub network: NetworkStatus,
    pub accel: Accel,
}

/// A full-screen app. Only the active app exists: switching apps drops the
/// old one and constructs the new one.
pub trait App {
    /// Advance state by one frame.
    fn update(&mut self, frame: &Frame);

    /// Draw the current state into a cleared framebuffer.
    fn draw(&self, fb: &mut Framebuffer);
}
