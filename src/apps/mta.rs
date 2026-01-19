//! MTA transit information app.

extern crate alloc;

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};

use anyhow::Result;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    Drawable,
    geometry::Point,
    mono_font::{
        MonoTextStyle,
        ascii::{FONT_6X13, FONT_6X13_BOLD, FONT_7X13},
    },
    prelude::*,
    primitives::{Circle, PrimitiveStyle},
    text::Text,
};
use esp_hub75::Color;
use log::info;
use prost::Message as _;
use reqwless::request::Method;

use crate::{
    apps::App,
    http::fetch,
    proto::{
        app_state::{
            AppId, MatrixHubState, Platform as ProtoPlat, StationStatus, Train as ProtoTrain,
            app_id::{Id, Mta},
        },
        transit_realtime::FeedMessage,
    },
    state::SharedMatrixHubState,
    tasks::hub75::{COLS, FrameBuffer},
    wifi::SharedHttpTcpClient,
};

// ============================================================================
// Constants
// ============================================================================

// Timing
const SCROLL_HOLD_START: Duration = Duration::from_millis(2500);
const SCROLL_TIME_PER_PIXEL: Duration = Duration::from_millis(50);
const SCROLL_HOLD_END: Duration = Duration::from_millis(1000);
const MIN_STATION_DISPLAY_TIME: Duration = Duration::from_millis(5000);
const FETCH_INTERVAL: Duration = Duration::from_secs(15);
const MAX_MINUTES_AHEAD: i64 = 30;
const MAX_TRAINS_PER_PLATFORM: usize = 8;
const MAX_DISPLAYED_TIMES: usize = 2;

// Layout
const TRAIN_CIRCLE_RADIUS: i32 = 6;
const TRAIN_CIRCLE_X: i32 = 1;
const ROW_HEIGHT: i32 = 16;
const FIRST_ROW_Y: i32 = 11;
const TIME_SPACING: i32 = 4;
const CLIP_MARGIN: i32 = 2;
const CHAR_WIDTH: i32 = 7;
const TIME_CHAR_WIDTH: i32 = 7;

// Spinner
const SPINNER_X: i32 = 7;
const SPINNER_Y: i32 = 8;
const SPINNER_FRAME_DURATION_MS: u64 = 150;
const SPINNER_FRAMES: u64 = 8;

// ============================================================================
// Data Structures
// ============================================================================

struct RouteInfo {
    color: Color,
    letter_color: Color,
    font: &'static embedded_graphics::mono_font::MonoFont<'static>,
    feed_url: &'static str,
}

impl RouteInfo {
    const fn new(
        color: Color,
        letter_color: Color,
        font: &'static embedded_graphics::mono_font::MonoFont<'static>,
        feed_url: &'static str,
    ) -> Self {
        Self {
            color,
            letter_color,
            font,
            feed_url,
        }
    }
}

struct ScrollState {
    max_cycle_ms: u64,
    elapsed_ms: u64,
}

impl ScrollState {
    fn calculate_offset(&self, text_width: i32, available_width: i32) -> i32 {
        if text_width <= available_width {
            return 0;
        }

        let max_scroll = text_width - available_width;
        let scroll_duration = SCROLL_TIME_PER_PIXEL * (max_scroll as u32);
        let scroll_in_cycle = Duration::from_millis(self.elapsed_ms % self.max_cycle_ms);

        if scroll_in_cycle < SCROLL_HOLD_START {
            0
        } else if scroll_in_cycle < SCROLL_HOLD_START + scroll_duration {
            let scroll_progress = scroll_in_cycle - SCROLL_HOLD_START;
            let offset = (scroll_progress.as_millis() / SCROLL_TIME_PER_PIXEL.as_millis()) as i32;
            -offset.min(max_scroll)
        } else {
            -max_scroll
        }
    }
}

// ============================================================================
// Route Configuration
// ============================================================================

const FEED_URL_DEFAULT: &str = "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs";
const FEED_URL_ACE: &str = "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-ace";
const FEED_URL_BDFM: &str =
    "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-bdfm";
