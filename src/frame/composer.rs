//! Frame composition: build a [`Frame`] either as an asymmetric self-test pattern
//! (to validate transport + channel order against the glass) or as a live **dashboard**
//! — a light-themed vertical list: GPU model banner, GPU Temp/Usage cards, CPU model
//! banner, CPU Temp/Usage cards. Each temp card turns red when it reaches its limit.

use rusttype::Font;

use crate::config::Spec;
use crate::font;
use crate::frame::Frame;
use crate::sensor::models::SystemModels;
use crate::sensor::Readout;

/// Builds frames for a fixed panel spec, optionally over a preloaded RGB background.
pub struct Composer {
    spec: Spec,
    /// Kept for the test pattern / legacy text path; the dashboard uses proportional sizes.
    scale: u8,
    /// Background as an RGB byte buffer already sized `width*height*3`.
    bg: Option<Vec<u8>>,
    /// Bold face used for all dashboard text — heavier weight reads better on the low-res panel.
    font_bold: Option<Font<'static>>,
    // --- static display context, set once at startup (mirrors how `bg` is handled) ---
    gpu_model: String,
    cpu_model: String,
    gpu_temp_limit: u16,
    cpu_temp_limit: u16,
}

impl Composer {
    pub fn new(spec: Spec, scale: u8) -> Self {
        Self {
            spec,
            scale,
            bg: None,
            font_bold: font::load_face(true),
            gpu_model: "GPU".to_string(),
            cpu_model: "CPU".to_string(),
            gpu_temp_limit: 85,
            cpu_temp_limit: 90,
        }
    }

    /// Set the static identity + thermal-alarm context (model names and limits), read once at startup.
    pub fn set_context(&mut self, m: &SystemModels, gpu_limit: u16, cpu_limit: u16) {
        self.gpu_model = m.gpu_model.clone();
        self.cpu_model = m.cpu_model.clone();
        self.gpu_temp_limit = gpu_limit;
        self.cpu_temp_limit = cpu_limit;
    }

    /// Attach a preloaded/resized RGB background (used by live mode).
    pub fn set_bg(&mut self, rgb: Vec<u8>) {
        let want = (self.spec.width as usize) * (self.spec.height as usize) * 3;
        if rgb.len() == want {
            self.bg = Some(rgb);
        } else {
            // Wrong size -> fall back to a page fill rather than crash.
            self.bg = None;
        }
    }

