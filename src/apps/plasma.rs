//! Plasma visualization app.

extern crate alloc;

use alloc::boxed::Box;
use core::fmt::Write;

use embedded_graphics::{
    Drawable,
    geometry::Point,
    mono_font::{MonoTextStyle, ascii::FONT_5X7},
    pixelcolor::Rgb888,
    prelude::*,
    text::{Text, renderer::CharacterStyle},
};
use heapless::String as HString;
use libm::sinf;

use crate::{
    apps::{App, RenderContext},
    proto::app_state::{
        AppId, MatrixHubState, PlasmaAppState,
        app_id::{Id, Plasma},
    },
    state::SharedMatrixHubState,
    tasks::hub75::{COLS, FrameBuffer, ROWS},
};

/// Render a plasma effect to the framebuffer using interpolated sine waves
///
/// This function computes sine waves at a grid of sample points, then interpolates
/// between them for smooth gradients. This is much faster than computing sine for
/// every pixel.
fn render_plasma<DT>(draw_target: &mut DT, frame: u32, rows: usize, cols: usize, brightness: f32)
where
    DT: DrawTarget<Color = Rgb888>,
{
    // More optimized - larger blocks, interpolate RGB directly instead of hue
    let time = frame as f32 * 0.05;
    const STEP: i32 = 16; // Larger blocks = 4x fewer calculations

    for y in (0..rows as i32).step_by(STEP as usize) {
        for x in (0..cols as i32).step_by(STEP as usize) {
            // Calculate plasma and RGB for corners
            let calc_rgb = |fx: f32, fy: f32| -> (u8, u8, u8) {
                let v1 = sinf(fx * 0.1 + time);
                let v2 = sinf(fy * 0.12 - time * 0.7);
                let v3 = sinf((fx + fy) * 0.07 + time * 0.5);
                let plasma = (v1 + v2 + v3) / 3.0;
                let hue = (plasma + 1.0) * 0.5 + time * 0.1;
                (
                    ((sinf(hue * 6.28) * 0.5 + 0.5) * 255.0 * brightness) as u8,
                    ((sinf(hue * 6.28 + 2.09) * 0.5 + 0.5) * 255.0 * brightness) as u8,
                    ((sinf(hue * 6.28 + 4.19) * 0.5 + 0.5) * 255.0 * brightness) as u8,
                )
            };

            let (r_tl, g_tl, b_tl) = calc_rgb(x as f32, y as f32);
            let (r_tr, g_tr, b_tr) = calc_rgb((x + STEP) as f32, y as f32);
            let (r_bl, g_bl, b_bl) = calc_rgb(x as f32, (y + STEP) as f32);
            let (r_br, g_br, b_br) = calc_rgb((x + STEP) as f32, (y + STEP) as f32);

            // Interpolate RGB directly (faster than interpolating plasma then converting)
            for dy in 0..STEP {
                for dx in 0..STEP {
                    let px = x + dx;
                    let py = y + dy;
                    if px < cols as i32 && py < rows as i32 {
                        let tx = dx as f32 / STEP as f32;
                        let ty = dy as f32 / STEP as f32;

                        let r_top = (r_tl as f32 * (1.0 - tx) + r_tr as f32 * tx) as u8;
                        let r_bot = (r_bl as f32 * (1.0 - tx) + r_br as f32 * tx) as u8;
                        let r = (r_top as f32 * (1.0 - ty) + r_bot as f32 * ty) as u8;

                        let g_top = (g_tl as f32 * (1.0 - tx) + g_tr as f32 * tx) as u8;
                        let g_bot = (g_bl as f32 * (1.0 - tx) + g_br as f32 * tx) as u8;
                        let g = (g_top as f32 * (1.0 - ty) + g_bot as f32 * ty) as u8;

                        let b_top = (b_tl as f32 * (1.0 - tx) + b_tr as f32 * tx) as u8;
                        let b_bot = (b_bl as f32 * (1.0 - tx) + b_br as f32 * tx) as u8;
                        let b = (b_top as f32 * (1.0 - ty) + b_bot as f32 * ty) as u8;

                        Pixel(Point::new(px, py), Rgb888::new(r, g, b))
                            .draw(draw_target)
                            .ok();
                    }
                }
            }
        }
    }
}

pub struct PlasmaApp;

impl PlasmaApp {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait(?Send)]
impl App for PlasmaApp {
    fn build(_state: &SharedMatrixHubState, _: AppId) -> Self {
        Self::new()
    }

    fn id(&self) -> AppId {
        AppId {
            id: Some(Id::Plasma(Plasma {})),
        }
    }

    async fn render(&self, ctx: &RenderContext<'_>) -> anyhow::Result<()> {
        let mut state_ref = ctx.state.borrow_mut();
        let state: &mut MatrixHubState = &mut *state_ref;
        let mut display_ref = ctx.display.borrow_mut();
        let display: &mut FrameBuffer = &mut *display_ref;

        // Initialize plasma state if needed
        if state.plasma.is_none() {
            state.plasma = Some(PlasmaAppState { phase: 0.0 });
        }

        // Update plasma phase
        let plasma = state.plasma.get_or_insert_default();
        plasma.phase += 0.5;

        // Render plasma effect
        let phase = plasma.phase as u32;

        render_plasma(display, phase, ROWS, COLS, 1.0);

        // Render FPS and TPS in bottom right
        let system_info = state.system_info.get_or_insert_default();
        let fps = system_info.fps;
        let tps = system_info.tps;

        let mut fps_text = HString::<16>::new();
        let mut tps_text = HString::<16>::new();
        write!(&mut fps_text, "FPS: {}", fps)?;
        write!(&mut tps_text, "TPS: {}", tps)?;

        let mut style = MonoTextStyle::new(&FONT_5X7, Rgb888::WHITE);
        style.set_background_color(Some(Rgb888::BLACK));

        // Position in bottom right, right-aligned
        let text_width = fps_text.len() as i32 * 5; // FONT_5X7 is 5 pixels per character
        let font_height = 7;
        let fps_point = Point::new(COLS as i32 - text_width, ROWS as i32 - font_height * 2 + 3);
        let tps_point = Point::new(COLS as i32 - text_width, ROWS as i32 - font_height + 2);

        Text::new(&fps_text, fps_point, style).draw(display)?;
        Text::new(&tps_text, tps_point, style).draw(display)?;

        Ok(())
    }
}
