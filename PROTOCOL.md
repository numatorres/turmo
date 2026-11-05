# Turmo USB Monitor — Protocol Notes (Phase 0)

Reverse-engineering reference for driving the QinHeng **UsbMonitor** over Linux.
Fill in the `TBD` fields after running the capture, then mirror them into `turmo.toml`.

## Confirmed against hardware (no guesswork needed)

- Device: `1a86:5722`, product string `UsbMonitor` (verified via `/sys/.../idVendor|idProduct`).
- Transport: **USB CDC ACM serial**. Linux exposes it as **`/dev/ttyACM*`** (kernel module `cdc_acm`, major 166). No vendor driver required on Linux.
- The Windows `driver/TURMO_display_driver/*.inf` is an unmodified Microsoft *Indirect Display Driver (IddCx)* template — a Windows-only WDDM path, **not** relevant to this port. Ignore it.
- On the dev box the node was `/dev/ttyACM0`, owned by `nfsnobody:nfsnobody` mode `0660`. Grant access with `udev/99-turmo.rules` (or add your user to `dialout`).

## How to capture the byte layout

Run `tools/capture.sh`. It runs the known-good Windows build under Wine while
`strace -f -e trace=read,write` logs every serial write. In the app, trigger a theme
load / static image and watch the glass. Then read outbound frames from the log:

```
grep -a 'write(' /tmp/turmo-capture/turmo.trace | less
```

(Fallback: USBPcap on a Windows box — see the script's printed instructions.)

## Fields to determine → config mapping (`turmo.toml`)

| Protocol detail (find in capture) | Config key | Default | Notes |
|---|---|---|---|
| Serial baud rate | `baud` | `921600` | Tune if frames garble/drop. |
| One-time init/handshake bytes | `init_bytes` | `""` | Sent once after open, before first frame. |
| Per-frame header/preamble | `frame_header` | `""` | Hex string, e.g. `"a55a"`. |
| Width/height fields present + endianness | `include_size_fields` | `true` | We emit little-endian u16 w,h before pixels. |
| Color depth (bytes/pixel) | `bytes_per_pixel` | `3` | 3 = RGB888; adjust if device wants 16-bit etc. |
| Channel order | `channel_order` | `"rgb"` | Verify with `turmo --test` (red vs blue quadrants). |
| Per-frame footer/trailer | `frame_footer` | `""` | Hex string, e.g. `"0d"`. |
| Update model (full frame vs delta) | — | full frame | If only deltas are supported, add a diff path in `transport`. |
| Max sustainable rate / flow control | `refresh_ms` | `500` | Serial bandwidth caps FPS; prefer smaller updates if needed. |

## Geometry / orientation (confirmed on the dev box)

- The panel is **landscape 480×320**. We compose a **portrait canvas** (`width=320`, `height=480`) and apply `rotation = "cw90"`, which yields an output buffer of `height × width` = **480×320** — matching the native panel, so no scroll/tearing.
- Sending a buffer that does *not* equal the native 480×320 makes the device tile/wrap it → visible scrolling. Always send exactly `width*height*bpp` for the *rotated* size.
- If text appears mirrored/reversed after `cw90`, switch to `rotation = "cw270"` (one-line config change).

Panel size: confirm exact resolution from capture/datasheet; set canvas `width`/`height` so that after `rotation` the output equals the native panel.

## Validation checklist

Phase 1 — transport (challenge #1):
- [ ] After installing the udev rule / dialout access, run `turmo --test --once`. The asymmetric pattern appears on glass.
- [ ] Red top-left / blue bottom-right confirm **no B/R swap** and correct orientation (border is white). If colors are mirrored/swapped, flip `channel_order` to `"bgr"`.
- [ ] Adjust `frame_header`/`include_size_fields`/`baud` until the full frame renders cleanly.

Phase 2 — live values (challenge #2):
- [ ] Run `turmo` (no `--test`). GPU temp + usage% overlay updates at `refresh_ms`.
- [ ] Values match ground truth (`nvidia-smi` or `/sys/class/drm/*/device/hwmon/`).

## Manual runbook (dev box)

```
sudo cp udev/99-turmo.rules /etc/udev/rules.d/ && sudo udevadm control --reload-rules && sudo udevadm trigger
sudo usermod -aG dialout $USER   # then re-login / newgrp dialout
cargo build --release
./target/release/turmo --test --once          # validate transport
./target/release/turmo                        # live GPU readout
```

## Per-frame home / cursor reset (fixes post-interrupt shift)

If a frame transmission is interrupted mid-write (process killed, USB blip), the device's
internal write pointer is left mid-RAM; the next run writes its full frame from that offset →
the image appears **shifted/wrapped**. Fix: force the pointer home before *every* frame.

- The hook already exists: `frame_header` (`turmo.toml`) is prepended to every frame's pixels by
  `Framing::encode`. Put the winning per-frame home bytes there — no Rust change needed.
- `init_bytes` is once-at-open only, so it does **not** fix this (an interrupt can land between frames).

### Finding the home bytes on hardware (`--probe-home`)

The probe mode reproduces a known mid-buffer offset, applies one candidate, then sends one full
asymmetric test pattern so any shift is immediately visible on glass:

```
./target/release/turmo --probe-home none        # baseline — expect a visible SHIFT (sanity check)
./target/release/turmo --probe-home 1b48        # try ESC H (cursor home)
./target/release/turmo --probe-home 00          # try NUL
./target/release/turmo --probe-home ff          # try 0xFF
./target/release/turmo --probe-home reset       # USB port reset (needs sudo — see below)
```

`--offset-bytes <N>` overrides the offset (default: ¼ of one frame). The first candidate that leaves
the pattern aligned is your `frame_header`. Record it here and in `turmo.toml`.

### Notes on the reset candidate (`reset`)

`--probe-home reset` tries two mechanisms in order, both of which re-home the panel:

1. **USB port reset** on `/dev/bus/usb/<bus>/<dev>` (resolved via sysfs). A CDC-ACM *tty* rejects
   `USBDEVFS_RESET` with ENOTTY; the raw node is required. On this box the kernel refuses to port-reset this
   driver-bound device — EPERM even as root — so step 1 fails here.
2. **cdc_acm sysfs rebind** (fallback): unbind + rebind the device's `cdc_acm` interface instances via
   `/sys/bus/usb/drivers/cdc_acm/{unbind,bind}`, forcing a full ACM re-init → pointer home. Needs **root**.

So on this box: `sudo ./target/release/turmo --probe-home reset`. A physical USB unplug/replug is the
no-privilege equivalent (power cycle → pointer home), and byte candidates via `--probe-home <hex>` need no privilege.
