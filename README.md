# turmo

A small **Rust** application that drives a **QinHeng USB Monitor** (`USB VID:PID 1a86:5722`, product string `UsbMonitor`) on **Linux** to render a live hardware dashboard on the panel. It is a from-scratch Linux port of an originally Windows x86-64 tool: the goal is to (1) get pixel frames onto the USB display and (2) fill those frames with live system values (GPU/CPU temperature, usage, VRAM, power, RAM, clock).

```
sensors ──▶ Readout ──▶ Composer ──▶ Frame (portrait canvas) ──▶ rotate ──▶ Framing ──▶ ByteSink ──▶ /dev/ttyACM* ──▶ panel
 sysfs/proc   (per-frame)  (draw 8 cards)      320×480            cw90      header+px+footer   serial    CDC-ACM      480×320
```

## What it displays

The panel is a landscape **480×320** screen. `turmo` composes a portrait **320×480** canvas and rotates it 90° clockwise (`rotation = "cw90"`) so the output buffer is exactly the native 480×320 — avoiding the tile/wrap scrolling that a mismatched buffer causes.

The dashboard is two model banners (GPU / CPU, names detected once at startup) plus **8 metric cards**:

| Row  | Cards (left → right)                                    |
|------|---------------------------------------------------------|
| GPU  | `TEMP` · `USAGE` · `VRAM` · `PWR`                       |
| CPU  | `TEMP` · `USAGE` · `RAM`  · `FREQ`                      |

Each card shows a value and a progress bar. Bars are normalized against a meaningful denominator — temperature vs. ~90 °C, usage/VRAM/RAM vs. 100 %, **power vs. the GPU's power cap**, and **CPU clock vs. its max frequency**. The two temperature cards turn **red** at or above their configured limits (`gpu_temp_limit` = 85 °C, `cpu_temp_limit` = 90 °C by default); other cards never alarm. Any value whose sensor is unavailable renders as `—` with an empty bar.

## The two hard parts, and how they're solved

**1. Getting images onto the USB screen.** The device is a **USB CDC-ACM serial** gadget — the Linux kernel exposes it as `/dev/ttyACM*` (module `cdc_acm`), so **no vendor driver is needed**. The Windows `driver/` tree is an unmodified Microsoft *Indirect Display Driver (IddCx)* template and is irrelevant to this port. A frame is a sequence of raw bytes: an optional per-frame header, an optional little-endian `width`/`height` pair, the RGB888 pixel buffer, and an optional footer. All of those knobs are config-driven (see `turmo.toml.example`) and were finalized by capturing the known-good Windows build under Wine with `strace` (see `PROTOCOL.md`). A second, optional **raw USB bulk** transport (`--features bulk`, `libusb`) exists for panels that take frames over a bulk endpoint instead of the ACM tty.

A subtle device quirk: if a frame write is interrupted (process killed, USB blip), the panel's internal write pointer is left mid-RAM and the next frame appears shifted/wrapped. `turmo` fixes this by prepending a **per-frame home/reset sequence** (`frame_header`) before every frame, and ships a `--probe-home` mode to find the right byte sequence on your hardware.

**2. Getting live values into the image.** Values are read Linux-natively from `sysfs`/`proc` each refresh (no external daemons):

| Metric        | Source |
|---------------|--------|
| GPU temp / usage / VRAM / power | `nvidia-smi` (NVIDIA) or `/sys/class/drm/card*/device/` + its `hwmon` (AMD/Intel); power from `power1_average`/`power1_input` (µW) and cap from `power1_cap` |
| CPU temp      | `/sys/class/hwmon/*/temp*_input` (package reading preferred) |
| CPU usage %   | aggregate busy/total delta from `/proc/stat` (stateful meter) |
| RAM %         | `/proc/meminfo` |
| CPU clock / max | `/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq` (kHz) and `cpuinfo_max_freq`/`scaling_max_freq` |

## Architecture

