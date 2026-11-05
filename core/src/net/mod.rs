//! Network-facing logic that doesn't depend on the WiFi or socket APIs.

pub mod body;
pub mod dns;

use std::net::Ipv4Addr;

/// What the network connection is doing, as reported to apps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkStatus {
    /// Trying to join the configured WiFi network
    Connecting,
    /// The setup access point is up: join it to configure WiFi
    SetupAccessPoint(Ipv4Addr),
    /// Joined the configured WiFi network
    Connected(Ipv4Addr),
}

impl NetworkStatus {
    pub const fn is_connected(self) -> bool {
        matches!(self, Self::Connected(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use googletest::prelude::*;

    #[gtest]
    fn only_connected_counts_as_connected() {
        let ip = Ipv4Addr::new(10, 0, 0, 2);
        expect_true!(NetworkStatus::Connected(ip).is_connected());
        expect_false!(NetworkStatus::SetupAccessPoint(ip).is_connected());
        expect_false!(NetworkStatus::Connecting.is_connected());
    }
}
