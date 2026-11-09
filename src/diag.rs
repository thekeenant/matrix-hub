//! Runtime diagnostics for chasing hangs on a device nobody is watching.
//!
//! - **Breadcrumbs**: counters plus a "what is this thread doing right now"
//!   stage marker per thread. They live in `.noinit` RAM, so they survive the
//!   watchdog reboot and the *previous* boot's values can be read afterwards.
//! - **Hang detection**: the main loop and display thread are watched by the
//!   ESP-IDF task watchdog; the background worker by a stuck-stage check.
//!   Either way the device panics (saving a coredump) and reboots.
//! - **Report**: a JSON snapshot served at `/api/debug`.

use log::{info, warn};
use serde::Serialize;
use serde_json::{Map, Value};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use crate::platform::{self, coredump, HeapStats, TaskInfo};
use matrix_hub_core::named_enum;

// ============================================================================
// Breadcrumbs
// ============================================================================

named_enum! {
    /// One `.noinit` slot per breadcrumb.
    pub enum Crumb {
        BootCount => "boot_count",
        UptimeS => "uptime_s",
        MainFrames => "main_frames",
        MainStage => "main_stage",
        DisplayFrames => "display_frames",
        DisplayStage => "display_stage",
        WorkerStage => "worker_stage",
        /// Uptime (s) when `WorkerStage` last changed
        WorkerStageSinceS => "worker_stage_since_s",
        FetchStarted => "fetch_started",
        FetchOk => "fetch_ok",
        FetchErr => "fetch_err",
        LastFetchBytes => "last_fetch_bytes",
        MaxFetchBytes => "max_fetch_bytes",
        LastFetchMs => "last_fetch_ms",
        LastFetchEndS => "last_fetch_end_s",
        WifiConnected => "wifi_connected",
        WifiReconnects => "wifi_reconnects",
        MinFreeInternal => "min_free_internal",
        MinLargestInternal => "min_largest_internal",
        MinFreePsram => "min_free_psram",
        MinLargestPsram => "min_largest_psram",
    }
}

named_enum! {
    /// Where a thread currently is.
    pub enum Stage {
        Boot => "boot",
        // Main loop
        Input => "input",
        WifiCheck => "wifi_check",
        WifiReconnect => "wifi_reconnect",
        Update => "update",
        Draw => "draw",
        Send => "send",
        Sleep => "sleep",
        // Display thread
        DisplayWait => "display_wait",
        DisplayBrightness => "display_brightness",
        DisplayClear => "display_clear",
        DisplayDraw => "display_draw",
        DisplayFlip => "display_flip",
        // Background worker
        WorkerSleep => "worker_sleep",
        HttpConnect => "http_connect",
        HttpRead => "http_read",
        Parse => "parse",
        Publish => "publish",
    }
}

impl Crumb {
    /// Crumbs whose value is a `Stage`, reported by name
    const fn is_stage(self) -> bool {
        matches!(
            self,
            Self::MainStage | Self::DisplayStage | Self::WorkerStage
        )
    }
}

/// Records which stage one thread is in. Use the `MAIN`, `DISPLAY` and
/// `WORKER` instances: `diag::MAIN.enter(Stage::Draw)`.
pub struct StageTracker(Crumb);

pub const MAIN: StageTracker = StageTracker(Crumb::MainStage);
pub const DISPLAY: StageTracker = StageTracker(Crumb::DisplayStage);
pub const WORKER: StageTracker = StageTracker(Crumb::WorkerStage);

impl StageTracker {
    pub fn enter(&self, stage: Stage) {
        if self.0 == Crumb::WorkerStage {
            set(Crumb::WorkerStageSinceS, uptime_s());
        }
        set(self.0, stage as u32);
    }
}

/// Marks the `.noinit` crumbs as valid. Includes the crumb count so a
/// firmware with a different layout doesn't misread the previous one's.
const MAGIC: u32 = 0xD1A6_0000 | Crumb::COUNT as u32;