const FEED_URL_G: &str = "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-g";
const FEED_URL_JZ: &str = "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-jz";
const FEED_URL_L: &str = "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-l";
const FEED_URL_NQRW: &str =
    "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-nqrw";
const FEED_URL_SI: &str = "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-si";

fn get_route_info(route: &str) -> RouteInfo {
    match route {
        "1" | "2" | "3" => RouteInfo::new(
            Color::new(0x8C, 0x0C, 0x0C),
            Color::WHITE,
            &FONT_6X13_BOLD,
            FEED_URL_DEFAULT,
        ),
        "4" | "5" | "6" => RouteInfo::new(
            Color::new(0x00, 0x2A, 0x00),
            Color::WHITE,
            &FONT_6X13_BOLD,
            FEED_URL_DEFAULT,
        ),
        "7" => RouteInfo::new(
            Color::new(0x3A, 0x0F, 0x42),
            Color::WHITE,
            &FONT_6X13_BOLD,
            FEED_URL_DEFAULT,
        ),
        "A" | "C" | "E" => RouteInfo::new(
            Color::new(0x00, 0x2D, 0x72),
            Color::WHITE,
            &FONT_6X13_BOLD,
            FEED_URL_ACE,
        ),
        "B" | "D" | "F" | "M" => RouteInfo::new(
            Color::new(0xFF, 0x20, 0x00),
            Color::WHITE,
            &FONT_6X13_BOLD,
            FEED_URL_BDFM,
        ),
        "G" => RouteInfo::new(
            Color::new(0x00, 0xA0, 0x00),
            Color::WHITE,
            &FONT_6X13_BOLD,
            FEED_URL_G,
        ),
        "L" => RouteInfo::new(
            Color::new(0x20, 0x20, 0x20),
            Color::WHITE,
            &FONT_6X13_BOLD,
            FEED_URL_L,
        ),
        "N" | "Q" | "R" | "W" => RouteInfo::new(
            Color::new(0x98, 0x78, 0x06),
            Color::BLACK,
            &FONT_6X13,
            FEED_URL_NQRW,
        ),
        "J" | "Z" => RouteInfo::new(
            Color::new(0x40, 0x2A, 0x15),
            Color::WHITE,
            &FONT_6X13_BOLD,
            FEED_URL_JZ,
        ),
        "S" => RouteInfo::new(
            Color::new(0x60, 0x60, 0x60),
            Color::WHITE,
            &FONT_6X13_BOLD,
            FEED_URL_SI,
        ),
        _ => RouteInfo::new(
            Color::BLACK,
            Color::WHITE,
            &FONT_6X13_BOLD,
            FEED_URL_DEFAULT,
        ),
    }
}

// Clipping helper (same as in display.rs)
struct ClippedDisplay<'a, T> {
    target: &'a mut T,
    clip_left: i32,
    clip_right: i32,
    clip_top: i32,
    clip_bottom: i32,
}

impl<T: DrawTarget<Color = Color>> DrawTarget for ClippedDisplay<'_, T> {
    type Color = Color;
    type Error = T::Error;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let clipped_pixels = pixels.into_iter().filter(|Pixel(coord, _)| {
            coord.x >= self.clip_left
                && coord.x < self.clip_right
                && coord.y >= self.clip_top
                && coord.y < self.clip_bottom
        });

        self.target.draw_iter(clipped_pixels)
    }
}

impl<T: DrawTarget<Color = Color>> OriginDimensions for ClippedDisplay<'_, T> {
    fn size(&self) -> Size {
        Size::new(
            (self.clip_right - self.clip_left).max(0) as u32,
            (self.clip_bottom - self.clip_top).max(0) as u32,
        )
    }
}

// ============================================================================
// Destination Mapping
// ============================================================================

