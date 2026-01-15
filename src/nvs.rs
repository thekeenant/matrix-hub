//! Persistent key/value storage via protobuf, stored in the NVS partition.

extern crate alloc;

use alloc::{string::String, vec::Vec};
use core::{
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

use embedded_storage::{ReadStorage, Storage};
use log::info;
use prost::Message;

use crate::proto::app_state::{
    AppId, AppRotationConfig, Backtrace, Config, KeyValueStorage, MtaConfig, StationConfig,
    WifiConfig, app_id, key_value_storage,
};

const STORAGE_MAX_SIZE: usize = 4096;

static GLOBAL_FLASH_STORAGE: AtomicPtr<esp_storage::FlashStorage<'static>> =
    AtomicPtr::new(ptr::null_mut());

/// Register a global flash storage handle for use in crash/panic contexts.
///
/// # Safety
/// The pointer must remain valid for the lifetime of the program and must not be used
/// concurrently with other flash operations.
pub fn register_flash_storage(storage: *mut esp_storage::FlashStorage<'static>) {
    GLOBAL_FLASH_STORAGE.store(storage, Ordering::Release);
}

pub(crate) fn with_global_flash_storage<R>(
    f: impl FnOnce(&mut esp_storage::FlashStorage<'static>) -> R,
) -> Option<R> {
    let ptr = GLOBAL_FLASH_STORAGE.load(Ordering::Acquire);
    if ptr.is_null() {
        return None;
    }

    // Safety: caller upholds pointer validity; intended for best-effort panic-time writes.
    Some(f(unsafe { &mut *ptr }))
}

fn default_kv_storage() -> KeyValueStorage {
    KeyValueStorage {
        entries: alloc::vec![key_value_storage::Entry {
            key: key_value_storage::Key::Config as i32,
            value: Some(key_value_storage::Value {
                value: Some(key_value_storage::value::Value::Config(default_config())),
            }),
        }],
    }
}

fn extract_config(kv: &KeyValueStorage) -> Option<Config> {
    kv.entries
        .iter()
        .find(|e| e.key == key_value_storage::Key::Config as i32)
        .and_then(|e| e.value.as_ref())
        .and_then(|v| v.value.as_ref())
        .and_then(|oneof| match oneof {
            key_value_storage::value::Value::Config(cfg) => Some(cfg.clone()),
            _ => None,
        })
}

fn extract_backtrace(kv: &KeyValueStorage) -> Option<Backtrace> {
    kv.entries
        .iter()
        .find(|e| e.key == key_value_storage::Key::Backtrace as i32)
        .and_then(|e| e.value.as_ref())
        .and_then(|v| v.value.as_ref())
        .and_then(|oneof| match oneof {
            key_value_storage::value::Value::Backtrace(bt) => Some(bt.clone()),
            _ => None,
        })
}

/// Load the full key/value storage from NVS flash.
///
/// If the storage is empty or corrupt, returns a default store that includes a default config
/// under the `CONFIG` key.
pub fn load(
    storage: &mut esp_storage::FlashStorage<'static>,
) -> Result<KeyValueStorage, &'static str> {
    // Find the NVS partition from the partition table
    let mut pt_mem = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];
    let pt = esp_bootloader_esp_idf::partitions::read_partition_table(storage, &mut pt_mem)
        .map_err(|_| "Failed to read partition table")?;

    let nvs = pt
        .find_partition(esp_bootloader_esp_idf::partitions::PartitionType::Data(
            esp_bootloader_esp_idf::partitions::DataPartitionSubType::Nvs,
        ))
        .map_err(|_| "Failed to find NVS partition")?
        .ok_or("NVS partition not found")?;

    let mut nvs_partition = nvs.as_embedded_storage(storage);
    let mut buffer = [0u8; STORAGE_MAX_SIZE];

    // Read from the NVS partition (offset 0 relative to partition start)
    nvs_partition
        .read(0, &mut buffer)
        .map_err(|_| "Failed to read from flash")?;

    // Check if there's valid data (first 4 bytes should not all be 0xFF for erased flash)
    if buffer[0..4] == [0xFF, 0xFF, 0xFF, 0xFF] {
        return Ok(default_kv_storage());
    }

    // Try to decode the protobuf message
    // First 4 bytes contain the length of the actual data
    let len = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;

    if len == 0 || len > STORAGE_MAX_SIZE - 4 {
        return Ok(default_kv_storage());
    }

    KeyValueStorage::decode(&buffer[4..4 + len])
        .map_err(|_| "Failed to decode storage from flash")
        .or_else(|_| Ok(default_kv_storage()))
}

