use crate::apps::App;
use crate::buffer::Framebuffer;
use crate::config::{HEIGHT, WIDTH};
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle};
use rand::Rng;

const NUM_PARTICLES: usize = 120;

// The Particle struct is now completely private!
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    color: Rgb888,
}

pub struct ParticleApp {
    particles: Vec<Particle>,
    gravity_x: f32,
    gravity_y: f32,
}

impl ParticleApp {
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        let mut particles = Vec::with_capacity(NUM_PARTICLES);

        for _ in 0..NUM_PARTICLES {
            particles.push(Particle {
                x: rng.gen_range(10.0..(WIDTH - 10.0)),
                y: rng.gen_range(0.0..(HEIGHT / 2.0)),
                vx: rng.gen_range(-2.0..2.0),
                vy: rng.gen_range(-1.0..1.0),
                color: Rgb888::new(
                    rng.gen_range(50..255),
                    rng.gen_range(50..255),
                    rng.gen_range(50..255),
                ),
            });
        }

        Self {
            particles,
            gravity_x: 0.0,
            gravity_y: 0.15,
        }
    }
}

impl App for ParticleApp {
    fn set_accelerometer(&mut self, x: f32, y: f32, _z: f32) {
        // Positive X tilt = Left side up (so particles should roll left, negative vx)
        self.gravity_x = -x * 0.25;
        // Add raw Y tilt to default gravity
        self.gravity_y = y * 0.25 + 0.08;
    }

    fn update(&mut self, _dt_ms: f32) {
        let mut rng = rand::thread_rng();

        for p in &mut self.particles {
            p.vx += self.gravity_x;
            p.vy += self.gravity_y;
            p.x += p.vx;
            p.y += p.vy;

            // X-axis collision (left and right walls)
            if p.x <= 0.0 {
                p.x = 0.0;
                p.vx = -p.vx * 0.5; // Bounce
            } else if p.x >= WIDTH - 2.0 {
                p.x = WIDTH - 2.0;
                p.vx = -p.vx * 0.5; // Bounce
            }

            // Y-axis collision (floor and ceiling)
            if p.y >= HEIGHT - 2.0 {
                p.y = HEIGHT - 2.0;
                p.vy = -p.vy * 0.5;
                p.vx += rng.gen_range(-0.1..0.1); // Add some random friction noise

                // If resting on the ground and gravity is mostly downwards, occasionally bounce
                if p.vy.abs() < 1.0 && self.gravity_y > 0.05 {
                    // Small chance to randomly pop up to keep it lively
                    if rng.gen_bool(0.005) {
                        p.vy = rng.gen_range(-2.0..-1.0);
                        p.vx = rng.gen_range(-1.0..1.0);
                    }
                }
            } else if p.y <= 0.0 {
                // Bounce off ceiling if device is upside down
                p.y = 0.0;
                p.vy = -p.vy * 0.5;
            }
        }
    }

    fn draw(&self, buffer: &mut Framebuffer) {
        for p in &self.particles {
            let _ = Circle::new(Point::new(p.x as i32, p.y as i32), 2)
                .into_styled(PrimitiveStyle::with_fill(p.color))
                .draw(buffer);
        }
    }
}
