use crate::apps::App;
use crate::buffer::Framebuffer;
use embedded_graphics::mono_font::{ascii::FONT_6X10, MonoTextStyle};
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{
    Line, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle, Triangle,
};
use embedded_graphics::text::Text;

pub struct SettingsApp {
    pub ip: Option<String>,
    time: f32,
}

impl SettingsApp {
    pub const fn new() -> Self {
        Self {
            ip: None,
            time: 0.0,
        }
    }
}

impl App for SettingsApp {
    fn update(&mut self, dt_ms: f32) {
        self.time += dt_ms * 0.005;
    }

    fn draw(&self, buffer: &mut Framebuffer) {
        let pulse = (self.time * 2.0).sin().mul_add(50.0, 50.0) as u8;

        let is_ap = self.ip.as_ref().is_some_and(|s| s.starts_with("AP:"));
        let is_connected = self.ip.is_some() && !is_ap;

        let border_color = if is_connected {
            Rgb888::new(0, pulse, 0) // Pulsing green
        } else if is_ap {
            Rgb888::new(pulse, 0, 0) // Pulsing red
        } else {
            Rgb888::new(pulse, pulse / 2, 0) // Pulsing orange
        };

        buffer.clear();

        let text_style = MonoTextStyle::new(&FONT_6X10, Rgb888::WHITE);
        let sub_style = MonoTextStyle::new(&FONT_6X10, Rgb888::CYAN);

        if is_ap {
            // Scrolling instruction text
            let instruction = format!(
                "Join WiFi 'Matrix-Hub' then visit {}",
                self.ip.as_deref().unwrap_or("").replace("AP: ", "")
            );
            let text_width = (instruction.len() * 6) as i32;
            let speed = 40.0 / 4.8; // Target ~40 pixels per real-life second
            let total_scroll = text_width + 128 - 20;
            let offset = 128 - ((self.time * speed) as i32 % total_scroll);

            // Draw text
            let _ = Text::new(
                &instruction,
                Point::new(offset, 19),
                MonoTextStyle::new(&FONT_6X10, Rgb888::CYAN),
            )
            .draw(buffer);

            // Draw a black rectangle on the left to hide text scrolling behind the warning icon
            let _ = Rectangle::new(Point::new(1, 1), Size::new(21, 30))
                .into_styled(
                    PrimitiveStyleBuilder::new()
                        .fill_color(Rgb888::BLACK)
                        .build(),
                )
                .draw(buffer);

            // Draw a 1px black column on the right so text doesn't overlap the right border before it's drawn
            let _ = Rectangle::new(Point::new(126, 1), Size::new(1, 30))
                .into_styled(
                    PrimitiveStyleBuilder::new()
                        .fill_color(Rgb888::BLACK)
                        .build(),
                )
                .draw(buffer);

            // Draw Warning Triangle over the black background
            let warning_color = Rgb888::new(255, 200, 0); // Yellow
            let _ = Triangle::new(Point::new(10, 10), Point::new(4, 20), Point::new(16, 20))
                .into_styled(PrimitiveStyle::with_stroke(warning_color, 1))
                .draw(buffer);
            let _ = Line::new(Point::new(10, 13), Point::new(10, 16))
                .into_styled(PrimitiveStyle::with_stroke(warning_color, 1))
                .draw(buffer);
            let _ = Pixel(Point::new(10, 18), warning_color).draw(buffer);
        } else if is_connected {
            // Connected Mode
            let _ = Text::new("WiFi Connected", Point::new(4, 12), text_style).draw(buffer);
            let _ = Text::new(
                self.ip.as_deref().unwrap_or(""),
                Point::new(4, 26),
                sub_style,
            )
            .draw(buffer);
        } else {
            // Connecting Mode
            let _ = Text::new("WiFi Setup", Point::new(4, 12), text_style).draw(buffer);
            let dots = match (self.time * 2.0) as i32 % 4 {
                0 => "Connecting",
                1 => "Connecting.",
                2 => "Connecting..",
                _ => "Connecting...",
            };
            let _ = Text::new(dots, Point::new(4, 26), sub_style).draw(buffer);
        }

        // Draw the 1-pixel pulsing border LAST so it sits on top of everything
        let style = PrimitiveStyleBuilder::new()
            .stroke_color(border_color)
            .stroke_width(1)
            .build();

        let _ = Rectangle::new(Point::new(0, 0), Size::new(128, 32))
            .into_styled(style)
            .draw(buffer);
    }
}
