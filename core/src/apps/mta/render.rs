//! Draws the arrival board: per platform, a route bullet, a scrolling
//! destination, and right-aligned arrival times.

use embedded_graphics::{
    geometry::{Point, Size},
    mono_font::{
        MonoTextStyle,
        ascii::{FONT_5X8, FONT_7X13},
    },
    pixelcolor::Rgb888,
    prelude::*,
    primitives::{Circle, PrimitiveStyle, Triangle},
    text::Text,
};

use crate::apps::mta::data::{Platform, StationState, Train};
use crate::apps::mta::routes::{get_destination, get_route_info};
use crate::apps::mta::stops::get_stop_name;
use crate::buffer::Framebuffer;
use crate::config::WIDTH;
use crate::fonts::{FONT_5X9, FONT_5X9_BOLD, FONT_6X12, FONT_6X12_BOLD};

// ============================================================================
// Layout
// ============================================================================

const TRAIN_CIRCLE_RADIUS: i32 = 6;
/// Left edge of the route bullet
const TRAIN_CIRCLE_X: i32 = 2;
/// Gap between the route bullet and the destination
const LEFT_TEXT_MARGIN: i32 = 2;
/// Left edge of the destination text
const DESTINATION_X: i32 =
    TRAIN_CIRCLE_X + TRAIN_CIRCLE_RADIUS * 2 + LEFT_TEXT_MARGIN;
const ROW_HEIGHT: i32 = 16;
/// Text baseline of the first row
const FIRST_ROW_Y: i32 = 11;
/// Gap between the two columns of arrival times
const TIME_SPACING: i32 = 3;
/// Gap between the destination and the arrival times
const RIGHT_MARGIN: i32 = 3;
/// Glyph widths of FONT_7X13 (destination, big times) and FONT_5X8 (small)
const CHAR_WIDTH: i32 = 7;
const SMALL_CHAR_WIDTH: i32 = 5;
/// Width of the first time column, e.g. "10m"
const COL1_MAX_W: i32 = 3 * CHAR_WIDTH;

const TIME_GRAY: Rgb888 = Rgb888::new(0x55, 0x55, 0x55);
/// Marks a train going somewhere other than the first train's terminal
const MARKER_RED: Rgb888 = Rgb888::RED;

// ============================================================================
// Scrolling
// ============================================================================

pub const SCROLL_HOLD_START_MS: f32 = 2500.0;
pub const SCROLL_TIME_PER_PIXEL_MS: f32 = 50.0;
pub const SCROLL_HOLD_END_MS: f32 = 1000.0;
/// How long a station is shown when nothing needs scrolling
const NO_SCROLL_CYCLE_MS: f32 = 5000.0;

/// Destinations too wide to fit hold, scroll left one pixel per
/// `SCROLL_TIME_PER_PIXEL_MS`, then hold at the end.
pub struct ScrollState {
    /// Time since the station came on screen
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
            return NO_SCROLL_CYCLE_MS;
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
    /// The underlying framebuffer's size: clipping happens in `draw_iter`.
    fn size(&self) -> Size {
        self.target.size()
    }
}

// ============================================================================
// Drawing Helpers
// ============================================================================

/// The route bullet: a circle, or a diamond for express ("X") routes.
fn draw_train_circle(fb: &mut Framebuffer, route: &str, x: i32, y: i32) {
    let express_route = route.strip_suffix('X');
    let display_route = express_route.unwrap_or(route);
    let info = get_route_info(display_route);
    let fill = PrimitiveStyle::with_fill(info.color);
    let r = TRAIN_CIRCLE_RADIUS;
    let top = y - 9;
    let center = Point::new(x + r, top + r);

    let (font, char_width, clip) = if express_route.is_some() {
        let left = Point::new(x, center.y);
        let right = Point::new(x + 2 * r, center.y);
        for tip in
            [Point::new(center.x, top), Point::new(center.x, top + 2 * r)]
        {
            let _ = Triangle::new(left, tip, right).into_styled(fill).draw(fb);
        }
        let font = if info.is_bold {
            &FONT_5X9_BOLD
        } else {
            &FONT_5X9
        };
        (
            font,
            5,
            ClipShape::Diamond {
                cx: center.x,
                cy: center.y,
                r,
            },
        )
    } else {
        let _ = Circle::new(Point::new(x, top), (2 * r) as u32)
            .into_styled(fill)
            .draw(fb);
        let font = if info.is_bold {
            &FONT_6X12_BOLD
        } else {
            &FONT_6X12
        };
        let clip = ClipShape::BoundingBox {
            left: x,
            right: x + 2 * r,
            top,
            bottom: top + 2 * r,
        };
        (font, 6, clip)
    };

    // Center the letters on the bullet, clipped to its shape
    let text_width = display_route.len() as i32 * char_width;
    let mut clipped = ClippedFramebuffer {
        target: fb,
        shape: clip,
    };
    let _ = Text::new(
        display_route,
        Point::new(center.x - text_width / 2, y),
        MonoTextStyle::new(font, info.letter_color),
    )
    .draw(&mut clipped);
}

