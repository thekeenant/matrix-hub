//! Safe wrappers around the raw ESP-IDF APIs this firmware uses.
//!
//! All direct ESP-IDF FFI lives here (plus the HUB75 driver binding in
//! `display.rs`), so the rest of the crate stays free of `unsafe`.
#![allow(
    unsafe_code,
    reason = "This module is the safe boundary around ESP-IDF FFI"
)]

use core::ffi::CStr;
use esp_idf_svc::netif::EspNetif;
use esp_idf_svc::sys::{self, esp, EspError};
use serde::Serialize;

// ============================================================================
// Time and system
// ============================================================================

/// Time since boot, from the high-resolution ESP timer.
pub fn uptime_ms() -> u64 {
    // SAFETY: no preconditions; reads a monotonic hardware timer.
    let us = unsafe { sys::esp_timer_get_time() };
    u64::try_from(us / 1000).unwrap_or(0)
}

/// Why the chip last reset, e.g. `"POWERON"`, `"PANIC"`, `"TASK_WDT"`.
pub fn reset_reason() -> &'static str {
    // SAFETY: no preconditions.
    match unsafe { sys::esp_reset_reason() } {
        sys::esp_reset_reason_t_ESP_RST_POWERON => "POWERON",
        sys::esp_reset_reason_t_ESP_RST_EXT => "EXT",
        sys::esp_reset_reason_t_ESP_RST_SW => "SW",
        sys::esp_reset_reason_t_ESP_RST_PANIC => "PANIC",
        sys::esp_reset_reason_t_ESP_RST_INT_WDT => "INT_WDT",
        sys::esp_reset_reason_t_ESP_RST_TASK_WDT => "TASK_WDT",
        sys::esp_reset_reason_t_ESP_RST_WDT => "WDT",
        sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP => "DEEPSLEEP",
        sys::esp_reset_reason_t_ESP_RST_BROWNOUT => "BROWNOUT",
        sys::esp_reset_reason_t_ESP_RST_SDIO => "SDIO",
        sys::esp_reset_reason_t_ESP_RST_USB => "USB",
        sys::esp_reset_reason_t_ESP_RST_JTAG => "JTAG",
        sys::esp_reset_reason_t_ESP_RST_EFUSE => "EFUSE",
        sys::esp_reset_reason_t_ESP_RST_PWR_GLITCH => "PWR_GLITCH",
        sys::esp_reset_reason_t_ESP_RST_CPU_LOCKUP => "CPU_LOCKUP",
        _ => "UNKNOWN",
    }
}

/// Change the FreeRTOS priority of the calling task.
pub fn set_current_task_priority(priority: u32) {
    // SAFETY: a null handle means "the calling task", which is always valid.
    unsafe { sys::vTaskPrioritySet(core::ptr::null_mut(), priority) }
}

// ============================================================================
// Task watchdog
// ============================================================================

/// Subscribe the calling task to the task watchdog. It must then call
/// [`feed_watchdog`] at least every `CONFIG_ESP_TASK_WDT_TIMEOUT_S` seconds,
/// or the device panics (saving a coredump) and reboots.
pub fn subscribe_to_watchdog() -> Result<(), EspError> {
    // SAFETY: a null handle subscribes the calling task.
    esp!(unsafe { sys::esp_task_wdt_add(core::ptr::null_mut()) })
}

pub fn feed_watchdog() {
    // SAFETY: no preconditions; a no-op error for unsubscribed tasks.
    unsafe { sys::esp_task_wdt_reset() };
}

// ============================================================================
// Heap
// ============================================================================

#[derive(Clone, Copy, Debug, Serialize)]
pub struct HeapStats {
    pub internal_free: u32,
    pub internal_largest: u32,
    pub internal_min_ever: u32,
    pub psram_free: u32,
    pub psram_largest: u32,
    pub psram_min_ever: u32,
}

pub fn heap_stats() -> HeapStats {
    fn stats(caps: u32) -> (u32, u32, u32) {
        // SAFETY: these only read allocator bookkeeping.
        unsafe {
            (
                sys::heap_caps_get_free_size(caps) as u32,
                sys::heap_caps_get_largest_free_block(caps) as u32,
                sys::heap_caps_get_minimum_free_size(caps) as u32,
            )
        }
    }
    let (internal_free, internal_largest, internal_min_ever) =
        stats(sys::MALLOC_CAP_INTERNAL | sys::MALLOC_CAP_8BIT);
    let (psram_free, psram_largest, psram_min_ever) =
        stats(sys::MALLOC_CAP_SPIRAM);
    HeapStats {
        internal_free,
        internal_largest,
        internal_min_ever,
        psram_free,
        psram_largest,
        psram_min_ever,
    }
}

// ============================================================================
// FreeRTOS tasks
// ============================================================================

#[derive(Clone, Debug, Serialize)]
pub struct TaskInfo {
    pub name: String,
    pub state: &'static str,
    pub prio: u32,
    /// Core the task is pinned to; `None` if it may run on either
    pub core: Option<u32>,
    /// Lowest free stack ever observed, in bytes
    pub stack_free_min: u32,
    /// CPU time consumed (run-time stats clock ticks)
    pub runtime: u64,
}

