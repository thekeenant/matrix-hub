extern crate alloc;
use alloc::boxed::Box;

use anyhow::Result;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embedded_graphics::{geometry::Point, pixelcolor::Rgb888, prelude::*};
use log::warn;
// use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use rhai::{AST, Dynamic, Engine, Scope};

use crate::{
    apps::{App, RenderContext, RunContext},
    proto::app_state::{
        AppId,
        app_id::{AppScript as ProtoAppScript, Id},
    },
    state::SharedMatrixHubState,
    tasks::{RateLimiter, hub75::FrameBuffer},
};

pub struct AppScript {
    pub script: &'static str,
    pub state: SharedMatrixHubState,
    pub scope: Mutex<CriticalSectionRawMutex, Scope<'static>>,
    pub ast: Mutex<CriticalSectionRawMutex, Option<AST>>,
}

impl AppScript {
    pub fn new(state: SharedMatrixHubState, script: &'static str) -> Self {
        Self {
            script,
            state,
            scope: Mutex::new(Scope::new()),
            ast: Mutex::new(None),
        }
    }

    /// Compile and execute top-level script initialization. Caller must hold `scope` lock.
    async fn compile_and_init(&self, engine: &rhai::Engine) -> Result<()> {
        let mut ast = self.ast.lock().await;
        // Compile to a temporary AST first
        match engine.compile(self.script) {
            Ok(compiled_ast) => {
                *ast = Some(compiled_ast);
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Script compile error: {}", e));
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait(?Send)]
impl App for AppScript {
    fn build(state: &SharedMatrixHubState, _: AppId) -> Self {
        // Use a test script by default that exercises the fast framebuffer API.
        AppScript::new(state.clone(), TEST_SCRIPT)
    }

    fn id(&self) -> AppId {
        AppId {
            id: Some(Id::AppScript(ProtoAppScript {})),
        }
    }

    async fn run(&self, ctx: &RunContext) -> Result<()> {
        self.compile_and_init(*ctx.engine.lock().await).await?;
        let ast = {
            let ast_lock = self.ast.lock().await;
            ast_lock
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Script AST not compiled"))?
                .clone()
        };
        let mut rate_limiter = RateLimiter::new(20, "AppScript");
        loop {
            {
                let engine = *ctx.engine.lock().await;
                let mut scope = self.scope.lock().await;

                match engine.call_fn::<()>(&mut *scope, &ast, "update", ()) {
                    Ok(_) => {}
                    Err(e) => {
                        warn!("Script update() error: {}", e);
                    }
                }
            }
            rate_limiter.sleep().await;
        }
    }

    async fn render(&self, ctx: &RenderContext<'_>) -> Result<()> {
        // Provide the real FrameBuffer pointer to the script API, call script's `render()` to draw pixels
        let mut display_ref = ctx.display.borrow_mut();
        let display: &mut FrameBuffer = &mut *display_ref;

        // Expose this framebuffer pointer for direct fast access from Rhai.
        crate::apps::framebuffer_api::set_current_framebuffer(display as *mut _ as usize);

        let ast_lock = self.ast.lock().await;
        let ast = match ast_lock.as_ref() {
            Some(ast) => ast,
            None => {
                crate::apps::framebuffer_api::clear_current_framebuffer();
                return Ok(());
            }
        };

        let engine = *ctx.engine.lock().await;
        let mut scope = self.scope.lock().await;
        match engine.call_fn::<()>(&mut *scope, ast, "render", ()) {
            Ok(_) => {}
            Err(e) => {
                warn!("Script render() error: {}", e);
            }
        }

        // Always clear the global pointer after the script finishes.
        crate::apps::framebuffer_api::clear_current_framebuffer();

        Ok(())
    }
}

pub const RED_CIRCLE_SCRIPT: &str = "[16, 16, 10, 0xFF0000]";

pub const TRIPPY_SCRIPT: &str = r#"// Trippy animated pattern for 128x32
let t = 0;

fn update() {
    // Increment animation frame counter
    t = t + 1;
}

fn render() {
    print("hey from rhai!");
}
"#;

/// Simple test script that draws a horizontal line using `set_pixel`.
pub const TEST_SCRIPT: &str = r#"
fn update() {
    // no-op
}

fn render() {
    // Clear to black
    clear(0x000000);

    let w = fb_width();
    let h = fb_height();
    let c = 0xFF0000; // red

    let x = 0;
    while x < w {
        set_pixel(x, x % h, c);
    }
}
"#;
