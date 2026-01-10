//! App trait and app implementations.

extern crate alloc;

use alloc::boxed::Box;

use anyhow::Result;
use embassy_executor::Spawner;

use crate::{
    proto::app_state::{AppId, MatrixHubState},
    state::SharedMatrixHubState,
    tasks::hub75::FrameBuffer,
    wifi::SharedHttpTcpClient,
};

pub mod mta;
pub mod plasma;
pub mod sandbox;

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
    async fn mount(&self, _spawner: Spawner, _http_client: SharedHttpTcpClient) -> Result<()> {
        // Default: nothing to spawn
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
    async fn run(&self, _http_client: SharedHttpTcpClient) -> Result<()> {
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
    fn render(&self, state: &mut MatrixHubState, display: &mut FrameBuffer) -> Result<()>;
}