/// Snapshot of all FreeRTOS tasks, plus the total run-time counter.
pub fn tasks() -> (Vec<TaskInfo>, u64) {
    // SAFETY: uxTaskGetSystemState fills at most `capacity` entries of a
    // buffer we own and returns how many it wrote; pcTaskName points at the
    // NUL-terminated name inside each live TCB, copied out immediately.
    unsafe {
        // Headroom in case tasks are created between the two calls
        let capacity = sys::uxTaskGetNumberOfTasks() as usize + 4;
        let mut statuses: Vec<sys::TaskStatus_t> =
            (0..capacity).map(|_| core::mem::zeroed()).collect();
        let mut total_runtime = 0;
        let n = sys::uxTaskGetSystemState(
            statuses.as_mut_ptr(),
            capacity as _,
            &mut total_runtime,
        ) as usize;

        let tasks = statuses[..n]
            .iter()
            .map(|t| TaskInfo {
                name: CStr::from_ptr(t.pcTaskName)
                    .to_string_lossy()
                    .into_owned(),
                state: match t.eCurrentState {
                    sys::eTaskState_eRunning => "running",
                    sys::eTaskState_eReady => "ready",
                    sys::eTaskState_eBlocked => "blocked",
                    sys::eTaskState_eSuspended => "suspended",
                    sys::eTaskState_eDeleted => "deleted",
                    _ => "invalid",
                },
                prio: t.uxCurrentPriority,
                // tskNO_AFFINITY is INT_MAX
                core: u32::try_from(t.xCoreID).ok().filter(|&c| c < 8),
                stack_free_min: t.usStackHighWaterMark,
                runtime: t.ulRunTimeCounter as u64,
            })
            .collect();
        (tasks, total_runtime as u64)
    }
}

// ============================================================================
// Coredump partition
// ============================================================================

pub mod coredump {
    use super::*;

    /// Summary of the stored coredump's crashing task.
    #[derive(Clone, Debug, Serialize)]
    pub struct Summary {
        pub task: String,
        pub pc: String,
        pub backtrace: Vec<String>,
        pub backtrace_corrupted: bool,
    }

    /// Flash address and size of the stored image, valid or not.
    fn image() -> Option<(usize, usize)> {
        let (mut addr, mut size) = (0, 0);
        // SAFETY: writes two out-parameters we own.
        let found =
            unsafe { sys::esp_core_dump_image_get(&mut addr, &mut size) };
        (found == sys::ESP_OK).then_some((addr, size))
    }

    fn checksum_ok() -> bool {
        // SAFETY: no preconditions; reads the coredump partition.
        unsafe { sys::esp_core_dump_image_check() == sys::ESP_OK }
    }

    /// Flash address and size of a *valid* stored coredump, if any.
    pub fn location() -> Option<(usize, usize)> {
        image().filter(|_| checksum_ok())
    }

    /// True if the partition holds something that isn't a valid dump (e.g.
    /// leftover bytes from an older partition layout).
    pub fn holds_garbage() -> bool {
        image().is_some() && !checksum_ok()
    }

    pub fn erase() -> Result<(), EspError> {
        // SAFETY: no preconditions; erases the coredump partition.
        esp!(unsafe { sys::esp_core_dump_image_erase() })
    }

    /// Read `buf.len()` bytes of flash at `addr`.
    pub fn read(addr: usize, buf: &mut [u8]) -> Result<(), EspError> {
        // SAFETY: writes at most buf.len() bytes into `buf`; a null chip
        // selects the default flash chip.
        esp!(unsafe {
            sys::esp_flash_read(
                core::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                addr as u32,
                buf.len() as u32,
            )
        })
    }

    /// The crashing task, PC, and backtrace recorded in the stored dump.
    pub fn summary() -> Option<Summary> {
        // SAFETY: fills a zero-initialised plain-data struct we own; exc_task
        // is a NUL-terminated array inside it.
        let s = unsafe {
            let mut s: sys::esp_core_dump_summary_t = core::mem::zeroed();
            if sys::esp_core_dump_get_summary(&mut s) != sys::ESP_OK {
                return None;
            }
            s
        };
        // SAFETY: exc_task is NUL-terminated by esp_core_dump_get_summary.
        let task = unsafe { CStr::from_ptr(s.exc_task.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let bt = &s.exc_bt_info;
        let depth = (bt.depth as usize).min(bt.bt.len());
        Some(Summary {
            task,
            pc: format!("0x{:08x}", s.exc_pc),
            backtrace: bt.bt[..depth]
                .iter()
                .map(|pc| format!("0x{pc:08x}"))
                .collect(),
            backtrace_corrupted: bt.corrupted,
        })
    }
}

// ============================================================================
// Networking
// ============================================================================

/// Make the DHCP server on `ap` hand out the AP's own address as the DNS
/// server, so clients send their lookups to the captive-portal responder.
pub fn advertise_self_as_dns(ap: &EspNetif) {
    use esp_idf_svc::handle::RawHandle;

    let netif = ap.handle() as *mut _;
    // SAFETY: `netif` is a live esp_netif owned by `ap`; the structs passed
    // are zero-initialised plain data we own, and the DHCP option value is
    // a single byte as the API expects.
    unsafe {
        let mut ip_info: sys::esp_netif_ip_info_t = core::mem::zeroed();
        sys::esp_netif_get_ip_info(netif, &mut ip_info);

        let mut dns_info: sys::esp_netif_dns_info_t = core::mem::zeroed();
        dns_info.ip.type_ = sys::ESP_IPADDR_TYPE_V4 as _;
        dns_info.ip.u_addr.ip4 = ip_info.ip;
        sys::esp_netif_set_dns_info(
            netif,
            sys::esp_netif_dns_type_t_ESP_NETIF_DNS_MAIN,
            &mut dns_info,
        );

        let mut offer_dns: u8 = 1; // OFFER_DNS_MAIN
        sys::esp_netif_dhcps_option(
            netif,
            sys::esp_netif_dhcp_option_mode_t_ESP_NETIF_OP_SET,
            sys::esp_netif_dhcp_option_id_t_ESP_NETIF_DOMAIN_NAME_SERVER,
            (&raw mut offer_dns).cast(),
            1,
        );
    }
}
