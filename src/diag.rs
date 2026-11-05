//! Runtime diagnostics for chasing hangs on a device nobody is watching.
//!
//! - **Breadcrumbs**: counters plus a "what is this thread doing right now"
//!   stage marker per thread. They live in `.noinit` RAM, so they survive the
//!   watchdog reboot and the *previous* boot's values can be read afterwards.
//! - **Watchdog**: helpers to subscribe a thread to the ESP-IDF task watchdog
//!   (configured to panic + coredump + reboot in `sdkconfig.defaults`).
//! - **Snapshot**: a JSON report served at `/api/debug` (heap, task list,
//!   breadcrumbs, previous boot, coredump summary).
//! - **Coredump**: raw access to the coredump partition for `/api/coredump`.

use esp_idf_svc::sys;
use log::{info, warn};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

// ============================================================================
// Breadcrumbs
// ============================================================================

/// One slot per breadcrumb. Order must match `CRUMB_NAMES`.
#[derive(Clone, Copy)]
pub enum Crumb {
    BootCount,
    UptimeS,
    MainFrames,
    MainStage,
    DisplayFrames,
    DisplayStage,
    WorkerStage,
    FetchStarted,
    FetchOk,
    FetchErr,
    LastFetchBytes,
    MaxFetchBytes,
    LastFetchMs,
    LastFetchEndS,
    WifiConnected,
    WifiReconnects,
    MinFreeInternal,
    MinLargestInternal,
    MinFreePsram,
    MinLargestPsram,
}

const CRUMB_COUNT: usize = 20;

const CRUMB_NAMES: [&str; CRUMB_COUNT] = [
    "boot_count",
    "uptime_s",
    "main_frames",
    "main_stage",
    "display_frames",
    "display_stage",
    "worker_stage",
    "fetch_started",
    "fetch_ok",
    "fetch_err",
    "last_fetch_bytes",
    "max_fetch_bytes",
    "last_fetch_ms",
    "last_fetch_end_s",
    "wifi_connected",
    "wifi_reconnects",
    "min_free_internal",
    "min_largest_internal",
    "min_free_psram",
    "min_largest_psram",
];

/// Crumbs whose value is a `Stage` and should be rendered by name.
const STAGE_CRUMBS: [usize; 3] = [
    Crumb::MainStage as usize,
    Crumb::DisplayStage as usize,
    Crumb::WorkerStage as usize,
];

/// Where a thread currently is. Order must match `STAGE_NAMES`.
#[derive(Clone, Copy)]
pub enum Stage {
    Boot,
    // Main loop
    Input,
    Gesture,
    WifiCheck,
    WifiReconnect,
    Update,
    Draw,
    Send,
    Sleep,
    // Display thread
    DisplayWait,
    DisplayBrightness,
    DisplayClear,
    DisplayDraw,
    DisplayFlip,
    // Background worker
    WorkerSleep,
    HttpConnect,
    HttpRead,
    Parse,
    Publish,
}

const STAGE_NAMES: [&str; 19] = [
    "boot",
    "input",
    "gesture",
    "wifi_check",
    "wifi_reconnect",
    "update",
    "draw",
    "send",
    "sleep",
    "display_wait",
    "display_brightness",
    "display_clear",
    "display_draw",
    "display_flip",
    "worker_sleep",
    "http_connect",
    "http_read",
    "parse",
    "publish",
];

const MAGIC: u32 = 0xD1A6_0001;

#[allow(
    clippy::declare_interior_mutable_const,
    reason = "Used only as an array-repeat initializer"
)]
const ZERO: AtomicU32 = AtomicU32::new(0);

/// Slot 0 is a magic marker; slots 1.. are the crumbs.
///
/// Placed in `.noinit` (internal DRAM that the startup code doesn't zero), so
/// values written before a panic/watchdog reset are still there after it.
/// The initializer is ignored; `init_boot` validates via the magic marker.
#[allow(
    unsafe_code,
    reason = "link_section places breadcrumbs in RAM that survives soft resets"
)]
#[link_section = ".noinit.diag"]
static CRUMBS: [AtomicU32; CRUMB_COUNT + 1] = [ZERO; CRUMB_COUNT + 1];