fn get_destination<'a>(route: &str, direction: &'a str) -> &'a str {
    // Use a simple lookup table instead of massive match
    const DESTINATIONS: &[(&str, &str, &str)] = &[
        ("L", "N", "8 Ave"),
        ("L", "S", "Canarsie-Rockaway Pkwy"),
        ("G", "N", "Court Sq"),
        ("G", "S", "Church Ave"),
        ("1", "N", "Van Cortlandt Pk"),
        ("1", "S", "South Ferry"),
        ("2", "N", "Wakefield-241 St"),
        ("2", "S", "Flatbush Ave"),
        ("3", "N", "Harlem-148 St"),
        ("3", "S", "New Lots Ave"),
        ("4", "N", "Woodlawn"),
        ("4", "S", "Crown Hts"),
        ("5", "N", "Nereid Ave"),
        ("5", "S", "Flatbush Ave"),
        ("6", "N", "Pelham"),
        ("6", "S", "Brooklyn Bridge"),
        ("7", "E", "Flushing-Main St"),
        ("7", "W", "34 St-Hudson Yds"),
        ("A", "N", "Inwood-207 St"),
        ("A", "S", "Far Rockaway"),
        ("C", "N", "168 St"),
        ("C", "S", "Euclid Ave"),
        ("E", "E", "Jamaica Ctr"),
        ("E", "W", "World Trade Ctr"),
        ("B", "N", "Bedford Pk Blvd"),
        ("B", "S", "Brighton Beach"),
        ("D", "N", "Norwood-205 St"),
        ("D", "S", "Coney Island-Stillwell Ave"),
        ("F", "N", "Jamaica-179 St"),
        ("F", "S", "Coney Island-Stillwell Ave"),
        ("M", "N", "Forest Hills-71 Ave"),
        ("M", "S", "Middle Village-Metropolitan Ave"),
        ("N", "N", "Astoria-Ditmars Blvd"),
        ("N", "S", "Coney Island-Stillwell Ave"),
        ("Q", "N", "96 St-2 Ave"),
        ("Q", "S", "Coney Island-Stillwell Ave"),
        ("R", "N", "Forest Hills-71 Ave"),
        ("R", "S", "Bay Ridge-95 St"),
        ("W", "N", "Astoria-Ditmars Blvd"),
        ("W", "S", "Whitehall St"),
        ("J", "E", "Jamaica Ctr"),
        ("J", "W", "Broad St"),
        ("Z", "E", "Jamaica Ctr"),
        ("Z", "W", "Broad St"),
    ];

    DESTINATIONS
        .iter()
        .find(|(r, d, _)| *r == route && *d == direction)
        .map(|(_, _, dest)| *dest)
        .unwrap_or_else(|| if route == "S" { "Shuttle" } else { direction })
}

// ============================================================================
// Drawing Utilities
// ============================================================================

fn draw_train_circle<D: DrawTarget<Color = Color>>(
    display: &mut D,
    route: &str,
    x: i32,
    y: i32,
) -> Result<(), D::Error> {
    let route_info = get_route_info(route);
    let circle_y = y - 9;

    Circle::new(Point::new(x, circle_y), (TRAIN_CIRCLE_RADIUS as u32) * 2)
        .into_styled(PrimitiveStyle::with_fill(route_info.color))
        .draw(display)?;

    let letter_style = MonoTextStyle::new(route_info.font, route_info.letter_color);
    let text_x = x + TRAIN_CIRCLE_RADIUS - (CHAR_WIDTH / 2);
    Text::new(route, Point::new(text_x, y), letter_style).draw(display)?;

    Ok(())
}

fn draw_spinner<D: DrawTarget<Color = Color>>(
    display: &mut D,
    now_millis: u64,
) -> Result<(), D::Error> {
    let spinner_frame = (now_millis / SPINNER_FRAME_DURATION_MS) % SPINNER_FRAMES;

    // Dot positions in a circle
    const DOT_POSITIONS: [(i32, i32); 8] = [
        (5, 0),
        (4, 3),
        (0, 5),
        (-4, 3),
        (-5, 0),
        (-4, -3),
        (0, -5),
        (4, -3),
    ];

    for (i, &(dx, dy)) in DOT_POSITIONS.iter().enumerate() {
        let x = SPINNER_X + dx;
        let y = SPINNER_Y + dy;

        let brightness = if i == spinner_frame as usize {
            255
        } else if i == ((spinner_frame + 7) % SPINNER_FRAMES) as usize {
            128
        } else if i == ((spinner_frame + 6) % SPINNER_FRAMES) as usize {
            64
        } else {
            32
        };

        let color = Color::new(brightness, brightness, brightness);
        Circle::new(Point::new(x - 1, y - 1), 2)
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(display)?;
    }

    Ok(())
}

