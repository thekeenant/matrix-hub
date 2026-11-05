// MTA route metadata: colors, feed URLs, and destination name lookup.

use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::RgbColor;

// ============================================================================
// Feed URLs — append ".json" for JSON format
// ============================================================================

pub const FEED_DEFAULT: &str =
    "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs";
pub const FEED_ACE: &str =
    "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-ace";
pub const FEED_BDFM: &str =
    "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-bdfm";
pub const FEED_G: &str =
    "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-g";
pub const FEED_JZ: &str =
    "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-jz";
pub const FEED_L: &str =
    "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-l";
pub const FEED_NQRW: &str =
    "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-nqrw";

// ============================================================================
// Route Metadata
// ============================================================================

pub struct RouteInfo {
    pub color: Rgb888,
    pub letter_color: Rgb888,
    pub is_bold: bool,
    pub feed_url: &'static str,
}

pub fn get_route_info(route: &str) -> RouteInfo {
    match route {
        "1" | "2" | "3" => RouteInfo {
            color: Rgb888::new(0x8C, 0x0C, 0x0C),
            letter_color: Rgb888::WHITE,
            is_bold: true,
            feed_url: FEED_DEFAULT,
        },
        "4" | "5" | "6" => RouteInfo {
            color: Rgb888::new(0x00, 0x6B, 0x39),
            letter_color: Rgb888::WHITE,
            is_bold: true,
            feed_url: FEED_DEFAULT,
        },
        "7" | "7X" => RouteInfo {
            color: Rgb888::new(0x6A, 0x1A, 0x72),
            letter_color: Rgb888::WHITE,
            is_bold: true,
            feed_url: FEED_DEFAULT,
        },
        "A" | "C" | "E" => RouteInfo {
            color: Rgb888::new(0x00, 0x2D, 0x72),
            letter_color: Rgb888::WHITE,
            is_bold: true,
            feed_url: FEED_ACE,
        },
        "B" | "D" | "F" | "M" => RouteInfo {
            color: Rgb888::new(0xCC, 0x33, 0x00),
            letter_color: Rgb888::WHITE,
            is_bold: true,
            feed_url: FEED_BDFM,
        },
        "G" => RouteInfo {
            color: Rgb888::new(0x00, 0x8C, 0x00),
            letter_color: Rgb888::WHITE,
            is_bold: true,
            feed_url: FEED_G,
        },
        "L" => RouteInfo {
            color: Rgb888::new(0x30, 0x30, 0x30),
            letter_color: Rgb888::WHITE,
            is_bold: true,
            feed_url: FEED_L,
        },
        "N" | "Q" | "R" | "W" => RouteInfo {
            color: Rgb888::new(0x98, 0x78, 0x06),
            letter_color: Rgb888::BLACK,
            is_bold: false,
            feed_url: FEED_NQRW,
        },
        "J" | "Z" => RouteInfo {
            color: Rgb888::new(0x40, 0x2A, 0x15),
            letter_color: Rgb888::WHITE,
            is_bold: true,
            feed_url: FEED_JZ,
        },
        _ => RouteInfo {
            color: Rgb888::new(0x20, 0x20, 0x20),
            letter_color: Rgb888::WHITE,
            is_bold: true,
            feed_url: FEED_DEFAULT,
        },
    }
}

// ============================================================================
// Destination Lookup — same table as v0
// ============================================================================

pub fn get_destination<'a>(route: &str, direction: &'a str) -> &'a str {
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
        ("6", "N", "Pelham Bay"),
        ("6", "S", "Brooklyn Bridge"),
        ("7", "N", "Flushing-Main St"),
        ("7", "S", "34 St-Hudson Yds"),
        ("7X", "N", "Flushing-Main St"),
        ("7X", "S", "34 St-Hudson Yds"),
        ("A", "N", "Inwood-207 St"),
        ("A", "S", "Far Rockaway"),
        ("C", "N", "168 St"),
        ("C", "S", "Euclid Ave"),
        ("E", "N", "Jamaica Ctr"),
        ("E", "S", "World Trade Ctr"),
        ("B", "N", "Bedford Pk Blvd"),
        ("B", "S", "Brighton Beach"),
        ("D", "N", "Norwood-205 St"),
        ("D", "S", "Coney Island"),
        ("F", "N", "Jamaica-179 St"),
        ("F", "S", "Coney Island"),
        ("M", "N", "Forest Hills-71 Ave"),
        ("M", "S", "Middle Village"),
        ("N", "N", "Astoria-Ditmars Blvd"),
        ("N", "S", "Coney Island"),
        ("Q", "N", "96 St-2 Ave"),
        ("Q", "S", "Coney Island"),
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
        .map_or_else(
            || if route == "S" { "Shuttle" } else { direction },
            |(_, _, dest)| *dest,
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use googletest::prelude::*;

    #[gtest]
    fn routes_map_to_their_trunk_line_colors_and_feeds() {
        let cases = [
            ("1", 0x8C0C0C, FEED_DEFAULT),
            ("5", 0x006B39, FEED_DEFAULT),
            ("7X", 0x6A1A72, FEED_DEFAULT),
            ("E", 0x002D72, FEED_ACE),
            ("F", 0xCC3300, FEED_BDFM),
            ("G", 0x008C00, FEED_G),
            ("L", 0x303030, FEED_L),
            ("Q", 0x987806, FEED_NQRW),
            ("Z", 0x402A15, FEED_JZ),
            ("unknown", 0x202020, FEED_DEFAULT),
        ];
        for (route, rgb, feed) in cases {
            let info = get_route_info(route);
            let [_, r, g, b] = u32::to_be_bytes(rgb);
            expect_that!(info.color, eq(Rgb888::new(r, g, b)), "route {route}");
            expect_that!(info.feed_url, eq(feed), "route {route}");
        }
    }

    #[gtest]
    fn yellow_lines_use_black_regular_lettering() {
        let n = get_route_info("N");
        expect_that!(n.letter_color, eq(Rgb888::BLACK));
        expect_false!(n.is_bold);
        let a = get_route_info("A");
        expect_that!(a.letter_color, eq(Rgb888::WHITE));
        expect_true!(a.is_bold);
    }

    #[gtest]
    fn destinations_fall_back_to_shuttle_or_the_direction() {
        expect_that!(get_destination("G", "N"), eq("Court Sq"));
        expect_that!(get_destination("J", "W"), eq("Broad St"));
        expect_that!(get_destination("S", "N"), eq("Shuttle"));
        expect_that!(get_destination("G", "E"), eq("E"));
    }
}
