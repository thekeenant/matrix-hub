//! Over-the-air firmware updates.
//!
//! The flash has two app slots. An upload (`POST /api/ota`) is written to the
//! inactive slot while the current firmware keeps running, then the device
//! restarts into it. The new image boots *unverified*: it must prove itself
//! healthy (WiFi up, main loop running for a minute) and mark itself valid.
//! If it crashes or reboots first, or never gets on WiFi, the bootloader rolls
//! back to the previous image, so a bad update can't brick the device.

use anyhow::{anyhow, bail, Context, Result};
use embedded_svc::ota::SlotState;
use esp_idf_svc::ota::EspOta;
use log::{info, warn};
use serde::Serialize;
use std::time::Duration;

/// Must be presented as `Authorization: Bearer <password>` to upload
const OTA_PASSWORD: &str = dotenvy_macro::dotenv!("OTA_PASSWORD");

/// An unverified image is accepted once it has run this long with WiFi up
const HEALTHY_AFTER_S: u32 = 60;
/// ...and rolled back if it hasn't managed that by this uptime
const GIVE_UP_AFTER_S: u32 = 300;

/// Upload chunk size
const CHUNK_BYTES: usize = 4096;

/// Firmware identity, for /api/debug.
#[derive(Serialize)]
pub struct FirmwareInfo {
    /// Git commit the firmware was built from
    pub git: &'static str,
    pub running_slot: String,
    pub state: &'static str,
}

pub fn firmware_info() -> FirmwareInfo {
    let slot = EspOta::new().and_then(|ota| ota.get_running_slot());
    FirmwareInfo {
        git: env!("MATRIX_HUB_GIT"),
        running_slot: slot
            .as_ref()
            .map_or_else(|e| format!("? ({e})"), |s| s.label.to_string()),
        state: slot.map_or("?", |s| state_name(s.state)),
    }
}

const fn state_name(state: SlotState) -> &'static str {
    match state {
        SlotState::Factory => "factory",
        SlotState::Valid => "valid",
        SlotState::Invalid => "invalid",
        SlotState::Unverified => "unverified",
        SlotState::Unknown => "unknown",
    }
}

/// Decides when a freshly updated image is good, or must be rolled back.
pub struct HealthCheck {
    pending: bool,
}

impl HealthCheck {
    pub fn new() -> Self {
        let pending = EspOta::new()
            .and_then(|ota| ota.get_running_slot())
            .is_ok_and(|slot| slot.state == SlotState::Unverified);
        if pending {
            info!("ota: running a new, unverified image; checking health");
        }
        Self { pending }
    }

    /// Call about once per second from the main loop.
    pub fn tick(&mut self, uptime_s: u32, wifi_connected: bool) {
        if !self.pending {
            return;
        }
        if uptime_s >= HEALTHY_AFTER_S && wifi_connected {
            match EspOta::new()
                .and_then(|mut ota| ota.mark_running_slot_valid())
            {
                Ok(()) => info!("ota: new image marked valid"),
                Err(e) => warn!("ota: failed to mark image valid: {e}"),
            }
            self.pending = false;
        } else if uptime_s >= GIVE_UP_AFTER_S {
            warn!("ota: new image never got healthy, rolling back");
            if let Ok(mut ota) = EspOta::new() {
                // Reboots into the previous image on success
                let e = ota.mark_running_slot_invalid_and_reboot();
                warn!("ota: rollback failed: {e}");
            }
            self.pending = false;
        }
    }
}

/// Checks the `Authorization` header of an upload.
pub fn authorized(header: Option<&str>) -> bool {
    let Some(token) = header.and_then(|h| h.strip_prefix("Bearer ")) else {
        return false;
    };
    // Compare in full regardless of where they differ
    token.len() == OTA_PASSWORD.len()
        && token
            .bytes()
            .zip(OTA_PASSWORD.bytes())
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            == 0
}

/// Write an uploaded image into the inactive slot and make it the boot
/// image. `read` fills the buffer with the next chunk, returning 0 at the end.
pub fn install(
    content_length: Option<usize>,
    mut read: impl FnMut(&mut [u8]) -> Result<usize>,
) -> Result<usize> {
    let mut ota = EspOta::new().context("OTA not available")?;
    let mut update = match content_length {
        Some(len) => ota.initiate_update_with_known_size(len),
        None => ota.initiate_update(),
    }
    .context("Failed to start update")?;

    let mut buf = vec![0u8; CHUNK_BYTES];
    let mut written = 0;
    let result = loop {
        let n = match read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) => break Err(e),
        };
        if let Err(e) = update.write(&buf[..n]) {
            break Err(anyhow!(
                "Flash write failed after {written} bytes: {e}"
            ));
        }
        written += n;
    };

    if let Err(e) = result.and_then(|()| match content_length {
        Some(len) if len != written => {
            bail!("Upload truncated: got {written} of {len} bytes")
        }
        _ => Ok(()),
    }) {
        let _ = update.abort();
        return Err(e);
    }

    // Validates the image (format, checksum, chip) and sets it to boot next
    update.complete().context("Uploaded image is invalid")?;
    info!("ota: installed {written} bytes; restarting into the new image");
    Ok(written)
}

/// Restart shortly, so the HTTP response confirming the upload gets sent.
pub fn restart_soon() {
    let _ = std::thread::Builder::new().stack_size(2048).spawn(|| {
        std::thread::sleep(Duration::from_secs(1));
        esp_idf_svc::hal::reset::restart();
    });
}
