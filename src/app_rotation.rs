//! App rotation signal
//!
//! Provides a shared signal for triggering app rotation.
//! This can be used by any component (buttons, HTTP endpoints, timers, etc.)
//! to request an app rotation without depending on physical hardware.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

/// Direction for app rotation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppRotationDirection {
    /// Rotate to the next app
    Next,
    /// Rotate to the previous app
    Prev,
}

/// Signal for requesting app rotation
pub type AppRotationSignal = Signal<CriticalSectionRawMutex, AppRotationDirection>;
