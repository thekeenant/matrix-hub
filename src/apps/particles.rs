use crate::apps::App;
use crate::buffer::Framebuffer;
use crate::config::{HEIGHT, WIDTH};
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle};
use rand::Rng;

const NUM_PARTICLES: usize = 120;
const GRAVITY: f32 = 0.15;

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

        Self { particles }
    }
}

impl App for ParticleApp {
    fn update(&mut self, _dt_ms: f32) {
        let mut rng = rand::thread_rng();

        for p in &mut self.particles {
            p.vy += GRAVITY;
            p.x += p.vx;
            p.y += p.vy;

            if p.x <= 0.0 || p.x >= WIDTH - 2.0 {
                p.vx = -p.vx * 0.7;
                p.x = p.x.clamp(0.0, WIDTH - 2.0);
            }

            if p.y >= HEIGHT - 2.0 {
                p.vy = -p.vy * 0.7;
                p.y = HEIGHT - 2.0;
                p.vx += rng.gen_range(-0.2..0.2);

                if p.vy.abs() < 1.0 {
                    p.vy = rng.gen_range(-4.0..-2.0);
                    p.vx = rng.gen_range(-2.0..2.0);
                }
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
