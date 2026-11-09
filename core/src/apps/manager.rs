//! Owns the active app, switches between apps, and draws the navigation
//! overlays (swipe flash and tilt arrow) on top of it.

use crate::apps::{App, Frame};
use crate::buffer::Framebuffer;
use crate::config::{HEIGHT, WIDTH};
use crate::input::InputEvent;
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyleBuilder, Triangle};
use log::info;

pub type AppFactory = fn() -> Box<dyn App>;

/// Length of the edge flash shown after switching apps
const TRANSITION_MS: f32 = 1000.0;
/// How far the edge flash reaches into the screen, in pixels
const TRANSITION_REACH_PX: f32 = 40.0;
/// Fade-in/out time of the tilt arrow
const ARROW_FADE_MS: f32 = 70.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Left,
    Right,
}

pub struct AppManager {
    factories: Vec<AppFactory>,
    current: usize,
    active: Box<dyn App>,
    /// Remaining time of the edge flash, and which edge it's on
    transition_ms: f32,
    transition_edge: Direction,
    /// Smoothed analog tilt (-1..1), decaying to 0 without input
    tilt: f32,
    arrow_fade_ms: f32,
    arrow_direction: Direction,
    time_ms: f32,
}

impl AppManager {
    /// `factories` construct each app; the first is shown initially.
    pub fn new(factories: Vec<AppFactory>) -> Self {
        assert!(!factories.is_empty(), "Must provide at least one app");
        let active = factories[0]();
        Self {
            factories,
            current: 0,
            active,
            transition_ms: 0.0,
            transition_edge: Direction::Right,
            tilt: 0.0,
            arrow_fade_ms: 0.0,
            arrow_direction: Direction::Right,
            time_ms: 0.0,
        }
    }

    pub fn handle(&mut self, event: InputEvent) {
        let count = self.factories.len();
        match event {
            InputEvent::Next => {
                self.switch_to((self.current + 1) % count, Direction::Right);
            }
            InputEvent::Previous => {
                self.switch_to(
                    (self.current + count - 1) % count,
                    Direction::Left,
                );
            }
            // Ease toward the reported tilt
            InputEvent::Tilt(tilt) => self.tilt = self.tilt * 0.5 + tilt * 0.5,
        }
    }

    fn switch_to(&mut self, index: usize, edge: Direction) {
        self.current = index;
        // Drop the old app first so its resources (e.g. a fetch task) are
        // released before the new one is built
        self.active = (self.factories[index])();
        self.transition_ms = TRANSITION_MS;
        self.transition_edge = edge;
        self.tilt = 0.0;
        info!("Switched to app index {index}");
    }

    pub fn update(&mut self, frame: &Frame) {
        let dt = frame.dt_ms;
        self.active.update(frame);
        self.transition_ms = (self.transition_ms - dt).max(0.0);
        self.time_ms += dt;

        if self.tilt.abs() > 0.05 {
            self.arrow_fade_ms += dt;
            self.arrow_direction = if self.tilt > 0.0 {
                Direction::Right
            } else {
                Direction::Left
            };
        } else {
            self.arrow_fade_ms -= dt;
        }
        self.arrow_fade_ms = self.arrow_fade_ms.clamp(0.0, ARROW_FADE_MS);

        // Decay quickly so the arrow disappears once tilt events stop
        self.tilt *= 0.5;
    }

    pub fn draw(&self, fb: &mut Framebuffer) {
        self.active.draw(fb);
        if self.transition_ms > 0.0 {
            self.draw_transition(fb);
        }
        if self.arrow_fade_ms > 0.0 {
            self.draw_tilt_arrow(fb);
        }
    }

    /// A cyan glow on the edge we swiped toward, fading out over
    /// `TRANSITION_MS`, added on top of the app's pixels.
    fn draw_transition(&self, fb: &mut Framebuffer) {
        let strength = (self.transition_ms / TRANSITION_MS).clamp(0.0, 1.0);
        for x in 0..WIDTH {
            let dist_from_edge = match self.transition_edge {
                Direction::Right => WIDTH - 1 - x,
                Direction::Left => x,
            } as f32;
            let alpha = (1.0 - dist_from_edge / TRANSITION_REACH_PX)
                .clamp(0.0, 1.0)
                * strength;
            if alpha <= 0.0 {
                continue;
            }
            let add_g = (alpha * 200.0) as u8;
            let add_b = (alpha * 255.0) as u8;
            for y in 0..HEIGHT {
                let pixel = &mut fb.pixels[Framebuffer::index(x, y)];
                *pixel = Rgb888::new(
                    pixel.r(),
                    pixel.g().saturating_add(add_g),
                    pixel.b().saturating_add(add_b),
                );
            }
        }
    }

