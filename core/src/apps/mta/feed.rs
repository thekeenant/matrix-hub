//! Parses a raw GTFS-RT FeedMessage into internal StationData. All
//! MTA-specific quirks (route from trip_id, terminal skipping, etc.) live here.

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

/// One top-level protobuf field; only length-delimited payloads are kept.
#[derive(Debug, PartialEq, Eq)]
enum WireField<'a> {
    Varint,
    Fixed64,
    LengthDelimited(&'a [u8]),
    Fixed32,
}

/// Walks the top-level fields of a protobuf message without decoding it, so
/// a feed's entities can be decoded one at a time instead of all at once
/// (feeds are 100KB+ of protobuf, several times that once decoded).
struct ProtobufStream<'a> {
    buf: &'a [u8],
}

impl<'a> ProtobufStream<'a> {
    const fn new(buf: &'a [u8]) -> Self {
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
                Some((field_num, WireField::Varint))
            }
            1 => {
                if self.buf.len() < 8 {
                    return None;
                }
                self.buf = &self.buf[8..];
                Some((field_num, WireField::Fixed64))
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
                Some((field_num, WireField::Fixed32))
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

/// Arrivals for one route at one station, from a raw GTFS-RT feed. Trains
/// arriving within `min_minutes` are left out.
pub fn process_station(
    data: &[u8],
    route: &str,
    station_id: &str,
    min_minutes: u32,
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

    // Decide "no trains" only after filtering, so a min_minutes filter that
    // removes every train shows "No Trains" rather than a blank board.
    let platforms = build_platforms(arrivals, now_secs, min_minutes);
    let state = if platforms.is_empty() {
        StationState::NoTrains
    } else {
        StationState::Live(platforms)
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

    // Match the route itself or its express variant ("7" matches "7X"), but
    // not other routes sharing a prefix (e.g. "G" vs the "GS" shuttle).
    if trip_route != route && trip_route.strip_suffix('X') != Some(route) {
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
    min_minutes: u32,
) -> Vec<Platform> {
    let min_secs = u64::from(min_minutes) * 60;

    let mut platforms: Vec<Platform> = arrivals
        .into_iter()
        .map(|(stop_id, mut trains)| {
            let direction = stop_id.chars().last().unwrap_or('?').to_string();
            trains.sort_by_key(|arr| arr.arrival_time);

            let trains = trains
                .into_iter()
                .filter(|arr| {
                    arr.arrival_time.saturating_sub(now_secs) >= min_secs
                })
                .take(MAX_TRAINS_PER_PLATFORM)
                .map(|arr| Train {
                    route: arr.route,
                    arrives_in_secs: arr.arrival_time.saturating_sub(now_secs),
                    terminal_stop_id: arr.terminal_stop_id,
                })
                .collect();

            Platform { direction, trains }
        })
        .filter(|p| !p.trains.is_empty())
        .collect();

    // N/W (northbound/westbound) first
    platforms.sort_by(|a, b| a.direction.cmp(&b.direction));
    platforms
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::proto::transit_realtime::{
        FeedMessage, TripDescriptor, TripUpdate,
        trip_update::{StopTimeEvent, StopTimeUpdate},
    };
    use googletest::prelude::*;

    pub(crate) const NOW: u64 = 1_700_000_000;

    /// A trip stopping at `stops` (stop_id, seconds from NOW; `None` for no
    /// arrival time). The last stop is the trip's terminal.
    pub(crate) fn trip(
        route_id: &str,
        trip_id: &str,
        stops: &[(&str, Option<i64>)],
    ) -> FeedEntity {
        FeedEntity {
            trip_update: buffa::MessageField::some(TripUpdate {
                trip: buffa::MessageField::some(TripDescriptor {
                    route_id: Some(route_id.into()),
                    trip_id: Some(trip_id.into()),
                    ..Default::default()
                }),
                stop_time_update: stops
                    .iter()
                    .map(|&(stop_id, secs)| StopTimeUpdate {
                        stop_id: (!stop_id.is_empty()).then(|| stop_id.into()),
                        arrival: buffa::MessageField::some(StopTimeEvent {
                            time: secs.map(|s| NOW as i64 + s),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    pub(crate) fn feed(entities: Vec<FeedEntity>) -> Vec<u8> {
        FeedMessage {
            header: buffa::MessageField::some(FeedHeader {
                timestamp: Some(NOW),
                ..Default::default()
            }),
            entity: entities,
            ..Default::default()
        }
        .encode_to_vec()
    }

    /// (route, minutes away, terminal)
    type TrainSummary = (String, u64, String);

    /// Live arrivals as (direction, trains); empty without live data.
    fn arrivals(
        data: &[u8],
        route: &str,
        min_minutes: u32,
    ) -> Vec<(String, Vec<TrainSummary>)> {
        let station = process_station(data, route, "G24", min_minutes);
        station
            .state
            .platforms()
            .iter()
            .map(|p| {
                let trains = p
                    .trains
                    .iter()
                    .map(|t| {
                        let minutes = t.arrives_in_secs / 60;
                        (t.route.clone(), minutes, t.terminal_stop_id.clone())
                    })
                    .collect();
                (p.direction.clone(), trains)
            })
            .collect()
    }

    fn train(
        route: &str,
        minutes: u64,
        terminal: &str,
    ) -> (String, u64, String) {
        (route.into(), minutes, terminal.into())
    }

    #[gtest]
    fn groups_arrivals_by_platform_sorted_by_time() {
        let data = feed(vec![
            trip("G", "a", &[("G24N", Some(600)), ("G22N", Some(900))]),
            trip("G", "b", &[("G24N", Some(180)), ("G22N", Some(400))]),
            trip("G", "c", &[("G24S", Some(300)), ("F27S", Some(700))]),
        ]);
        expect_that!(
            arrivals(&data, "G", 0),
            elements_are![
                eq(&(
                    "N".to_string(),
                    vec![train("G", 3, "G22N"), train("G", 10, "G22N")]
                )),
                eq(&("S".to_string(), vec![train("G", 5, "F27S")])),
            ]
        );
    }

    #[gtest]
    fn matches_express_variants_but_not_other_routes() {
        let data = feed(vec![
            trip("7", "a", &[("G24N", Some(60)), ("T", Some(99))]),
            trip("7X", "b", &[("G24N", Some(120)), ("T", Some(99))]),
            trip("75", "c", &[("G24N", Some(180)), ("T", Some(99))]),
        ]);
        expect_that!(
            arrivals(&data, "7", 0),
            elements_are![eq(&(
                "N".to_string(),
                vec![train("7", 1, "T"), train("7X", 2, "T")]
            ))]
        );
    }

    #[gtest]
    fn falls_back_to_the_route_in_the_trip_id() {
        let data = feed(vec![
            trip("", "042800_G..N08R", &[("G24N", Some(60)), ("T", Some(99))]),
            trip("", "no-underscore", &[("G24N", Some(120)), ("T", Some(99))]),
            trip("", "0428_no-dots", &[("G24N", Some(180)), ("T", Some(99))]),
        ]);
        expect_that!(
            arrivals(&data, "G", 0),
            elements_are![eq(&("N".to_string(), vec![train("G", 1, "T")]))]
        );
    }

    #[gtest]
    fn skips_stops_that_cannot_be_shown() {
        let data = feed(vec![
            trip(
                "G",
                "a",
                &[
                    ("", Some(60)),          // no stop id
                    ("A27N", Some(60)),      // another station
                    ("G24N", None),          // no arrival time
                    ("G24N", Some(-60)),     // already left
                    ("G24N", Some(0)),       // arriving right now
                    ("G24N", Some(31 * 60)), // too far ahead
                    ("G24N", Some(120)),     // shown
                    ("T", Some(99)),
                ],
            ),
            // Trains terminating here aren't "arriving" for riders
            trip("G", "b", &[("X", Some(30)), ("G24S", Some(240))]),
        ]);
        expect_that!(
            arrivals(&data, "G", 0),
            elements_are![eq(&("N".to_string(), vec![train("G", 2, "T")]))]
        );
    }

    #[gtest]
    fn hides_trains_sooner_than_min_minutes() {
        let data = feed(vec![
            trip("G", "a", &[("G24N", Some(90)), ("T", Some(99))]),
            trip("G", "b", &[("G24N", Some(300)), ("T", Some(999))]),
        ]);
        expect_that!(
            arrivals(&data, "G", 2),
            elements_are![eq(&("N".to_string(), vec![train("G", 5, "T")]))]
        );
    }

    #[gtest]
    fn shows_at_most_eight_trains_per_platform() {
        let trips = (1..=10)
            .map(|m| {
                trip("G", "t", &[("G24N", Some(m * 60)), ("T", Some(9999))])
            })
            .collect();
        let platforms = arrivals(&feed(trips), "G", 0);
        expect_that!(platforms[0].1.len(), eq(MAX_TRAINS_PER_PLATFORM));
    }

    #[gtest]
    fn reports_no_trains_when_everything_is_filtered_out() {
        let data =
            feed(vec![trip("G", "a", &[("G24N", Some(60)), ("T", Some(99))])]);
        let station = process_station(&data, "G", "G24", 5);
        expect_that!(station.route, eq("G"));
        expect_that!(station.state, matches_pattern!(StationState::NoTrains));
    }

    #[gtest]
    fn ignores_undecodable_fields() {
        // Garbage and unknown fields before one valid entity
        let mut data = vec![
            0x0A, 0x02, 0xFF, 0xFF, // field 1 (header), undecodable
            0x12, 0x02, 0xFF, 0xFF, // field 2 (entity), undecodable
            0x1A, 0x00, // field 3, unknown
            0x20, 0x01, // field 4, a varint
        ];
        data.extend(feed(vec![trip(
            "G",
            "a",
            &[("G24N", Some(60)), ("T", Some(99))],
        )]));
        expect_that!(arrivals(&data, "G", 0).len(), eq(1));
    }

    #[gtest]
    fn streams_every_wire_type() {
        let data = [
            0x08, 0x96, 0x01, // field 1, varint 150
            0x11, 1, 2, 3, 4, 5, 6, 7, 8, // field 2, fixed64
            0x1A, 0x02, b'h', b'i', // field 3, 2 bytes
            0x25, 1, 2, 3, 4, // field 4, fixed32
        ];
        let fields: Vec<_> = ProtobufStream::new(&data).collect();
        expect_that!(
            fields,
            elements_are![
                eq(&(1, WireField::Varint)),
                eq(&(2, WireField::Fixed64)),
                eq(&(3, WireField::LengthDelimited(b"hi"))),
                eq(&(4, WireField::Fixed32)),
            ]
        );
    }

    #[gtest]
    fn stops_streaming_at_malformed_data() {
        let first_then = |rest: &[u8]| {
            let mut data = vec![0x08, 0x01];
            data.extend_from_slice(rest);
            ProtobufStream::new(&data).count()
        };
        expect_that!(first_then(&[0x08]), eq(1), "varint value missing");
        expect_that!(first_then(&[0x11, 1, 2]), eq(1), "fixed64 truncated");
        expect_that!(first_then(&[0x1A, 0x05, 1]), eq(1), "length beyond data");
        expect_that!(first_then(&[0x1A]), eq(1), "length missing");
        expect_that!(first_then(&[0x25, 1]), eq(1), "fixed32 truncated");
        expect_that!(
            first_then(&[0x0B]),
            eq(1),
            "group wire types unsupported"
        );
        expect_that!(first_then(&[0xFF; 11]), eq(1), "varint over 64 bits");
    }
}