/// Convenience helper: load the `Config` from `KeyValueStorage`.
pub fn load_config(
    storage: &mut esp_storage::FlashStorage<'static>,
) -> Result<Config, &'static str> {
    let kv = load(storage)?;
    Ok(extract_config(&kv).unwrap_or_else(default_config))
}

/// Load the stored panic backtrace (if any) from `KeyValueStorage`.
pub fn load_backtrace(
    storage: &mut esp_storage::FlashStorage<'static>,
) -> Result<Option<Backtrace>, &'static str> {
    let kv = load(storage)?;
    Ok(extract_backtrace(&kv))
}

/// Save a single entry to NVS flash storage.
///
/// Loads the current store, removes any existing entry for `key`, appends the new entry,
/// and writes back the full store.
pub fn save(
    storage: &mut esp_storage::FlashStorage<'static>,
    key: key_value_storage::Key,
    value: key_value_storage::Value,
) -> Result<(), &'static str> {
    let mut kv = load(storage)?;
    kv.entries.retain(|e| e.key != key as i32);
    kv.entries.push(key_value_storage::Entry {
        key: key as i32,
        value: Some(value),
    });

    // Find the NVS partition from the partition table
    let mut pt_mem = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];
    let pt = esp_bootloader_esp_idf::partitions::read_partition_table(storage, &mut pt_mem)
        .map_err(|_| "Failed to read partition table")?;

    let nvs = pt
        .find_partition(esp_bootloader_esp_idf::partitions::PartitionType::Data(
            esp_bootloader_esp_idf::partitions::DataPartitionSubType::Nvs,
        ))
        .map_err(|_| "Failed to find NVS partition")?
        .ok_or("NVS partition not found")?;

    let mut nvs_partition = nvs.as_embedded_storage(storage);

    info!(
        "NVS partition offset: 0x{:X}, size: {} bytes",
        nvs.offset(),
        nvs.len()
    );
    info!("NVS partition capacity: {} bytes", nvs_partition.capacity());

    // Encode the full store to protobuf
    let mut encoded = Vec::new();
    kv.encode(&mut encoded)
        .map_err(|_| "Failed to encode storage")?;

    info!("Encoded storage size: {} bytes", encoded.len());

    if encoded.len() > STORAGE_MAX_SIZE - 4 {
        return Err("Storage too large to fit in flash storage");
    }

    // Prepare buffer with length prefix - only allocate what we need
    let total_size = 4 + encoded.len();
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
    buffer.extend_from_slice(&encoded);

    info!("Writing {} bytes to NVS partition at offset 0", total_size);

    // Write in small chunks to avoid size limitations
    const CHUNK_SIZE: usize = 32;
    let mut offset = 0;
    while offset < buffer.len() {
        let end = core::cmp::min(offset + CHUNK_SIZE, buffer.len());
        let chunk = &buffer[offset..end];
        info!("Writing chunk at offset {}: {} bytes", offset, chunk.len());
        nvs_partition.write(offset as u32, chunk).map_err(|e| {
            info!("Write error at offset {}: {:?}", offset, e);
            "Failed to write storage to flash"
        })?;
        offset = end;
    }

    info!("Write completed successfully");
    Ok(())
}

/// Returns the default configuration when no config is found in NVS.
pub fn default_config() -> Config {
    Config {
        wifi: Some(WifiConfig {
            ssid: String::from("Quack"),
            password: String::from(""),
        }),
        mta: Some(MtaConfig {
            stations: alloc::vec![
                StationConfig {
                    route: String::from("L"),
                    station_id: String::from("L10"),
                },
                StationConfig {
                    route: String::from("G"),
                    station_id: String::from("G29"),
                },
            ],
        }),
        app_rotation: Some(AppRotationConfig {
            enabled_apps: alloc::vec![
                AppId {
                    id: Some(app_id::Id::Mta(app_id::Mta {}))
                },
                AppId {
                    id: Some(app_id::Id::Plasma(app_id::Plasma {}))
                },
                AppId {
                    id: Some(app_id::Id::Sandbox(app_id::Sandbox {}))
                },
            ],
        }),
    }
}
