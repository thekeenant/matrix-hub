pub mod dns;
pub mod http;
pub mod server;
pub mod wifi;

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
