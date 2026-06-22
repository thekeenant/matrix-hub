//! MTA real-time subway arrivals app.
//!
//! Fetches GTFS-RT JSON feeds from the MTA API in the background, parses them
//! into internal data structures, and renders a pixel-perfect arrival board
//! with colored train circles, scrolling destination text, and arrival times.

pub mod data;
pub mod feed;
pub mod render;
pub mod routes;
pub mod stops {
    include!(concat!(env!("OUT_DIR"), "/stops.rs"));
}

use crate::apps::App;
use crate::buffer::Framebuffer;
use crate::network::http;
use crate::task::TimerTask;

use data::{StationData, StationState};
use render::{compute_cycle_ms, render_station};
use routes::get_route_info;

use log::info;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ============================================================================
// Station Configuration
// ============================================================================

struct StationConfig {
    route: &'static str,
    stop_id: &'static str,
}

/// The stations to display. stop_id is the GTFS stop prefix (without N/S suffix).
/// Routes: 7 train (default feed), G (G feed), E (ACE feed), F (BDFM feed).
const STATION_CONFIGS: &[StationConfig] = &[
    StationConfig {
        route: "E",
        stop_id: "A27",
    },
    StationConfig {
        route: "G",
        stop_id: "G24",
    },
    StationConfig {
        route: "7",
        stop_id: "718",
    },
    // StationConfig { route: "1", stop_id: "120" },
    // StationConfig { route: "2", stop_id: "120" },
    // StationConfig { route: "3", stop_id: "120" },
    // StationConfig { route: "4", stop_id: "631" },
    // StationConfig { route: "5", stop_id: "631" },
    // StationConfig { route: "6", stop_id: "631" },
    // StationConfig { route: "A", stop_id: "A27" },
    // StationConfig { route: "C", stop_id: "A27" },
    // StationConfig { route: "B", stop_id: "D17" },
    // StationConfig { route: "D", stop_id: "D17" },
    // StationConfig { route: "F", stop_id: "D17" },
    // StationConfig { route: "M", stop_id: "D17" },
    // StationConfig { route: "N", stop_id: "R16" },
    // StationConfig { route: "Q", stop_id: "R16" },
    // StationConfig { route: "R", stop_id: "R16" },
    // StationConfig { route: "W", stop_id: "R16" },
    // StationConfig { route: "L", stop_id: "L08" },
    // StationConfig { route: "J", stop_id: "M11" },
    // StationConfig { route: "Z", stop_id: "M11" },
];

// Cycle through stations every N seconds if there is nothing to scroll
const MIN_STATION_DISPLAY_MS: f32 = 5000.0;

// ============================================================================
// MtaApp
// ============================================================================

pub struct MtaApp {
    fetch_task: Option<TimerTask<()>>,
    shared_stations: Arc<Mutex<(bool, Vec<StationData>)>>,
    stations: Vec<StationData>,
    current_idx: usize,
    scroll_elapsed_ms: f32,
    scroll_cycle_ms: f32,
    time_ms: f32, // for spinner animation
    is_connected: bool,
}

impl MtaApp {
    pub fn new() -> Self {
        let initial_stations: Vec<StationData> = STATION_CONFIGS
            .iter()
            .map(|c| StationData {
                route: c.route.to_string(),
                state: StationState::Loading,
            })
            .collect();

        Self {
            fetch_task: None,
            shared_stations: Arc::new(Mutex::new((
                false,
                initial_stations.clone(),
            ))),
            stations: initial_stations,
            current_idx: 0,
            scroll_elapsed_ms: 0.0,
            scroll_cycle_ms: MIN_STATION_DISPLAY_MS,
            time_ms: 0.0,
            is_connected: false,
        }
    }

    fn current_station(&self) -> &StationData {
        &self.stations
            [self.current_idx.min(self.stations.len().saturating_sub(1))]
    }

    fn advance_scroll(&mut self, dt_ms: f32) {
        self.scroll_elapsed_ms += dt_ms;
        if self.scroll_elapsed_ms >= self.scroll_cycle_ms {
            self.scroll_elapsed_ms = 0.0;
            self.current_idx =
                (self.current_idx + 1) % self.stations.len().max(1);
            self.recalculate_cycle();
        }
    }

    fn recalculate_cycle(&mut self) {
        let station = self.current_station();
        self.scroll_cycle_ms = match &station.state {
            StationState::Live(platforms) => {
                compute_cycle_ms(platforms).max(MIN_STATION_DISPLAY_MS)
            }
            _ => MIN_STATION_DISPLAY_MS,
        };
    }
}

impl App for MtaApp {
    fn set_network_status(&mut self, is_connected: bool, _ip: Option<String>) {
        self.is_connected = is_connected;
    }

    fn update(&mut self, dt_ms: f32) {
        self.time_ms += dt_ms;

        // Delay network fetch by 250ms and WAIT for WiFi connection
        if self.fetch_task.is_none()
            && self.time_ms > 250.0
            && self.is_connected
        {
            let shared = self.shared_stations.clone();
            self.fetch_task =
                Some(TimerTask::spawn(Duration::from_secs(60), move || {
                    fetch_all_stations(&shared);
                }));
        }

        // Poll shared background fetch for fresh data incrementally
        let mut should_recalculate = false;
        if let Ok(mut guard) = self.shared_stations.try_lock() {
            if guard.0 {
                // dirty flag
                self.stations = guard.1.clone();
                should_recalculate = true;
                guard.0 = false;
            }
        }

        if should_recalculate {
            self.recalculate_cycle();
        }

        self.advance_scroll(dt_ms);
    }

    fn draw(&self, fb: &mut Framebuffer) {
        let station = self.current_station();
        let scroll = render::ScrollState {
            elapsed_ms: self.scroll_elapsed_ms,
        };

        render_station(fb, &station.state, &station.route, &scroll);
    }
}

// ============================================================================
// Background Fetch Logic
// ============================================================================

fn fetch_all_stations(shared: &Arc<Mutex<(bool, Vec<StationData>)>>) {
    for (idx, config) in STATION_CONFIGS.iter().enumerate() {
        let feed_url = get_route_info(config.route).feed_url;
        info!(
            "MTA: fetching {} for route {} at {}",
            feed_url, config.route, config.stop_id
        );

        let station = match http::fetch_binary(feed_url) {
            Ok(data) => {
                let st = feed::process_station(
                    data.as_slice(),
                    config.route,
                    config.stop_id,
                );
                info!("MTA: parsed station {}: {:?}", config.stop_id, st.state);
                st
            }
            Err(e) => {
                info!("MTA: fetch error for {}: {}", config.route, e);
                StationData {
                    route: config.route.to_string(),
                    state: StationState::NoTrains,
                }
            }
        };

        if let Ok(mut guard) = shared.lock() {
            guard.1[idx] = station;
            guard.0 = true; // set dirty flag
        }
    }
}