fn slot(c: Crumb) -> &'static AtomicU32 {
    &CRUMBS[c as usize + 1]
}

pub fn set(c: Crumb, v: u32) {
    slot(c).store(v, Ordering::Relaxed);
}

pub fn inc(c: Crumb) {
    slot(c).fetch_add(1, Ordering::Relaxed);
}

// Each min/max crumb has a single writer thread, so a plain load + store is
// enough. (Also: `fetch_max`/`fetch_min` hit an "Undefined temporary symbol"
// assembler bug in the Xtensa LLVM backend.)
pub fn set_max(c: Crumb, v: u32) {
    if v > slot(c).load(Ordering::Relaxed) {
        slot(c).store(v, Ordering::Relaxed);
    }
}

pub fn set_min(c: Crumb, v: u32) {
    if v < slot(c).load(Ordering::Relaxed) {
        slot(c).store(v, Ordering::Relaxed);
    }
}

pub fn stage(c: Crumb, s: Stage) {
    set(c, s as u32);
}

struct PrevBoot {
    reset_reason: u32,
    /// `None` if the noinit RAM didn't hold valid crumbs (e.g. power-on).
    crumbs: Option<[u32; CRUMB_COUNT]>,
}

static PREV_BOOT: OnceLock<PrevBoot> = OnceLock::new();

fn snapshot_crumbs() -> [u32; CRUMB_COUNT] {
    let mut out = [0u32; CRUMB_COUNT];
    for (i, v) in out.iter_mut().enumerate() {
        *v = CRUMBS[i + 1].load(Ordering::Relaxed);
    }
    out
}

/// Call once, first thing in `main`. Captures the previous boot's
/// breadcrumbs (if any survived) and resets them for this boot.
pub fn init_boot() {
    #[allow(unsafe_code, reason = "ESP-IDF FFI")]
    let reason = unsafe { sys::esp_reset_reason() } as u32;

    let valid = CRUMBS[0].load(Ordering::Relaxed) == MAGIC;
    let prev = valid.then(snapshot_crumbs);
    let boots = prev.map_or(1, |p| p[Crumb::BootCount as usize] + 1);

    for c in &CRUMBS {
        c.store(0, Ordering::Relaxed);
    }
    set(Crumb::BootCount, boots);
    for c in [Crumb::MainStage, Crumb::DisplayStage, Crumb::WorkerStage] {
        stage(c, Stage::Boot);
    }
    for c in [
        Crumb::MinFreeInternal,
        Crumb::MinLargestInternal,
        Crumb::MinFreePsram,
        Crumb::MinLargestPsram,
    ] {
        set(c, u32::MAX);
    }
    CRUMBS[0].store(MAGIC, Ordering::Relaxed);

    info!(
        "diag: boot #{boots}, reset reason {}",
        reset_reason_name(reason)
    );
    if let Some(p) = &prev {
        info!("diag: previous boot crumbs: {}", crumbs_json(p));
    }

    let _ = PREV_BOOT.set(PrevBoot {
        reset_reason: reason,
        crumbs: prev,
    });

    init_coredump();
}

/// Call about once per second: tracks uptime and heap low-water marks.
pub fn tick_1s() {
    set(Crumb::UptimeS, uptime_s());
    let h = heap_stats();
    set_min(Crumb::MinFreeInternal, h.internal_free);
    set_min(Crumb::MinLargestInternal, h.internal_largest);
    set_min(Crumb::MinFreePsram, h.psram_free);
    set_min(Crumb::MinLargestPsram, h.psram_largest);
}

pub fn uptime_ms() -> u64 {
    #[allow(unsafe_code, reason = "ESP-IDF FFI")]
    let us = unsafe { sys::esp_timer_get_time() };
    (us / 1000) as u64
}

pub fn uptime_s() -> u32 {
    (uptime_ms() / 1000) as u32
}

// ============================================================================
// Task watchdog
// ============================================================================

