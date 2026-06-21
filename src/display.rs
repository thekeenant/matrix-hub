use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use esp_idf_svc::sys::hub75::{
    hub75_c_begin, hub75_c_clear, hub75_c_create, hub75_c_draw_pixel,
    hub75_c_flip_buffer, hub75_handle_t,
};
use std::sync::atomic::AtomicU8;

extern "C" {
    fn hub75_c_set_brightness(handle: hub75_handle_t, brightness: u8);
}

pub static GLOBAL_BRIGHTNESS: AtomicU8 = AtomicU8::new(128);

pub struct MatrixDisplay {
    handle: hub75_handle_t,
    width: u16,
    height: u16,
}

pub struct MatrixConfig {
    pub width: u16,
    pub height: u16,
    pub r1: i32,
    pub g1: i32,
    pub b1: i32,
    pub r2: i32,
    pub g2: i32,
    pub b2: i32,
    pub a: i32,
    pub b: i32,
    pub c: i32,
    pub d: i32,
    pub e: i32,
    pub lat: i32,
    pub oe: i32,
    pub clk: i32,
}

impl MatrixDisplay {
    pub fn new(cfg: MatrixConfig) -> Result<Self, &'static str> {
        #[allow(
            unsafe_code,
            reason = "Calling C FFI for Hub75 display creation"
        )]
        let handle = unsafe {
            hub75_c_create(
                cfg.width, cfg.height, cfg.r1, cfg.g1, cfg.b1, cfg.r2, cfg.g2,
                cfg.b2, cfg.a, cfg.b, cfg.c, cfg.d, cfg.e, cfg.lat, cfg.oe,
                cfg.clk,
            )
        };

        if handle.is_null() {
            return Err("Failed to allocate Hub75 display");
        }

        #[allow(unsafe_code, reason = "Calling C FFI to begin Hub75 display")]
        let success = unsafe { hub75_c_begin(handle) };
        if !success {
            return Err("Failed to initialize Hub75 display hardware");
        }

        Ok(Self {
            handle,
            width: cfg.width,
            height: cfg.height,
        })
    }

    pub fn flip(&self) {
        #[allow(unsafe_code, reason = "Calling C FFI to flip Hub75 buffer")]
        unsafe {
            hub75_c_flip_buffer(self.handle)
        };
    }

    pub fn clear_display(&self) {
        #[allow(unsafe_code, reason = "Calling C FFI to clear Hub75 buffer")]
        unsafe {
            hub75_c_clear(self.handle)
        };
    }

    pub fn set_brightness(&self, brightness: u8) {
        #[allow(
            unsafe_code,
            reason = "Calling C FFI for Hub75 display brightness"
        )]
        unsafe {
            hub75_c_set_brightness(self.handle, brightness);
        }
    }
}

impl DrawTarget for MatrixDisplay {
    type Color = Rgb888;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels.into_iter() {
            if coord.x >= 0
                && coord.y >= 0
                && coord.x < self.width as i32
                && coord.y < self.height as i32
            {
                #[allow(
                    unsafe_code,
                    reason = "Calling C FFI to draw pixel on Hub75 display"
                )]
                unsafe {
                    hub75_c_draw_pixel(
                        self.handle,
                        coord.x as u16,
                        coord.y as u16,
                        color.r(),
                        color.g(),
                        color.b(),
                    );
                }
            }
        }
        Ok(())
    }
}

impl OriginDimensions for MatrixDisplay {
    fn size(&self) -> Size {
        Size::new(self.width as u32, self.height as u32)
    }
}