// ============================================================================
// Rendering
// ============================================================================

fn render_arrival_times<D: DrawTarget<Color = Color>>(
    display: &mut D,
    y_pos: i32,
    times: &[String],
) -> Result<i32, D::Error> {
    let times_style = MonoTextStyle::new(&FONT_7X13, Color::WHITE);
    let display_width = COLS as i32;

    let total_width: i32 = times
        .iter()
        .enumerate()
        .map(|(i, time_str)| {
            let width = (time_str.len() as i32) * TIME_CHAR_WIDTH;
            if i < times.len() - 1 {
                width + TIME_SPACING
            } else {
                width
            }
        })
        .sum();

    let mut times_x = display_width - total_width;
    for time_str in times {
        Text::new(time_str.as_str(), Point::new(times_x, y_pos), times_style).draw(display)?;
        times_x += (time_str.len() as i32) * TIME_CHAR_WIDTH + TIME_SPACING;
    }

    Ok(display_width - total_width)
}

fn render_destination<D: DrawTarget<Color = Color>>(
    display: &mut D,
    y_pos: i32,
    destination: &str,
    circle_end: i32,
    clip_right: i32,
    scroll_state: &ScrollState,
) -> Result<(), D::Error> {
    let available_width = (clip_right - circle_end).max(0);
    if available_width <= 0 {
        return Ok(());
    }

    let text_style = MonoTextStyle::new(&FONT_7X13, Color::WHITE);
    let text_width = (destination.len() as i32) * CHAR_WIDTH;
    let scroll_offset = scroll_state.calculate_offset(text_width, available_width);
    let x_offset = circle_end + scroll_offset;
    let text_right_edge = x_offset + text_width;

    if text_right_edge > circle_end && x_offset < clip_right {
        let mut clipped_display = ClippedDisplay {
            target: display,
            clip_left: circle_end,
            clip_right,
            clip_top: y_pos - 10,
            clip_bottom: y_pos + 3,
        };
        Text::new(destination, Point::new(x_offset, y_pos), text_style)
            .draw(&mut clipped_display)?;
    }

    Ok(())
}

fn render_mta_row(
    frame_buffer: &mut FrameBuffer,
    y_pos: i32,
    route: &str,
    destination: &str,
    times: &[String],
    scroll_state: &ScrollState,
) -> Result<()> {
    draw_train_circle(frame_buffer, route, TRAIN_CIRCLE_X, y_pos)?;

    let times_start = render_arrival_times(frame_buffer, y_pos, times)?;
    let circle_end = TRAIN_CIRCLE_X + (TRAIN_CIRCLE_RADIUS * 2) + 2;
    let clip_right = times_start - CLIP_MARGIN;

    render_destination(
        frame_buffer,
        y_pos,
        destination,
        circle_end,
        clip_right,
        scroll_state,
    )?;

    Ok(())
}

fn calculate_max_scroll_cycle(platforms: &[ProtoPlat]) -> Duration {
    let mut max_cycle = MIN_STATION_DISPLAY_TIME;

    for platform in platforms {
        if let Some(train) = platform.trains.first() {
            let dest = get_destination(&train.route, &platform.direction);
            let text_len = (dest.len() as i32) * CHAR_WIDTH;
            let avail =
                (COLS as i32) - (platform.trains.len().min(MAX_DISPLAYED_TIMES) * 25) as i32 - 17;

            if text_len > avail {
                let scroll_distance = (text_len - avail) as u32;
                let cycle =
                    SCROLL_HOLD_START + SCROLL_TIME_PER_PIXEL * scroll_distance + SCROLL_HOLD_END;
                max_cycle = if cycle > max_cycle { cycle } else { max_cycle };
            }
        }
    }

    max_cycle
}