/// Set via `POST /api/debug/test-hang` to deliberately wedge the main loop,
/// to verify the watchdog -> coredump -> reboot pipeline end to end.
pub static TEST_HANG: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Called from the main loop. Never returns if a test hang was requested.
pub fn maybe_test_hang() {
    if TEST_HANG.load(Ordering::Relaxed) {
        warn!("diag: test hang requested, blocking main loop forever");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
}

/// Subscribe the calling thread to the task watchdog. From now on it must
/// call `wdt_feed` at least every `CONFIG_ESP_TASK_WDT_TIMEOUT_S` seconds or
/// the device panics, writes a coredump, and reboots.
pub fn wdt_subscribe_current() {
    #[allow(unsafe_code, reason = "ESP-IDF FFI")]
    let err = unsafe { sys::esp_task_wdt_add(core::ptr::null_mut()) };
    if err != sys::ESP_OK {
        warn!("diag: esp_task_wdt_add failed: {err}");
    }
}

pub fn wdt_feed() {
    #[allow(unsafe_code, reason = "ESP-IDF FFI")]
    unsafe {
        sys::esp_task_wdt_reset();
    }
}

// ============================================================================
// Heap
// ============================================================================

struct HeapStats {
    internal_free: u32,
    internal_largest: u32,
    internal_min_ever: u32,
    psram_free: u32,
    psram_largest: u32,
    psram_min_ever: u32,
}

fn heap_stats() -> HeapStats {
    let internal = sys::MALLOC_CAP_INTERNAL | sys::MALLOC_CAP_8BIT;
    let psram = sys::MALLOC_CAP_SPIRAM;
    #[allow(unsafe_code, reason = "ESP-IDF FFI")]
    unsafe {
        HeapStats {
            internal_free: sys::heap_caps_get_free_size(internal) as u32,
            internal_largest: sys::heap_caps_get_largest_free_block(internal)
                as u32,
            internal_min_ever: sys::heap_caps_get_minimum_free_size(internal)
                as u32,
            psram_free: sys::heap_caps_get_free_size(psram) as u32,
            psram_largest: sys::heap_caps_get_largest_free_block(psram) as u32,
            psram_min_ever: sys::heap_caps_get_minimum_free_size(psram) as u32,
        }
    }
}

// ============================================================================
// Tasks
// ============================================================================

fn task_list() -> (Vec<Value>, u64) {
    #[allow(unsafe_code, reason = "ESP-IDF FFI")]
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
            .map(|t| {
                let name = core::ffi::CStr::from_ptr(t.pcTaskName)
                    .to_string_lossy()
                    .into_owned();
                let state = match t.eCurrentState {
                    0 => "running",
                    1 => "ready",
                    2 => "blocked",
                    3 => "suspended",
                    4 => "deleted",
                    _ => "invalid",
                };
                json!({
                    "name": name,
                    "state": state,
                    "prio": t.uxCurrentPriority,
                    "core": t.xCoreID,
                    "stack_free_min": t.usStackHighWaterMark,
                    "runtime": t.ulRunTimeCounter,
                })
            })
            .collect();
        (tasks, total_runtime as u64)
    }
}

// ============================================================================
// Coredump
// ============================================================================

/// The coredump partition may hold leftover bytes from the old app partition
/// layout. If what's there isn't a valid dump, erase it once so that later
/// "is there a dump?" checks are reliable.
fn init_coredump() {
    #[allow(unsafe_code, reason = "ESP-IDF FFI")]
    unsafe {
        let mut addr = 0;
        let mut size = 0;
        let has_image =
            sys::esp_core_dump_image_get(&mut addr, &mut size) == sys::ESP_OK;
        if !has_image {
            return;
        }
        if sys::esp_core_dump_image_check() == sys::ESP_OK {
            warn!("diag: coredump from a previous crash is stored ({size} bytes), fetch it from /api/coredump");
        } else {
            warn!("diag: coredump partition holds invalid data, erasing");
            let _ = sys::esp_core_dump_image_erase();
        }
    }
}

/// Flash address and size of a valid stored coredump, if any.
pub fn coredump_location() -> Option<(usize, usize)> {
    #[allow(unsafe_code, reason = "ESP-IDF FFI")]
    unsafe {
        let mut addr = 0;
        let mut size = 0;
        (sys::esp_core_dump_image_get(&mut addr, &mut size) == sys::ESP_OK
            && sys::esp_core_dump_image_check() == sys::ESP_OK)
            .then_some((addr, size))
    }
}