/// A worker stuck mid-fetch this long is considered hung. Fetches normally
/// take a few seconds, and the HTTP client times out individual operations
/// after 10s, so this only trips on a genuine hang.
const WORKER_STUCK_S: u32 = 300;

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
static CRUMBS: [AtomicU32; Crumb::COUNT + 1] = [ZERO; Crumb::COUNT + 1];

fn slot(c: Crumb) -> &'static AtomicU32 {
    &CRUMBS[c as usize + 1]
}

pub fn set(c: Crumb, v: u32) {
    slot(c).store(v, Ordering::Relaxed);
}

pub fn get(c: Crumb) -> u32 {
    slot(c).load(Ordering::Relaxed)
}

pub fn inc(c: Crumb) {
    slot(c).fetch_add(1, Ordering::Relaxed);
}

// Each min/max crumb has a single writer thread, so a plain load + store is
// enough. (Also: `fetch_max`/`fetch_min` hit an "Undefined temporary symbol"
// assembler bug in the Xtensa LLVM backend.)
pub fn set_max(c: Crumb, v: u32) {
    if v > get(c) {
        set(c, v);
    }
}

fn set_min(c: Crumb, v: u32) {
    if v < get(c) {
        set(c, v);
    }
}

type CrumbValues = [u32; Crumb::COUNT];

fn snapshot_crumbs() -> CrumbValues {
    core::array::from_fn(|i| get(Crumb::ALL[i]))
}

struct PrevBoot {
    reset_reason: &'static str,
    /// `None` if the noinit RAM didn't hold valid crumbs (e.g. power-on)
    crumbs: Option<CrumbValues>,
}

static PREV_BOOT: OnceLock<PrevBoot> = OnceLock::new();

