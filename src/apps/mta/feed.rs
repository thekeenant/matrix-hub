// Parses a raw GTFS-RT FeedMessage into internal StationData.
// All MTA-specific quirks (trip_id route extraction, terminal skipping, etc.) live here.

use crate::apps::mta::data::{Platform, StationData, StationState, Train};
use crate::proto::transit_realtime::{FeedEntity, FeedHeader};
use buffa::Message;
use std::collections::BTreeMap;

const MAX_MINUTES_AHEAD: u64 = 30;
const MAX_TRAINS_PER_PLATFORM: usize = 8;

struct RawArrival {
    route: String,
    arrival_time: u64,
    terminal_stop_id: String,
}

/// A lightweight streaming parser for raw protobuf bytes to avoid loading
/// huge ASTs (like the 2MB+ MTA feed) into our constrained memory.
pub enum WireField<'a> {
    Varint(()),
    Fixed64(()),
    LengthDelimited(&'a [u8]),
    Fixed32(()),
}

pub struct ProtobufStream<'a> {
    buf: &'a [u8],
}

impl<'a> ProtobufStream<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }
}

impl<'a> Iterator for ProtobufStream<'a> {
    type Item = (u32, WireField<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        let (tag_wire, new_buf) = read_varint(self.buf)?;
        self.buf = new_buf;

        let wire_type = tag_wire & 7;
        let field_num = (tag_wire >> 3) as u32;

        match wire_type {
            0 => {
                let (_, new_buf) = read_varint(self.buf)?;
                self.buf = new_buf;
                Some((field_num, WireField::Varint(())))
            }
            1 => {
                if self.buf.len() < 8 {
                    return None;
                }
                self.buf = &self.buf[8..];
                Some((field_num, WireField::Fixed64(())))
            }
            2 => {
                let (len, new_buf) = read_varint(self.buf)?;
                if new_buf.len() < len as usize {
                    return None;
                }
                let val = &new_buf[..len as usize];
                self.buf = &new_buf[len as usize..];
                Some((field_num, WireField::LengthDelimited(val)))
            }
            5 => {
                if self.buf.len() < 4 {
                    return None;
                }
                self.buf = &self.buf[4..];
                Some((field_num, WireField::Fixed32(())))
            }
            _ => None,
        }
    }
}

fn read_varint(buf: &[u8]) -> Option<(u64, &[u8])> {
    let mut value = 0;
    let mut shift = 0;
    for (i, &b) in buf.iter().enumerate() {
        value |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((value, &buf[i + 1..]));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

/// Process a single station from a parsed feed stream.
pub fn process_station(
    data: &[u8],
    route: &str,
    station_id: &str,
) -> StationData {
    let mut now_secs = 0;
    let mut arrivals = BTreeMap::new();

    for (field_num, field) in ProtobufStream::new(data) {
        if let WireField::LengthDelimited(mut value) = field {
            if field_num == 1 {
                if let Ok(header) = FeedHeader::decode(&mut value) {
                    now_secs = header.timestamp.unwrap_or(0);
                }
            } else if field_num == 2 {
                if let Ok(entity) = FeedEntity::decode(&mut value) {
                    process_entity(
                        &entity,
                        route,
                        station_id,
                        now_secs,
                        &mut arrivals,
                    );
                }
            }
        }
    }

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

fn process_entity(
    entity: &FeedEntity,
    route: &str,
    station_prefix: &str,
    now_secs: u64,
    arrivals: &mut BTreeMap<String, Vec<RawArrival>>,
) {
    let trip_update = &entity.trip_update;
    let trip = &trip_update.trip;

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
        return;
    }

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
        }

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
