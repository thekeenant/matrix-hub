//! Panic-time custom halt hook.
//!
//! `esp-backtrace` is configured to call the `custom_halt()` symbol on panic.

extern crate alloc;

use alloc::vec::Vec;

use crate::{
    nvs,
    proto::app_state::{
        Backtrace as ProtoBacktrace,
        key_value_storage::{self, Key, value::Value},
    },
};

/// Custom halt hook invoked by esp-backtrace.
///
/// Must be `extern "Rust"` with an unmangled name to match esp-backtrace’s expectation.
#[unsafe(no_mangle)]
pub extern "Rust" fn custom_halt() -> ! {
    // Try to persist a backtrace into KeyValueStorage. If anything fails, we still halt.
    let backtrace = esp_backtrace::Backtrace::capture();

    let mut pcs: Vec<u32> = Vec::new();
    for frame in backtrace.frames() {
        pcs.push(frame.program_counter() as u32);
    }

    let proto = ProtoBacktrace {
        program_counters: pcs,
    };

    let _ = nvs::with_global_flash_storage(|storage| {
        let value = key_value_storage::Value {
            value: Some(Value::Backtrace(proto)),
        };
        // Best-effort only.
        let _ = nvs::save(storage, Key::Backtrace, value);
    });

    // Don’t return.
    loop {}
}
