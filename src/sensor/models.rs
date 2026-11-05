//! Static hardware *identity* (model names) for the dashboard banners.
//!
//! Unlike [`crate::sensor::Readout`] these are read **once at startup** and cached —
//! they don't change per frame, so we never re-shell `lspci` in the render loop.

/// The GPU + CPU model strings shown on their banner rows.
#[derive(Debug, Clone)]
pub struct SystemModels {
    pub gpu_model: String,
    pub cpu_model: String,
}

impl Default for SystemModels {
    fn default() -> Self {
        Self {
            gpu_model: "GPU".to_string(),
            cpu_model: "CPU".to_string(),
        }
    }
}

impl SystemModels {
    /// Detect both model names. Falls back to `"GPU"` / `"CPU"` when a source is unavailable.
    #[must_use]
    pub fn detect() -> Self {
        let max = 28usize; // ~fits the banner at ~22px in a 448px row
        let cpu_model = match std::fs::read_to_string("/proc/cpuinfo") {
            Ok(info) => info
                .lines()
                .find_map(|l| l.strip_prefix("model name"))
                .map(|rest| rest.trim_start().trim_start_matches(':').trim())
                .filter(|v| !v.is_empty())
                .map(|v| truncate_to_fit(&trim_cpu_suffix(v), max))
                .unwrap_or_else(|| "CPU".to_string()),
            Err(_) => "CPU".to_string(),
        };
        let gpu_model = gpu_model_name(max);
        Self { gpu_model, cpu_model }
    }
}

/// GPU model: `nvidia-smi` name (NVIDIA) → dynamic card scan + `lspci` name → PCI id → `"GPU"`.
fn gpu_model_name(max: usize) -> String {
    if let Some(name) = nvidia_smi_name() {
        return truncate_to_fit(&name, max);
    }

    // The DRM card index is NOT stable (it was `card0`, later `card1`), so scan for the
    // GPU card by driver / busy-sensor rather than assuming an index.
    if let Some((slot, pci_id)) = find_gpu_card() {
        // lspci prints the slot without its domain prefix ("03:00.0" vs "0000:03:00.0").
        let short_slot = slot.split(':').skip(1).collect::<Vec<_>>().join(":");
        if let Ok(out) = std::process::Command::new("lspci").arg("-nn").output() {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(name) = lspci_line_for(&text, &short_slot).and_then(gpu_name_from_lspci) {
                return truncate_to_fit(&name, max);
            }
        }
        if !pci_id.is_empty() {
            return pci_id;
        }
    }
    "GPU".to_string()
}

/// `nvidia-smi --query-gpu=name` first line (NVIDIA only). `None` if absent/fails.
fn nvidia_smi_name() -> Option<String> {
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Find the GPU card under `/sys/class/drm`, returning its PCI slot + `vendor:device` id.
fn find_gpu_card() -> Option<(String, String)> {
    let cards = std::fs::read_dir("/sys/class/drm").ok()?;
    for entry in cards.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("card") {
            continue; // skip cardN-DP-x, renderD*, version
        }
        let dev = entry.path().join("device");
        let ue = match std::fs::read_to_string(dev.join("uevent")) {
            Ok(u) => u,
            Err(_) => continue,
        };

        // A GPU card has a known GPU driver, or exposes gpu_busy_percent (AMD).
        let driver = std::fs::read_link(dev.join("driver"))
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()));
        let is_gpu_driver = matches!(
            driver.as_deref(),
            Some("amdgpu") | Some("nouveau") | Some("radeon")
        );
        if !is_gpu_driver && !dev.join("gpu_busy_percent").exists() {
            continue;
        }

        let mut slot = String::new();
        let mut id = String::new();
        for line in ue.lines() {
            if let Some(v) = line.strip_prefix("PCI_SLOT_NAME=") {
                slot = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("PCI_ID=") {
                id = v.trim().to_string();
            }
        }
        return Some((slot, id));
    }
    None
}

/// The `lspci -nn` line for our PCI slot; falls back to the first VGA `[0300]` controller.
fn lspci_line_for<'a>(text: &'a str, short_slot: &'a str) -> Option<&'a str> {
    text.lines()
        .find(|l| l.starts_with(short_slot))
        .or_else(|| text.lines().find(|l| l.contains("[0300]")))
}

