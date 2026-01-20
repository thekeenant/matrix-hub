use alloc::vec::Vec;
extern crate alloc;
use alloc::boxed::Box;

use anyhow::Result;
use embedded_graphics::{
    pixelcolor::Rgb888,
    prelude::*,
    primitives::{Circle, PrimitiveStyle},
};
use log::warn;
// use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use rhai::{Dynamic, Engine, Scope};

use crate::{
    apps::App,
    proto::app_state::{AppId, MatrixHubState, app_id::Id},
    state::SharedMatrixHubState,
    tasks::hub75::FrameBuffer,
    wifi::SharedHttpTcpClient,
};

pub struct AppScript {
    pub script: &'static str,
    pub state: SharedMatrixHubState,
}

impl AppScript {
    pub fn new(state: SharedMatrixHubState, script: &'static str) -> Self {
        Self { script, state }
    }
}

#[async_trait::async_trait(?Send)]
impl App for AppScript {
    fn build(state: &SharedMatrixHubState, _: AppId) -> Self {
        AppScript::new(state.clone(), RED_CIRCLE_SCRIPT)
    }

    fn id(&self) -> AppId {
        AppId {
            id: Some(Id::AppScript(crate::proto::app_state::app_id::AppScript {})),
        }
    }

    async fn run(&self, _http_client: SharedHttpTcpClient) -> Result<()> {
        // No-op for script app
        core::future::pending::<()>().await;
        Ok(())
    }

    fn render(&self, _state: &mut MatrixHubState, display: &mut FrameBuffer) -> Result<()> {
        let engine = Engine::new_raw();
        // Drawing command: (x, y, r, color)
        // Only allow script to compute values, not emit drawing commands
        let script = self.script;
        let mut scope = Scope::new();
        // Example: script returns [x, y, r, color] as an array
        let result: Result<Vec<Dynamic>, _> = engine.eval_with_scope(&mut scope, script);
        if let Ok(arr) = result {
            if arr.len() == 4 {
                let x = arr[0].clone().cast::<i64>();
                let y = arr[1].clone().cast::<i64>();
                let r = arr[2].clone().cast::<i64>();
                let color = arr[3].clone().cast::<i64>();
                let center = Point::new(x as i32, y as i32);
                let radius = r as u32;
                let rgb = Rgb888::new(
                    ((color >> 16) & 0xFF) as u8,
                    ((color >> 8) & 0xFF) as u8,
                    (color & 0xFF) as u8,
                );
                let style = PrimitiveStyle::with_fill(rgb);
                let _ = Circle::new(center, radius * 2)
                    .into_styled(style)
                    .draw(display);
            } else {
                warn!("Script did not return an array of 4 elements");
            }
        } else if let Err(ref _e) = result {
            warn!("Script error: {:?}", _e);
        }
        Ok(())
    }
}

pub const RED_CIRCLE_SCRIPT: &str = "[16, 16, 10, 0xFF0000]";
