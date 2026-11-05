//! MTA real-time subway arrivals app.
//!
//! Fetches GTFS-RT protobuf feeds from the MTA API in the background, parses
//! them into internal data structures, and renders an arrival board with
//! colored route bullets, scrolling destinations, and arrival times.
//!
//! The platform supplies how feeds are downloaded ([`FeedSource`]) and how the
//! periodic background job runs ([`StartFetching`]).

pub mod data;
pub mod feed;
pub mod render;
#[cfg(test)]
mod render_golden;
pub mod routes;
pub mod stops {
    include!(concat!(env!("OUT_DIR"), "/stops.rs"));
}

use crate::apps::{App, Frame};
use crate::buffer::Framebuffer;

use data::{StationData, StationState};
use render::{compute_cycle_ms, render_station};
use routes::get_route_info;

use log::info;
use std::sync::{Arc, Mutex, PoisonError};
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

/// Don't start fetching until the app has been open this long, so quickly
/// swiping past it doesn't trigger a fetch
const FETCH_START_DELAY_MS: f32 = 250.0;

/// How often the background job refreshes every station
pub const FETCH_INTERVAL: Duration = Duration::from_secs(60);

// Keep showing a station's last good data through this many consecutive
// failed fetches (one per minute) before showing "No data"
const MAX_STALE_FETCHES: u32 = 3;

// ============================================================================
// Background fetching
// ============================================================================

/// Station data published by the background job for the app to pick up.
#[derive(Debug)]
pub struct SharedStations {
    /// Set when `stations` changed since the app last read it
    dirty: bool,
    stations: Vec<StationData>,
}

pub type Shared = Arc<Mutex<SharedStations>>;

/// Platform services the background fetch needs.
pub trait FeedSource {
    /// Download a GTFS-RT feed.
    fn fetch(&mut self, url: &str) -> anyhow::Result<Vec<u8>>;

    /// Hide trains arriving sooner than this (user setting).
    fn min_minutes(&self) -> u32;

    /// Progress hooks, e.g. for diagnostics.
    fn on_parse(&mut self) {}
    fn on_publish(&mut self) {}
}

/// Starts calling [`Fetcher::run`] (with the platform's [`FeedSource`]) now
/// and then every [`FETCH_INTERVAL`] in the background, until the returned
/// handle is dropped.
pub type StartFetching = fn(Fetcher) -> Box<dyn Send>;

/// One fetch-everything pass per call, remembering consecutive failures.
#[derive(Debug)]
pub struct Fetcher {
    shared: Shared,
    /// Consecutive failed fetches per station
    failures: Vec<u32>,
}

impl Fetcher {
    pub fn new(shared: Shared) -> Self {
        Self {
            shared,
            failures: vec![0; STATION_CONFIGS.len()],
        }
    }

    pub fn run(&mut self, source: &mut dyn FeedSource) {
        for (idx, config) in STATION_CONFIGS.iter().enumerate() {
            let feed_url = get_route_info(config.route).feed_url;
            info!(
                "MTA: fetching {feed_url} for route {} at {}",
                config.route, config.stop_id
            );

            let station = match source.fetch(feed_url) {
                Ok(data) => {
                    self.failures[idx] = 0;
                    source.on_parse();
                    let station = feed::process_station(
                        &data,
                        config.route,
                        config.stop_id,
                        source.min_minutes(),
                    );
                    info!(
                        "MTA: parsed station {}: {:?}",
                        config.stop_id, station.state
                    );
                    Some(station)
                }
                Err(e) => {
                    self.failures[idx] += 1;
                    let failures = self.failures[idx];
                    info!(
                        "MTA: fetch error for {} ({failures} in a row): {e:#}",
                        config.route
                    );
                    // Keep showing the last good data for a while
                    (failures > MAX_STALE_FETCHES).then(|| StationData {
                        route: config.route.to_string(),
                        state: StationState::Unavailable,
                    })
                }
            };

            source.on_publish();
            if let Some(station) = station {
                let mut shared =
                    self.shared.lock().unwrap_or_else(PoisonError::into_inner);
                shared.stations[idx] = station;
                shared.dirty = true;
            }
        }
    }
}

// ============================================================================
// MtaApp
// ============================================================================

pub struct MtaApp {
    start_fetching: StartFetching,
    /// Keeps the background job alive; dropping it stops fetching
    fetch_job: Option<Box<dyn Send>>,
    shared: Shared,
    stations: Vec<StationData>,
    current_idx: usize,
    scroll_elapsed_ms: f32,
    scroll_cycle_ms: f32,
    /// Time since the app was opened
    time_ms: f32,
}