/// Extract the marketing name from an `lspci -nn` display line, e.g.
/// `…[AMD/ATI] Navi 48 [Radeon AI PRO R9700] [1002:7551] (rev c0)` → `Some("Radeon AI PRO R9700")`.
#[must_use]
pub fn gpu_name_from_lspci(line: &str) -> Option<String> {
    // Description follows `<class> [<id>]: `.
    let rest = line.split("]: ").nth(1)?;
    let rest = rest.split(" (rev ").next()?.trim();
    for b in brackets(rest) {
        if is_pci_id(&b) {
            continue; // "1002:7551"
        }
        if !b.contains(' ') {
            continue; // vendor tags like "AMD/ATI", "Intel", "NVIDIA"
        }
        return Some(b);
    }
    None
}

/// Drop a trailing integrated-GPU suffix such as `" with Radeon Graphics"` so the CPU label
/// reads as just the core model on a dashboard that already has a GPU row.
#[must_use]
pub fn trim_cpu_suffix(s: &str) -> String {
    let lower = s.to_lowercase();
    if let Some(pos) = lower.rfind(" with ") {
        let tail = &lower[pos + " with ".len()..];
        if tail.trim_end().ends_with("graphics") {
            return s[..pos].trim_end().to_string();
        }
    }
    s.to_string()
}

/// Cap `s` to `max_chars`, appending an ellipsis when it was longer.
#[must_use]
pub fn truncate_to_fit(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s.to_string();
    }
    let mut out: String = chars[..max_chars.saturating_sub(1)].iter().collect();
    out.push('…');
    out
}

/// Contents of each `[ … ]` group in `s`.
fn brackets(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == ']') {
                out.push(chars[i + 1..i + 1 + end].iter().collect());
                i += end + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// True for a `hhhh:dddd` PCI id token.
fn is_pci_id(b: &str) -> bool {
    let mut it = b.split(':');
    match (it.next(), it.next()) {
        (Some(v), Some(d)) if v.len() == 4 && d.len() == 4 => {
            v.chars().all(|c| c.is_ascii_hexdigit()) && d.chars().all(|c| c.is_ascii_hexdigit())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_cpu_suffix_drops_trailing_gpu() {
        assert_eq!(
            trim_cpu_suffix("AMD Ryzen 9 7945HX with Radeon Graphics"),
            "AMD Ryzen 9 7945HX"
        );
        // No trailing graphics suffix -> unchanged.
        assert_eq!(trim_cpu_suffix("Intel(R) Core(TM) i7-12700H"), "Intel(R) Core(TM) i7-12700H");
        // A non-trailing "with" must not be cut.
        assert_eq!(trim_cpu_suffix("CPU with boost and cores"), "CPU with boost and cores");
    }

    #[test]
    fn gpu_name_from_lspci_extracts_marketing_name() {
        let line = "03:00.0 VGA compatible controller [0300]: Advanced Micro Devices, Inc. \
                    [AMD/ATI] Navi 48 [Radeon AI PRO R9700] [1002:7551] (rev c0)";
        assert_eq!(gpu_name_from_lspci(line), Some("Radeon AI PRO R9700".to_string()));

        // NVIDIA-style single-bracket name.
        let nv = "01:00.0 VGA compatible controller [0300]: NVIDIA Corporation \
                 GA106 [GeForce RTX 3060] [10de:2584] (rev a1)";
        assert_eq!(gpu_name_from_lspci(nv), Some("GeForce RTX 3060".to_string()));

        // No marketing bracket -> None.
        assert_eq!(gpu_name_from_lspci("03:00.0 VGA compatible controller [0300]: Unknown [1002:7551]"), None);
    }

    #[test]
    fn truncate_to_fit_caps_and_ellipsizes() {
        assert_eq!(truncate_to_fit("Ryzen 9", 28), "Ryzen 9");
        let long = "a very long model name that exceeds the budget";
        let t = truncate_to_fit(long, 10);
        assert_eq!(t.chars().count(), 10);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn brackets_and_pci_id() {
        assert_eq!(brackets("x [AMD/ATI] y [Radeon X] z [1002:7551]"), vec!["AMD/ATI", "Radeon X", "1002:7551"]);
        assert!(is_pci_id("1002:7551"));
        assert!(!is_pci_id("AMD/ATI"));
        assert!(!is_pci_id("abcd:ef")); // wrong length
    }
}
