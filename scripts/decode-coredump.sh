#!/usr/bin/env bash
# Decode a coredump downloaded from http://<device>/api/coredump into
# per-task backtraces (which function every thread was in when it died).
#
# Usage: scripts/decode-coredump.sh <coredump.bin> [firmware.elf]
#
# The ELF must be the exact build that was running on the device. The
# monitor script saves a copy next to each dump; otherwise the current
# release build is used (esp_coredump warns if the SHA doesn't match).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CORE="${1:?usage: $0 <coredump.bin> [firmware.elf]}"
ELF="${2:-$ROOT/target/xtensa-esp32s3-espidf/release/matrix-hub}"

export IDF_TOOLS_PATH="$ROOT/.embuild/espressif"
IDF="$IDF_TOOLS_PATH/esp-idf/v5.5.3"
PY="$(ls -d "$IDF_TOOLS_PATH"/python_env/idf5.5_py*_env/bin/python | head -1)"

find_gdb() {
    find "$IDF_TOOLS_PATH/tools" -type f -name xtensa-esp32s3-elf-gdb 2>/dev/null | head -1
}

GDB="$(find_gdb)"
if [ -z "$GDB" ]; then
    echo "Installing xtensa-esp-elf-gdb into $IDF_TOOLS_PATH/tools (one time)..."
    IDF_PATH="$IDF" "$PY" "$IDF/tools/idf_tools.py" install xtensa-esp-elf-gdb
    GDB="$(find_gdb)"
fi

# ROM symbols, so frames inside the chip's ROM aren't shown as "??"
ROM_ELF="$(ls "$IDF_TOOLS_PATH"/tools/esp-rom-elfs/*/esp32s3_rev0_rom.elf | head -1)"

# esp_coredump refuses dumps whose embedded app ELF SHA256 doesn't match the
# ELF file. Cargo/ldproxy does the final link outside ESP-IDF, so the device
# always reports an all-zero SHA and the check can never pass. Bypass it by
# making the loader see an all-zero SHA for the ELF too. (Keep each dump with
# the ELF that was running; scripts/monitor.py saves them side by side.)
exec "$PY" -c '
import sys
import esp_coredump.corefile.loader as loader
from esp_coredump.__main__ import main

class ZeroShaElfFile(loader.ElfFile):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.sha256 = bytes(32)

loader.ElfFile = ZeroShaElfFile
sys.argv[0] = "esp_coredump"
main()
' --chip esp32s3 info_corefile \
    --gdb "$GDB" --rom-elf "$ROM_ELF" --core "$CORE" --core-format raw "$ELF"