    /// A bouncing arrow at the edge the device is tilted toward.
    fn draw_tilt_arrow(&self, fb: &mut Framebuffer) {
        let alpha = self.arrow_fade_ms / ARROW_FADE_MS;
        let color = Rgb888::new(
            (100.0 * alpha) as u8,
            (255.0 * alpha) as u8,
            (255.0 * alpha) as u8,
        );
        let style = PrimitiveStyleBuilder::new()
            .fill_color(color)
            .stroke_color(Rgb888::BLACK)
            .stroke_width(1)
            .build();

        // |cos| gives sharp "impacts" and smooth apexes, like a bouncing ball
        let bounce = ((self.time_ms / 80.0).cos().abs() * 6.0) as i32;
        let cy = HEIGHT as i32 / 2;
        let (base_x, tip_dx) = match self.arrow_direction {
            Direction::Right => (WIDTH as i32 - 5 - bounce, 4),
            Direction::Left => (4 + bounce, -4),
        };

        let _ = Triangle::new(
            Point::new(base_x, cy - 4),
            Point::new(base_x, cy + 4),
            Point::new(base_x + tip_dx, cy),
        )
        .into_styled(style)
        .draw(fb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Accel;
    use crate::net::NetworkStatus;
    use googletest::prelude::*;
    use std::cell::Cell;

    thread_local! {
        static BUILT: Cell<usize> = const { Cell::new(0) };
        static LAST_DT: Cell<f32> = const { Cell::new(0.0) };
    }

    /// Lights one pixel at (id, 0), so tests can tell which app is active.
    struct Marker(u32);

    impl App for Marker {
        fn update(&mut self, frame: &Frame) {
            LAST_DT.set(frame.dt_ms);
        }
        fn draw(&self, fb: &mut Framebuffer) {
            fb.pixels[Framebuffer::index(self.0, 0)] = Rgb888::WHITE;
        }
    }

    fn marker<const ID: u32>() -> Box<dyn App> {
        BUILT.set(BUILT.get() + 1);
        Box::new(Marker(ID))
    }

    fn manager() -> AppManager {
        AppManager::new(vec![marker::<10>, marker::<20>, marker::<30>])
    }

    fn frame(dt_ms: f32) -> Frame {
        Frame {
            dt_ms,
            network: NetworkStatus::Connecting,
            accel: Accel::default(),
        }
    }

    /// The marker app currently shown, after the transition has faded
    fn active(apps: &mut AppManager) -> u32 {
        apps.update(&frame(TRANSITION_MS));
        let mut fb = Framebuffer::new();
        apps.draw(&mut fb);
        (0..WIDTH)
            .find(|&x| fb.pixels[Framebuffer::index(x, 0)] == Rgb888::WHITE)
            .unwrap_or(u32::MAX)
    }

    #[gtest]
    fn starts_on_the_first_app_and_wraps_both_ways() {
        let mut apps = manager();
        expect_that!(active(&mut apps), eq(10));
        apps.handle(InputEvent::Previous);
        expect_that!(active(&mut apps), eq(30));
        apps.handle(InputEvent::Next);
        apps.handle(InputEvent::Next);
        expect_that!(active(&mut apps), eq(20));
    }

    #[gtest]
    fn switching_builds_a_fresh_app() {
        let mut apps = manager();
        let before = BUILT.get();
        apps.handle(InputEvent::Next);
        expect_that!(BUILT.get(), eq(before + 1));
    }

    #[gtest]
    fn forwards_frames_to_the_active_app() {
        let mut apps = manager();
        apps.update(&frame(16.0));
        expect_that!(LAST_DT.get(), eq(16.0));
    }

    #[gtest]
    #[should_panic(expected = "at least one app")]
    fn needs_an_app() {
        let _ = AppManager::new(vec![]);
    }

    /// How much the overlay brightened the given column's blue channel
    fn blue_at(apps: &AppManager, x: u32) -> u8 {
        let mut fb = Framebuffer::new();
        apps.draw(&mut fb);
        fb.pixels[Framebuffer::index(x, HEIGHT - 1)].b()
    }

    #[gtest]
    fn switching_flashes_the_edge_swiped_toward_then_fades() {
        let mut apps = manager();
        apps.handle(InputEvent::Next);
        expect_that!(blue_at(&apps, WIDTH - 1), eq(255), "right edge, full");
        expect_that!(blue_at(&apps, 0), eq(0), "left edge untouched");

        apps.update(&frame(TRANSITION_MS / 2.0));
        expect_that!(blue_at(&apps, WIDTH - 1), eq(127), "half faded");

        apps.handle(InputEvent::Previous);
        expect_that!(blue_at(&apps, 0), eq(255), "left edge");
        expect_that!(blue_at(&apps, WIDTH - 1), eq(0));

        apps.update(&frame(TRANSITION_MS));
        expect_that!(blue_at(&apps, 0), eq(0), "faded out");
    }

    fn arrow_pixels(apps: &AppManager, x_range: std::ops::Range<u32>) -> usize {
        let mut fb = Framebuffer::new();
        apps.draw(&mut fb);
        x_range
            // Row 0 holds the marker apps' pixels; the arrow is mid-height
            .flat_map(|x| (1..HEIGHT).map(move |y| (x, y)))
            .filter(|&(x, y)| fb.pixels[Framebuffer::index(x, y)].g() > 0)
            .count()
    }

    #[gtest]
    fn tilting_shows_an_arrow_on_that_side_until_released() {
        let mut apps = manager();
        apps.update(&frame(TRANSITION_MS));
        let left = 0..16;
        let right = WIDTH - 16..WIDTH;

        apps.handle(InputEvent::Tilt(1.0));
        apps.update(&frame(ARROW_FADE_MS));
        expect_that!(arrow_pixels(&apps, right.clone()), gt(0));
        expect_that!(arrow_pixels(&apps, left.clone()), eq(0));

        apps.handle(InputEvent::Tilt(-1.0));
        apps.handle(InputEvent::Tilt(-1.0));
        apps.update(&frame(ARROW_FADE_MS));
        expect_that!(arrow_pixels(&apps, left.clone()), gt(0));

        // Without tilt events the tilt decays and the arrow fades out
        for _ in 0..10 {
            apps.update(&frame(ARROW_FADE_MS));
        }
        expect_that!(
            arrow_pixels(&apps, left) + arrow_pixels(&apps, right),
            eq(0)
        );
    }
}
