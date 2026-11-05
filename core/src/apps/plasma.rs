use crate::apps::{App, Frame};
use crate::buffer::Framebuffer;
use crate::config::{HEIGHT, WIDTH};
use core::f32::consts::PI;
use embedded_graphics::pixelcolor::Rgb888;

pub struct PlasmaApp {
    time: f32,
}

impl Default for PlasmaApp {
    fn default() -> Self {
        Self::new()
    }
}

impl PlasmaApp {
    pub const fn new() -> Self {
        Self { time: 0.0 }
    }
}

impl App for PlasmaApp {
    fn update(&mut self, frame: &Frame) {
        // Slowly advance time based on delta time
        self.time += frame.dt_ms * 0.005;
    }

    fn draw(&self, buffer: &mut Framebuffer) {
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let xf = x as f32;
                let yf = y as f32;

                // Create intersecting sine waves for dynamic multi-directional movement
                let v1 = xf.mul_add(0.1, self.time * 0.5).sin(); // Moves left slowly
                let v2 = yf.mul_add(0.2, -self.time * 0.8).sin(); // Moves down

                // Diagonal wave moving bottom-right
                let v3 = (xf + yf).mul_add(0.08, -self.time * 1.1).sin();

                // Circular waves from an orbiting center
                let cx = WIDTH as f32 / 2.0 + (self.time * 0.3).cos() * 30.0;
                let cy = HEIGHT as f32 / 2.0 + (self.time * 0.4).sin() * 10.0;
                let dist = (xf - cx).hypot(yf - cy);
                let v4 = (dist * 0.12 - self.time * 1.5).sin();

                // Combine them
                let v = v1 + v2 + v3 + v4; // Range roughly -4.0 to 4.0

                // Map value to Cyberpunk RGB (Cyan, Magenta, Deep Blue, Hot Pink)
                let phase = v * PI / 4.0;

                // Red peaks for pink/magenta, dips for cyan
                let r = phase.sin().mul_add(127.0, 128.0) as u8;

                // Green peaks out of phase for cyan, dips for pink/purple
                let g = (phase + PI / 1.5).sin().mul_add(96.0, 96.0) as u8;

                // Blue stays relatively high across the board for that neon backglow
                let b = (phase * 0.5).sin().mul_add(64.0, 191.0) as u8;

                buffer.pixels[Framebuffer::index(x, y)] = Rgb888::new(r, g, b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Accel;
    use crate::net::NetworkStatus;
    use googletest::prelude::*;

    fn render(app: &PlasmaApp) -> Framebuffer {
        let mut fb = Framebuffer::new();
        app.draw(&mut fb);
        fb
    }

    #[gtest]
    fn fills_every_pixel_and_animates() {
        let mut app = PlasmaApp::default();
        let first = render(&app);
        expect_that!(first.lit_pixels().count(), eq(Framebuffer::LEN));

        app.update(&Frame {
            dt_ms: 100.0,
            network: NetworkStatus::Connecting,
            accel: Accel::default(),
        });
        expect_that!(render(&app).pixels, not(eq(&first.pixels)));
    }
}
