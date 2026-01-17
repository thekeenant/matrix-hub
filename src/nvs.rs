//! Protobuf-based key-value storage in the NVS partition.
//!
//! This module provides a safe, high-level interface for storing protobuf messages
//! in the ESP32's NVS (Non-Volatile Storage) partition. The `Kvs` struct handles
//! all flash operations including multi-core synchronization to prevent corruption.

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use embedded_storage::{ReadStorage, Storage};
use esp_hal::system::{Cpu, CpuControl};
use log::info;
use portable_atomic::AtomicBool;
use prost::Message;

use crate::proto::app_state::{KeyValueStorage, key_value_storage};

const STORAGE_MAX_SIZE: usize = 4096;
const CHUNK_SIZE: usize = 32;

/// Flag indicating core 0 is writing to flash.
pub static CORE_0_WRITING_TO_FLASH: AtomicBool = AtomicBool::new(false);

/// Flag indicating core 1 has paused and is ready for flash operations.
pub static CORE_1_PAUSED: AtomicBool = AtomicBool::new(true);

/// Key-Value Storage wrapper that manages flash storage and provides convenient get/set methods.
///
/// This struct encapsulates all flash operations and handles multi-core synchronization
/// automatically. Operations that write to flash will safely pause core 1 to prevent
/// cache coherency issues.
pub struct Kvs {
    storage: esp_storage::FlashStorage<'static>,
    cpu_ctrl: CpuControl<'static>,
    kv_storage: KeyValueStorage,
}

impl Kvs {
    /// Open the KVS by loading from flash. If loading fails, uses the provided fallback.
    ///
    /// # Arguments
    /// * `storage` - Flash storage peripheral
    /// * `cpu_ctrl` - CPU control peripheral for multi-core synchronization
    /// * `fallback` - Fallback storage to use if loading from flash fails
    pub fn open(
        mut storage: esp_storage::FlashStorage<'static>,
        cpu_ctrl: CpuControl<'static>,
        fallback: KeyValueStorage,
    ) -> Self {
        let kv_storage = match Self::load(&mut storage) {
            Ok(kv) => {
                info!("Loaded KeyValueStorage with {} entries", kv.entries.len());
                kv
            }
            Err(e) => {
                info!("Failed to load KeyValueStorage ({}), using fallback", e);
                fallback
            }
        };

        Self {
            storage,
            cpu_ctrl,
            kv_storage,
        }
    }

    /// Get a value for the specified key.
    pub fn get(&self, key: key_value_storage::Key) -> Option<&key_value_storage::Value> {
        self.kv_storage
            .entries
            .iter()
            .find(|e| e.key == key as i32)
            .and_then(|e| e.value.as_ref())
    }

    /// Set a value for the specified key and save to flash.
    ///
    /// This method safely pauses the other core during the flash write operation.
    pub fn set(
        &mut self,
        key: key_value_storage::Key,
        value: key_value_storage::Value,
    ) -> Result<(), &'static str> {
        // Update or add the entry
        if let Some(entry) = self
            .kv_storage
            .entries
            .iter_mut()
            .find(|e| e.key == key as i32)
        {
            entry.value = Some(value);
        } else {
            self.kv_storage.entries.push(key_value_storage::Entry {
                key: key as i32,
                value: Some(value),
            });
        }

