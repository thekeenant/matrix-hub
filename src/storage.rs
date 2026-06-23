use crate::proto::config::{DeviceConfig, WifiCredentials};
use anyhow::{anyhow, Result};
use buffa::Message;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs};
use log::error;
use std::sync::{OnceLock, RwLock};

impl DeviceConfig {
    pub fn default_config() -> Self {
        Self {
            wifi: buffa::MessageField::some(WifiCredentials {
                ssid: dotenvy_macro::dotenv!("WIFI_SSID").to_string(),
                pass: dotenvy_macro::dotenv!("WIFI_PASS").to_string(),
                __buffa_unknown_fields: Default::default(),
            }),
            brightness: 50,
            min_minutes: Some(1),
            __buffa_unknown_fields: Default::default(),
        }
    }
}

pub fn global_config() -> &'static RwLock<DeviceConfig> {
    static CONFIG: OnceLock<RwLock<DeviceConfig>> = OnceLock::new();
    CONFIG.get_or_init(|| RwLock::new(DeviceConfig::default_config()))
}

pub fn get_config() -> DeviceConfig {
    match global_config().read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub fn global_nvs() -> &'static OnceLock<EspDefaultNvsPartition> {
    static NVS: OnceLock<EspDefaultNvsPartition> = OnceLock::new();
    &NVS
}

fn read_nvs_config(
    nvs_partition: &EspDefaultNvsPartition,
) -> Result<Option<DeviceConfig>> {
    let nvs = EspNvs::new(nvs_partition.clone(), "matrix_config", true)
        .map_err(|e| anyhow!("Failed to open NVS: {:?}", e))?;

    let mut buf = vec![0u8; 512];
    let Some(blob) = nvs
        .get_blob("config", &mut buf)
        .map_err(|e| anyhow!("Failed to read blob: {:?}", e))?
    else {
        return Ok(None);
    };

    let decoded = DeviceConfig::decode(&mut &blob[..])
        .map_err(|e| anyhow!("Failed to decode blob: {:?}", e))?;

    Ok(Some(decoded))
}

/// Merges fields loaded from NVS into a baseline configuration.
///
/// If you add a new field to `config.proto`, the compiler will force you
/// to update the exhaustive destructuring pattern here. You must then add
/// logic to handle merging the new field into the `base` config.
fn validate_and_apply_loaded_config(
    potentially_invalid_loaded_config: DeviceConfig,
    mut config: DeviceConfig,
) -> DeviceConfig {
    let DeviceConfig {
        wifi,
        brightness,
        min_minutes,
        __buffa_unknown_fields: _,
    } = potentially_invalid_loaded_config;

    if !wifi.ssid.is_empty() {
        config.wifi = buffa::MessageField::some(WifiCredentials {
            ssid: wifi.ssid.clone(),
            pass: wifi.pass.clone(),
            __buffa_unknown_fields: Default::default(),
        });
    } else if !wifi.pass.is_empty() {
        config.wifi = buffa::MessageField::some(WifiCredentials {
            ssid: config.wifi.ssid.clone(),
            pass: wifi.pass.clone(),
            __buffa_unknown_fields: Default::default(),
        });
    }

    if brightness != 0 {
        // Enforce max brightness to fit in u8
        config.brightness = brightness.clamp(1, 255);
    }

    if let Some(m) = min_minutes {
        config.min_minutes = Some(m);
    }

    config
}

pub fn init_config(nvs_partition: EspDefaultNvsPartition) {
    let _ = global_nvs().set(nvs_partition.clone());
    let mut config = DeviceConfig::default_config();
    let mut needs_save = true;

    match read_nvs_config(&nvs_partition) {
        Ok(Some(decoded)) => {
            let merged = validate_and_apply_loaded_config(
                decoded.clone(),
                config.clone(),
            );

            if decoded == merged {
                needs_save = false;
            }

            config = merged;
        }
        Ok(None) => {}
        Err(e) => {
            error!("Failed to load NVS config: {}", e);
        }
    }

    if needs_save {
        let _ = save_config(&nvs_partition, &config);
    }

    if let Ok(mut g) = global_config().write() {
        *g = config;
    }
}

pub fn save_config(
    nvs_partition: &EspDefaultNvsPartition,
    config: &DeviceConfig,
) -> Result<()> {
    let nvs = EspNvs::new(nvs_partition.clone(), "matrix_config", true)
        .map_err(|e| anyhow!("Failed to open NVS matrix_config: {:?}", e))?;

    let mut buf = Vec::new();
    config.encode(&mut buf);

    nvs.set_blob("config", &buf)
        .map_err(|e| anyhow!("Failed to save config blob to NVS: {:?}", e))?;

    Ok(())
}

pub fn update_config<F>(update_fn: F) -> Result<()>
where
    F: FnOnce(&mut DeviceConfig),
{
    let mut guard = global_config()
        .write()
        .map_err(|_| anyhow!("Failed to acquire write lock"))?;
    let original = guard.clone();

    update_fn(&mut guard);

    if *guard != original {
        if let Some(nvs) = global_nvs().get() {
            save_config(nvs, &guard)?;
        }
    }

    Ok(())
}
