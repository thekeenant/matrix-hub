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

// Scroll timing configuration
const SCROLL_HOLD_START: Duration = Duration::from_millis(2500);
const SCROLL_TIME_PER_PIXEL: Duration = Duration::from_millis(50);
const SCROLL_HOLD_END: Duration = Duration::from_millis(1000);

// Map routes to their GTFS-RT feed URLs
fn get_feed_url_for_route(route: &str) -> &'static str {
    match route {
        "1" | "2" | "3" | "4" | "5" | "6" | "7" => {
            "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs"
        }
        "A" | "C" | "E" => "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-ace",
        "B" | "D" | "F" | "M" => {
            "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-bdfm"
        }
        "G" => "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-g",
        "J" | "Z" => "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-jz",
        "L" => "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-l",
        "N" | "Q" | "R" | "W" => {
            "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-nqrw"
        }
        "S" => "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-si",
        _ => "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs",
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

fn get_destination<'a>(route: &str, direction: &'a str) -> &'a str {
    match (route, direction) {
        ("L", "N") => "8 Ave",
        ("L", "S") => "Canarsie-Rockaway Pkwy",
        ("G", "N") => "Court Sq",
        ("G", "S") => "Church Ave",
        ("1", "N") => "Van Cortlandt Pk",
        ("1", "S") => "South Ferry",
        ("2", "N") => "Wakefield-241 St",
        ("2", "S") => "Flatbush Ave",
        ("3", "N") => "Harlem-148 St",
        ("3", "S") => "New Lots Ave",
        ("4", "N") => "Woodlawn",
        ("4", "S") => "Crown Hts",
        ("5", "N") => "Nereid Ave",
        ("5", "S") => "Flatbush Ave",
        ("6", "N") => "Pelham",
        ("6", "S") => "Brooklyn Bridge",
        ("7", "E") => "Flushing-Main St",
        ("7", "W") => "34 St-Hudson Yds",
        ("A", "N") => "Inwood-207 St",
        ("A", "S") => "Far Rockaway",
        ("C", "N") => "168 St",
        ("C", "S") => "Euclid Ave",
        ("E", "E") => "Jamaica Ctr",
        ("E", "W") => "World Trade Ctr",
        ("B", "N") => "Bedford Pk Blvd",
        ("B", "S") => "Brighton Beach",
        ("D", "N") => "Norwood-205 St",
        ("D", "S") => "Coney Island-Stillwell Ave",
        ("F", "N") => "Jamaica-179 St",
        ("F", "S") => "Coney Island-Stillwell Ave",
        ("M", "N") => "Forest Hills-71 Ave",
        ("M", "S") => "Middle Village-Metropolitan Ave",
        ("N", "N") => "Astoria-Ditmars Blvd",
        ("N", "S") => "Coney Island-Stillwell Ave",
        ("Q", "N") => "96 St-2 Ave",
        ("Q", "S") => "Coney Island-Stillwell Ave",
        ("R", "N") => "Forest Hills-71 Ave",
        ("R", "S") => "Bay Ridge-95 St",
        ("W", "N") => "Astoria-Ditmars Blvd",
        ("W", "S") => "Whitehall St",
        ("J", "E") => "Jamaica Ctr",
        ("J", "W") => "Broad St",
        ("Z", "E") => "Jamaica Ctr",
        ("Z", "W") => "Broad St",
        ("S", _) => "Shuttle",
        _ => direction,
    }
}

fn get_train_color(route: &str) -> Color {
    match route {
        "1" | "2" | "3" => Color::new(0x8C, 0x0C, 0x0C),
        "4" | "5" | "6" => Color::new(0x00, 0x2A, 0x00),
        "7" => Color::new(0x3A, 0x0F, 0x42),
        "A" | "C" | "E" => Color::new(0x00, 0x2D, 0x72),
        "B" | "D" | "F" | "M" => Color::new(0xFF, 0x20, 0x00),
        "G" => Color::new(0x00, 0xA0, 0x00),
        "L" => Color::new(0x20, 0x20, 0x20),
        "N" | "Q" | "R" | "W" => Color::new(0x98, 0x78, 0x06),
        "J" | "Z" => Color::new(0x40, 0x2A, 0x15),
        "S" => Color::new(0x60, 0x60, 0x60),
        _ => Color::BLACK,
    }
}

fn get_train_letter_color(route: &str) -> Color {
    match route {
        "N" | "Q" | "R" | "W" => Color::BLACK,
        _ => Color::WHITE,
    }
}

