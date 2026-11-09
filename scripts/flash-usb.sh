#!/usr/bin/env bash
# Flash over USB: bootloader (built by ESP-IDF, with OTA rollback support),
# partition table, and app into the first OTA slot. Resets the OTA slot
# choice so the board boots what was just flashed. Also the cargo runner.
#
# Usage: scripts/flash-usb.sh [firmware.elf]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ELF="${1:-$ROOT/target/xtensa-esp32s3-espidf/release/matrix-hub}"
BOOTLOADER="$(dirname "$ELF")/bootloader.bin"

# Keep the ELF: coredumps can only be decoded against the exact build
mkdir -p "$ROOT/logs"
cp "$ELF" "$ROOT/logs/firmware-$(git -C "$ROOT" describe --always --dirty).elf"

${DRY_RUN:+echo} espflash flash \
    --bootloader "$BOOTLOADER" \
    --partition-table "$ROOT/partitions.csv" \
    --erase-data-parts otadata \
    --target-app-partition ota_0 \
    --flash-size 8mb \
    --non-interactive \
    "$ELF"
