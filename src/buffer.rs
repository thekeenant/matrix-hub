use crate::config::{HEIGHT, WIDTH};
use embedded_graphics::{pixelcolor::Rgb888, prelude::*};

#[derive(Clone)]
pub struct Framebuffer {
    pub pixels: Vec<Rgb888>,
}

impl Framebuffer {
    pub fn new() -> Self {
        Self {
            // Allocate the 12KB buffer on the heap to save thread stack space
            pixels: vec![Rgb888::BLACK; (WIDTH * HEIGHT) as usize],
        }
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
