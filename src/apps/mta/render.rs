// Pixel-perfect port of the v0 MTA rendering pipeline.
// Uses the same layout constants, clipping logic, scroll behavior, and fonts.

use embedded_graphics::{
    geometry::{Point, Size},
    mono_font::{ascii::FONT_7X13, MonoTextStyle},
    pixelcolor::Rgb888,
    prelude::*,
    primitives::{Circle, PrimitiveStyle, Triangle},
    text::Text,
};

use crate::apps::mta::data::{Platform, StationState, Train};
use crate::apps::mta::routes::get_destination;
use crate::apps::mta::routes::get_route_info;
use crate::buffer::Framebuffer;
use crate::fonts::{FONT_6X12, FONT_6X12_BOLD};

// ============================================================================
// Layout Constants — identical to v0
// ============================================================================

const TRAIN_CIRCLE_RADIUS: i32 = 6;
const TRAIN_CIRCLE_X: i32 = 1;
pub const ROW_HEIGHT: i32 = 16;
pub const FIRST_ROW_Y: i32 = 11;
const TIME_SPACING: i32 = 4;
const CLIP_MARGIN: i32 = 4;
const CHAR_WIDTH: i32 = 7; // FONT_7X13
const TIME_CHAR_WIDTH: i32 = 7; // FONT_7X13

// ============================================================================
// Scroll State
// ============================================================================

pub const SCROLL_HOLD_START_MS: f32 = 2500.0;
pub const SCROLL_TIME_PER_PIXEL_MS: f32 = 50.0;
pub const SCROLL_HOLD_END_MS: f32 = 1000.0;

pub struct ScrollState {
    pub elapsed_ms: f32,
}

impl ScrollState {
    pub fn calculate_offset(
        &self,
        text_width: i32,
        available_width: i32,
    ) -> i32 {
        if text_width <= available_width {
            return 0;
        }
        let max_scroll = text_width - available_width;
        let scroll_duration = SCROLL_TIME_PER_PIXEL_MS * max_scroll as f32;
        let elapsed = self.elapsed_ms;

        if elapsed < SCROLL_HOLD_START_MS {
            0
        } else if elapsed < SCROLL_HOLD_START_MS + scroll_duration {
            let progress = elapsed - SCROLL_HOLD_START_MS;
            let offset = (progress / SCROLL_TIME_PER_PIXEL_MS) as i32;
            -(offset.min(max_scroll))
        } else {
            -max_scroll
        }
    }

    /// Total duration of one full scroll cycle in ms (hold → scroll → hold)
    pub fn cycle_ms_for(text_width: i32, available_width: i32) -> f32 {
        if text_width <= available_width {
            return 5000.0; // no scroll needed, just display time
        }
        let max_scroll = text_width - available_width;
        SCROLL_TIME_PER_PIXEL_MS
            .mul_add(max_scroll as f32, SCROLL_HOLD_START_MS)
            + SCROLL_HOLD_END_MS
    }
}

// ============================================================================
// Clipped Drawing Target
// ============================================================================

enum ClipShape {
    BoundingBox {
        left: i32,
        right: i32,
        top: i32,
        bottom: i32,
    },
    Diamond {
        cx: i32,
        cy: i32,
        r: i32,
    },
}

struct ClippedFramebuffer<'a> {
    target: &'a mut Framebuffer,
    shape: ClipShape,
}

impl DrawTarget for ClippedFramebuffer<'_> {
    type Color = Rgb888;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let filtered = pixels.into_iter().filter(|Pixel(coord, _)| match &self
            .shape
        {
            ClipShape::BoundingBox {
                left,
                right,
                top,
                bottom,
            } => {
                coord.x >= *left
                    && coord.x < *right
                    && coord.y >= *top
                    && coord.y < *bottom
            }
            ClipShape::Diamond { cx, cy, r } => {
                (coord.x - cx).abs() + (coord.y - cy).abs() <= *r
            }
        });
        self.target.draw_iter(filtered)
    }
}

impl OriginDimensions for ClippedFramebuffer<'_> {
    fn size(&self) -> Size {
        match &self.shape {
            ClipShape::BoundingBox {
                left,
                right,
                top,
                bottom,
            } => Size::new(
                (right - left).max(0) as u32,
                (bottom - top).max(0) as u32,
            ),
            ClipShape::Diamond { r, .. } => {
                Size::new((r * 2) as u32, (r * 2) as u32)
            }
        }
    }
}