/// Call once, first thing in `main`. Captures the previous boot's
/// breadcrumbs (if any survived) and resets them for this boot.
pub fn init_boot() {
    let reset_reason = platform::reset_reason();

    let prev =
        (CRUMBS[0].load(Ordering::Relaxed) == MAGIC).then(snapshot_crumbs);
    let boots = prev.map_or(1, |p| p[Crumb::BootCount as usize] + 1);

    for c in &CRUMBS {
        c.store(0, Ordering::Relaxed);
    }
    set(Crumb::BootCount, boots);
    for tracker in [MAIN, DISPLAY, WORKER] {
        tracker.enter(Stage::Boot);
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

    info!("diag: boot #{boots}, reset reason {reset_reason}");
    if let Some(p) = &prev {
        info!("diag: previous boot crumbs: {}", crumbs_json(p));
    }
    let _ = PREV_BOOT.set(PrevBoot {
        reset_reason,
        crumbs: prev,
    });

    // The coredump partition may hold leftover bytes from an older partition
    // layout. Erase non-dumps once so "is there a dump?" checks are reliable.
    if let Some((_, size)) = coredump::location() {
        warn!("diag: coredump from a previous crash is stored ({size} bytes), fetch it from /api/coredump");
    } else if coredump::holds_garbage() {
        warn!("diag: coredump partition holds invalid data, erasing");
        let _ = coredump::erase();
    }
}

/// Call about once per second from the main loop: tracks uptime and heap
/// low-water marks, and checks the background worker for hangs.
pub fn tick_1s() {
    check_worker();

    set(Crumb::UptimeS, uptime_s());
    let h = platform::heap_stats();
    set_min(Crumb::MinFreeInternal, h.internal_free);
    set_min(Crumb::MinLargestInternal, h.internal_largest);
    set_min(Crumb::MinFreePsram, h.psram_free);
    set_min(Crumb::MinLargestPsram, h.psram_largest);
}

/// The task watchdog only covers the main loop and display thread. The
/// background worker can legitimately block for a while on network I/O, so
/// instead check that it never sits in one mid-fetch stage for too long. If
/// it does, panic to get a coredump of where it's stuck, then reboot.
fn check_worker() {
    let stage = Stage::from_index(get(Crumb::WorkerStage));
    let idle = matches!(stage, Some(Stage::Boot | Stage::WorkerSleep));
    let stuck_for = uptime_s().saturating_sub(get(Crumb::WorkerStageSinceS));
    if !idle && stuck_for > WORKER_STUCK_S {
        let name = stage.map_or("?", Stage::name);
        panic!("diag: background worker stuck in '{name}' for {stuck_for}s");
    }
}

pub fn uptime_ms() -> u64 {
    platform::uptime_ms()
}

pub fn uptime_s() -> u32 {
    (uptime_ms() / 1000) as u32
}

// ============================================================================
// Watchdog
// ============================================================================

/// Subscribe the calling thread to the task watchdog: it must call
/// [`feed_watchdog`] at least every `CONFIG_ESP_TASK_WDT_TIMEOUT_S` seconds
/// or the device panics, writes a coredump, and reboots.
pub fn watch_current_thread() {
    if let Err(e) = platform::subscribe_to_watchdog() {
        warn!("diag: failed to subscribe to task watchdog: {e}");
    }
}

pub use platform::feed_watchdog;

// ============================================================================
// Test hangs
// ============================================================================

/// Set via `POST /api/debug/test-hang` to deliberately wedge the main loop,
/// to verify the watchdog -> coredump -> reboot pipeline end to end.
pub static TEST_HANG: AtomicBool = AtomicBool::new(false);

/// Set via `POST /api/debug/test-hang-worker` to wedge the background
/// worker mid-fetch, to verify the worker hang guard (~5 min).
pub static TEST_HANG_WORKER: AtomicBool = AtomicBool::new(false);

/// Never returns if a test hang of the main loop was requested.
pub fn maybe_test_hang() {
    hang_if_requested(&TEST_HANG, "main loop");
}

/// Never returns if a test hang of the background worker was requested.
pub fn maybe_test_hang_worker() {
    hang_if_requested(&TEST_HANG_WORKER, "background worker");
}

fn hang_if_requested(flag: &AtomicBool, what: &str) {
    if flag.load(Ordering::Relaxed) {
        warn!("diag: test hang requested, blocking {what} forever");
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

// ============================================================================
// Report
// ============================================================================

fn crumbs_json(values: &CrumbValues) -> Value {
    let map: Map<String, Value> = Crumb::ALL
        .iter()
        .zip(values)
        .map(|(&c, &v)| {
            let value = if c.is_stage() {
                Value::from(Stage::from_index(v).map_or("?", Stage::name))
            } else {
                Value::from(v)
            };
            (c.name().to_string(), value)
        })
        .collect();
    Value::Object(map)
}

#[derive(Serialize)]
struct CoredumpReport {
    present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<usize>,
    #[serde(flatten)]
    summary: Option<coredump::Summary>,
}

#[derive(Serialize)]
struct Report {
    firmware: crate::ota::FirmwareInfo,
    uptime_ms: u64,
    reset_reason: Option<&'static str>,
    now: Value,
    prev_boot: Option<Value>,
    heap: HeapStats,
    total_runtime: u64,
    tasks: Vec<TaskInfo>,
    coredump: CoredumpReport,
}

/// Full diagnostic snapshot for `/api/debug`.
pub fn report_json() -> String {
    let (tasks, total_runtime) = platform::tasks();
    let prev = PREV_BOOT.get();
    let dump = coredump::location();

    let report = Report {
        firmware: crate::ota::firmware_info(),
        uptime_ms: uptime_ms(),
        reset_reason: prev.map(|p| p.reset_reason),
        now: crumbs_json(&snapshot_crumbs()),
        prev_boot: prev.and_then(|p| p.crumbs.as_ref()).map(crumbs_json),
        heap: platform::heap_stats(),
        total_runtime,
        tasks,
        coredump: CoredumpReport {
            present: dump.is_some(),
            size: dump.map(|(_, size)| size),
            summary: dump.and_then(|_| coredump::summary()),
        },
    };
    serde_json::to_string(&report)
        .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}