/// Read `buf.len()` bytes of flash at `addr`.
#[allow(unsafe_code, reason = "ESP-IDF FFI")]
pub fn read_flash(addr: usize, buf: &mut [u8]) -> Result<(), sys::EspError> {
    sys::esp!(unsafe {
        sys::esp_flash_read(
            core::ptr::null_mut(), // default flash chip
            buf.as_mut_ptr().cast(),
            addr as u32,
            buf.len() as u32,
        )
    })
}

#[allow(unsafe_code, reason = "ESP-IDF FFI")]
pub fn erase_coredump() -> Result<(), sys::EspError> {
    sys::esp!(unsafe { sys::esp_core_dump_image_erase() })
}

fn coredump_json() -> Value {
    let Some((_, size)) = coredump_location() else {
        return json!({ "present": false });
    };

    #[allow(unsafe_code, reason = "ESP-IDF FFI")]
    let summary = unsafe {
        let mut s: sys::esp_core_dump_summary_t = core::mem::zeroed();
        (sys::esp_core_dump_get_summary(&mut s) == sys::ESP_OK).then_some(s)
    };

    let Some(s) = summary else {
        return json!({ "present": true, "size": size });
    };

    #[allow(unsafe_code, reason = "Reading a NUL-terminated C string")]
    let task = unsafe { core::ffi::CStr::from_ptr(s.exc_task.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let depth = (s.exc_bt_info.depth as usize).min(s.exc_bt_info.bt.len());
    let backtrace: Vec<String> = s.exc_bt_info.bt[..depth]
        .iter()
        .map(|pc| format!("0x{pc:08x}"))
        .collect();

    json!({
        "present": true,
        "size": size,
        "task": task,
        "pc": format!("0x{:08x}", s.exc_pc),
        "backtrace": backtrace,
        "backtrace_corrupted": s.exc_bt_info.corrupted,
    })
}

// ============================================================================
// JSON report
// ============================================================================

fn reset_reason_name(r: u32) -> &'static str {
    match r {
        1 => "POWERON",
        2 => "EXT",
        3 => "SW",
        4 => "PANIC",
        5 => "INT_WDT",
        6 => "TASK_WDT",
        7 => "WDT",
        8 => "DEEPSLEEP",
        9 => "BROWNOUT",
        10 => "SDIO",
        11 => "USB",
        12 => "JTAG",
        13 => "EFUSE",
        14 => "PWR_GLITCH",
        15 => "CPU_LOCKUP",
        _ => "UNKNOWN",
    }
}

fn crumbs_json(values: &[u32; CRUMB_COUNT]) -> Value {
    let mut map = serde_json::Map::new();
    for (i, (&name, &v)) in CRUMB_NAMES.iter().zip(values).enumerate() {
        let value = if STAGE_CRUMBS.contains(&i) {
            json!(STAGE_NAMES.get(v as usize).copied().unwrap_or("?"))
        } else {
            json!(v)
        };
        map.insert(name.to_string(), value);
    }
    Value::Object(map)
}

/// Full diagnostic snapshot for `/api/debug`.
pub fn report_json() -> String {
    let h = heap_stats();
    let (tasks, total_runtime) = task_list();
    let prev = PREV_BOOT.get();

    json!({
        "uptime_ms": uptime_ms(),
        "reset_reason": prev.map(|p| reset_reason_name(p.reset_reason)),
        "now": crumbs_json(&snapshot_crumbs()),
        "prev_boot": prev.and_then(|p| p.crumbs.as_ref()).map(crumbs_json),
        "heap": {
            "internal_free": h.internal_free,
            "internal_largest": h.internal_largest,
            "internal_min_ever": h.internal_min_ever,
            "psram_free": h.psram_free,
            "psram_largest": h.psram_largest,
            "psram_min_ever": h.psram_min_ever,
        },
        "total_runtime": total_runtime,
        "tasks": tasks,
        "coredump": coredump_json(),
    })
    .to_string()
}
