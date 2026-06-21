// Parses a raw GTFS-RT FeedMessage into internal StationData.
// All MTA-specific quirks (trip_id route extraction, terminal skipping, etc.) live here.

use crate::apps::mta::data::{Platform, StationData, StationState, Train};
use crate::proto::transit_realtime::FeedMessage;
use std::collections::BTreeMap;

const MAX_MINUTES_AHEAD: u64 = 30;
const MAX_TRAINS_PER_PLATFORM: usize = 8;

struct RawArrival {
    route: String,
    arrival_time: u64,
    terminal_stop_id: String,
}

/// Process a single station from a parsed feed.
pub fn process_station(
    feed: &FeedMessage,
    route: &str,
    station_id: &str,
) -> StationData {
    let now_secs = feed.header.timestamp.unwrap_or(0);
    let arrivals = collect_arrivals(feed, route, station_id, now_secs);

    let state = if arrivals.is_empty() {
        StationState::NoTrains
    } else {
        StationState::Live(build_platforms(arrivals, now_secs))
    };

    StationData {
        route: route.to_string(),
        state,
    }
}

/// Extracts the route ID from an MTA GTFS-RT trip_id when `route_id` is absent.
///
/// The MTA default feed (`nyct/gtfs`) encodes route info inside `trip_id` as:
///   `"HHMMSS_ROUTE..DIRECTION"` — e.g. `"042800_7..S08R"` → `"7"`
fn route_from_trip_id(trip_id: &str) -> String {
    if let Some(underscore_pos) = trip_id.find('_') {
        let after = &trip_id[underscore_pos + 1..];
        if let Some(dotdot_pos) = after.find("..") {
            return after[..dotdot_pos].to_string();
        }
    }
    String::new()
}

fn collect_arrivals(
    feed: &FeedMessage,
    route: &str,
    station_prefix: &str,
    now_secs: u64,
) -> BTreeMap<String, Vec<RawArrival>> {
    let mut arrivals: BTreeMap<String, Vec<RawArrival>> = BTreeMap::new();

    for entity in &feed.entity {
        let trip_update = &entity.trip_update;
        let trip = &trip_update.trip;

        // Fall back to extracting route from trip_id if route_id is missing (MTA default feed quirk)
        let route_owned;
        let trip_route = match trip.route_id.as_deref() {
            Some(r) if !r.is_empty() => r,
            _ => {
                route_owned =
                    route_from_trip_id(trip.trip_id.as_deref().unwrap_or(""));
                route_owned.as_str()
            }
        };

        if !trip_route.starts_with(route) {
            continue;
        }

        // Identify the terminal (last stop) — skip arrivals at terminal stops
        let last_stop = trip_update
            .stop_time_update
            .last()
            .and_then(|s| s.stop_id.as_deref());

        for stu in &trip_update.stop_time_update {
            let Some(stop_id) = stu.stop_id.as_deref() else {
                continue;
            };

            if !stop_id.starts_with(station_prefix) {
                continue;
            }
            if Some(stop_id) == last_stop {
                continue;
            } // skip terminal

            let arrival = &stu.arrival;
            let Some(arrival_time) =
                arrival.time.and_then(|t| u64::try_from(t).ok())
            else {
                continue;
            };

            if arrival_time <= now_secs {
                continue;
            }

            let secs_away = arrival_time - now_secs;
            if secs_away / 60 > MAX_MINUTES_AHEAD {
                continue;
            }

            arrivals
                .entry(stop_id.to_string())
                .or_default()
                .push(RawArrival {
                    route: trip_route.to_string(),
                    arrival_time,
                    terminal_stop_id: last_stop.unwrap_or("").to_string(),
                });
        }
    }

    arrivals
}

fn build_platforms(
    arrivals: BTreeMap<String, Vec<RawArrival>>,
    now_secs: u64,
) -> Vec<Platform> {
    let mut platforms: Vec<Platform> = arrivals
        .into_iter()
        .map(|(stop_id, mut trains)| {
            let direction = stop_id.chars().last().unwrap_or('?').to_string();
            trains.sort_by_key(|arr| arr.arrival_time);

            let trains = trains
                .into_iter()
                .take(MAX_TRAINS_PER_PLATFORM)
                .map(|arr| Train {
                    route: arr.route,
                    arrives_in_secs: arr.arrival_time.saturating_sub(now_secs),
                    terminal_stop_id: arr.terminal_stop_id,
                })
                .collect();

            Platform { direction, trains }
        })
        .collect();

    // N/W (northbound/westbound) first
    platforms.sort_by(|a, b| a.direction.cmp(&b.direction));
    platforms
}