        self.write_to_flash()
    }

    /// Get a reference to the entire KeyValueStorage.
    pub fn get_all(&self) -> &KeyValueStorage {
        &self.kv_storage
    }

    /// Save the current state to flash without modifying it.
    ///
    /// This method safely pauses the other core during the flash write operation.
    pub fn save(&mut self) -> Result<(), &'static str> {
        self.write_to_flash()
    }

    /// Load KeyValueStorage from NVS flash partition.
    fn load(
        storage: &mut esp_storage::FlashStorage<'static>,
    ) -> Result<KeyValueStorage, &'static str> {
        // Find the NVS partition
        let mut pt_mem = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];
        let pt = esp_bootloader_esp_idf::partitions::read_partition_table(storage, &mut pt_mem)
            .map_err(|_| "Failed to read partition table")?;

        let nvs = pt
            .find_partition(esp_bootloader_esp_idf::partitions::PartitionType::Data(
                esp_bootloader_esp_idf::partitions::DataPartitionSubType::Nvs,
            ))
            .map_err(|_| "Failed to find NVS partition")?
            .ok_or("NVS partition not found")?;

        info!(
            "NVS partition offset: 0x{:X}, size: {} bytes",
            nvs.offset(),
            nvs.len()
        );

        let mut nvs_partition = nvs.as_embedded_storage(storage);
        let mut buffer = [0u8; STORAGE_MAX_SIZE];

        // Read from the NVS partition
        nvs_partition
            .read(0, &mut buffer)
            .map_err(|_| "Failed to read from flash")?;

        // Check if there's valid data (first 4 bytes should not all be 0xFF for erased flash)
        if buffer[0..4] == [0xFF, 0xFF, 0xFF, 0xFF] {
            info!("Flash is empty, returning empty KeyValueStorage");
            return Ok(KeyValueStorage::default());
        }

        // First 4 bytes contain the length of the actual data
        let len = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;

        if len == 0 || len > STORAGE_MAX_SIZE - 4 {
            info!("Invalid length: {}, returning empty KeyValueStorage", len);
            return Ok(KeyValueStorage::default());
        }

        // Decode the protobuf
        KeyValueStorage::decode(&buffer[4..4 + len]).map_err(|e| {
            info!("Failed to decode KeyValueStorage: {:?}", e);
            "Failed to decode protobuf from flash"
        })
    }

    /// Write the current KeyValueStorage to flash with multi-core synchronization.
    #[allow(unsafe_code)]
    fn write_to_flash(&mut self) -> Result<(), &'static str> {
        info!("KVS: Setting CORE_0_WRITING_TO_FLASH flag");
        CORE_0_WRITING_TO_FLASH.store(true, Ordering::Release);

        // Wait for core 1 to acknowledge and pause
        while !CORE_1_PAUSED.load(Ordering::Acquire) {
            core::hint::spin_loop();
        }

        info!("KVS: Parking app core");
        unsafe {
            self.cpu_ctrl.park_core(Cpu::AppCpu);
        }

        // Perform the flash write
        let result = Self::write_storage(&mut self.storage, &self.kv_storage);

        // Unpark and clear flags
        info!("KVS: Unparking app core");
        self.cpu_ctrl.unpark_core(Cpu::AppCpu);
        CORE_0_WRITING_TO_FLASH.store(false, Ordering::Release);

        result
    }

    /// Write KeyValueStorage to the NVS partition.
    ///
    /// Format: [4-byte length][protobuf data]
    fn write_storage(
        storage: &mut esp_storage::FlashStorage<'static>,
        kv_storage: &KeyValueStorage,
    ) -> Result<(), &'static str> {
        // Find the NVS partition
        let mut pt_mem = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];
        let pt = esp_bootloader_esp_idf::partitions::read_partition_table(storage, &mut pt_mem)
            .map_err(|_| "Failed to read partition table")?;

        let nvs = pt
            .find_partition(esp_bootloader_esp_idf::partitions::PartitionType::Data(
                esp_bootloader_esp_idf::partitions::DataPartitionSubType::Nvs,
            ))
            .map_err(|_| "Failed to find NVS partition")?
            .ok_or("NVS partition not found")?;

        info!(
            "NVS partition offset: 0x{:X}, size: {} bytes",
            nvs.offset(),
            nvs.len()
        );

        let mut nvs_partition = nvs.as_embedded_storage(storage);

        // Encode the protobuf
        let mut data = Vec::new();
        kv_storage
            .encode(&mut data)
            .map_err(|_| "Failed to encode protobuf")?;

        if data.len() > STORAGE_MAX_SIZE - 4 {
            return Err("Protobuf too large to fit in flash storage");
        }

        // Prepare buffer with length prefix
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buffer.extend_from_slice(&data);

        info!("Writing {} bytes to NVS partition", buffer.len());

        // Write in small chunks to avoid size limitations
        let mut offset = 0;
        while offset < buffer.len() {
            let end = core::cmp::min(offset + CHUNK_SIZE, buffer.len());
            let chunk = &buffer[offset..end];
            nvs_partition
                .write(offset as u32, chunk)
                .map_err(|_| "Failed to write to flash")?;
            offset = end;
        }

        info!("Write completed successfully");
        Ok(())
    }
}
