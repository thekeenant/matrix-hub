//! Network status screen, including setup instructions when the setup access
//! point is up.

use crate::apps::{App, Frame};
use crate::buffer::Framebuffer;
use crate::config::{HEIGHT, WIDTH};
use crate::net::NetworkStatus;
use embedded_graphics::mono_font::{MonoTextStyle, ascii::FONT_6X10};
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{
    Line, PrimitiveStyle, Rectangle, Triangle,
};
use embedded_graphics::text::Text;
use std::net::Ipv4Addr;

/// FONT_6X10 glyph width
const CHAR_WIDTH: i32 = 6;
/// Instruction scroll speed, in pixels per unit of `time`
const SCROLL_SPEED: f32 = 40.0 / 4.8;
const WARNING_YELLOW: Rgb888 = Rgb888::new(255, 200, 0);

pub struct SettingsApp {
    /// Name of the setup access point, shown in the instructions
    ap_ssid: &'static str,
    network: NetworkStatus,
    /// Animation clock (arbitrary units: 0.005 per ms)
    time: f32,
}

impl SettingsApp {
    pub const fn new(ap_ssid: &'static str) -> Self {
        Self {
            ap_ssid,
            network: NetworkStatus::Connecting,
            time: 0.0,
        }
    }

    fn draw_setup_instructions(&self, fb: &mut Framebuffer, ip: Ipv4Addr) {
        let instruction =
            format!("Join WiFi '{}' then visit {ip}", self.ap_ssid);
        let text_width = instruction.len() as i32 * CHAR_WIDTH;
        let width = WIDTH as i32;
        let total_scroll = text_width + width - 20;
        let offset = width - ((self.time * SCROLL_SPEED) as i32 % total_scroll);
        let _ = Text::new(
            &instruction,
            Point::new(offset, 19),
            MonoTextStyle::new(&FONT_6X10, Rgb888::CYAN),
        )
        .draw(fb);

        // Mask the text where it scrolls under the warning icon (left) and
        // the border (right)
        let black = PrimitiveStyle::with_fill(Rgb888::BLACK);
        let _ = Rectangle::new(Point::new(1, 1), Size::new(21, HEIGHT - 2))
            .into_styled(black)
            .draw(fb);
        let _ =
            Rectangle::new(Point::new(width - 2, 1), Size::new(1, HEIGHT - 2))
                .into_styled(black)
                .draw(fb);

        // Warning triangle with an exclamation mark
        let stroke = PrimitiveStyle::with_stroke(WARNING_YELLOW, 1);
        let _ = Triangle::new(
            Point::new(10, 10),
            Point::new(4, 20),
            Point::new(16, 20),
        )
        .into_styled(stroke)
        .draw(fb);
        let _ = Line::new(Point::new(10, 13), Point::new(10, 16))
            .into_styled(stroke)
            .draw(fb);
        let _ = Pixel(Point::new(10, 18), WARNING_YELLOW).draw(fb);
    }

    fn draw_two_lines(fb: &mut Framebuffer, title: &str, detail: &str) {
        let _ = Text::new(
            title,
            Point::new(4, 12),
            MonoTextStyle::new(&FONT_6X10, Rgb888::WHITE),
        )
        .draw(fb);
        let _ = Text::new(
            detail,
            Point::new(4, 26),
            MonoTextStyle::new(&FONT_6X10, Rgb888::CYAN),
        )
        .draw(fb);
    }
}

impl App for SettingsApp {
    fn update(&mut self, frame: &Frame) {
        self.network = frame.network;
        self.time += frame.dt_ms * 0.005;
    }

    fn draw(&self, fb: &mut Framebuffer) {
        let pulse = (self.time * 2.0).sin().mul_add(50.0, 50.0) as u8;

        let border_color = match self.network {
            NetworkStatus::Connected(ip) => {
                Self::draw_two_lines(fb, "WiFi Connected", &ip.to_string());
                Rgb888::new(0, pulse, 0) // pulsing green
            }
            NetworkStatus::SetupAccessPoint(ip) => {
                self.draw_setup_instructions(fb, ip);
                Rgb888::new(pulse, 0, 0) // pulsing red
            }
            NetworkStatus::Connecting => {
                let dots = (self.time * 2.0) as usize % 4;
                let status = format!("Connecting{}", ".".repeat(dots));
                Self::draw_two_lines(fb, "WiFi Setup", &status);
                Rgb888::new(pulse, pulse / 2, 0) // pulsing orange
            }
        };

        // Pulsing 1px border, drawn last so it sits on top
        let _ = Rectangle::new(Point::zero(), Size::new(WIDTH, HEIGHT))
            .into_styled(PrimitiveStyle::with_stroke(border_color, 1))
            .draw(fb);
    }
}