    /// Asymmetric test pattern: red top-left, blue bottom-right, white border, dark
    /// elsewhere. A B/R swap or mirroring is immediately visible against this.
    pub fn test_pattern(&self) -> Frame {
        let mut f = Frame::new(self.spec);
        let w = self.spec.width as usize;
        let h = self.spec.height as usize;
        for y in 0..h {
            for x in 0..w {
                if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                    f.set(x as u32, y as u32, 255, 255, 255); // border: orientation check
                } else if (x < w / 2) && (y < h / 2) {
                    f.set(x as u32, y as u32, 255, 0, 0); // red top-left
                } else if (x >= w / 2) && (y >= h / 2) {
                    f.set(x as u32, y as u32, 0, 0, 255); // blue bottom-right
                } else {
                    f.set(x as u32, y as u32, 40, 40, 48); // dark fill elsewhere
                }
            }
        }
        f
    }

    /// Live dashboard frame: light page fill, then a vertical list — GPU model banner,
    /// GPU Temp/Usage cards, CPU model banner, CPU Temp/Usage cards. Each temp card turns
    /// red when its value reaches the configured limit; usage cards never alarm.
    pub fn compose(&self, r: &Readout) -> Frame {
        let mut f = Frame::new(self.spec);
        match &self.bg {
            Some(rgb) => f.blit_rgb(rgb),
            None => f.fill(PAGE_BG[0], PAGE_BG[1], PAGE_BG[2]), // light neutral page
        }

        const MARGIN_X: u32 = 16;
        const MARGIN_Y: u32 = 14;
        const GAP: u32 = 12;
        const BANNER_H: u32 = 46;
        const VALS_H: u32 = 82;

        let w = self.spec.width;
        let h = self.spec.height;
        if w < 8 || h < 8 {
            return f; // too small for a dashboard (test safety) — bg only
        }
        // Guard: need room for the four stacked rows.
        let total_h = MARGIN_Y * 2 + BANNER_H * 2 + VALS_H * 2 + GAP * 3;
        if w < MARGIN_X * 2 || h < total_h {
            return f;
        }

        let content_w = w - MARGIN_X * 2; // 448 at a 480px canvas
        let card_gap = GAP;
        let card_w = (content_w - 2 * card_gap) / 3; // three cards per row (~141 at 480px)
        let col_x = [MARGIN_X, MARGIN_X + card_w + card_gap, MARGIN_X + 2 * (card_w + card_gap)];

        // Row 1: GPU banner + Temp / Usage / Vram cards.
        self.banner(&mut f, MARGIN_X, MARGIN_Y, content_w, BANNER_H, "GPU", &self.gpu_model);
        let gpu_row_y = MARGIN_Y + BANNER_H + GAP;
        let gpu_alarm = r.gpu_temp.is_some_and(|t| t >= self.gpu_temp_limit as f64);
        self.metric_card(&mut f, col_x[0], gpu_row_y, card_w, VALS_H, "TEMP", &fmt_temp(r.gpu_temp), norm_temp(r.gpu_temp), gpu_alarm);
        self.metric_card(&mut f, col_x[1], gpu_row_y, card_w, VALS_H, "USAGE", &fmt_pct(r.gpu_usage), norm_pct(r.gpu_usage), false);
        self.metric_card(&mut f, col_x[2], gpu_row_y, card_w, VALS_H, "VRAM", &fmt_pct(r.gpu_vram), norm_pct(r.gpu_vram), false);

        // Row 2: CPU banner + Temp / Usage / Ram cards.
        let cpu_banner_y = gpu_row_y + VALS_H + GAP;
        self.banner(&mut f, MARGIN_X, cpu_banner_y, content_w, BANNER_H, "CPU", &self.cpu_model);
        let cpu_row_y = cpu_banner_y + BANNER_H + GAP;
        let cpu_alarm = r.cpu_temp.is_some_and(|t| t >= self.cpu_temp_limit as f64);
        self.metric_card(&mut f, col_x[0], cpu_row_y, card_w, VALS_H, "TEMP", &fmt_temp(r.cpu_temp), norm_temp(r.cpu_temp), cpu_alarm);
        self.metric_card(&mut f, col_x[1], cpu_row_y, card_w, VALS_H, "USAGE", &fmt_pct(r.cpu_usage), norm_pct(r.cpu_usage), false);
        self.metric_card(&mut f, col_x[2], cpu_row_y, card_w, VALS_H, "RAM", &fmt_pct(r.ram), norm_pct(r.ram), false);

        f
    }

    /// A banner row: small muted category tag on the left + the model name to its right.
    #[allow(clippy::too_many_arguments)] // drawing primitive; args are the row's geometry + content
    fn banner(&self, f: &mut Frame, x: u32, y: u32, w: u32, h: u32, tag: &str, model: &str) {
        if w == 0 || h == 0 {
            return;
        }
        let zoom = self.scale.max(1) as f32 / 2.0;
        let mid_y = y as i32 + (h as f64 * 0.5).round() as i32;
        if let Some(fb) = &self.font_bold {
            // Category tag, small and light (for the dark page bg), vertically centered.
            let tag_px = (h as f64 * 0.30).max(12.0) as f32 * zoom;
            font::draw_text_aa(f, fb, x as i32, mid_y + (tag_px * 0.35) as i32, tag, tag_px, BANNER_TAG);

            // Model name to the right of the tag — white and larger for a stronger readout.
            let model_x = x + font::text_width(fb, tag, tag_px) as u32 + 10;
            let model_px = (h as f64 * 0.45).max(15.0) as f32 * zoom;
            font::draw_text_aa(f, fb, model_x as i32, mid_y + (model_px * 0.35) as i32, model, model_px, TXT_MODEL);
        }
    }

    /// Draw one metric card: white (or red when `alarm`) rounded panel with a muted sub-label,
    /// a large value below it, and a grayscale progress bar at the bottom.
    #[allow(clippy::too_many_arguments)] // drawing primitive; args are the card's geometry + content
    fn metric_card(
        &self,
        f: &mut Frame,
        x: u32,
        y: u32,
        cw: u32,
        ch: u32,
        label: &str,
        value: &str,
        norm: f64,
        alarm: bool,
    ) {
        let (cw_f, ch_f) = (cw as f64, ch as f64);
        if cw == 0 || ch == 0 {
            return;
        }
        let inset = (cw_f * 0.05).max(8.0).round() as u32; // ~11 at card width 218
        let radius = (cw.min(ch) / 12).max(6);
        let zoom = self.scale.max(1) as f32 / 2.0;

        // Card background: red when in thermal alarm, else white.
        f.rounded_rect(x, y, cw, ch, radius, if alarm { CARD_RED } else { CARD_WHITE });

        let value_color = if alarm { TXT_ON_ALARM } else { TXT_VALUE };
        let label_color = if alarm { TXT_ON_ALARM } else { TXT_LABEL };

        if let Some(fb) = &self.font_bold {
            // Sub-label top-left.
            let label_px = (ch_f * 0.17).max(12.0) as f32 * zoom;
            font::draw_text_aa(f, fb, x as i32 + inset as i32, y as i32 + (ch_f * 0.26).round() as i32, label, label_px, label_color);

            // Large value, left-aligned under the sub-label.
            let num_px = (ch_f * 0.34).max(22.0) as f32 * zoom;
            font::draw_text_aa(f, fb, x as i32 + inset as i32, y as i32 + (ch_f * 0.56).round() as i32, value, num_px, value_color);
        }

        // Grayscale progress bar at the bottom: light track + dark fill ∝ normalized value.
        let bar_x = x + inset;
        let bar_w = cw.saturating_sub(2 * inset);
        let bar_y = y + (ch_f * 0.84).round() as u32;
        let bar_h = (ch_f * 0.07).max(5.0).round() as u32; // ~6px at height 82
        f.rounded_rect(bar_x, bar_y, bar_w, bar_h, bar_h / 2, BAR_TRACK);
        if norm > 0.0 {
            let fill_w = ((bar_w as f64) * norm.clamp(0.0, 1.0)).round().max(1.0) as u32;
            f.rounded_rect(bar_x, bar_y, fill_w.min(bar_w), bar_h, bar_h / 2, BAR_FILL);
        }
    }
}

