//! Validating and applying device configuration: values loaded from flash,
//! and partial updates from the web UI.

use buffa::Message;

use crate::proto::config::{
    DeviceConfig, UpdateConfigRequest, WifiCredentials,
};

/// Upper bound for the "minimum minutes" train filter (matches the frontend)
pub const MAX_MIN_MINUTES: u32 = 60;
/// A config update is a few dozen bytes of protobuf
pub const MAX_UPDATE_BYTES: usize = 1024;
/// WiFi limits (802.11 SSID, WPA2 passphrase)
const MAX_SSID_BYTES: usize = 32;
const MAX_PASSWORD_BYTES: usize = 64;

/// Merges fields loaded from flash into a baseline (default) configuration,
/// ignoring unset or invalid values.
///
/// The exhaustive destructuring makes adding a field to `config.proto` a
/// compile error here until its merge rule is written.
pub fn merge_loaded(
    loaded: DeviceConfig,
    mut config: DeviceConfig,
) -> DeviceConfig {
    let DeviceConfig {
        wifi,
        brightness,
        min_minutes,
        __buffa_unknown_fields: _,
    } = loaded;

    if !wifi.ssid.is_empty() {
        config.wifi = credentials(wifi.ssid.clone(), wifi.pass.clone());
    } else if !wifi.pass.is_empty() {
        config.wifi = credentials(config.wifi.ssid.clone(), wifi.pass.clone());
    }
    // 0 means "unset"; it's used as a u8
    if brightness != 0 {
        config.brightness = brightness.min(255);
    }
    if let Some(m) = min_minutes {
        config.min_minutes = Some(m.min(MAX_MIN_MINUTES));
    }
    config
}

fn credentials(
    ssid: String,
    pass: String,
) -> buffa::MessageField<WifiCredentials> {
    buffa::MessageField::some(WifiCredentials {
        ssid,
        pass,
        __buffa_unknown_fields: Default::default(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateError {
    /// Not a valid UpdateConfigRequest
    Malformed,
    InvalidWifi,
}

impl core::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Malformed => "Bad Request",
            Self::InvalidWifi => {
                "SSID must be 1-32 bytes and password at most 64 bytes"
            }
        })
    }
}

impl std::error::Error for UpdateError {}

/// A validated partial update (AIP-134 style: only fields named in the
/// update mask change; an empty mask means every field).
#[derive(Debug)]
pub struct ConfigUpdate {
    values: DeviceConfig,
    wifi: bool,
    brightness: bool,
    min_minutes: bool,
}

impl ConfigUpdate {
    /// Parse an encoded `UpdateConfigRequest`. `Ok(None)` if it carries no
    /// config (nothing to do).
    pub fn parse(mut body: &[u8]) -> Result<Option<Self>, UpdateError> {
        let request = UpdateConfigRequest::decode(&mut body)
            .map_err(|_| UpdateError::Malformed)?;
        let Some(values) = request.config.into_option() else {
            return Ok(None);
        };

        let mask = &request.update_mask;
        let wants =
            |field: &str| mask.is_empty() || mask.iter().any(|m| m == field);
        let update = Self {
            wifi: wants("wifi"),
            brightness: wants("brightness"),
            min_minutes: wants("min_minutes"),
            values,
        };

        // Reject credentials WiFi would fail on, rather than saving them
        let ssid_len = update.values.wifi.ssid.len();
        let pass_len = update.values.wifi.pass.len();
        if update.wifi
            && (ssid_len == 0
                || ssid_len > MAX_SSID_BYTES
                || pass_len > MAX_PASSWORD_BYTES)
        {
            return Err(UpdateError::InvalidWifi);
        }
        Ok(Some(update))
    }

    /// Whether applying this update changes the WiFi credentials (so WiFi
    /// should reconnect).
    pub const fn changes_wifi(&self) -> bool {
        self.wifi
    }

