#!/usr/bin/env bash
# Phase 0: capture the on-wire byte protocol for the Turmo USB Monitor (1a86:5722).
#
# The device is closed-firmware, so we learn its framing empirically by running the
# known-good Windows build under Wine and tracing the serial bytes it writes to
# /dev/ttyACM*. Record what you find in ../PROTOCOL.md, then fill turmo.toml.
#
# This script is read-only/diagnostic: it never reprograms the device.

set -euo pipefail
cd "$(dirname "$0")/.."

REPO="$(pwd)"
EXE="$REPO/windows-x64-installer/TURMO.exe"
OUT_DIR="${TMPDIR:-/tmp}/turmo-capture"

echo "==> Locating Turmo CDC serial node(s)..."
nodes="$(ls /dev/ttyACM* 2>/dev/null || true)"
if [ -z "$nodes" ]; then
    echo "No /dev/ttyACM* found. Is the monitor plugged in? (check: lsusb | grep 1a86)"
else
    printf '  %s\n' $nodes
fi

echo
echo "==> Windows build present at: $EXE"
[ -f "$EXE" ] || { echo "  (not found — the capture needs TURMO.exe)"; }

have=()
command -v wine   >/dev/null 2>&1 && have+=("wine")
command -v strace >/dev/null 2>&1 && have+=("strace")
echo "==> Tools available: ${have[*]:-none}"

if [ "${#have[@]}" -ge 2 ] && [ -f "$EXE" ]; then
    mkdir -p "$OUT_DIR"
    log="$OUT_DIR/turmo.trace"
    echo
    echo "==> Capturing serial I/O from the Windows build under Wine + strace."
    echo "    Output: $log"
    echo "    Start it, then in the Turmo app trigger a theme load / static image,"
    echo "    and watch the screen. Stop with Ctrl-C."
    echo
    # Trace read/write syscalls (large buffers) from wine and its children.
    strace -f -e trace=read,write -s 8192 -o "$log" \
        wine "$EXE"
    echo
    echo "==> Done. Now inspect the captured bytes:"
    echo "    grep -a 'write(' $log | less      # outbound frames/handshake to device"
    echo "    Record header/size/color/footer + any init sequence in ../PROTOCOL.md,"
    echo "    then set frame_header/frame_footer/init_bytes/baud/channel_order in turmo.toml."
else
    cat <<'EOF'

Missing tools or the Windows build. To capture manually:

  PATH A (recommended, on this Linux box):
    sudo apt-get install wine strace          # (or your distro's equivalents)
    then re-run: tools/capture.sh
    It runs `wine TURMO.exe` under strace and logs the serial bytes to /tmp.

  PATH B (on a Windows machine with the monitor attached):
    - Install USBPcap (https://www.netgresql.org/usbpcap/) for the 1a86:5722 device.
    - Start Wireshark, capture on that interface, trigger a theme in TURMO.exe, stop.
    - Export the bulk/serial frames to PCAP and read the byte layout from it.

  WHAT TO RECORD IN ../PROTOCOL.md (and mirror into turmo.toml):
    * baud rate
    * any one-time init/handshake bytes sent before a frame
    * per-frame header/preamble bytes
    * width/height fields: present? endianness? u16?
    * color depth + channel order (RGB vs BGR) — verify with `turmo --test`
    * per-frame footer/trailer bytes
    * update model: full frame each refresh, or delta/partial updates
    * max sustainable rate / any flow control

EOF
fi