fn draw_train_circle<D: DrawTarget<Color = Color>>(
    display: &mut D,
    route: &str,
    x: i32,
    y: i32,
) -> Result<(), D::Error> {
    let color = get_train_color(route);
    let circle_radius = 6;
    let circle_y = y - 9;

    Circle::new(Point::new(x, circle_y), (circle_radius as u32) * 2)
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(display)?;

    let letter_color = get_train_letter_color(route);
    let font = if letter_color == Color::BLACK {
        &FONT_6X13
    } else {
        &FONT_6X13_BOLD
    };
    let letter_style = MonoTextStyle::new(font, letter_color);
    let char_width = 6;
    let text_x = x + circle_radius - (char_width / 2);
    Text::new(route, Point::new(text_x, y), letter_style).draw(display)?;

    Ok(())
}

fn render_mta_row(
    frame_buffer: &mut FrameBuffer,
    y_pos: i32,
    route: &str,
    destination: &str,
    times: &[String],
    scroll_elapsed_millis: u64,
    shared_cycle_ms: u64,
) -> Result<()> {
    let circle_radius = 6;
    let circle_x = 1;

    draw_train_circle(frame_buffer, route, circle_x, y_pos)?;

    let times_style = MonoTextStyle::new(&FONT_7X13, Color::WHITE);
    let display_width = COLS as i32;
    let spacing = 4;

    let mut total_width = 0;
    for (i, time_str) in times.iter().enumerate() {
        total_width += (time_str.len() * 7) as i32;
        if i < times.len().saturating_sub(1) {
            total_width += spacing;
        }
    }

    let mut times_x = display_width - total_width;
    for time_str in times {
        Text::new(time_str.as_str(), Point::new(times_x, y_pos), times_style).draw(frame_buffer)?;
        times_x += (time_str.len() * 7) as i32 + spacing;
    }

    let times_start = display_width - total_width;
    let circle_end = circle_x + (circle_radius * 2) + 2;
    let clip_right = times_start - 2;
    let available_width = (clip_right - circle_end).max(0);

    if available_width > 0 {
        let text_style = MonoTextStyle::new(&FONT_7X13, Color::WHITE);
        let text_width = (destination.len() * 7) as i32;

        let x_offset = if text_width > available_width {
            // Time-based scrolling
            let max_scroll = text_width - available_width;
            let scroll_duration = SCROLL_TIME_PER_PIXEL * (max_scroll as u32);

            let scroll_in_cycle = Duration::from_millis(scroll_elapsed_millis % shared_cycle_ms);

            if scroll_in_cycle < SCROLL_HOLD_START {
                // Phase 1: Hold at start
                circle_end
            } else if scroll_in_cycle < SCROLL_HOLD_START + scroll_duration {
                // Phase 2: Scroll
                let scroll_progress = scroll_in_cycle - SCROLL_HOLD_START;
                let scroll_offset =
                    (scroll_progress.as_millis() / SCROLL_TIME_PER_PIXEL.as_millis()) as i32;
                circle_end - scroll_offset.min(max_scroll)
            } else if scroll_in_cycle < SCROLL_HOLD_START + scroll_duration + SCROLL_HOLD_END {
                // Phase 3: Hold at end
                circle_end - max_scroll
            } else {
                // Wait for other rows to complete their cycles
                circle_end - max_scroll
            }
        } else {
            circle_end
        };

        let text_right_edge = x_offset + text_width;
        if text_right_edge > circle_end && x_offset < clip_right {
            let mut clipped_display = ClippedDisplay {
                target: frame_buffer,
                clip_left: circle_end,
                clip_right,
                clip_top: y_pos - 10,
                clip_bottom: y_pos + 3,
            };
            Text::new(destination, Point::new(x_offset, y_pos), text_style)
                .draw(&mut clipped_display)?;
        }
    }

    Ok(())
}

pub struct MtaApp {
    state: SharedMatrixHubState,
}