- **`transport`** — a minimal `ByteSink` trait (`open` / `init` / `write_all` / `close` / optional `reset`) with a `serial` implementation (default) and an optional `bulk` implementation. It owns only the wire lifecycle.
- **`frame::framing`** — `Framing` builds the on-wire byte layout (header + size fields + pixels + footer) from a logical frame. Kept separate from transport so it's unit-testable without hardware.
- **`frame::composer`** — `Composer` draws the banners and cards onto an RGB888 canvas (rounded panels, antialiased text via `rusttype`, progress bars), and produces the test pattern.
- **`sensor`** — `GpuValues` / `Readout` aggregate the per-frame hardware sample; `cpu`, `gpu`, `mem`, and `models` are the individual sources.
- **`config`** — loads `turmo.toml` (all keys optional, sensible defaults) and the CLI overrides.
- **`main`** — argument parsing, the refresh loop, and the one-shot `--probe-home` helper.

## Requirements

- Linux with the device enumerated as a CDC-ACM tty (`/dev/ttyACM*`); `udev` rule `udev/99-turmo.rules` (or membership in the `dialout` group) for non-root access.
- Rust toolchain (edition 2021).
- For AMD/Intel GPU sensors: the kernel `drm` + `hwmon` sysfs (standard). For NVIDIA: `nvidia-smi` on `PATH`.
- Only for the optional bulk transport: system `libusb` and `--features bulk`.

## Build

```sh
cargo build --release
# optional raw-USB-bulk transport:
cargo build --release --features bulk
```

## Configure

```sh
cp turmo.toml.example turmo.toml    # then adjust device/baud/framing to your capture
```

Key settings: `device`, `baud`, `width`/`height`/`rotation`, `bytes_per_pixel`, `channel_order`, `init_bytes`/`frame_header`/`frame_footer`, `include_size_fields`, `refresh_ms`, `bg_image`, `sensors`, `font_scale`, `gpu_temp_limit`, `cpu_temp_limit`. Protocol values should be confirmed from `PROTOCOL.md` (Phase 0 capture).

## Run

```sh
# device access (once):
sudo cp udev/99-turmo.rules /etc/udev/rules.d/ && sudo udevadm control --reload-rules && sudo udevadm trigger
sudo usermod -aG dialout $USER        # then re-login / newgrp dialout

# 1) validate transport + orientation (asymmetric pattern; red TL / blue BR = no B/R swap)
./target/release/turmo --test --once

# 2) live dashboard
./target/release/turmo
```

### CLI

```
turmo [OPTIONS]
 -c, --config <PATH>       config file (default ./turmo.toml)
     --device <PATH>       device node override (e.g. /dev/ttyACM0)
     --baud <N>            serial baud rate override
     --width/--height <N>  canvas size override
     --bpp <N>             bytes/pixel: 1|2|3
     --rotation <R>        none|cw90|cw180|cw270
     --no-size-fields      omit the width/height header
     --init <HEX>          one-time bytes after open
     --frame-header <HEX>  bytes before every frame's pixels
     --probe-home <V>      one-shot probe: hex | reset | none
     --offset-bytes <N>    probe offset (default 1/4 frame)
     --test                send test pattern (no sensor text)
     --once                send a single frame, then exit
 -h, --help                show help
```

## Project layout

```
src/
  main.rs            entry point, CLI, refresh loop, probe mode
  config.rs          turmo.toml loading + defaults
  font/              embedded TTF loading + antialiased text
  frame/
    framing.rs       on-wire byte layout (header/size/pixels/footer)
    composer.rs      canvas drawing: banners, cards, bars, test pattern
  sensor/
    gpu.rs  cpu.rs  mem.rs  models.rs
  transport/
    serial.rs        CDC-ACM tty sink (default)
    bulk.rs          raw USB bulk sink (optional, --features bulk)
assets/              bundled fonts
udev/99-turmo.rules  device permission rule
tools/capture.sh     Wine + strace capture of the Windows byte protocol
PROTOCOL.md          reverse-engineering reference + validation runbook
```

## License

GPL-3.0 — see [LICENSE](LICENSE).
