#!/usr/bin/env bash
# Update the firmware over WiFi.
#
# Usage: scripts/ota.sh [device-ip] [firmware.elf]
#   The IP defaults to MATRIX_HUB_IP (environment or .env).
#   Build first: cargo build --release
#
# The device installs the image into its spare slot and restarts into it.
# If the new firmware doesn't come up healthy (WiFi + 60s running), the
# device rolls back to the previous firmware on its own.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
env_value() { grep -E "^$1=" "$ROOT/.env" 2>/dev/null | cut -d= -f2-; }
IP="${1:-${MATRIX_HUB_IP:-$(env_value MATRIX_HUB_IP)}}"
if [ -z "$IP" ]; then
    echo "Usage: $0 <device-ip> (or set MATRIX_HUB_IP in .env)" >&2
    exit 1
fi
ELF="${2:-$ROOT/target/xtensa-esp32s3-espidf/release/matrix-hub}"
PASSWORD="$(env_value OTA_PASSWORD)"
VERSION="$(git -C "$ROOT" describe --always --dirty)"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
espflash save-image --chip esp32s3 --flash-size 8mb "$ELF" "$TMP/app.bin" >/dev/null

# Keep the ELF: coredumps can only be decoded against the exact build
mkdir -p "$ROOT/logs"
cp "$ELF" "$ROOT/logs/firmware-$VERSION.elf"

echo "Uploading $VERSION ($(stat -c %s "$TMP/app.bin") bytes) to $IP..."
curl --fail-with-body -sS -m 300 \
    -H "Authorization: Bearer $PASSWORD" \
    -H "Content-Type: application/octet-stream" \
    --data-binary "@$TMP/app.bin" \
    "http://$IP/api/ota"
echo

echo "Waiting for the device to come back..."
for _ in $(seq 1 60); do
    sleep 2
    if INFO="$(curl -sS -m 3 "http://$IP/api/debug" 2>/dev/null)"; then
        python3 -c '
import json, sys
fw = json.loads(sys.argv[1])["firmware"]
print("Running", fw["git"], "from", fw["running_slot"], "(" + fw["state"] + ")")
sys.exit(0 if fw["git"] == sys.argv[2] else 1)' "$INFO" "$VERSION" && {
            echo "Done. It confirms itself after 60s healthy; otherwise it rolls back."
            exit 0
        }
    fi
done
echo "Device didn't report the new version; check http://$IP/api/debug" >&2
exit 1
