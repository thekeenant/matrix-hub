//! Sandbox app - Interactive particle physics controlled by accelerometer

extern crate alloc;

use alloc::{boxed::Box, vec::Vec};

use embedded_graphics::{Drawable, geometry::Point, pixelcolor::Rgb888, prelude::*};
use log::info;
use micromath::F32Ext;

use crate::{
    apps::{App, RenderContext, RunContext},
    proto::app_state::{
        AppId, MatrixHubState, Particle,
        app_id::{Id, Sandbox},
    },
    state::SharedMatrixHubState,
    tasks::hub75::FrameBuffer,
};

const WIDTH: i32 = 128;
const HEIGHT: i32 = 32;
pub const MAX_PARTICLES: usize = 600;

// Spatial grid for fast collision detection
const GRID_SIZE: usize = 4; // Each cell is 4x4 pixels
const GRID_WIDTH: usize = (WIDTH as usize + GRID_SIZE - 1) / GRID_SIZE;
const GRID_HEIGHT: usize = (HEIGHT as usize + GRID_SIZE - 1) / GRID_SIZE;

pub struct SandboxApp {
    state: SharedMatrixHubState,
}

impl SandboxApp {
    pub fn new(state: SharedMatrixHubState) -> Self {
        Self { state }
    }
}

#[async_trait::async_trait(?Send)]
impl App for SandboxApp {
    fn build(state: &SharedMatrixHubState, _: AppId) -> Self {
        Self::new(state.clone())
    }

    async fn run(&self, _ctx: &RunContext) -> anyhow::Result<()> {
        {
            let mut state = self.state.lock().await;
            let sandbox = state.sandbox.get_or_insert_default();

            // Always ensure we have MAX_PARTICLES
            if sandbox.particles.len() < MAX_PARTICLES {
                info!(
                    "Initializing particles: current={}, target={}",
                    sandbox.particles.len(),
                    MAX_PARTICLES
                );
                sandbox.particles.clear();
                let mut particles = Vec::new();
                for _ in 0..MAX_PARTICLES {
                    particles.push(Particle {
                        active: false,
                        x: 0.0,
                        y: 0.0,
                        vx: 0.0,
                        vy: 0.0,
                        lifetime: 0,
                        color_r: 0,
                        color_g: 0,
                        color_b: 0,
                    });
                }
                sandbox.particles = particles;
            }

            info!("Initializing {} particles", sandbox.particles.len());
            for (i, particle) in sandbox.particles.iter_mut().enumerate() {
                particle.active = true;
                let px = (i % WIDTH as usize) as f32;
                let py = (i / WIDTH as usize) as f32;
                particle.x = px + ((i * 17) % 100) as f32 * 0.01;
                particle.y = py + ((i * 31) % 100) as f32 * 0.01;
                particle.vx = 0.0;
                particle.vy = 0.0;

                let hue = (i as f32 * 0.05) % 1.0;
                let (r, g, b) = hsv_to_rgb(hue, 0.8, 1.0);
                particle.color_r = r as u32;
                particle.color_g = g as u32;
                particle.color_b = b as u32;
            }
            info!("Particles initialized!");
        }
        core::future::pending::<()>().await;
        Ok(())
    }

    fn id(&self) -> AppId {
        AppId {
            id: Some(Id::Sandbox(Sandbox {})),
        }
    }

