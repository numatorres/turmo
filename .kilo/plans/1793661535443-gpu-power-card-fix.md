# Make the GPU PWR card functional (live watts + bar vs power cap)

## Goal
The 4th card in the GPU row ("PWR") currently always shows `"—"` with an empty track. Make it show live GPU power draw in watts, and fill its progress bar relative to the GPU's power limit.

## Root cause (verified on this box)
`gpu.rs::amd_sysfs()` reads `hwmonN/power1_input`, but this driver exposes **no** such file. The actual nodes under `/sys/class/drm/card1/device/hwmon/hwmon2/`:
- `power1_average` = 24000000 → hwmon power is **microwatts** → 24 W (label "PPT")
- `power1_cap` = 300000000 → 300 W limit

So `power_w` is always `None`. Fix: read `power1_average` (fallback `power1_input` for other drivers), and also read `power1_cap` to normalize the bar. Files are world-readable — no permission/udev work needed.

## Decisions
- **Bar fill = draw / cap** (user's choice): 24/300 ≈ 8% now; falls back to empty track (`norm=0`) if the cap is missing or ≤ 0.
- **Fallback chain:** `power1_average` → `power1_input`, first parseable hwmon dir wins. NVIDIA path already queries `power.draw`; unchanged (no bar for NVIDIA — no limit fetched; out of scope).

## Changes (ordered)
1. **`src/sensor/gpu.rs`**
   - Add `pub power_cap_w: Option<f64>` to `GpuValues`.
   - In `amd_sysfs()`, replace the current `power1_input`-only block: for each hwmon dir, try `power1_average` then `power1_input` (µW → `/ 1e6` = W), break on first success; in the same dir read `power1_cap` (µW → W) into `power_cap_w` only when > 0. Update both `GpuValues { … }` literals (`nvidia()` gets `power_cap_w: None`).
2. **`src/sensor/mod.rs`** — add `gpu_power_cap: Option<f64>` to `Readout`; set it in `Readout::new` from `gpu.power_cap_w`. (No signature change; no `main.rs` change.)
3. **`src/frame/composer.rs`**
   - Add helper next to the other norm fns:
     ```rust
     fn norm_power(v: Option<f64>, cap: Option<f64>) -> f64 {
         match (v, cap) {
             (Some(p), Some(c)) if c > 0.0 => (p / c).clamp(0.0, 1.0),
             _ => 0.0,
         }
     }
     ```
   - PWR card call: pass `norm_power(r.gpu_power, r.gpu_power_cap)` instead of `0.0`.
4. **Tests**
   - Update the three test `Readout { … }` literals in composer.rs (preview test, `compose_light_layout_cards_text_and_alarm`, `compose_renders_power_and_freq_cards`) with `gpu_power_cap` values (e.g. preview: cap 300; layout test: None; power/freq test: Some(300)).
   - Add `norm_power` unit tests: `(24,300)≈0.08`, overshoot clamps to 1.0, `cap=0`/`None` → 0.0.

## Validation
- `cargo clippy --all-targets` clean; `cargo test` all pass; `cargo build --release`.
- Live check on the panel: `./target/release/turmo` → PWR shows ~"24 W" (varies with load) and a small red bar fill (~8%); under GPU load the value and fill rise.

## Failure modes / notes
- Cap unreadable or 0 → empty track, watts still shown. Average unreadable → `"—"`, empty track (as today). Graceful throughout.
- NVIDIA GPUs: value works via `power.draw`; bar stays empty (limit not fetched) — acceptable, out of scope.

## Out of scope
- Red "alarm" state for PWR near cap; NVIDIA power-limit normalization; GPU clock cards from hwmon `freq1/2_input` (they exist and could be a future card).