// ============================================================================
// Drawing Helpers
// ============================================================================

fn draw_train_circle(fb: &mut Framebuffer, route: &str, x: i32, y: i32) {
    let is_express = route.ends_with('X');
    let is_diamond = is_express;
    let display_route = if is_express {
        &route[..route.len() - 1]
    } else {
        route
    };

    let info = get_route_info(display_route);
    let fill = PrimitiveStyle::with_fill(info.color);
    let circle_y = y - 9;
    let r = TRAIN_CIRCLE_RADIUS;

    if is_diamond {
        // Diamond shape for express trains
        let top = Point::new(x + r, circle_y);
        let right = Point::new(x + r * 2, circle_y + r);
        let bottom = Point::new(x + r, circle_y + r * 2);
        let left = Point::new(x, circle_y + r);
        let _ = Triangle::new(left, top, right).into_styled(fill).draw(fb);
        let _ = Triangle::new(left, bottom, right)
            .into_styled(fill)
            .draw(fb);
    } else {
        let _ = Circle::new(Point::new(x, circle_y), (r as u32) * 2)
            .into_styled(fill)
            .draw(fb);
    }

    let font = if info.is_bold {
        &FONT_6X12_BOLD
    } else {
        &FONT_6X12
    };
    let letter_style = MonoTextStyle::new(font, info.letter_color);
    let text_width = display_route.len() as i32 * 6; // FONT_6X12 is 6px wide

    // Shift text +1x and +1y for diamonds to perfectly center it
    let text_x = x + r - (text_width / 2) + if is_diamond { 1 } else { 0 };
    let text_y = y + if is_diamond { 1 } else { 0 }; // baseline

    let mut clipped = ClippedFramebuffer {
        target: fb,
        shape: if is_diamond {
            ClipShape::Diamond {
                cx: x + r,
                cy: circle_y + r,
                r,
            }
        } else {
            ClipShape::BoundingBox {
                left: x,
                right: x + r * 2,
                top: circle_y,
                bottom: circle_y + r * 2,
            }
        },
    };
    let _ = Text::new(display_route, Point::new(text_x, text_y), letter_style)
        .draw(&mut clipped);
}

pub struct ArrivalTime {
    pub text: String,
    pub has_different_dest: bool,
}

fn render_arrival_times(
    fb: &mut Framebuffer,
    y_pos: i32,
    times: &[ArrivalTime],
) -> i32 {
    let style = MonoTextStyle::new(&FONT_7X13, Rgb888::new(0x88, 0x88, 0x88));
    let display_width = crate::config::WIDTH as i32;

    let total_width: i32 = times
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let w = t.text.len() as i32 * TIME_CHAR_WIDTH;
            if i < times.len() - 1 {
                w + TIME_SPACING
            } else {
                w
            }
        })
        .sum();

    let mut x = display_width - total_width;
    for t in times {
        let _ = Text::new(&t.text, Point::new(x, y_pos), style).draw(fb);

        if t.has_different_dest {
            // Draw a tiny red superscript '+' in the empty top-right space of the 'm' character
            let m_x = x + (t.text.len() as i32 - 1) * TIME_CHAR_WIDTH;
            let cx = m_x + 5; // right side of the 'm' cell
            let cy = y_pos - 10; // top of the 'm' cell (above the lowercase ascender)
            let red = Rgb888::new(0xFF, 0x00, 0x00);

            let _ = Pixel(Point::new(cx, cy - 1), red).draw(fb);
            let _ = Pixel(Point::new(cx - 1, cy), red).draw(fb);
            let _ = Pixel(Point::new(cx, cy), red).draw(fb);
            let _ = Pixel(Point::new(cx + 1, cy), red).draw(fb);
            let _ = Pixel(Point::new(cx, cy + 1), red).draw(fb);
        }

        x += t.text.len() as i32 * TIME_CHAR_WIDTH + TIME_SPACING;
    }

    display_width - total_width
}