pub struct ArrivalTime {
    pub text: String,
    pub has_different_dest: bool,
}

/// A red marker at the top right of an arrival time: a 5-pixel cross for big
/// times, a single pixel for small ones.
fn draw_marker(fb: &mut Framebuffer, center: Point, big: bool) {
    let offsets: &[(i32, i32)] = if big {
        &[(0, -1), (-1, 0), (0, 0), (1, 0), (0, 1)]
    } else {
        &[(0, 0)]
    };
    for &(dx, dy) in offsets {
        let _ = Pixel(center + Point::new(dx, dy), MARKER_RED).draw(fb);
    }
}

/// Draw one arrival time with its right edge at `right`.
fn draw_time(
    fb: &mut Framebuffer,
    time: &ArrivalTime,
    right: i32,
    y: i32,
    big: bool,
) {
    let (font, char_width) = if big {
        (&FONT_7X13, CHAR_WIDTH)
    } else {
        (&FONT_5X8, SMALL_CHAR_WIDTH)
    };
    let x = right - time.text.len() as i32 * char_width;
    let _ = Text::new(
        &time.text,
        Point::new(x, y),
        MonoTextStyle::new(font, TIME_GRAY),
    )
    .draw(fb);
    if time.has_different_dest {
        let marker = if big {
            Point::new(right - 2, y - 10)
        } else {
            Point::new(right - 1, y - 6)
        };
        draw_marker(fb, marker, big);
    }
}

/// Draw up to three arrival times right-aligned on the row: the first big,
/// then either a second big one or two small ones stacked. Returns the left
/// edge of the first time (the destination must end before it).
fn render_arrival_times(
    fb: &mut Framebuffer,
    y: i32,
    times: &[ArrivalTime],
) -> i32 {
    let display_width = WIDTH as i32;
    let width =
        |t: &ArrivalTime, char_width: i32| t.text.len() as i32 * char_width;

    let (first, rest) = match times {
        [] => return display_width,
        [first, rest @ ..] => (first, rest),
    };
    let col2_width = match rest {
        [] => 0,
        [second] => width(second, CHAR_WIDTH),
        [second, third, ..] => {
            width(second, SMALL_CHAR_WIDTH).max(width(third, SMALL_CHAR_WIDTH))
        }
    };
    let total_width = if rest.is_empty() {
        COL1_MAX_W
    } else {
        COL1_MAX_W + TIME_SPACING + col2_width
    };

    let col1_right = display_width - total_width + COL1_MAX_W;
    draw_time(fb, first, col1_right, y, true);

    let col2_right = col1_right + TIME_SPACING + col2_width;
    match rest {
        [] => {}
        [second] => draw_time(fb, second, col2_right, y, true),
        [second, third, ..] => {
            draw_time(fb, second, col2_right, y - 5, false);
            draw_time(fb, third, col2_right, y + 3, false);
        }
    }

    col1_right - width(first, CHAR_WIDTH)
}

/// The destination, clipped between the route bullet and the times, and
/// scrolled if it doesn't fit.
fn render_destination(
    fb: &mut Framebuffer,
    y: i32,
    dest: &str,
    clip_right: i32,
    scroll: &ScrollState,
) {
    // Nothing is drawn if there's no room: the clip box is then empty
    let available = clip_right - DESTINATION_X;
    let text_width = dest.len() as i32 * CHAR_WIDTH;
    let x = DESTINATION_X + scroll.calculate_offset(text_width, available);

    let mut clipped = ClippedFramebuffer {
        target: fb,
        shape: ClipShape::BoundingBox {
            left: DESTINATION_X,
            right: clip_right,
            top: y - 10,
            bottom: y + 3,
        },
    };
    let _ = Text::new(
        dest,
        Point::new(x, y),
        MonoTextStyle::new(&FONT_7X13, Rgb888::WHITE),
    )
    .draw(&mut clipped);
}

// ============================================================================
// Public API
// ============================================================================

/// One row: route bullet, scrolling destination, right-aligned times.
fn render_row(
    fb: &mut Framebuffer,
    y: i32,
    route: &str,
    dest: &str,
    times: &[ArrivalTime],
    scroll: &ScrollState,
) {
    draw_train_circle(fb, route, TRAIN_CIRCLE_X, y);
    let times_left = render_arrival_times(fb, y, times);
    render_destination(fb, y, dest, times_left - RIGHT_MARGIN, scroll);
}

/// The train's terminal station name, or the route's usual destination in
/// that direction if the terminal is unknown.
fn resolve_destination<'a>(train: &Train, direction: &'a str) -> &'a str {
    // Platform IDs end in N/S; names are listed under the parent station
    let terminal = train.terminal_stop_id.as_str();
    let parent = terminal.strip_suffix(['N', 'S']).unwrap_or(terminal);
    get_stop_name(parent)
        .unwrap_or_else(|| get_destination(&train.route, direction))
}

