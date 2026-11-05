//! RAM usage from `/proc/meminfo`: used = MemTotal − MemAvailable, as a % of total.
//!
//! The pure math ([`ram_used_pct`]) is separated from the /proc read and unit-tested.

/// Read `MemTotal` and `MemAvailable` (kB) from `/proc/meminfo`. `None` if unreadable.
fn read_meminfo() -> Option<(u128, u128)> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut total: Option<u128> = None;
    let mut available: Option<u128> = None;
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("MemTotal:") {
            if let Some(first) = v.split_whitespace().next() {
                total = first.parse::<u128>().ok();
            }
        } else if let Some(v) = line.strip_prefix("MemAvailable:") {
            if let Some(first) = v.split_whitespace().next() {
                available = first.parse::<u128>().ok();
            }
        }
    }
    match (total, available) {
        (Some(t), Some(a)) => Some((t, a)),
        _ => None,
    }
}

/// RAM used as % of total from `(total_kib, available_kib)` where `used = total − available`.
/// Clamped to 0..=100. `None` if the total is not positive.
#[must_use]
pub fn ram_used_pct(total_kib: u128, available_kib: u128) -> Option<f64> {
    (total_kib > 0).then(|| {
        let used = total_kib.saturating_sub(available_kib);
        (used as f64 / total_kib as f64 * 100.0).clamp(0.0, 100.0)
    })
}

/// Current RAM usage % of total, or `None` if `/proc/meminfo` is unavailable.
#[must_use]
pub fn ram_pct() -> Option<f64> {
    read_meminfo().and_then(|(t, a)| ram_used_pct(t, a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_used_pct_computes_used_fraction() {
        // total 1000, available 250 -> used 750 -> 75%.
        assert!((ram_used_pct(1000, 250).unwrap() - 75.0).abs() < f64::EPSILON);
    }

    #[test]
    fn ram_used_pct_zero_total_is_none() {
        assert_eq!(ram_used_pct(0, 0), None);
    }

    #[test]
    fn ram_used_pct_available_exceeding_total_clamps_to_zero() {
        // available > total (transient) -> saturating_sub yields 0 used -> 0%.
        assert!((ram_used_pct(100, 200).unwrap() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn ram_used_pct_full_is_100() {
        assert_eq!(ram_used_pct(1000, 0), Some(100.0));
    }
}