// --- Dashboard palette (RGB; packed to RGB565 by [`Frame`]) ---
const PAGE_BG: [u8; 3] = [45, 62, 118]; // #2D3E76 page / gaps
const CARD_WHITE: [u8; 3] = [255, 255, 255]; // card background (normal)
const CARD_RED: [u8; 3] = [226, 58, 54]; // card background when temp ≥ limit
const TXT_VALUE: [u8; 3] = [51, 57, 68]; // main numerals on white
const TXT_ON_ALARM: [u8; 3] = [255, 255, 255]; // value/label text on a red card
const TXT_LABEL: [u8; 3] = [130, 137, 149]; // sub-labels on white cards (muted)
const BANNER_TAG: [u8; 3] = [176, 186, 214]; // category tag on the dark page (light slate)
const TXT_MODEL: [u8; 3] = [255, 255, 255]; // model-name text in banners (white)
const BAR_TRACK: [u8; 3] = [224, 227, 231]; // progress track (light gray)
const BAR_FILL: [u8; 3] = [58, 63, 73]; // progress fill (dark gray / near-black)

/// Normalize a temperature against ~90°C so the bar fills as it approaches hot.
fn norm_temp(v: Option<f64>) -> f64 {
    v.map(|t| (t / 90.0).clamp(0.0, 1.0)).unwrap_or(0.0)
}

