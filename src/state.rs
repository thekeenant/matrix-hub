//! Shared application state types.

extern crate alloc;

use alloc::sync::Arc;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};

use crate::proto::app_state::MatrixHubState;

/// Shared state for the matrix hub, protected by a mutex
pub type SharedMatrixHubState = Arc<Mutex<CriticalSectionRawMutex, MatrixHubState>>;