impl MtaApp {
    pub fn new(start_fetching: StartFetching) -> Self {
        let stations: Vec<StationData> = STATION_CONFIGS
            .iter()
            .map(|c| StationData {
                route: c.route.to_string(),
                state: StationState::Loading,
            })
            .collect();

        Self {
            start_fetching,
            fetch_job: None,
            shared: Arc::new(Mutex::new(SharedStations {
                dirty: false,
                stations: stations.clone(),
            })),
            stations,
            current_idx: 0,
            scroll_elapsed_ms: 0.0,
            scroll_cycle_ms: MIN_STATION_DISPLAY_MS,
            time_ms: 0.0,
        }
    }

    fn current_station(&self) -> &StationData {
        &self.stations[self.current_idx]
    }

    fn advance_scroll(&mut self, dt_ms: f32) {
        self.scroll_elapsed_ms += dt_ms;
        if self.scroll_elapsed_ms >= self.scroll_cycle_ms {
            self.scroll_elapsed_ms = 0.0;
            self.current_idx = (self.current_idx + 1) % self.stations.len();
            self.recalculate_cycle();
        }
    }

    fn recalculate_cycle(&mut self) {
        let platforms = self.current_station().state.platforms();
        self.scroll_cycle_ms =
            compute_cycle_ms(platforms).max(MIN_STATION_DISPLAY_MS);
    }

    /// Adopt fresh data from the background job, if there is any. Never
    /// blocks: if the job holds the lock, try again next frame.
    fn take_fresh_data(&mut self) {
        let Ok(mut shared) = self.shared.try_lock() else {
            return;
        };
        if shared.dirty {
            shared.dirty = false;
            self.stations = shared.stations.clone();
            drop(shared);
            self.recalculate_cycle();
        }
    }
}

impl App for MtaApp {
    fn update(&mut self, frame: &Frame) {
        self.time_ms += frame.dt_ms;

        if self.fetch_job.is_none()
            && self.time_ms > FETCH_START_DELAY_MS
            && frame.network.is_connected()
        {
            let fetcher = Fetcher::new(Arc::clone(&self.shared));
            self.fetch_job = Some((self.start_fetching)(fetcher));
        }

        self.take_fresh_data();
        self.advance_scroll(frame.dt_ms);
    }