fn render_loading_state(display: &mut FrameBuffer, configured_routes: &[String]) -> Result<()> {
    let now_millis = embassy_time::Instant::now().as_millis();
    draw_spinner(display, now_millis)?;

    let circle_diameter = (TRAIN_CIRCLE_RADIUS * 2) as i32;
    let circle_spacing = 3;
    let start_x = SPINNER_X + 6 + circle_spacing;

    let mut current_x = start_x;
    let mut current_y = FIRST_ROW_Y;

    for route in configured_routes {
        if current_x + circle_diameter > COLS as i32 {
            current_x = 1;
            current_y += ROW_HEIGHT;
        }

        draw_train_circle(display, route, current_x, current_y)?;
        current_x += circle_diameter + circle_spacing;
    }

    Ok(())
}

// ============================================================================
// App Implementation
// ============================================================================

pub struct MtaApp {
    state: SharedMatrixHubState,
}

impl MtaApp {
    pub fn new(state: SharedMatrixHubState) -> Self {
        Self { state }
    }

    fn get_configured_routes(&self, app_state: &MatrixHubState) -> Vec<String> {
        app_state
            .config
            .as_ref()
            .and_then(|c| c.mta.as_ref())
            .map(|mta| mta.stations.iter().map(|s| s.route.clone()).collect())
            .unwrap_or_default()
    }
}

#[async_trait::async_trait(?Send)]
impl App for MtaApp {
    fn build(state: &SharedMatrixHubState, _: AppId) -> Self {
        Self {
            state: state.clone(),
        }
    }

    fn id(&self) -> AppId {
        AppId {
            id: Some(Id::Mta(Mta {})),
        }
    }

    async fn run(&self, http_client: SharedHttpTcpClient) -> Result<()> {
        loop {
            let station_configs = {
                let app_state = self.state.lock().await;
                app_state
                    .config
                    .as_ref()
                    .and_then(|c| c.mta.as_ref())
                    .map(|mta| mta.stations.clone())
                    .unwrap_or_default()
            };

            let combined_stations = self.fetch_all_stations(&http_client, station_configs).await;
            self.update_state(combined_stations).await;

            info!("MTA sleeping for {} seconds...", FETCH_INTERVAL.as_secs());
            Timer::after(FETCH_INTERVAL).await;
        }
    }

    fn render(&self, state: &mut MatrixHubState, display: &mut FrameBuffer) -> Result<()> {
        let has_data = state
            .mta
            .as_ref()
            .map(|m| !m.stations.is_empty())
            .unwrap_or(false);

        if !has_data {
            let configured_routes = self.get_configured_routes(state);
            return render_loading_state(display, &configured_routes);
        }

        let (station_idx, scroll_state) = self.calculate_scroll_state(state);
        self.render_station(state, display, station_idx, &scroll_state)
    }
}

impl MtaApp {
    async fn fetch_all_stations(
        &self,
        http_client: &SharedHttpTcpClient,
        station_configs: Vec<crate::proto::app_state::StationConfig>,
    ) -> Vec<StationStatus> {
        let feed_stations = self.group_stations_by_feed(station_configs);
        let mut combined_stations = Vec::new();

        for (feed_url, station_configs) in feed_stations {
            if let Some(stations) = self
                .fetch_feed_and_process(http_client, feed_url, station_configs)
                .await
            {
                combined_stations.extend(stations);
            }
        }

        combined_stations
    }

    fn group_stations_by_feed(
        &self,
        station_configs: Vec<crate::proto::app_state::StationConfig>,
    ) -> alloc::collections::BTreeMap<&str, Vec<crate::proto::app_state::StationConfig>> {
        let mut feed_stations: alloc::collections::BTreeMap<&str, Vec<_>> =
            alloc::collections::BTreeMap::new();

        for station_config in station_configs {
            let feed_url = get_route_info(&station_config.route).feed_url;
            feed_stations
                .entry(feed_url)
                .or_insert_with(Vec::new)
                .push(station_config);
        }

        feed_stations
    }

