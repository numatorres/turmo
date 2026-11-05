//! CPU sensors: package temperature from hwmon and overall usage % from /proc/stat.
//!
//! Usage is **stateful** — [`CpuUsageMeter`] keeps the previous aggregate sample so it
//! can report the busy/total delta between calls. The pure math ([`usage_pct`],
//! [`select_package_temp`]) is separated out and unit-tested without touching /proc or
//! sysfs.

/// Read the package CPU temperature (°C) from hwmon, preferring a reading whose label
/// contains "package". Returns `None` if no hwmon temp source is available.
#[must_use]
pub fn temp_c() -> Option<f64> {
    let dir = std::fs::read_dir("/sys/class/hwmon").ok()?;
    for entry in dir.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let mut entries: Vec<(String, f64)> = Vec::new();
        if let Ok(hw) = std::fs::read_dir(entry.path()) {
            for h in hw.flatten() {
                let name = h.file_name().to_string_lossy().to_string();
                if name.starts_with("temp") && name.ends_with("_input") {
                    if let Ok(t) = std::fs::read_to_string(h.path()) {
                        // Value is in millidegrees Celsius.
                        if let Ok(milli) = t.trim().parse::<f64>() {
                            let label_name = name.replace("_input", "_label");
                            let label = std::fs::read_to_string(entry.path().join(&label_name))
                                .ok()
                                .map(|s| s.trim().to_string())
                                .unwrap_or_default();
                            entries.push((label, milli / 1000.0));
                        }
                    }
                }
            }
        }
        if let Some(v) = select_package_temp(&entries) {
            return Some(v);
        }
    }
    None
}

/// Pick the package temperature from `(label, value)` entries: prefer a label containing
/// "package" (case-insensitive), else fall back to the first reading. `None` if empty.
#[must_use]
pub fn select_package_temp(entries: &[(String, f64)]) -> Option<f64> {
    if let Some((_, v)) = entries
        .iter()
        .find(|(label, _)| label.to_lowercase().contains("package"))
    {
        return Some(*v);
    }
    entries.first().map(|(_, v)| *v)
}

/// Read the aggregate `cpu` line from `/proc/stat` as `(total_jiffies, idle_plus_iowait)`.
fn read_proc_stat() -> Option<(u128, u128)> {
    let content = std::fs::read_to_string("/proc/stat").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("cpu ") {
            // Fields: user nice system idle iowait irq softirq steal guest guest_nice
            let nums: Vec<u128> = rest
                .split_whitespace()
                .filter_map(|t| t.parse::<u128>().ok())
                .collect();
            if nums.len() >= 4 {
                let total: u128 = nums.iter().sum();
                let idle_iowait = nums[3].saturating_add(nums.get(4).copied().unwrap_or(0));
                return Some((total, idle_iowait));
            }
        }
    }
    None
}

/// Overall CPU usage % between two `(total, idle+iowait)` samples. `None` if no time
/// elapsed (current total not greater than previous).
#[must_use]
pub fn usage_pct(prev: (u128, u128), cur: (u128, u128)) -> Option<f64> {
    let (pt, pi) = prev;
    let (ct, ci) = cur;
    if ct <= pt {
        return None;
    }
    let dtotal = (ct - pt) as f64;
    let dbusy = ((ct - ci).saturating_sub(pt - pi)) as f64;
    Some((dbusy / dtotal) * 100.0)
}

/// Stateful CPU usage meter: holds the previous `/proc/stat` sample so each call returns
/// the % since the last call. The first call returns `None` (no delta available yet).
#[derive(Default)]
pub struct CpuUsageMeter {
    prev: Option<(u128, u128)>,
}

impl CpuUsageMeter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sample overall CPU % since the previous call. `None` on the first sample or if
    /// `/proc/stat` is unreadable.
    pub fn sample(&mut self) -> Option<f64> {
        let cur = read_proc_stat()?;
        let pct = self.prev.and_then(|p| usage_pct(p, cur));
        self.prev = Some(cur);
        pct
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_pct_computes_busy_fraction() {
        // prev: total=100, idle=50 -> busy 50. cur: total=200, idle=90 -> busy 110.
        // dtotal=100, dbusy=(200-90)-(100-50)=60 -> 60%.
        assert_eq!(usage_pct((100, 50), (200, 90)), Some(60.0));
    }

    #[test]
    fn usage_pct_no_time_elapsed_is_none() {
        assert_eq!(usage_pct((100, 50), (100, 50)), None);
        // Current total less than previous (clock wrap / reset) -> None.
        assert_eq!(usage_pct((200, 90), (100, 50)), None);
    }

    #[test]
    fn usage_pct_all_idle_is_zero() {
        // Total advances only by idle time -> 0% busy.
        assert_eq!(usage_pct((100, 100), (200, 200)), Some(0.0));
    }

    #[test]
    fn select_package_prefers_package_label() {
        let entries = vec![
            ("Core 0".into(), 45.0_f64),
            ("Package id 0".into(), 70.0),
            ("Core 1".into(), 60.0),
        ];
        assert_eq!(select_package_temp(&entries), Some(70.0));
    }

    #[test]
    fn select_package_falls_back_to_first() {
        let entries = vec![("Tctl".into(), 52.0_f64), ("Tdie".into(), 61.0)];
        assert_eq!(select_package_temp(&entries), Some(52.0));
        assert_eq!(select_package_temp(&[]), None);
    }
}
