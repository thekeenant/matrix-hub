//! Golden-image regression tests for the MTA renderer: each scenario's
//! framebuffer is hashed and compared with the hash recorded when the
//! layout was last deliberately changed. A mismatch means pixels changed.

use crate::apps::mta::data::{Platform, StationState, Train};
use crate::apps::mta::render::{ScrollState, render_station};
use crate::buffer::Framebuffer;
use embedded_graphics::prelude::RgbColor;

/// FNV-1a over the RGB bytes (stable across Rust versions, unlike
/// DefaultHasher)
pub fn fingerprint(fb: &Framebuffer) -> u64 {
    fb.pixels
        .iter()
        .flat_map(|p| [p.r(), p.g(), p.b()])
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}

fn train(route: &str, minutes: u64, terminal: &str) -> Train {
    Train {
        route: route.into(),
        arrives_in_secs: minutes * 60 + 7,
        terminal_stop_id: terminal.into(),
    }
}

fn platform(direction: &str, trains: Vec<Train>) -> Platform {
    Platform {
        direction: direction.into(),
        trains,
    }
}

/// Three arrivals toward a destination too long to fit: it scrolls
fn long_destination() -> StationState {
    StationState::Live(vec![platform(
        "S",
        vec![
            train("A", 3, "H11S"),
            train("A", 12, "H11S"),
            train("A", 25, "H11S"),
        ],
    )])
}

/// (name, state, route, scroll elapsed ms)
pub fn scenarios() -> Vec<(&'static str, StationState, &'static str, f32)> {
    let g_north =
        || platform("N", vec![train("G", 3, "G22N"), train("G", 11, "G22N")]);
    vec![
        ("loading", StationState::Loading, "E", 0.0),
        ("no_trains", StationState::NoTrains, "G", 0.0),
        ("unavailable", StationState::Unavailable, "7", 0.0),
        (
            "one_time",
            StationState::Live(vec![platform(
                "N",
                vec![train("G", 4, "G22N")],
            )]),
            "G",
            0.0,
        ),
        ("two_times", StationState::Live(vec![g_north()]), "G", 0.0),
        (
            "three_times_and_two_platforms",
            StationState::Live(vec![
                platform(
                    "N",
                    vec![
                        train("E", 2, "G05N"),
                        train("E", 9, "G05N"),
                        train("E", 15, "G05N"),
                    ],
                ),
                platform(
                    "S",
                    vec![train("E", 6, "E01S"), train("E", 21, "E01S")],
                ),
            ]),
            "E",
            0.0,
        ),
        (
            "markers_on_big_second_time",
            StationState::Live(vec![platform(
                "N",
                vec![train("7", 1, "701N"), train("7X", 4, "726N")],
            )]),
            "7",
            0.0,
        ),
        (
            "markers_on_small_times",
            StationState::Live(vec![platform(
                "S",
                vec![
                    train("7", 2, "726S"),
                    train("7", 5, "712S"),
                    train("7", 8, "702S"),
                ],
            )]),
            "7",
            0.0,
        ),
        (
            "marker_on_first_time",
            StationState::Live(vec![platform(
                "N",
                vec![train("G", 2, ""), train("G", 30, "G22N")],
            )]),
            "G",
            0.0,
        ),
        (
            "express_diamond_bold",
            StationState::Live(vec![platform(
                "N",
                vec![train("7X", 3, "701N")],
            )]),
            "7",
            0.0,
        ),
        (
            "express_diamond_regular",
            StationState::Live(vec![platform(
                "N",
                vec![train("QX", 3, "D43N")],
            )]),
            "Q",
            0.0,
        ),
        (
            "regular_letters_circle",
            StationState::Live(vec![platform(
                "N",
                vec![train("N", 3, "R01N")],
            )]),
            "N",
            0.0,
        ),
        (
            "fallback_destination",
            StationState::Live(vec![platform("S", vec![train("G", 3, "")])]),
            "G",
            0.0,
        ),
        (
            "unknown_terminal_falls_back",
            StationState::Live(vec![platform(
                "S",
                vec![train("G", 3, "ZZZ9S")],
            )]),
            "G",
            0.0,
        ),
        (
            "terminal_without_direction_suffix",
            StationState::Live(vec![platform("N", vec![train("G", 3, "G22")])]),
            "G",
            0.0,
        ),
        (
            "empty_platform_is_skipped",
            StationState::Live(vec![platform("N", vec![]), g_north()]),
            "G",
            0.0,
        ),
        ("long_destination_hold", long_destination(), "A", 0.0),
        (
            "long_destination_mid_scroll",
            long_destination(),
            "A",
            3000.0,
        ),
        ("long_destination_end", long_destination(), "A", 60_000.0),
    ]
}

pub fn render(
    state: &StationState,
    route: &str,
    elapsed_ms: f32,
) -> Framebuffer {
    let mut fb = Framebuffer::new();
    render_station(&mut fb, state, route, &ScrollState { elapsed_ms });
    fb
}

/// Recorded from the renderer before the cleanup refactor; update only
/// when changing the layout on purpose (run with `--nocapture` to see the
/// new values).
const GOLDEN: &[(&str, u64)] = &[
    ("loading", 0x4383a3d0874c2a59),
    ("no_trains", 0x69fbf362ba9c32d7),
    ("unavailable", 0xae39682554e52b3e),
    ("one_time", 0x80315558b614b89b),
    ("two_times", 0x07729e112d2abe25),
    ("three_times_and_two_platforms", 0x0593715872b086e1),
    ("markers_on_big_second_time", 0x0b1cadd7b02b55c1),
    ("markers_on_small_times", 0x016d494c554c5e17),
    ("marker_on_first_time", 0x3441c6fb7df8d172),
    ("express_diamond_bold", 0x30035bfb866d2151),
    ("express_diamond_regular", 0x40a8fac6b82b3443),
    ("regular_letters_circle", 0x27aa1efe08d02dde),
    ("fallback_destination", 0xddd3231da918e577),
    ("unknown_terminal_falls_back", 0xddd3231da918e577),
    ("terminal_without_direction_suffix", 0x940f612c08b50e44),
    ("empty_platform_is_skipped", 0x50b42fd6530abe25),
    ("long_destination_hold", 0xcbce388b0f33f78f),
    ("long_destination_mid_scroll", 0xcb6d9d8b15367026),
    ("long_destination_end", 0x9b1904e3f3101372),
];

#[googletest::prelude::gtest]
fn renders_every_scenario_pixel_for_pixel() {
    use googletest::prelude::*;
    let expected: std::collections::HashMap<_, _> =
        GOLDEN.iter().copied().collect();
    for (name, state, route, elapsed) in scenarios() {
        let actual = fingerprint(&render(&state, route, elapsed));
        println!("(\"{name}\", 0x{actual:016x}),");
        expect_that!(expected.get(name), some(eq(&actual)), "scenario {name}");
    }
}