    async fn fetch_feed_and_process(
        &self,
        http_client: &SharedHttpTcpClient,
        feed_url: &str,
        station_configs: Vec<crate::proto::app_state::StationConfig>,
    ) -> Option<Vec<StationStatus>> {
        info!("Fetching MTA feed: {}", feed_url);

        let data = match fetch(&mut *http_client.lock().await, Method::GET, feed_url).await {
            Ok(data) => data,
            Err(e) => {
                info!("HTTP request failed for {}: {:?}", feed_url, e);
                return None;
            }
        };

        info!("MTA feed fetched: {} bytes from {}", data.len(), feed_url);

        let feed_message = match FeedMessage::decode(&data[..]) {
            Ok(feed) => Box::new(feed),
            Err(e) => {
                info!("Failed to parse feed {}: {:?}", feed_url, e);
                return None;
            }
        };

        info!(
            "Feed {} parsed: {} entities",
            feed_url,
            feed_message.entity.len()
        );

        Some(self.process_stations_from_feed(&feed_message, station_configs))
    }

    fn process_stations_from_feed(
        &self,
        feed: &FeedMessage,
        station_configs: Vec<crate::proto::app_state::StationConfig>,
    ) -> Vec<StationStatus> {
        station_configs
            .into_iter()
            .filter_map(|config| {
                match process_feed_single_station(feed, &config.route, &config.station_id) {
                    Ok(Some(station)) => Some(station),
                    Ok(None) => {
                        info!(
                            "No data found for route {} at station {}",
                            config.route, config.station_id
                        );
                        None
                    }
                    Err(e) => {
                        info!("Error processing station {}: {:?}", config.station_id, e);
                        None
                    }
                }
            })
            .collect()
    }

    async fn update_state(&self, combined_stations: Vec<StationStatus>) {
        let total_trains: usize = combined_stations
            .iter()
            .flat_map(|s| &s.platforms)
            .map(|p| p.trains.len())
            .sum();

        info!(
            "Processed {} stations, {} trains",
            combined_stations.len(),
            total_trains
        );

        let mut app_state = self.state.lock().await;
        let mta = app_state.mta.get_or_insert_default();

        let stations_changed = mta.stations.len() != combined_stations.len()
            || mta
                .stations
                .iter()
                .zip(&combined_stations)
                .any(|(old, new)| old.station_id != new.station_id);

        if stations_changed {
            info!("Station configuration changed, resetting index");
            mta.current_station_index = 0;
            mta.scroll_start_secs = embassy_time::Instant::now().as_millis() / 1000;
        }

        mta.stations = combined_stations;
        mta.last_updated_secs = embassy_time::Instant::now().as_secs();
    }

    fn calculate_scroll_state(&self, state: &mut MatrixHubState) -> (usize, ScrollState) {
        let now_millis = embassy_time::Instant::now().as_millis();
        let mta = state.mta.as_ref().expect("MTA state initialized");

        let idx = mta.current_station_index as usize;
        let station = &mta.stations[idx.min(mta.stations.len().saturating_sub(1))];

        let max_cycle = calculate_max_scroll_cycle(&station.platforms);
        let max_cycle_ms = max_cycle.as_millis();
        let elapsed = now_millis.saturating_sub(mta.scroll_start_secs * 1000);

        if elapsed >= max_cycle_ms {
            let next_idx = (idx + 1) % mta.stations.len().max(1);
            if let Some(mta_mut) = state.mta.as_mut() {
                mta_mut.scroll_start_secs = now_millis / 1000;
                mta_mut.current_station_index = next_idx as u32;
            }
            (
                next_idx,
                ScrollState {
                    max_cycle_ms,
                    elapsed_ms: 0,
                },
            )
        } else {
            (
                idx,
                ScrollState {
                    max_cycle_ms,
                    elapsed_ms: elapsed,
                },
            )
        }
    }

    fn render_station(
        &self,
        state: &MatrixHubState,
        display: &mut FrameBuffer,
        station_idx: usize,
        scroll_state: &ScrollState,
    ) -> Result<()> {
        let mta = state.mta.as_ref().expect("MTA state initialized");
        let station = &mta.stations[station_idx.min(mta.stations.len().saturating_sub(1))];

        for (i, platform) in station.platforms.iter().enumerate() {
            if let Some(train) = platform.trains.first() {
                let times: Vec<String> = platform
                    .trains
                    .iter()
                    .take(MAX_DISPLAYED_TIMES)
                    .map(|t| alloc::format!("{}m", t.arrives_in_secs / 60))
                    .collect();

                let y_pos = FIRST_ROW_Y + (i as i32) * ROW_HEIGHT;
                render_mta_row(
                    display,
                    y_pos,
                    &train.route,
                    get_destination(&train.route, &platform.direction),
                    &times,
                    scroll_state,
                )?;
            }
        }

        Ok(())
    }
}

