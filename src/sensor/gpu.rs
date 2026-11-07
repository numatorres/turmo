//! GPU temperature + usage sampling with vendor auto-detection.

use crate::sensor::GpuValues;

/// Sample the first available GPU, trying NVIDIA then AMD/intel sysfs.
pub fn detect() -> GpuValues {
    if let Some(v) = nvidia() {
        return v;
    }
    if let Some(v) = amd_sysfs() {
        return v;
    }
    GpuValues::default()
}

/// NVIDIA via `nvidia-smi`. Returns the first GPU's temp (°C) and utilization (%).
fn nvidia() -> Option<GpuValues> {
    let out = std::process::Command::new("nvidia-smi")
        .args([
            // power.draw is reported in watts (or "[N/A]" when unsupported -> parse fails -> None).
            "--query-gpu=temperature_gpu,utilization.gpu,memory.used,memory.total,power.draw",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        // Expected shape: "63, 45, 1200, 8192, 68.4" (temp, usage %, used MiB, total MiB, power W)
        let mut it = line.split(',').map(|x| x.trim().parse::<f64>().ok());
        if let (Some(temp), Some(usage), Some(mem_used), Some(mem_total)) = (
            it.next().flatten(),
            it.next().flatten(),
            it.next().flatten(),
            it.next().flatten(),
        ) {
            // 5th field is power.draw in watts; None when nvidia-smi reports "[N/A]".
            let power_w = it.next().flatten();
            return Some(GpuValues {
                temp_c: Some(temp),
                usage_pct: Some(usage),
                vram_pct: pct_of(mem_used, mem_total),
                power_w,
                power_cap_w: None, // NVIDIA power limit not fetched here (out of scope)
            });
        }
    }
    None
}

/// AMD/intel via `/sys/class/drm/card*/device/`. Temperature comes from hwmon
/// `temp*_input` (millidegrees); utilization from `gpu_busy_percent`.
fn amd_sysfs() -> Option<GpuValues> {
    let cards = std::fs::read_dir("/sys/class/drm").ok()?;
    for entry in cards.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let dev = entry.path().join("device");

        // Temperature: the layout is device/hwmon/<hwmonN>/temp*_input, so descend one
        // level into each hwmon dir and collect (label, °C) per sensor.
        let mut entries: Vec<(String, f64)> = Vec::new();
        if let Ok(hw) = std::fs::read_dir(dev.join("hwmon")) {
            for h in hw.flatten() {
                if !h.path().is_dir() {
                    continue;
                }
                if let Ok(tdir) = std::fs::read_dir(h.path()) {
                    for t in tdir.flatten() {
                        let name = t.file_name().to_string_lossy().to_string();
                        if name.starts_with("temp") && name.ends_with("_input") {
                            if let Ok(v) = std::fs::read_to_string(t.path()) {
                                if let Ok(milli) = v.trim().parse::<f64>() {
                                    // Label lives beside the input as tempN_label.
                                    let label_name = name.replace("_input", "_label");
                                    let label = std::fs::read_to_string(h.path().join(&label_name))
                                        .ok()
                                        .map(|s| s.trim().to_string())
                                        .unwrap_or_default();
                                    entries.push((label, milli / 1000.0));
                                }
                            }
                        }
                    }
                }
            }
        }
        let temp_c = select_gpu_temp(&entries);

        // Utilization: device/gpu_busy_percent.
        let mut usage_pct = None;
        if let Ok(b) = std::fs::read_to_string(dev.join("gpu_busy_percent")) {
            usage_pct = b.trim().parse::<f64>().ok();
        }

        // VRAM: device/mem_info_vram_used + _total (bytes) -> % of total.
        let vram_pct = match (
            std::fs::read_to_string(dev.join("mem_info_vram_used")).ok(),
            std::fs::read_to_string(dev.join("mem_info_vram_total")).ok(),
        ) {
            (Some(u), Some(t)) => match (u.trim().parse::<f64>(), t.trim().parse::<f64>()) {
                (Ok(used), Ok(total)) => pct_of(used, total),
                _ => None,
            },
            _ => None,
        };

        // Power draw: hwmon `power1_average` (preferred) or `power1_input`, in microwatts ->
        // watts. Also grab `power1_cap` (the power limit) so the UI can normalize the bar.
        // Take the first hwmon dir whose average/input reads (amdgpu labels this "PPT").
        let mut power_w = None;
        let mut power_cap_w = None;
        if let Ok(hw) = std::fs::read_dir(dev.join("hwmon")) {
            for h in hw.flatten() {
                if !h.path().is_dir() {
                    continue;
                }
                let base = h.path();
                let draw = std::fs::read_to_string(base.join("power1_average"))
                    .ok()
                    .or_else(|| std::fs::read_to_string(base.join("power1_input")).ok());
                if let Some(v) = draw {
                    if let Ok(micro) = v.trim().parse::<f64>() {
                        power_w = Some(micro / 1_000_000.0);
                        if let Ok(c) = std::fs::read_to_string(base.join("power1_cap")) {
                            if let Ok(c_micro) = c.trim().parse::<f64>() {
                                if c_micro > 0.0 {
                                    power_cap_w = Some(c_micro / 1_000_000.0);
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        if temp_c.is_some() || usage_pct.is_some() {
            return Some(GpuValues {
                temp_c,
                usage_pct,
                vram_pct,
                power_w,
                power_cap_w,
            });
        }
    }
    None
}

/// Pick the primary GPU temperature from `(label, value)` entries: prefer an "edge"
/// sensor (AMD's main die temperature — what standard tools report as "GPU Temp"),
/// else fall back to the first reading. `None` if empty.
#[must_use]
pub fn select_gpu_temp(entries: &[(String, f64)]) -> Option<f64> {
    if let Some((_, v)) = entries
        .iter()
        .find(|(label, _)| label.to_lowercase().contains("edge"))
    {
        return Some(*v);
    }
    entries.first().map(|(_, v)| *v)
}

/// `used/total*100` as a percentage clamped to 0..=100. `None` if the denominator is not positive.
#[must_use]
pub fn pct_of(used: f64, total: f64) -> Option<f64> {
    (total > 0.0).then(|| (used / total * 100.0).clamp(0.0, 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_gpu_temp_prefers_edge() {
        let entries = vec![
            ("junction".into(), 77.0_f64),
            ("edge".into(), 66.0),
            ("mem".into(), 82.0),
        ];
        assert_eq!(select_gpu_temp(&entries), Some(66.0));
    }

    #[test]
    fn select_gpu_temp_falls_back_to_first() {
        let entries = vec![("core".into(), 55.0_f64), ("mem".into(), 71.0)];
        assert_eq!(select_gpu_temp(&entries), Some(55.0));
        assert_eq!(select_gpu_temp(&[]), None);
    }

    #[test]
    fn select_gpu_temp_case_insensitive() {
        let entries = vec![("Edge".into(), 64.0_f64)];
        assert_eq!(select_gpu_temp(&entries), Some(64.0));
    }

    #[test]
    fn pct_of_computes_and_clamps() {
        assert!((pct_of(5.0, 10.0).unwrap() - 50.0).abs() < f64::EPSILON);
        assert_eq!(pct_of(98.0, 100.0), Some(98.0));
        // Overshoot clamps to 100; a non-positive denominator is None.
        assert_eq!(pct_of(150.0, 100.0), Some(100.0));
        assert_eq!(pct_of(10.0, 0.0), None);
    }
}
