use crate::apps::App;
use crate::buffer::Framebuffer;
use log::info;

pub type AppFactory = fn() -> Box<dyn App>;

pub struct AppManager {
    factories: Vec<AppFactory>,
    current_idx: usize,
    active_app: Box<dyn App>,
    transition_timer_ms: f32,
    transition_direction: i8,
    pub current_tilt: f32,
    pub arrow_fade_ms: f32,
    pub arrow_direction: f32,
    pub time_ms: f32,
}

impl AppManager {
    pub fn new(factories: Vec<AppFactory>) -> Self {
        assert!(
            !factories.is_empty(),
            "Must provide at least one app factory"
        );
        let active_app = factories[0]();
        Self {
            factories,
            current_idx: 0,
            active_app,
            transition_timer_ms: 0.0,
            transition_direction: 1,
            current_tilt: 0.0,
            arrow_fade_ms: 0.0,
            arrow_direction: 1.0,
            time_ms: 0.0,
        }
    }

    pub fn next_app(&mut self) {
        self.current_idx = (self.current_idx + 1) % self.factories.len();
        self.active_app = self.factories[self.current_idx]();
        self.transition_timer_ms = 1000.0;
        self.transition_direction = 1;
        self.current_tilt = 0.0;
        info!("Switched to app index {}", self.current_idx);
    }

    pub fn previous_app(&mut self) {
        if self.current_idx == 0 {
            self.current_idx = self.factories.len() - 1;
        } else {
            self.current_idx -= 1;
        }
        self.active_app = self.factories[self.current_idx]();
        self.transition_timer_ms = 1000.0;
        self.transition_direction = -1;
        self.current_tilt = 0.0;
        info!("Switched to app index {}", self.current_idx);
    }

    pub fn update(
        &mut self,
        dt_ms: f32,
        is_connected: bool,
        ip: Option<String>,
        accel: Option<(f32, f32, f32)>,
    ) {
        self.active_app.set_network_status(is_connected, ip);
        if let Some((x, y, z)) = accel {
            self.active_app.set_accelerometer(x, y, z);
        }
        self.active_app.update(dt_ms);
        if self.transition_timer_ms > 0.0 {
            self.transition_timer_ms -= dt_ms;
        }

        self.time_ms += dt_ms;

        if self.current_tilt.abs() > 0.05 {
            self.arrow_fade_ms += dt_ms;
            self.arrow_direction = self.current_tilt.signum();
        } else {
            self.arrow_fade_ms -= dt_ms;
        }
        self.arrow_fade_ms = self.arrow_fade_ms.clamp(0.0, 70.0);

        // Decay tilt back to 0 fast so it turns off when GestureEvent::None happens
        self.current_tilt *= 0.5;
    }

    pub fn draw(&self, fb: &mut Framebuffer, _is_connected: bool) {
        self.active_app.draw(fb);

        // 1) Draw Swipe Flash Transition
        if self.transition_timer_ms > 0.0 {
            let progress = (self.transition_timer_ms / 1000.0).clamp(0.0, 1.0);
            let max_alpha = progress * 255.0;
            let width = 128i32;
            let height = 32i32;

            for y in 0..height {
                for x in 0..width {
                    let alpha = if self.transition_direction == 1 {
                        let dist_from_right = (width - 1 - x) as f32;
                        let intensity =
                            (1.0 - dist_from_right / 40.0).clamp(0.0, 1.0);
                        intensity * max_alpha
                    } else {
                        let dist_from_left = x as f32;
                        let intensity =
                            (1.0 - dist_from_left / 40.0).clamp(0.0, 1.0);
                        intensity * max_alpha
                    };

                    if alpha > 0.0 {
                        let index = (y * width + x) as usize;
                        let existing = fb.pixels[index];

                        use embedded_graphics::prelude::RgbColor;
                        let r = (existing.r() as f32 + (alpha * 0.0 / 255.0))
                            .clamp(0.0, 255.0)
                            as u8;
                        let g = (existing.g() as f32 + (alpha * 200.0 / 255.0))
                            .clamp(0.0, 255.0)
                            as u8;
                        let b = (existing.b() as f32 + (alpha * 255.0 / 255.0))
                            .clamp(0.0, 255.0)
                            as u8;

                        fb.pixels[index] =
                            embedded_graphics::pixelcolor::Rgb888::new(r, g, b);
                    }
                }
            }
        }

        // 2) Draw Analog Tilt Feedback Arrow
        if self.arrow_fade_ms > 0.0 {
            use embedded_graphics::pixelcolor::Rgb888;
            use embedded_graphics::prelude::*;
            use embedded_graphics::primitives::Triangle;

            let alpha = self.arrow_fade_ms / 70.0;
            // Color of the arrow: Cyan/White that actually fades to black as alpha goes to 0
            let color = Rgb888::new(
                (100.0 * alpha) as u8,
                (255.0 * alpha) as u8,
                (255.0 * alpha) as u8,
            );

            // Bouncy ball curve: abs(cos) creates sharp impacts and smooth apexes
            let bounce_progress = (self.time_ms / 80.0).cos().abs();
            let offset = (bounce_progress * 6.0) as i32;

            use embedded_graphics::primitives::PrimitiveStyleBuilder;
            let style = PrimitiveStyleBuilder::new()
                .fill_color(color)
                .stroke_color(Rgb888::BLACK)
                .stroke_width(1)
                .build();

            if self.arrow_direction > 0.0 {
                // Tilting right, draw arrow on right edge pointing right
                let x = 123 - offset; // Bounce inward

                let _ = Triangle::new(
                    Point::new(x, 12),
                    Point::new(x, 20),
                    Point::new(x + 4, 16),
                )
                .into_styled(style)
                .draw(fb);
            } else {
                // Tilting left, draw arrow on left edge pointing left
                let x = 4 + offset; // Bounce inward

                let _ = Triangle::new(
                    Point::new(x, 12),
                    Point::new(x, 20),
                    Point::new(x - 4, 16),
                )
                .into_styled(style)
                .draw(fb);
            }
        }
    }
}