/// Compute the full scroll cycle duration in ms for a given station's platforms.
pub fn compute_cycle_ms(platforms: &[Platform]) -> f32 {
    // Estimate the times as two "10m" columns
    let times_width = 2 * (COL1_MAX_W + TIME_SPACING);
    let available = WIDTH as i32 - DESTINATION_X - RIGHT_MARGIN - times_width;

    platforms
        .iter()
        .filter_map(|p| {
            Some(resolve_destination(p.trains.first()?, &p.direction))
        })
        .map(|dest| {
            ScrollState::cycle_ms_for(dest.len() as i32 * CHAR_WIDTH, available)
        })
        .fold(NO_SCROLL_CYCLE_MS, f32::max)
}

/// Labels for the first `max` trains, marking trains bound for a different
/// terminal than the first one.
fn arrival_times(trains: &[Train], max: usize) -> Vec<ArrivalTime> {
    let primary_terminal = trains.first().map(|t| t.terminal_stop_id.as_str());
    trains
        .iter()
        .take(max)
        .map(|t| ArrivalTime {
            text: format!("{}m", t.arrives_in_secs / 60),
            has_different_dest: !t.terminal_stop_id.is_empty()
                && Some(t.terminal_stop_id.as_str()) != primary_terminal,
        })
        .collect()
}

fn message_row(
    fb: &mut Framebuffer,
    route: &str,
    message: &str,
    scroll: &ScrollState,
) {
    render_row(fb, FIRST_ROW_Y, route, message, &[], scroll);
}

/// Draw a station: up to two platforms, or a status message.
pub fn render_station(
    fb: &mut Framebuffer,
    state: &StationState,
    route: &str,
    scroll: &ScrollState,
) {
    match state {
        StationState::Loading => message_row(fb, route, "Loading...", scroll),
        StationState::NoTrains => message_row(fb, route, "No Trains", scroll),
        StationState::Unavailable => message_row(fb, route, "No data", scroll),
        StationState::Live(platforms) => {
            for (i, platform) in platforms.iter().enumerate().take(2) {
                if let Some(first_train) = platform.trains.first() {
                    let dest =
                        resolve_destination(first_train, &platform.direction);
                    let times = arrival_times(&platform.trains, 3);
                    let y = FIRST_ROW_Y + i as i32 * ROW_HEIGHT;
                    render_row(fb, y, &first_train.route, dest, &times, scroll);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use googletest::prelude::*;

    fn offset_at(elapsed_ms: f32, text_width: i32, available: i32) -> i32 {
        ScrollState { elapsed_ms }.calculate_offset(text_width, available)
    }

    #[gtest]
    fn text_that_fits_never_scrolls() {
        expect_that!(offset_at(99_999.0, 50, 50), eq(0));
        expect_that!(ScrollState::cycle_ms_for(50, 80), eq(NO_SCROLL_CYCLE_MS));
    }

    #[gtest]
    fn long_text_holds_scrolls_then_holds_at_the_end() {
        // 20px too wide: hold 2.5s, scroll 20px at 50ms/px, then stay
        expect_that!(offset_at(2_499.0, 120, 100), eq(0));
        expect_that!(offset_at(2_500.0 + 10.0 * 50.0, 120, 100), eq(-10));
        expect_that!(offset_at(2_500.0 + 25.0 * 50.0, 120, 100), eq(-20));
        expect_that!(
            ScrollState::cycle_ms_for(120, 100),
            eq(SCROLL_HOLD_START_MS
                + 20.0 * SCROLL_TIME_PER_PIXEL_MS
                + SCROLL_HOLD_END_MS)
        );
    }

    fn platform(direction: &str, terminal: &str) -> Platform {
        Platform {
            direction: direction.into(),
            trains: vec![Train {
                route: "A".into(),
                arrives_in_secs: 60,
                terminal_stop_id: terminal.into(),
            }],
        }
    }

    #[gtest]
    fn station_cycle_lasts_as_long_as_its_longest_scroll() {
        let short = platform("N", "A02N"); // "Inwood-207 St" fits
        let long = platform("S", "H11S"); // "Far Rockaway-Mott Av" scrolls
        let empty = Platform {
            direction: "N".into(),
            trains: vec![],
        };

        expect_that!(compute_cycle_ms(&[]), eq(NO_SCROLL_CYCLE_MS));
        expect_that!(
            compute_cycle_ms(&[short.clone(), empty]),
            eq(NO_SCROLL_CYCLE_MS)
        );
        expect_that!(compute_cycle_ms(&[short, long]), gt(NO_SCROLL_CYCLE_MS));
    }

    #[gtest]
    fn clipped_drawing_reports_the_full_framebuffer_size() {
        let mut fb = Framebuffer::new();
        let clipped = ClippedFramebuffer {
            target: &mut fb,
            shape: ClipShape::Diamond { cx: 5, cy: 5, r: 2 },
        };
        expect_that!(clipped.size(), eq(Size::new(WIDTH, 32)));
    }

    #[gtest]
    fn only_live_states_have_platforms() {
        expect_that!(StationState::NoTrains.platforms().len(), eq(0));
        expect_that!(
            StationState::Live(vec![platform("N", "")])
                .platforms()
                .len(),
            eq(1)
        );
    }
}
