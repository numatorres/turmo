//! Minimal 5x7 bitmap font + text rasterizer for the on-screen readout overlay.
//!
//! Kept dependency-free and self-contained so the live mode can render numbers
//! without pulling in a full text stack. Covers A-Z, 0-9 and the symbols used by
//! the GPU readout (`:`, `%`, `-`, `.`). Unknown glyphs render as blank.

use rusttype::{Font, Point, Scale};

use crate::frame::Frame;

/// Embedded DejaVu Sans (OFL) — regular for labels, bold for large numerals.
static FONT_REGULAR_BYTES: &[u8] = include_bytes!("../../assets/font.ttf");
static FONT_BOLD_BYTES: &[u8] = include_bytes!("../../assets/font-bold.ttf");

/// Load a TTF face from the embedded bytes (`bold` selects the Bold cut). `None` if malformed.
#[must_use]
pub fn load_face(bold: bool) -> Option<Font<'static>> {
    let bytes: &'static [u8] = if bold { FONT_BOLD_BYTES } else { FONT_REGULAR_BYTES };
    Font::try_from_bytes(bytes)
}

/// Antialiased text width in pixels (sum of glyph advances) — used for alignment.
#[must_use]
pub fn text_width(font: &Font, text: &str, px: f32) -> f32 {
    let scale = Scale::uniform(px);
    text.chars()
        .map(|c| font.glyph(c).scaled(scale).h_metrics().advance_width)
        .sum()
}

/// Rasterize `text` antialiased onto `frame`, with the baseline's left edge at
/// `(x0, baseline_y)`. `px` is the pixel size (ascent+descent height); `color` is RGB.
pub fn draw_text_aa(frame: &mut Frame, font: &Font, x0: i32, baseline_y: i32, text: &str, px: f32, color: [u8; 3]) {
    let scale = Scale::uniform(px);
    let mut pen_x = x0 as f32;
    for ch in text.chars() {
        // Advance comes from the scaled glyph; `h_metrics` is not on PositionedGlyph.
        let sg = font.glyph(ch).scaled(scale);
        let adv = sg.h_metrics().advance_width;
        let glyph = sg.positioned(Point {
            x: pen_x,
            y: baseline_y as f32,
        });
        if let Some(bb) = glyph.pixel_bounding_box() {
            let bx = bb.min.x;
            let by = bb.min.y;
            // `draw` yields per-pixel coverage (0.0..=1.0); blend it onto the frame.
            glyph.draw(|gx, gy, cov| {
                if cov > 0.02 {
                    // Negative offsets wrap to huge u32 and are clipped inside the frame.
                    let fx = (bx + gx as i32) as u32;
                    let fy = (by + gy as i32) as u32;
                    frame.blend_pixel(fx, fy, color, cov as f64);
                }
            });
        }
        pen_x += adv;
    }
}

// Legacy dependency-free bitmap path, retained as a fallback if the embedded TTF is unavailable.
#[allow(dead_code)]
/// One glyph is 7 rows (top->bottom); each row packs 5 columns in bits 4..0.
fn glyph(c: u8) -> Option<&'static [u8; 7]> {
    Some(match c {
        b' ' => &[0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000],
        b'0' => &[0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        b'1' => &[0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        b'2' => &[0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        b'3' => &[0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
        b'4' => &[0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        b'5' => &[0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        b'6' => &[0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        b'7' => &[0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        b'8' => &[0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        b'9' => &[0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        b'A' => &[0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        b'B' => &[0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        b'C' => &[0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        b'D' => &[0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100],
        b'E' => &[0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        b'F' => &[0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        b'G' => &[0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111],
        b'H' => &[0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        b'I' => &[0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        b'J' => &[0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        b'K' => &[0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        b'L' => &[0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        b'M' => &[0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        b'N' => &[0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        b'O' => &[0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        b'P' => &[0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        b'Q' => &[0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        b'R' => &[0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        b'S' => &[0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        b'T' => &[0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        b'U' => &[0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        b'V' => &[0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        b'W' => &[0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b10101, 0b01010],
        b'X' => &[0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        b'Y' => &[0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        b'Z' => &[0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        b':' => &[0b00000, 0b00100, 0b00100, 0b00000, 0b00100, 0b00100, 0b00000],
        b'%' => &[0b11001, 0b11010, 0b00100, 0b00010, 0b01000, 0b01011, 0b10001],
        b'-' => &[0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        b'.' => &[0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100],
        _ => return None,
    })
}

#[allow(dead_code)]
/// Advance (in pixels) for one character at the given scale: glyph width + 1 gap.
pub fn char_advance(scale: u32) -> u32 {
    6 * scale
}

#[allow(dead_code)]
/// Rasterize `text` onto `frame` starting at `(x0, y0)` using `color`.
pub fn draw_text(frame: &mut Frame, x0: u32, y0: u32, text: &str, scale: u8, color: [u8; 3]) {
    let scale = scale.max(1) as u32;
    let mut x = x0;
    for ch in text.chars() {
        if (ch as u32) < 256 {
            if let Some(g) = glyph(ch as u8) {
                for (ry, row) in g.iter().enumerate() {
                    for cx in 0..5u32 {
                        if row & (1 << (4 - cx)) != 0 {
                            for dy in 0..scale {
                                for dx in 0..scale {
                                    frame.set(x + cx * scale + dx, y0 + ry as u32 * scale + dy, color[0], color[1], color[2]);
                                }
                            }
                        }
                    }
                }
            }
        }
        x += char_advance(scale);
    }
}
