use crate::apps::App;
use crate::buffer::Framebuffer;
use core::f32::consts::PI;
use embedded_graphics::pixelcolor::Rgb888;

pub struct PlasmaApp {
    time: f32,
}

impl PlasmaApp {
    pub const fn new() -> Self {
        Self { time: 0.0 }
    }
}

impl App for PlasmaApp {
    fn update(&mut self, dt_ms: f32) {
        // Slowly advance time based on delta time
        self.time += dt_ms * 0.005;
    }

    fn draw(&self, buffer: &mut Framebuffer) {
        let width = 128;
        let height = 32;

        for y in 0..height {
            for x in 0..width {
                let xf = x as f32;
                let yf = y as f32;

                // Create intersecting sine waves for that classic demo-scene plasma look
                let v1 = xf.mul_add(0.1, self.time).sin();
                let v2 = yf.mul_add(0.2, self.time * 0.8).sin();
                let v3 = (xf + yf).mul_add(0.1, self.time * 1.2).sin();

                let dist =
                    (xf - width as f32 / 2.0).hypot(yf - height as f32 / 2.0);
                let v4 = (dist * 0.15 + self.time * 1.5).sin();

                // Combine them
                let v = v1 + v2 + v3 + v4; // Range roughly -4.0 to 4.0

                // Map value to RGB using phase offsets
                let r = (v * PI / 2.0).sin().mul_add(127.0, 128.0) as u8;
                let g = (v * PI / 2.0 + 2.0 * PI / 3.0)
                    .sin()
                    .mul_add(127.0, 128.0) as u8;
                let b = (v * PI / 2.0 + 4.0 * PI / 3.0)
                    .sin()
                    .mul_add(127.0, 128.0) as u8;

                let index = (y * width + x) as usize;
                buffer.pixels[index] = Rgb888::new(r, g, b);
            }
        }
    }
}
