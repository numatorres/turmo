//! Hardware value sources for the dashboard.
//!
//! Linux-native equivalents of the Windows LibreHardwareMonitor sensors referenced
//! by the `.json` files:
//!   - GPU temperature + utilization: `nvidia-smi` (NVIDIA) or `/sys/class/drm/...`
//!     (AMD/intel). Stateless — sampled each frame.
//!   - CPU temperature: `/sys/class/hwmon/*/temp*_input` (package reading preferred).
//!   - CPU usage %: aggregate from `/proc/stat`. **Stateful** — a [`cpu::CpuUsageMeter`]
//!     is held across frames to compute the busy/total delta between samples.

pub mod cpu;
pub mod gpu;
pub mod mem;
pub mod models;

/// A single GPU sample. Values are `None` when the source is unavailable.
#[derive(Debug, Clone, Copy, Default)]
pub struct GpuValues {
    pub temp_c: Option<f64>,
    pub usage_pct: Option<f64>,
    /// VRAM used as a percentage of total (`used/total*100`), `None` if unavailable.
    pub vram_pct: Option<f64>,
    /// Power draw in watts, `None` if the source doesn't expose it.
    pub power_w: Option<f64>,
    /// Power limit (cap) in watts, for normalizing the bar; `None` if unavailable.
    pub power_cap_w: Option<f64>,
}

impl GpuValues {
    /// Sample the first GPU found, auto-detecting the vendor backend.
    pub fn sample() -> Self {
        gpu::detect()
    }
}

/// Combined hardware readout shown on the dashboard. Each field is `None` when its
/// source is unavailable; the UI renders those as "—" with an empty bar.
#[derive(Debug, Clone, Copy, Default)]
pub struct Readout {
    pub gpu_temp: Option<f64>,
    pub gpu_usage: Option<f64>,
    /// VRAM used as a percentage of total (GPU).
    pub gpu_vram: Option<f64>,
    /// GPU power draw in watts.
    pub gpu_power: Option<f64>,
    /// GPU power limit (cap) in watts, for normalizing the bar.
    pub gpu_power_cap: Option<f64>,
    pub cpu_temp: Option<f64>,
    pub cpu_usage: Option<f64>,
    /// RAM used as a percentage of total.
    pub ram: Option<f64>,
    /// CPU frequency in GHz.
    pub cpu_freq: Option<f64>,
    /// CPU maximum frequency in GHz, for normalizing the FREQ card's bar.
    pub cpu_freq_max: Option<f64>,
}

impl Readout {
    /// Build a readout from the GPU sample and CPU/memory sensors. The caller supplies the
    /// per-frame values (temperatures stateless; usage/RAM percentages sampled each frame).
    #[must_use]
    pub fn new(
        gpu: &GpuValues,
        cpu_temp: Option<f64>,
        cpu_usage: Option<f64>,
        ram: Option<f64>,
        cpu_freq: Option<f64>,
        cpu_freq_max: Option<f64>,
    ) -> Self {
        Self {
            gpu_temp: gpu.temp_c,
            gpu_usage: gpu.usage_pct,
            gpu_vram: gpu.vram_pct,
            gpu_power: gpu.power_w,
            gpu_power_cap: gpu.power_cap_w,
            cpu_temp,
            cpu_usage,
            ram,
            cpu_freq,
            cpu_freq_max,
        }
    }
}
