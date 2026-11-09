//! WiFi bring-up and supervision.
//!
//! The device starts in mixed mode: a client connection to the configured
//! network plus a setup access point (with a captive portal) for changing the
//! credentials. Once the client connection is up, the access point is dropped.

use anyhow::{anyhow, Context, Result};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{
    AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration,
    EspWifi,
};
use log::{info, warn};
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use super::NetworkStatus;
use crate::config::{AP_PASS, AP_SSID};
use crate::diag::{self, Crumb};

/// After new credentials are saved, wait this long before reconnecting so the
/// HTTP response confirming the save can reach the browser first.
const CREDENTIALS_RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// While disconnected in client-only mode, retry this often (in `check` calls,
/// i.e. seconds).
const RECONNECT_EVERY_CHECKS: u32 = 15;

pub struct WifiManager {
    wifi: EspWifi<'static>,
    status: NetworkStatus,
    /// The setup AP has been dropped and we run as a plain client
    client_only: bool,
    checks_disconnected: u32,
    reconnect_at: Option<Instant>,
}

impl WifiManager {
    /// Create the WiFi driver and start connecting with the stored
    /// credentials (non-blocking).
    pub fn start(
        modem: Modem<'static>,
        sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
    ) -> Result<Self> {
        let wifi = EspWifi::new(modem, sysloop, Some(nvs))
            .context("Failed to create WiFi driver")?;
        let mut manager = Self {
            wifi,
            status: NetworkStatus::Connecting,
            client_only: false,
            checks_disconnected: 0,
            reconnect_at: None,
        };
        manager.connect().context("Failed to start WiFi")?;
        Ok(manager)
    }

    pub const fn status(&self) -> NetworkStatus {
        self.status
    }

    /// New credentials were saved: reconnect with them shortly.
    pub fn credentials_changed(&mut self) {
        info!("WiFi credentials changed, reconnecting shortly");
        self.reconnect_at = Some(Instant::now() + CREDENTIALS_RECONNECT_DELAY);
    }

    /// Cheap; call every frame. Performs a scheduled reconnect when due.
    pub fn poll(&mut self) {
        if self.reconnect_at.is_some_and(|at| Instant::now() >= at) {
            self.reconnect_at = None;
            info!("Reconnecting WiFi with new credentials");
            if let Err(e) = self.connect() {
                warn!("WiFi reconnect failed: {e:#}");
            }
        }
    }

    /// Call about once per second: refreshes the status, drops the setup AP
    /// once connected, and retries if the connection was lost.
    pub fn check(&mut self) {
        // The netif keeps its old IP for a while after the link drops, so also
        // ask the driver whether we're actually associated.
        let associated = self.wifi.is_connected().unwrap_or(false);
        let sta_ip = self
            .wifi
            .sta_netif()
            .get_ip_info()
            .ok()
            .map(|info| info.ip)
            .filter(|ip| !ip.is_unspecified());

        self.status = match sta_ip.filter(|_| associated) {
            Some(ip) => {
                if !self.client_only {
                    self.drop_access_point();
                }
                self.checks_disconnected = 0;
                NetworkStatus::Connected(ip)
            }
            None if self.client_only => {
                self.retry_periodically();
                NetworkStatus::Connecting
            }
            None => self.access_point_ip().map_or(
                NetworkStatus::Connecting,
                NetworkStatus::SetupAccessPoint,
            ),
        };

        diag::set(Crumb::WifiConnected, u32::from(self.status.is_connected()));
    }

    /// (Re)configure mixed client + setup-AP mode and start connecting.
    fn connect(&mut self) -> Result<()> {
        let (ssid, pass) = crate::storage::with_config(|c| {
            (c.wifi.ssid.clone(), c.wifi.pass.clone())
        });

        let client = ClientConfiguration {
            ssid: ssid
                .as_str()
                .try_into()
                .map_err(|_| anyhow!("SSID too long"))?,
            password: pass
                .as_str()
                .try_into()
                .map_err(|_| anyhow!("Password too long"))?,
            auth_method: auth_method_for(&pass),
            ..Default::default()
        };
        let access_point = AccessPointConfiguration {
            ssid: AP_SSID
                .try_into()
                .map_err(|_| anyhow!("Setup AP SSID too long"))?,
            password: AP_PASS
                .try_into()
                .map_err(|_| anyhow!("Setup AP password too long"))?,
            auth_method: auth_method_for(AP_PASS),
            ..Default::default()
        };

        let _ = self.wifi.disconnect();
        self.wifi
            .set_configuration(&Configuration::Mixed(client, access_point))
            .context("Could not set WiFi configuration")?;
        crate::platform::advertise_self_as_dns(self.wifi.ap_netif());

        self.wifi.start().context("Could not start WiFi")?;
        info!("Connecting to WiFi SSID: {ssid}");
        self.wifi.connect().context("Could not connect WiFi")?;

        self.client_only = false;
        self.checks_disconnected = 0;
        self.status = NetworkStatus::Connecting;
        Ok(())
    }

    fn drop_access_point(&mut self) {
        if let Ok(Configuration::Mixed(client, _)) =
            self.wifi.get_configuration()
        {
            match self.wifi.set_configuration(&Configuration::Client(client)) {
                Ok(()) => info!(
                    "Connected; switched to client-only mode, setup AP dropped"
                ),
                Err(e) => warn!("Failed to drop setup AP: {e}"),
            }
        }
        self.client_only = true;
    }

    /// Neither ESP-IDF nor EspWifi reconnects by itself after the link drops
    /// (router reboot, DHCP failure, interference...). In client-only mode,
    /// retry periodically. (With the AP up we don't, as scans disrupt it.)
    fn retry_periodically(&mut self) {
        self.checks_disconnected += 1;
        if self.checks_disconnected % RECONNECT_EVERY_CHECKS == 0 {
            diag::MAIN.enter(diag::Stage::WifiReconnect);
            warn!(
                "WiFi down for ~{}s, retrying connect",
                self.checks_disconnected
            );
            let _ = self.wifi.connect();
            diag::inc(Crumb::WifiReconnects);
        }
    }

    fn access_point_ip(&self) -> Option<Ipv4Addr> {
        self.wifi.ap_netif().get_ip_info().ok().map(|info| info.ip)
    }
}

fn auth_method_for(password: &str) -> AuthMethod {
    if password.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    }
}