impl MtaApp {
    pub fn new(state: SharedMatrixHubState) -> Self {
        Self { state }
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
            info!("MTA fetch task: fetching updates...");

            // Get station configs from state
            let station_configs = {
                let app_state = self.state.lock().await;
                app_state
                    .config
                    .as_ref()
                    .and_then(|c| c.mta.as_ref())
                    .map(|mta| mta.stations.clone())
                    .unwrap_or_default()
            };

            let mut combined_stations = Vec::new();

            // Fetch stations based on config
            // Group stations by feed URL to minimize requests
            let mut feed_stations: alloc::collections::BTreeMap<&str, Vec<_>> =
                alloc::collections::BTreeMap::new();

            for station_config in station_configs {
                let feed_url = get_feed_url_for_route(&station_config.route);
                feed_stations
                    .entry(feed_url)
                    .or_insert_with(Vec::new)
                    .push(station_config);
            }

            // Fetch each unique feed once and process all stations from it
            for (feed_url, station_configs) in feed_stations {
                info!("Fetching MTA feed: {}", feed_url);
                let feed_data = fetch(&mut *http_client.lock().await, Method::GET, feed_url).await;

                let data = match feed_data {
                    Ok(data) => data,
                    Err(e) => {
                        info!("HTTP request failed for {}: {:?}", feed_url, e);
                        continue;
                    }
                };

                info!("MTA feed fetched: {} bytes from {}", data.len(), feed_url);
                let feed_message = match FeedMessage::decode(&data[..]) {
                    Ok(feed) => Box::new(feed),
                    Err(e) => {
                        info!("Failed to parse feed {}: {:?}", feed_url, e);
                        continue;
                    }
                };
                info!(
                    "Feed {} parsed: {} entities",
                    feed_url,
                    feed_message.entity.len()
                );

                // Process all stations from this single feed
                for station_config in station_configs {
                    if let Ok(Some(station)) = process_feed_single_station(
                        &feed_message,
                        &station_config.route,
                        &station_config.station_id,
                    ) {
                        combined_stations.push(station);
                    }
                }
            }

            // Update state
            if !combined_stations.is_empty() {
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
                mta.stations = combined_stations;
                mta.last_updated_secs = embassy_time::Instant::now().as_secs();
            }

            info!("MTA sleeping for 15 seconds...");
            Timer::after(Duration::from_secs(15)).await;
        }
    }

    fn render(&self, state: &mut MatrixHubState, display: &mut FrameBuffer) -> Result<()> {
        let mta = state.mta.as_ref();

        if mta.is_none() || mta.as_ref().map(|m| m.stations.is_empty()).unwrap_or(true) {
            // Show loading state with all configured routes
            let configured_routes: Vec<String> = state
                .config
                .as_ref()
                .and_then(|c| c.mta.as_ref())
                .map(|mta| mta.stations.iter().map(|s| s.route.clone()).collect())
                .unwrap_or_default();

            // Draw animated spinner
            let now_millis = embassy_time::Instant::now().as_millis();
            let spinner_frame = (now_millis / 150) % 8; // 8 frame animation, 150ms per frame
            let spinner_x = 7; // Center x to match route circles (1 + 6 radius)
            let spinner_y = 8; // Center y to match route circles (11 - 9 + 6 radius)

            // Pre-calculated dot positions in a circle (8 positions)
            let dot_positions = [
                (5, 0),   // Right
                (4, 3),   // Bottom-right
                (0, 5),   // Bottom
                (-4, 3),  // Bottom-left
                (-5, 0),  // Left
                (-4, -3), // Top-left
                (0, -5),  // Top
                (4, -3),  // Top-right
            ];

            // Draw spinning dots
            for i in 0..8 {
                let (dx, dy) = dot_positions[i];
                let x = spinner_x + dx;
                let y = spinner_y + dy;

                // Brighten the dot that's at the current frame position with a trailing effect
                let brightness = if i == spinner_frame as usize {
                    255
                } else if i == ((spinner_frame + 7) % 8) as usize {
                    128
                } else if i == ((spinner_frame + 6) % 8) as usize {
                    64
                } else {
                    32
                };

                let color = Color::new(brightness, brightness, brightness);
                Circle::new(Point::new(x - 1, y - 1), 2)
                    .into_styled(PrimitiveStyle::with_fill(color))
                    .draw(display)?;
            }

            // Draw route circles starting after spinner
            let circle_radius = 6;
            let circle_diameter = (circle_radius * 2) as i32;
            let circle_spacing = 3;
            let start_x = spinner_x + 6 + circle_spacing; // spinner center + spinner radius + spacing

            let mut current_x = start_x;
            let mut current_y = 11;

            for route in configured_routes.iter() {
                // Check if we need to wrap to next line
                if current_x + circle_diameter > COLS as i32 {
                    current_x = 1;
                    current_y += 16;
                }

                draw_train_circle(display, route, current_x, current_y)?;

                current_x += circle_diameter + circle_spacing;
            }
            return Ok(());
        }

        let now_millis = embassy_time::Instant::now().as_millis();

        let (station_idx, max_cycle_ms, scroll_elapsed_millis) = {
            let mta = state
                .mta
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("MTA state not initialized"))?;
            let idx = mta.current_station_index as usize;
            let station = &mta.stations[idx.min(mta.stations.len().saturating_sub(1))];

            // Calculate max scroll cycle
            let mut max_cycle = SCROLL_HOLD_START;
            for platform in &station.platforms {
                if let Some(train) = platform.trains.first() {
                    let dest = get_destination(&train.route, &platform.direction);
                    let text_len = (dest.len() * 7) as i32;
                    let avail = (COLS as i32) - (platform.trains.len().min(2) * 25) as i32 - 17;
                    if text_len > avail {
                        let scroll_distance = (text_len - avail) as u32;
                        let cycle = SCROLL_HOLD_START
                            + SCROLL_TIME_PER_PIXEL * scroll_distance
                            + SCROLL_HOLD_END;
                        max_cycle = if cycle > max_cycle { cycle } else { max_cycle };
                    }
                }
            }

            let elapsed = now_millis.saturating_sub(mta.scroll_start_secs * 1000);
            let max_cycle_ms = max_cycle.as_millis();
            if elapsed >= max_cycle_ms {
                let next_idx = (idx + 1) % mta.stations.len().max(1);
                if let Some(mta_mut) = state.mta.as_mut() {
                    mta_mut.scroll_start_secs = now_millis / 1000;
                    mta_mut.current_station_index = next_idx as u32;
                }
                (next_idx, max_cycle_ms, 0)
            } else {
                (idx, max_cycle_ms, elapsed)
            }
        };

        let mta = state
            .mta
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("MTA state not initialized"))?;
        let station = &mta.stations[station_idx.min(mta.stations.len().saturating_sub(1))];

        for (i, platform) in station.platforms.iter().enumerate() {
            if let Some(train) = platform.trains.first() {
                let times: Vec<String> = platform
                    .trains
                    .iter()
                    .take(2)
                    .map(|t| alloc::format!("{}m", t.arrives_in_secs / 60))
                    .collect();
                render_mta_row(
                    display,
                    11 + i as i32 * 16,
                    &train.route,
                    get_destination(&train.route, &platform.direction),
                    &times,
                    scroll_elapsed_millis,
                    max_cycle_ms,
                )?;
            }
        }

        Ok(())
    }
}

