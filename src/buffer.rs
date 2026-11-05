use crate::config::{HEIGHT, WIDTH};
use embedded_graphics::{pixelcolor::Rgb888, prelude::*};

pub struct Framebuffer {
    pub pixels: Vec<Rgb888>,
}

impl Framebuffer {
    pub fn new() -> Self {
        // 128x32 RGB888 is 12KB. ESP-IDF's malloc serves requests at or below
        // CONFIG_SPIRAM_MALLOC_ALWAYSINTERNAL (16KB) from scarce internal RAM
        // first, and only larger ones from PSRAM first. The framebuffer is only
        // touched by the CPU, so over-reserve capacity past that threshold to
        // land it in PSRAM and leave internal RAM for WiFi/lwIP/DMA.
        const PSRAM_THRESHOLD_BYTES: usize = 16 * 1024;
        let len = (WIDTH * HEIGHT) as usize;
        let capacity = len.max(PSRAM_THRESHOLD_BYTES / size_of::<Rgb888>() + 1);
        let mut pixels = Vec::with_capacity(capacity);
        pixels.resize(len, Rgb888::BLACK);
        Self { pixels }
    }

    pub fn clear(&mut self) {
        self.pixels.fill(Rgb888::BLACK);
    }
}

impl DrawTarget for Framebuffer {
    type Color = Rgb888;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels.into_iter() {
            if coord.x >= 0
                && coord.y >= 0
                && coord.x < WIDTH as i32
                && coord.y < HEIGHT as i32
            {
                let index = (coord.y * WIDTH as i32 + coord.x) as usize;
                self.pixels[index] = color;
            }
        }
        Ok(())
    }
}

impl OriginDimensions for Framebuffer {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}