    pub fn apply(&self, config: &mut DeviceConfig) {
        if self.wifi {
            config.wifi = self.values.wifi.clone();
        }
        if self.brightness {
            // Stored as u32 but used as u8; 0 means "unset" on load
            config.brightness = self.values.brightness.clamp(1, 255);
        }
        if self.min_minutes {
            config.min_minutes =
                self.values.min_minutes.map(|m| m.min(MAX_MIN_MINUTES));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use googletest::prelude::*;

    fn config(
        ssid: &str,
        pass: &str,
        brightness: u32,
        min_minutes: Option<u32>,
    ) -> DeviceConfig {
        DeviceConfig {
            wifi: credentials(ssid.into(), pass.into()),
            brightness,
            min_minutes,
            __buffa_unknown_fields: Default::default(),
        }
    }

    fn defaults() -> DeviceConfig {
        config("home", "secret", 50, Some(1))
    }

    fn encode(config: Option<DeviceConfig>, mask: &[&str]) -> Vec<u8> {
        UpdateConfigRequest {
            config: config.map_or_else(
                buffa::MessageField::none,
                buffa::MessageField::some,
            ),
            update_mask: mask.iter().map(|m| (*m).to_string()).collect(),
            __buffa_unknown_fields: Default::default(),
        }
        .encode_to_vec()
    }

    #[gtest]
    fn merge_takes_stored_values_over_defaults() {
        let merged =
            merge_loaded(config("cafe", "latte", 200, Some(5)), defaults());
        expect_that!(merged, eq(&config("cafe", "latte", 200, Some(5))));
    }

    #[gtest]
    fn merge_keeps_defaults_for_unset_values() {
        let merged = merge_loaded(config("", "", 0, None), defaults());
        expect_that!(merged, eq(&defaults()));
    }

    #[gtest]
    fn merge_applies_a_stored_password_to_the_default_ssid() {
        let merged = merge_loaded(config("", "newpass", 0, None), defaults());
        expect_that!(merged.wifi.ssid, eq("home"));
        expect_that!(merged.wifi.pass, eq("newpass"));
    }

    #[gtest]
    fn merge_clamps_out_of_range_values() {
        let merged = merge_loaded(config("", "", 999, Some(500)), defaults());
        expect_that!(merged.brightness, eq(255));
        expect_that!(merged.min_minutes, some(eq(MAX_MIN_MINUTES)));
    }

    #[gtest]
    fn update_changes_only_masked_fields() -> googletest::Result<()> {
        let body = encode(Some(config("x", "y", 10, Some(7))), &["brightness"]);
        let update = ConfigUpdate::parse(&body).or_fail()?.or_fail()?;
        let mut current = defaults();
        update.apply(&mut current);

        expect_false!(update.changes_wifi());
        expect_that!(current, eq(&config("home", "secret", 10, Some(1))));
        Ok(())
    }

    #[gtest]
    fn an_empty_mask_updates_everything() -> googletest::Result<()> {
        let body = encode(Some(config("cafe", "", 30, None)), &[]);
        let update = ConfigUpdate::parse(&body).or_fail()?.or_fail()?;
        let mut current = defaults();
        update.apply(&mut current);

        expect_true!(update.changes_wifi());
        expect_that!(current, eq(&config("cafe", "", 30, None)));
        Ok(())
    }

    #[gtest]
    fn update_clamps_brightness_and_minutes() -> googletest::Result<()> {
        let body = encode(
            Some(config("", "", 0, Some(90))),
            &["brightness", "min_minutes"],
        );
        let mut current = defaults();
        ConfigUpdate::parse(&body)
            .or_fail()?
            .or_fail()?
            .apply(&mut current);

        expect_that!(current.brightness, eq(1), "0 would mean unset/off");
        expect_that!(current.min_minutes, some(eq(MAX_MIN_MINUTES)));
        Ok(())
    }

    #[gtest]
    fn rejects_unusable_wifi_credentials() {
        let long_ssid = "s".repeat(33);
        let long_pass = "p".repeat(65);
        for (ssid, pass) in [
            ("", "x"),
            (long_ssid.as_str(), ""),
            ("ok", long_pass.as_str()),
        ] {
            let body = encode(Some(config(ssid, pass, 1, None)), &["wifi"]);
            expect_that!(
                ConfigUpdate::parse(&body),
                err(eq(&UpdateError::InvalidWifi))
            );
        }
    }

    #[gtest]
    fn ignores_wifi_values_when_wifi_is_not_masked() -> googletest::Result<()> {
        let body = encode(Some(config("", "", 5, None)), &["brightness"]);
        expect_that!(ConfigUpdate::parse(&body), ok(some(anything())));
        Ok(())
    }

    #[gtest]
    fn a_request_without_config_is_a_no_op() {
        expect_that!(ConfigUpdate::parse(&encode(None, &["wifi"])), ok(none()));
    }

    #[gtest]
    fn rejects_malformed_requests() {
        let error = ConfigUpdate::parse(&[0xFF, 0xFF, 0xFF]);
        expect_that!(error, err(eq(&UpdateError::Malformed)));
        expect_that!(UpdateError::Malformed.to_string(), eq("Bad Request"));
        expect_that!(
            UpdateError::InvalidWifi.to_string(),
            contains_substring("SSID")
        );
    }
}