    async fn render(&self, ctx: &RenderContext<'_>) -> anyhow::Result<()> {
        let mut state_ref = ctx.state.borrow_mut();
        let state: &mut MatrixHubState = &mut *state_ref;
        let mut display_ref = ctx.display.borrow_mut();
        let display: &mut FrameBuffer = &mut *display_ref;

        let sandbox = state.sandbox.get_or_insert_default();

        // Physics update - get accelerometer data from system_info
        let accel = state
            .system_info
            .get_or_insert_default()
            .accelerometer
            .get_or_insert_default();

        for particle in &mut sandbox.particles {
            if !particle.active {
                continue;
            }

            // Apply gravity and damping
            particle.vx = (particle.vx + accel.accel_x as f32 * 0.15) * 0.99;
            particle.vy = (particle.vy + accel.accel_y as f32 * 0.15) * 0.99;

            // Update position
            particle.x += particle.vx;
            particle.y += particle.vy;

            // Bounce off walls
            if particle.x < 0.0 {
                particle.x = 0.0;
                particle.vx = -particle.vx * 0.8;
            } else if particle.x >= WIDTH as f32 {
                particle.x = (WIDTH - 1) as f32;
                particle.vx = -particle.vx * 0.8;
            }

            if particle.y < 0.0 {
                particle.y = 0.0;
                particle.vy = -particle.vy * 0.8;
            } else if particle.y >= HEIGHT as f32 {
                particle.y = (HEIGHT - 1) as f32;
                particle.vy = -particle.vy * 0.8;
            }
        }

        // Fast spatial grid collision detection
        let mut grid: Vec<Vec<usize>> = Vec::with_capacity(GRID_WIDTH * GRID_HEIGHT);
        grid.resize_with(GRID_WIDTH * GRID_HEIGHT, Vec::new);

        // Assign particles to grid cells
        for (i, particle) in sandbox.particles.iter().enumerate() {
            if !particle.active {
                continue;
            }
            let gx = (particle.x as usize / GRID_SIZE).min(GRID_WIDTH - 1);
            let gy = (particle.y as usize / GRID_SIZE).min(GRID_HEIGHT - 1);
            grid[gy * GRID_WIDTH + gx].push(i);
        }

        // Check collisions only within and between adjacent cells
        for gy in 0..GRID_HEIGHT {
            for gx in 0..GRID_WIDTH {
                let cell_idx = gy * GRID_WIDTH + gx;
                let cell = &grid[cell_idx];

                // Check within this cell
                for i in 0..cell.len() {
                    for j in (i + 1)..cell.len() {
                        check_collision(&mut sandbox.particles, cell[i], cell[j]);
                    }
                }

                // Check with right neighbor
                if gx + 1 < GRID_WIDTH {
                    let right = &grid[gy * GRID_WIDTH + gx + 1];
                    for &i in cell {
                        for &j in right {
                            check_collision(&mut sandbox.particles, i, j);
                        }
                    }
                }

                // Check with bottom neighbor
                if gy + 1 < GRID_HEIGHT {
                    let bottom = &grid[(gy + 1) * GRID_WIDTH + gx];
                    for &i in cell {
                        for &j in bottom {
                            check_collision(&mut sandbox.particles, i, j);
                        }
                    }
                }

                // Check with bottom-right neighbor
                if gx + 1 < GRID_WIDTH && gy + 1 < GRID_HEIGHT {
                    let diag = &grid[(gy + 1) * GRID_WIDTH + gx + 1];
                    for &i in cell {
                        for &j in diag {
                            check_collision(&mut sandbox.particles, i, j);
                        }
                    }
                }
            }
        }

        // Clear and render
        display.clear(Rgb888::BLACK)?;
        for particle in &sandbox.particles {
            if !particle.active {
                continue;
            }
            let px = particle.x as i32;
            let py = particle.y as i32;
            if px >= 0 && px < WIDTH && py >= 0 && py < HEIGHT {
                let color = Rgb888::new(
                    particle.color_r as u8,
                    particle.color_g as u8,
                    particle.color_b as u8,
                );
                Pixel(Point::new(px, py), color).draw(display)?;
            }
        }

        Ok(())
    }
}

#[inline]
fn check_collision(particles: &mut [Particle], i: usize, j: usize) {
    let dx = particles[j].x - particles[i].x;
    let dy = particles[j].y - particles[i].y;
    let dist_sq = dx * dx + dy * dy;

    if dist_sq < 1.0 && dist_sq > 0.0001 {
        let dist = dist_sq.sqrt();
        let overlap = 1.0 - dist;
        let inv_dist = 1.0 / dist;
        let nx = dx * inv_dist;
        let ny = dy * inv_dist;
        let push = overlap * 0.5;

        particles[i].x -= nx * push;
        particles[i].y -= ny * push;
        particles[j].x += nx * push;
        particles[j].y += ny * push;

        particles[i].vx -= nx * 0.1;
        particles[i].vy -= ny * 0.1;
        particles[j].vx += nx * 0.1;
        particles[j].vy += ny * 0.1;
    }
}

/// Convert HSV to RGB
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let h_prime = (h * 6.0) % 6.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let m = v - c;

    let (r, g, b) = if h_prime < 1.0 {
        (c, x, 0.0)
    } else if h_prime < 2.0 {
        (x, c, 0.0)
    } else if h_prime < 3.0 {
        (0.0, c, x)
    } else if h_prime < 4.0 {
        (0.0, x, c)
    } else if h_prime < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}