fn render_destination(
    fb: &mut Framebuffer,
    y_pos: i32,
    dest: &str,
    circle_end: i32,
    clip_right: i32,
    scroll: &ScrollState,
) {
    let available = (clip_right - circle_end).max(0);
    if available <= 0 {
        return;
    }

    let style = MonoTextStyle::new(&FONT_7X13, Rgb888::WHITE);
    let text_width = dest.len() as i32 * CHAR_WIDTH;
    let offset = scroll.calculate_offset(text_width, available);
    let x = circle_end + offset;
    let x_end = x + text_width;

    if x_end > circle_end && x < clip_right {
        let mut clipped = ClippedFramebuffer {
            target: fb,
            shape: ClipShape::BoundingBox {
                left: circle_end,
                right: clip_right,
                top: y_pos - 10,
                bottom: y_pos + 3,
            },
        };
        let _ = Text::new(dest, Point::new(x, y_pos), style).draw(&mut clipped);
    }
}

// ============================================================================
// Public API
// ============================================================================

/// Render one row: train circle + scrolling destination + right-aligned arrival times.
pub fn render_row(
    fb: &mut Framebuffer,
    y_pos: i32,
    route: &str,
    dest: &str,
    times: &[ArrivalTime],
    scroll: &ScrollState,
) {
    draw_train_circle(fb, route, TRAIN_CIRCLE_X, y_pos);
    let times_start = render_arrival_times(fb, y_pos, times);
    let circle_end = TRAIN_CIRCLE_X + TRAIN_CIRCLE_RADIUS * 2 + 2;
    let clip_right = times_start - CLIP_MARGIN;
    render_destination(fb, y_pos, dest, circle_end, clip_right, scroll);
}

fn resolve_destination<'a>(train: &Train, direction: &'a str) -> &'a str {
    // Strip trailing direction N/S for lookup
    let base_id = if train.terminal_stop_id.ends_with('N')
        || train.terminal_stop_id.ends_with('S')
    {
        &train.terminal_stop_id[..train.terminal_stop_id.len() - 1]
    } else {
        &train.terminal_stop_id
    };

    if !base_id.is_empty() {
        if let Some(name) = crate::apps::mta::stops::get_stop_name(base_id) {
            return name;
        }
    }
    get_destination(&train.route, direction)
}

/// Compute the full scroll cycle duration in ms for a given station's platforms.
pub fn compute_cycle_ms(platforms: &[Platform]) -> f32 {
    let display_width = crate::config::WIDTH as i32;
    let mut max: f32 = 5000.0;

    for platform in platforms {
        if let Some(first) = platform.trains.first() {
            let dest = resolve_destination(first, &platform.direction);
            let text_w = dest.len() as i32 * CHAR_WIDTH;
            let times_w = 2 * (3 * TIME_CHAR_WIDTH + TIME_SPACING); // estimate "10m 5m"
            let avail = display_width
                - (TRAIN_CIRCLE_RADIUS * 2 + 2 + CLIP_MARGIN)
                - times_w;
            let cycle = ScrollState::cycle_ms_for(text_w, avail);
            if cycle > max {
                max = cycle;
            }
        }
    }

    max
}

/// Build arrival time strings and accurately compare final destinations using API `stop_ids`
pub fn arrival_times(platform: &Platform, max: usize) -> Vec<ArrivalTime> {
    if platform.trains.is_empty() {
        return vec![];
    }
    let primary_terminal = &platform.trains[0].terminal_stop_id;

    platform
        .trains
        .iter()
        .take(max)
        .map(|t| ArrivalTime {
            text: format!("{}m", t.arrives_in_secs / 60),
            has_different_dest: !t.terminal_stop_id.is_empty()
                && t.terminal_stop_id != *primary_terminal,
        })
        .collect()
}

/// Render a station's live data.
pub fn render_station(
    fb: &mut Framebuffer,
    state: &StationState,
    route: &str,
    scroll: &ScrollState,
) {
    match state {
        StationState::Loading => {
            // Just show route name + "Loading..."
            render_row(fb, FIRST_ROW_Y, route, "Loading...", &[], scroll);
        }
        StationState::NoTrains => {
            render_row(fb, FIRST_ROW_Y, route, "No Trains", &[], scroll);
        }
        StationState::Live(platforms) => {
            for (i, platform) in platforms.iter().enumerate().take(2) {
                if let Some(first_train) = platform.trains.first() {
                    let dest =
                        resolve_destination(first_train, &platform.direction);
                    let times = arrival_times(platform, 2);
                    let y = FIRST_ROW_Y + i as i32 * ROW_HEIGHT;
                    render_row(fb, y, &first_train.route, dest, &times, scroll);
                }
            }
        }
    }
}
