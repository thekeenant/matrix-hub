/// App trait and app implementations.
extern crate alloc;

use alloc::boxed::Box;

use anyhow::Result;
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use rhai::Engine;

use crate::{
    proto::app_state::{AppId, MatrixHubState},
    state::SharedMatrixHubState,
    tasks::hub75::FrameBuffer,
    wifi::SharedHttpTcpClient,
};

pub mod app_script;
pub mod mta;
pub mod plasma;
pub mod sandbox;

// Note: we use `mk_static!` (from `lib.rs`) at runtime to allocate the
// underlying storage for Engine/Scope and then keep raw pointers to the
// returned `'static` references. This avoids adding extra dependencies and
// keeps startup initialization explicit.

/// Context provided to the app renderer. Holds mutable access to the
/// `MatrixHubState`, the `FrameBuffer`, and shared scripting resources.
/// These are wrapped in `core::cell::RefCell` so we can expand the context
/// later without changing app signatures.
pub struct RenderContext<'a> {
    pub state: core::cell::RefCell<&'a mut MatrixHubState>,
    pub display: core::cell::RefCell<&'a mut FrameBuffer>,
    pub engine: &'static Mutex<CriticalSectionRawMutex, &'static Engine>,
}

/// Context provided to app lifecycle/run methods. Contains common runtime
/// resources such as the `Spawner` and `SharedHttpTcpClient`.
pub struct RunContext {
    pub spawner: Spawner,
    pub http_client: core::cell::RefCell<SharedHttpTcpClient>,
    pub matrix_state: SharedMatrixHubState,
    pub engine: &'static Mutex<CriticalSectionRawMutex, &'static Engine>,
}

/// Common interface for all apps.
#[async_trait::async_trait(?Send)]
pub trait App: Send + Sync {
    fn build(state: &SharedMatrixHubState, _: AppId) -> Self
    where
        Self: Sized;

    /// App ID for type-safe identification.
    fn id(&self) -> AppId;

    /// Lifecycle: System Startup
    ///
    /// Called once when firmware boots. Use this to spawn infinite background
    /// tasks that run forever (e.g., data fetching, monitoring).
    ///
    /// Default: No-op
    async fn mount(&self, _ctx: &RunContext) -> Result<()> {
        // Default: nothing to spawn
        Ok(())
    }

    /// Optional: Precompile or perform initialization that requires the `rhai::Engine`.
    ///
    /// Called by the controller on the run core when rebuilding the apps list. Default no-op.
    fn compile(&self, _engine: &Engine) -> Result<()> {
        Ok(())
    }

    /// Lifecycle: Active Loop
    ///
    /// Runs while the app is being displayed. The system cancels (drops) this
    /// future when the app is rotated out.
    ///
    /// CRITICAL: Must cooperate with cancellation:
    /// - Use Timer::after() or other .await points
    /// - Do NOT block with busy loops
    /// - Do NOT spawn infinite tasks here (use mount() instead)
    ///
    /// Default: Pending forever (no active logic)
    async fn run(&self, _ctx: &RunContext) -> Result<()> {
        core::future::pending::<()>().await;
        Ok(())
    }

    /// Render the app's current state to the framebuffer.
    ///
    /// CRITICAL PATH: Called at 60Hz+ on Core 1.
    ///
    /// CONSTRAINTS:
    /// - Must complete in <16ms (ideally <8ms for headroom)
    /// - Only mutate display-related state
    async fn render(&self, _ctx: &RenderContext<'_>) -> Result<()>;
}