/// Normalize a percentage against 100%.
fn norm_pct(v: Option<f64>) -> f64 {
    v.map(|p| (p / 100.0).clamp(0.0, 1.0)).unwrap_or(0.0)
}

fn fmt_temp(v: Option<f64>) -> String {
    v.map(|t| format!("{:.0}\u{00B0}", t)).unwrap_or_else(|| "—".to_string())
}

fn fmt_pct(v: Option<f64>) -> String {
    v.map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "—".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelOrder, Spec};

    fn spec() -> Spec {
        Spec {
            width: 8,
            height: 4,
            bytes_per_pixel: 3,
            channel_order: ChannelOrder::Rgb,
        }
    }

    #[test]
    fn test_pattern_has_correct_size_and_quadrants() {
        let c = Composer::new(spec(), 1);
        let f = c.test_pattern();
        assert_eq!(f.bytes.len(), (8 * 4) as usize * 3);

        // (0,0) is the border -> white.
        assert_eq!(&f.bytes[0..3], [255, 255, 255]);
        // (x=1,y=1) is interior top-left quadrant -> red. byte offset = ((1*8)+1)*3 = 27.
        let i = 9 * 3;
        assert_eq!(&f.bytes[i..i + 3], [255, 0, 0]);
    }

    #[test]
    fn compose_small_canvas_fills_page() {
        // An undersized canvas returns just the page fill (no dashboard layout).
        let c = Composer::new(spec(), 1);
        let f = c.compose(&Readout::default());
        assert_eq!(f.bytes.len(), (8 * 4) as usize * 3);
        assert_eq!(&f.bytes[0..3], PAGE_BG);
    }

    #[test]
    fn compose_full_canvas_size_is_correct() {
        // A real-sized canvas must produce exactly width*height*bpp bytes.
        let s = Spec {
            width: 480,
            height: 320,
            bytes_per_pixel: 2,
            channel_order: ChannelOrder::Rgb,
        };
        let c = Composer::new(s, 1);
        let f = c.compose(&Readout::default());
        assert_eq!(f.bytes.len(), (480 * 320) as usize * 2);
    }

    #[test]
    fn norm_helpers_clamp_and_handle_none() {
        assert_eq!(norm_temp(None), 0.0);
        assert!((norm_pct(Some(50.0)) - 0.5).abs() < f64::EPSILON);
        assert_eq!(norm_pct(Some(250.0)), 1.0); // clamped to max
        assert!((norm_temp(Some(-5.0))).abs() < f64::EPSILON); // negative -> 0
    }

    #[test]
    #[ignore] // manual: `cargo test write_dashboard_preview_png -- --ignored` writes /tmp/turmo_preview.png
    fn write_dashboard_preview_png() {
        let s = Spec {
            width: 480,
            height: 320,
            bytes_per_pixel: 3,
            channel_order: ChannelOrder::Rgb,
        };
        let mut c = Composer::new(s, 2); // scale=2 -> zoom 1.0 (default look)
        c.set_context(
            &SystemModels {
                gpu_model: "Radeon AI PRO R9700".into(),
                cpu_model: "AMD Ryzen 9 7945HX".into(),
            },
            85,
            90,
        );
        // GPU temp above limit so the preview shows a red alarm card; CPU below.
        let r = Readout {
            gpu_temp: Some(92.0),
            gpu_usage: Some(45.0),
            gpu_vram: Some(78.0),
            cpu_temp: Some(71.0),
            cpu_usage: Some(12.0),
            ram: Some(34.0),
        };
        let f = c.compose(&r);
        let img = image::RgbImage::from_raw(f.width, f.height, f.bytes).unwrap();
        img.save("/tmp/turmo_preview.png").expect("save preview");
    }

    // Scan a rectangle for palette counts used by the layout assertions below.
    #[derive(Debug)]
    struct Scan {
        red: usize,
        white: usize,
        ink: usize, // dark-ish pixels (text/bars) on the light theme
    }
    fn scan_box(f: &Frame, w: u32, x: u32, y: u32, cw: u32, ch: u32) -> Scan {
        let mut s = Scan { red: 0, white: 0, ink: 0 };
        for yy in y..y + ch {
            for xx in x..x + cw {
                let i = (yy as usize * w as usize + xx as usize) * 3;
                let p = [f.bytes[i], f.bytes[i + 1], f.bytes[i + 2]];
                if p == CARD_RED {
                    s.red += 1;
                }
                if p == CARD_WHITE {
                    s.white += 1;
                }
                let sum = p[0] as u16 + p[1] as u16 + p[2] as u16;
                if sum < 600 {
                    s.ink += 1; // darker than near-white -> text/bar ink
                }
            }
        }
        s
    }

    #[test]
    fn compose_light_layout_cards_text_and_alarm() {
        let w = 480u32;
        let h = 320u32;
        let s = Spec { width: w, height: h, bytes_per_pixel: 3, channel_order: ChannelOrder::Rgb };
        let mut c = Composer::new(s, 2); // scale=2 -> zoom 1.0 (default look)
        c.set_context(
            &SystemModels {
                gpu_model: "Radeon AI PRO R9700".into(),
                cpu_model: "AMD Ryzen 9 7945HX".into(),
            },
            85,
            90,
        );
        // GPU temp above its limit (red), CPU temp below (white). Usage/vram/ram cards never alarm.
        let r = Readout {
            gpu_temp: Some(90.0),
            gpu_usage: Some(45.0),
            gpu_vram: Some(60.0),
            cpu_temp: Some(71.0),
            cpu_usage: Some(12.0),
            ram: Some(34.0),
        };
        let f = c.compose(&r);

        // Geometry mirrors compose(): margin 16/14, gap 12, banner 46, values row 82, 3 cards/row.
        const MX: u32 = 16;
        let card_w = (w - 2 * MX - 2 * 12) / 3; // 141
        let col_x = [MX, MX + card_w + 12, MX + 2 * (card_w + 12)];
        let gpu_row_y = 14 + 46 + 12; // 72
        let cpu_row_y = gpu_row_y + 82 + 12 + 46 + 12; // 224

        assert_eq!(&f.bytes[0..3], PAGE_BG); // page corner (outside all cards/banners)

        // GPU temp card: in red alarm, with ink (dark bar fill).
        let g = scan_box(&f, w, col_x[0], gpu_row_y, card_w, 82);
        assert!(g.red > 50, "GPU temp card not in red alarm: {g:?}");
        assert!(g.ink > 20, "no ink in GPU temp card");

        // CPU temp card: white (below limit), no red.
        let cp = scan_box(&f, w, col_x[0], cpu_row_y, card_w, 82);
        assert_eq!(cp.red, 0, "CPU temp card should not be red: {cp:?}");
        assert!(cp.white > 50, "CPU temp card missing white bg");

        // GPU usage + vram cards (cols 1 & 2): never red even at high %.
        let gu = scan_box(&f, w, col_x[1], gpu_row_y, card_w, 82);
        assert_eq!(gu.red, 0, "usage card must not be red");
        let gv = scan_box(&f, w, col_x[2], gpu_row_y, card_w, 82);
        assert_eq!(gv.red, 0, "vram card must not be red");

        // Both banners render text (white model name and/or light tag).
        for by in [14u32, 166u32] {
            let b = scan_box(&f, w, MX, by, w - 2 * MX, 46);
            assert!(b.white + b.ink > 20, "banner at y={by} rendered no text: {b:?}");
        }
    }
}
