//! Configuration loaded from TOML (`turmo.toml`) with sensible defaults.
//!
//! The protocol-related fields (framing, geometry, color) are *knobs* that get
//! finalized by cross-referencing `PROTOCOL.md`, which is filled in during the
//! Phase 0 traffic-capture step. Until then they carry safe defaults and the
//! app is driven with `--test` to validate transport against a known pattern.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    #[default]
    Serial,
    Bulk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ChannelOrder {
    #[default]
    Rgb,
    Bgr,
}

/// Rotation applied to the composed canvas before it is sent, so our layout maps
/// onto the panel's native orientation. The confirmed 1a86:5722 panel is landscape
/// and displays our portrait canvas rotated 90° clockwise (the default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Rotation {
    None,
    #[default]
    Cw90,
    Cw180,
    Cw270,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    GpuTemp,
    GpuUsage,
}

/// Parse a hex string like `"aa55"` or `"aa 55"` into bytes. Empty -> empty vec.
fn de_hex<'de, D>(d: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !cleaned.len().is_multiple_of(2) {
        return Err(serde::de::Error::custom("hex string must have an even number of digits"));
    }
    let bytes = (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).map_err(serde::de::Error::custom))
        .collect::<Result<Vec<u8>, _>>()?;
    Ok(bytes)
}

fn default_device() -> String {
    "/dev/ttyACM0".to_string()
}
fn default_baud() -> u32 {
    921600
}
// Canvas is authored in portrait; `rotation` (default cw90) turns it into the
// landscape 480x320 panel. See PROTOCOL.md for how to confirm/adjust these.
fn default_width() -> u32 {
    320
}
fn default_height() -> u32 {
    480
}
fn default_bpp() -> u8 {
    3
}
fn default_refresh_ms() -> u64 {
    500
}
fn default_font_scale() -> u8 {
    2
}
fn default_sensors() -> Vec<SensorKind> {
    vec![SensorKind::GpuTemp, SensorKind::GpuUsage]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Device node for serial transport. Informational for bulk transport.
    #[serde(deserialize_with = "de_string")]
    pub device: String,

    /// `"serial"` (default) or `"bulk"`. Bulk requires `--features bulk` + system libusb.
    pub transport: TransportKind,

    /// Serial baud rate. Tune after Phase 0 capture.
    #[serde(deserialize_with = "de_baud")]
    pub baud: u32,

    // --- Frame geometry / color (confirm from PROTOCOL.md) ---
    /// Canvas width in pixels (pre-rotation). Authored portrait by default.
    pub width: u32,
    /// Canvas height in pixels (pre-rotation).
    pub height: u32,
    /// Bytes per pixel; 3 for RGB888.
    pub bytes_per_pixel: u8,
    pub channel_order: ChannelOrder,
    /// Rotate the composed canvas into panel orientation before sending.
    pub rotation: Rotation,

    // --- Protocol framing (fill from Phase 0 capture) ---
    /// One-time init/handshake bytes sent once after open (hex). Empty = none.
    #[serde(deserialize_with = "de_hex")]
    pub init_bytes: Vec<u8>,
    /// Per-frame preamble/header bytes (hex).
    #[serde(deserialize_with = "de_hex")]
    pub frame_header: Vec<u8>,
    /// Per-frame trailer/footer bytes (hex).
    #[serde(deserialize_with = "de_hex")]
    pub frame_footer: Vec<u8>,
    /// Prepend width/height as little-endian u16 before the pixel payload.
    pub include_size_fields: bool,

    // --- Refresh / content ---
    pub refresh_ms: u64,
    /// Optional background image (PNG). If absent, live mode uses a dark fill.
    #[serde(default)]
    pub bg_image: Option<String>,
    pub sensors: Vec<SensorKind>,
    pub font_scale: u8,
    /// Thermal alarm limit (°C): the GPU temp card turns red at/above this value.
    pub gpu_temp_limit: u16,
    /// Thermal alarm limit (°C): the CPU temp card turns red at/above this value.
    pub cpu_temp_limit: u16,

    // --- bulk-only (feature-gated) ---
    /// Used only when built with `--features bulk`.
    #[serde(default)]
    #[cfg_attr(not(feature = "bulk"), allow(dead_code))]
    pub usb_vid: Option<u16>,
    /// Used only when built with `--features bulk`.
    #[serde(default)]
    #[cfg_attr(not(feature = "bulk"), allow(dead_code))]
    pub usb_pid: Option<u16>,
    /// Out endpoint address for the bulk transfer (used under `bulk`).
    pub bulk_out_endpoint: u8,
}

// Wrapper deserializers so `#[serde(default)]` on the container can supply a
// per-field default when the key is absent (plain function defaults do not apply
// to fields that also carry a custom `deserialize_with`).
fn de_string<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<String>::deserialize(d)?;
    Ok(v.unwrap_or_else(default_device))
}

fn de_baud<'de, D>(d: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<u32>::deserialize(d)?;
    Ok(v.unwrap_or_else(default_baud))
}

impl Default for Config {
    fn default() -> Self {
        Config {
            device: default_device(),
            transport: TransportKind::default(),
            baud: default_baud(),
            width: default_width(),
            height: default_height(),
            bytes_per_pixel: default_bpp(),
            channel_order: ChannelOrder::default(),
            rotation: Rotation::default(),
            init_bytes: Vec::new(),
            frame_header: Vec::new(),
            frame_footer: Vec::new(),
            include_size_fields: true,
            refresh_ms: default_refresh_ms(),
            bg_image: None,
            sensors: default_sensors(),
            font_scale: default_font_scale(),
            gpu_temp_limit: 85,
            cpu_temp_limit: 90,
            usb_vid: None,
            usb_pid: None,
            bulk_out_endpoint: 0x03,
        }
    }
}

/// Cheap `Copy` projection of the frame-relevant config, handed to the frame
/// composer and framing code so they don't borrow the whole [`Config`].
#[derive(Debug, Clone, Copy)]
pub struct Spec {
    pub width: u32,
    pub height: u32,
    pub bytes_per_pixel: u8,
    pub channel_order: ChannelOrder,
}

impl Config {
    pub fn spec(&self) -> Spec {
        Spec {
            width: self.width,
            height: self.height,
            bytes_per_pixel: self.bytes_per_pixel,
            channel_order: self.channel_order,
        }
    }

    pub fn load(path: &str) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let cfg: Config = toml::from_str(&s)?;
                Ok(cfg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!(
                    "config '{path}' not found; using built-in defaults (see turmo.toml.example)"
                );
                Ok(Config::default())
            }
            Err(e) => Err(anyhow::anyhow!("failed to read config '{path}': {e}")),
        }
    }
}