// ============================================================================
// Feed Processing
// ============================================================================

fn process_feed_single_station(
    feed: &FeedMessage,
    route: &str,
    station_prefix: &str,
) -> Result<Option<StationStatus>> {
    let now_secs = feed.header.timestamp.unwrap_or(0);
    let arrivals_map = collect_arrivals(feed, route, station_prefix, now_secs);

    if arrivals_map.is_empty() {
        return Ok(None);
    }

    let platforms = build_platforms(arrivals_map, now_secs);
    Ok(Some(StationStatus {
        station_id: station_prefix.to_string(),
        platforms,
    }))
}

fn collect_arrivals(
    feed: &FeedMessage,
    route: &str,
    station_prefix: &str,
    now_secs: u64,
) -> alloc::collections::BTreeMap<String, Vec<(String, u64)>> {
    let mut arrivals_map: alloc::collections::BTreeMap<String, Vec<(String, u64)>> =
        alloc::collections::BTreeMap::new();

    for entity in &feed.entity {
        let Some(trip_update) = &entity.trip_update else {
            continue;
        };

        let trip = &trip_update.trip;
        let trip_route = trip.route_id.as_deref().unwrap_or("?");

        if trip_route != route {
            continue;
        }

        let last_stop_id = trip_update
            .stop_time_update
            .last()
            .and_then(|stu| stu.stop_id.as_ref());

        for stop_time_update in &trip_update.stop_time_update {
            if let Some(arrival) = process_stop_time_update(
                stop_time_update,
                station_prefix,
                last_stop_id,
                trip_route,
                now_secs,
            ) {
                let stop_id = stop_time_update.stop_id.as_ref().unwrap().clone();
                arrivals_map
                    .entry(stop_id)
                    .or_insert_with(Vec::new)
                    .push(arrival);
            }
        }
    }

    arrivals_map
}

fn process_stop_time_update(
    stop_time_update: &crate::proto::transit_realtime::trip_update::StopTimeUpdate,
    station_prefix: &str,
    last_stop_id: Option<&String>,
    trip_route: &str,
    now_secs: u64,
) -> Option<(String, u64)> {
    let stop_id = stop_time_update.stop_id.as_ref()?;

    if !stop_id.starts_with(station_prefix) {
        return None;
    }

    // Skip terminal stops
    if Some(stop_id.as_str()) == last_stop_id.map(|s| s.as_str()) {
        return None;
    }

    let arrival = stop_time_update.arrival.as_ref()?;
    let arrival_time = arrival.time? as u64;

    if arrival_time <= now_secs {
        return None;
    }

    let seconds_away = arrival_time - now_secs;
    if seconds_away / 60 <= MAX_MINUTES_AHEAD as u64 {
        Some((trip_route.to_string(), arrival_time))
    } else {
        None
    }
}

fn build_platforms(
    arrivals_map: alloc::collections::BTreeMap<String, Vec<(String, u64)>>,
    now_secs: u64,
) -> Vec<ProtoPlat> {
    let mut platforms: Vec<_> = arrivals_map
        .into_iter()
        .map(|(stop_id, mut arrivals)| {
            let direction = stop_id.chars().last().unwrap_or('?').to_string();
            arrivals.sort_by_key(|(_, time)| *time);

            let trains: Vec<ProtoTrain> = arrivals
                .iter()
                .take(MAX_TRAINS_PER_PLATFORM)
                .map(|(route, arrival_time)| ProtoTrain {
                    route: route.clone(),
                    arrives_at_secs: *arrival_time,
                    arrives_in_secs: arrival_time.saturating_sub(now_secs),
                })
                .collect();

            ProtoPlat { direction, trains }
        })
        .collect();

    // Sort platforms: N/W (northbound/westbound) first
    platforms.sort_by(|a, b| a.direction.cmp(&b.direction));
    platforms
}