fn process_feed_single_station(
    feed: &FeedMessage,
    route: &str,
    station_prefix: &str,
) -> Result<Option<StationStatus>> {
    let max_minutes_ahead = 30i64;
    let now_secs = feed.header.timestamp.unwrap_or(0);

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

        for stop_time_update in &trip_update.stop_time_update {
            let Some(stop_id) = &stop_time_update.stop_id else {
                continue;
            };

            if !stop_id.starts_with(station_prefix) {
                continue;
            }

            let Some(arrival) = &stop_time_update.arrival else {
                continue;
            };
            let Some(arrival_time) = arrival.time else {
                continue;
            };
            let arrival_time = arrival_time as u64;

            if arrival_time > now_secs {
                let seconds_away = arrival_time - now_secs;
                if seconds_away / 60 <= max_minutes_ahead as u64 {
                    arrivals_map
                        .entry(stop_id.clone())
                        .or_insert_with(Vec::new)
                        .push((trip_route.to_string(), arrival_time));
                }
            }
        }
    }

    if arrivals_map.is_empty() {
        return Ok(None);
    }

    let mut platforms = Vec::new();
    for (stop_id, mut arrivals) in arrivals_map {
        let direction = stop_id.chars().last().unwrap_or('?').to_string();

        arrivals.sort_by_key(|(_, time)| *time);

        let trains: Vec<ProtoTrain> = arrivals
            .iter()
            .take(8)
            .map(|(r, arrival_time)| {
                let arrives_in_secs = if *arrival_time > now_secs {
                    arrival_time - now_secs
                } else {
                    0
                };

                ProtoTrain {
                    route: r.clone(),
                    arrives_at_secs: *arrival_time,
                    arrives_in_secs,
                }
            })
            .collect();

        platforms.push(ProtoPlat { direction, trains });
    }

    // Sort platforms to show N (northbound/westbound) first
    platforms.sort_by(|a, b| a.direction.cmp(&b.direction));

    Ok(Some(StationStatus {
        station_id: station_prefix.to_string(),
        platforms,
    }))
}