    fn draw(&self, fb: &mut Framebuffer) {
        let station = self.current_station();
        let scroll = render::ScrollState {
            elapsed_ms: self.scroll_elapsed_ms,
        };
        render_station(fb, &station.state, &station.route, &scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::mta::feed::tests::{feed, trip};
    use crate::input::Accel;
    use crate::net::NetworkStatus;
    use googletest::prelude::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::net::Ipv4Addr;

    thread_local! {
        /// The Fetcher handed to `capture_fetcher`, so tests can run it
        static STARTED: RefCell<Vec<Fetcher>> = const { RefCell::new(Vec::new()) };
    }

    fn capture_fetcher(fetcher: Fetcher) -> Box<dyn Send> {
        STARTED.with(|s| s.borrow_mut().push(fetcher));
        Box::new(())
    }

    fn started_count() -> usize {
        STARTED.with(|s| s.borrow().len())
    }

    fn run_fetcher(source: &mut dyn FeedSource) {
        // Evaluate log messages too (there's no logger, but formatting runs)
        log::set_max_level(log::LevelFilter::Trace);
        STARTED.with(|s| s.borrow_mut()[0].run(source));
    }

    /// A source relying on the default (no-op) progress hooks
    struct OfflineSource;

    impl FeedSource for OfflineSource {
        fn fetch(&mut self, _url: &str) -> anyhow::Result<Vec<u8>> {
            Ok(feed(vec![]))
        }
        fn min_minutes(&self) -> u32 {
            0
        }
    }

    /// Serves canned feeds; URLs without one fail.
    #[derive(Default)]
    struct FakeSource {
        feeds: HashMap<&'static str, Vec<u8>>,
        min_minutes: u32,
        hooks: Vec<&'static str>,
    }

    impl FeedSource for FakeSource {
        fn fetch(&mut self, url: &str) -> anyhow::Result<Vec<u8>> {
            self.feeds
                .get(url)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unreachable"))
        }
        fn min_minutes(&self) -> u32 {
            self.min_minutes
        }
        fn on_parse(&mut self) {
            self.hooks.push("parse");
        }
        fn on_publish(&mut self) {
            self.hooks.push("publish");
        }
    }

    fn frame(dt_ms: f32, connected: bool) -> Frame {
        Frame {
            dt_ms,
            network: if connected {
                NetworkStatus::Connected(Ipv4Addr::new(10, 0, 0, 2))
            } else {
                NetworkStatus::Connecting
            },
            accel: Accel::default(),
        }
    }

    fn states(app: &MtaApp) -> Vec<String> {
        app.stations
            .iter()
            .map(|s| match &s.state {
                StationState::Live(p) => {
                    format!("{} live({})", s.route, p.len())
                }
                other => format!("{} {other:?}", s.route),
            })
            .collect()
    }

    /// A source where only the G feed works
    fn g_feed_only() -> FakeSource {
        let g = feed(vec![trip(
            "G",
            "a",
            &[("G24N", Some(300)), ("T", Some(900))],
        )]);
        FakeSource {
            feeds: HashMap::from([(routes::FEED_G, g)]),
            ..FakeSource::default()
        }
    }

    #[gtest]
    fn waits_for_wifi_and_a_short_delay_before_fetching() {
        let mut app = MtaApp::new(capture_fetcher);
        app.update(&frame(300.0, false));
        expect_that!(started_count(), eq(0), "not connected");

        let mut app = MtaApp::new(capture_fetcher);
        app.update(&frame(100.0, true));
        expect_that!(started_count(), eq(0), "too soon");
        app.update(&frame(200.0, true));
        app.update(&frame(16.0, true));
        expect_that!(started_count(), eq(1), "started exactly once");
    }

    #[gtest]
    fn shows_fetched_data_on_the_next_frame() {
        let mut app = MtaApp::new(capture_fetcher);
        app.update(&frame(300.0, true));
        expect_that!(
            states(&app),
            elements_are![eq("E Loading"), eq("G Loading"), eq("7 Loading")]
        );

        let mut source = g_feed_only();
        run_fetcher(&mut source);
        app.update(&frame(16.0, true));
        expect_that!(states(&app), contains(eq("G live(1)")));
        expect_that!(
            source.hooks,
            elements_are![
                eq(&"publish"),
                eq(&"parse"),
                eq(&"publish"),
                eq(&"publish")
            ]
        );
    }

    #[gtest]
    fn keeps_stale_data_through_a_few_failures_then_shows_no_data() {
        let mut app = MtaApp::new(capture_fetcher);
        app.update(&frame(300.0, true));
        let mut source = g_feed_only();
        run_fetcher(&mut source);

        source.feeds.clear();
        for _ in 0..MAX_STALE_FETCHES {
            run_fetcher(&mut source);
        }
        app.update(&frame(16.0, true));
        // G has failed 3 times: still showing its last data. E (which also
        // failed on the first run) has failed 4 times: no data.
        expect_that!(states(&app), contains(eq("G live(1)")));
        expect_that!(states(&app), contains(eq("E Unavailable")));

        run_fetcher(&mut source);
        app.update(&frame(16.0, true));
        expect_that!(
            states(&app),
            elements_are![
                eq("E Unavailable"),
                eq("G Unavailable"),
                eq("7 Unavailable")
            ]
        );
    }

    #[gtest]
    fn a_success_resets_the_failure_count() {
        let mut app = MtaApp::new(capture_fetcher);
        app.update(&frame(300.0, true));
        let mut failing = FakeSource::default();
        for _ in 0..MAX_STALE_FETCHES {
            run_fetcher(&mut failing);
        }
        run_fetcher(&mut g_feed_only());
        for _ in 0..MAX_STALE_FETCHES {
            run_fetcher(&mut failing);
        }
        app.update(&frame(16.0, true));
        expect_that!(states(&app), contains(eq("G live(1)")));
    }

    #[gtest]
    fn progress_hooks_are_optional() {
        let mut app = MtaApp::new(capture_fetcher);
        app.update(&frame(300.0, true));
        run_fetcher(&mut OfflineSource);
        app.update(&frame(16.0, true));
        expect_that!(
            states(&app),
            elements_are![eq("E NoTrains"), eq("G NoTrains"), eq("7 NoTrains")]
        );
    }

    #[gtest]
    fn rotates_through_stations() {
        let mut app = MtaApp::new(capture_fetcher);
        expect_that!(app.current_station().route, eq("E"));
        app.update(&frame(MIN_STATION_DISPLAY_MS, false));
        expect_that!(app.current_station().route, eq("G"));
        app.update(&frame(MIN_STATION_DISPLAY_MS, false));
        app.update(&frame(MIN_STATION_DISPLAY_MS, false));
        expect_that!(app.current_station().route, eq("E"), "wraps around");
    }

    #[gtest]
    fn waits_for_fresh_data_while_the_fetcher_holds_the_lock() {
        let mut app = MtaApp::new(capture_fetcher);
        app.update(&frame(300.0, true));
        run_fetcher(&mut g_feed_only());

        let shared = Arc::clone(&app.shared);
        let guard = shared.lock().unwrap();
        app.update(&frame(16.0, true));
        expect_that!(states(&app), contains(eq("G Loading")), "didn't block");
        drop(guard);
        app.update(&frame(16.0, true));
        expect_that!(states(&app), contains(eq("G live(1)")));
    }

    #[gtest]
    fn draws_the_current_station() {
        let app = MtaApp::new(capture_fetcher);
        let mut fb = Framebuffer::new();
        app.draw(&mut fb);
        expect_that!(fb.lit_pixels().count(), gt(0));
    }
}
