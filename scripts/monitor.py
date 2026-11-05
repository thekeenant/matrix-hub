#!/usr/bin/env python3
"""Watch a Matrix Hub over WiFi and log its health, e.g. overnight.

Polls http://<ip>/api/debug every N seconds, appends each snapshot to a JSONL
log, and prints a one-line summary. It flags:
  - the main loop / display thread / fetcher no longer making progress
  - a reboot (uptime went backwards) and the reset reason
  - a stored coredump, which it downloads together with a copy of the current
    firmware ELF so it can be decoded later with scripts/decode-coredump.sh

Usage: scripts/monitor.py <device-ip> [--elf FILE] [--interval 30] [--out logs/]

Pass --elf with the exact firmware ELF that is flashed on the device (keep a
copy when flashing), so building other code in the meantime can't mismatch it.
Standard library only.
"""

import argparse
import json
import shutil
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ELF = ROOT / "target/xtensa-esp32s3-espidf/release/matrix-hub"


def fetch(url, timeout=10):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return r.read()


def kb(n):
    return f"{n / 1024:.0f}K" if isinstance(n, (int, float)) else "?"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("ip")
    ap.add_argument("--interval", type=float, default=30)
    ap.add_argument("--out", default=str(ROOT / "logs"))
    ap.add_argument(
        "--elf",
        default=str(ELF),
        help="firmware ELF flashed on the device (default: current release build)",
    )
    args = ap.parse_args()
    elf = Path(args.elf)
    if not elf.exists():
        sys.exit(f"ELF not found: {elf}")

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    log_path = out / f"monitor-{stamp}.jsonl"
    print(f"Logging to {log_path}  (Ctrl+C to stop)")

    prev = None
    downloaded_dump_for_boot = None

    while True:
        ts = datetime.now().isoformat(timespec="seconds")
        try:
            d = json.loads(fetch(f"http://{args.ip}/api/debug"))
        except (urllib.error.URLError, OSError, ValueError) as e:
            with log_path.open("a") as f:
                f.write(json.dumps({"t": ts, "ok": False, "err": str(e)}) + "\n")
            print(f"{ts}  UNREACHABLE: {e}")
            time.sleep(args.interval)
            continue

        with log_path.open("a") as f:
            f.write(json.dumps({"t": ts, "ok": True, "d": d}) + "\n")

        now, heap = d["now"], d["heap"]
        notes = []

        if prev is not None:
            pn = prev["now"]
            if d["uptime_ms"] < prev["uptime_ms"]:
                notes.append(f"REBOOTED (reason {d['reset_reason']})")
            else:
                if now["main_frames"] == pn["main_frames"]:
                    notes.append(f"MAIN LOOP STALLED in {now['main_stage']}")
                if now["display_frames"] == pn["display_frames"]:
                    notes.append(f"DISPLAY STALLED in {now['display_stage']}")
        fetch_age = now["uptime_s"] - now["last_fetch_end_s"]
        if now["fetch_started"] and fetch_age > 300:
            notes.append(f"NO FETCH FOR {fetch_age}s (in {now['worker_stage']})")

        print(
            f"{ts}  up {d['uptime_ms'] // 60000}m"
            f"  frames main={now['main_frames']} disp={now['display_frames']}"
            f"  fetch ok={now['fetch_ok']} err={now['fetch_err']}"
            f" last={kb(now['last_fetch_bytes'])}/{now['last_fetch_ms']}ms"
            f"  wifi={'up' if now['wifi_connected'] else 'DOWN'}"
            f"  int free={kb(heap['internal_free'])} big={kb(heap['internal_largest'])}"
            f"  psram free={kb(heap['psram_free'])} big={kb(heap['psram_largest'])}"
            + (f"  !! {'; '.join(notes)}" if notes else "")
        )

        # Grab a coredump once per boot that reports one
        boot = now["boot_count"]
        if d["coredump"]["present"] and downloaded_dump_for_boot != boot:
            dump_path = out / f"coredump-{stamp}-boot{boot}.bin"
            try:
                dump_path.write_bytes(fetch(f"http://{args.ip}/api/coredump", 60))
                elf_copy = dump_path.with_suffix(".elf")
                shutil.copy2(elf, elf_copy)
                print(f"{ts}  !! Saved coredump to {dump_path}")
                print(f"     summary: {json.dumps(d['coredump'])}")
                print(f"     previous boot: {json.dumps(d['prev_boot'])}")
                print(f"     decode: scripts/decode-coredump.sh {dump_path} {elf_copy}")
                downloaded_dump_for_boot = boot
            except (urllib.error.URLError, OSError) as e:
                print(f"{ts}  coredump download failed: {e}")

        prev = d
        time.sleep(args.interval)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(0)
