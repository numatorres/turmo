//! Frame representation and composition.

pub mod composer;
pub mod framing;

use crate::config::{ChannelOrder, Rotation, Spec};

/// A single frame of raw pixels in row-major order.
///
/// Pixels are stored in the *final* channel order (i.e. already BGR if the device
/// expects it), so [`framing::Framing`] can ship `bytes` verbatim without a second
/// conversion pass.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    bytes_per_pixel: usize,
    bgr: bool,
    pub bytes: Vec<u8>,
}

impl Frame {
    pub fn new(spec: Spec) -> Self {
        let bpp = spec.bytes_per_pixel as usize;
        let size = (spec.width as usize) * (spec.height as usize) * bpp;
        Self {
            width: spec.width,
            height: spec.height,
            bytes_per_pixel: bpp,
            bgr: spec.channel_order == ChannelOrder::Bgr,
            bytes: vec![0u8; size],
        }
    }

    /// Return this frame rotated into panel orientation. `None` returns a clone;
    /// the 90°/270° variants swap width and height. Pixel data is copied in whole
    /// (bpp-sized) units, so channel order (`bgr`) is preserved automatically.
    pub fn rotated(&self, rot: Rotation) -> Frame {
        if matches!(rot, Rotation::None) {
            return self.clone();
        }

        let w = self.width as usize;
        let h = self.height as usize;
        let bpp = self.bytes_per_pixel;

        // Output dimensions after rotation.
        let (nw, nh) = match rot {
            Rotation::Cw90 | Rotation::Cw270 => (h, w),
            Rotation::Cw180 => (w, h),
            Rotation::None => unreachable!(),
        };

        let mut out = vec![0u8; nw * nh * bpp];
        for y in 0..h {
            for x in 0..w {
                let si = (y * w + x) * bpp;
                // Destination pixel after rotating the source.
                let (dx, dy) = match rot {
                    Rotation::Cw90 => (h - 1 - y, x),
                    Rotation::Cw270 => (y, w - 1 - x),
                    Rotation::Cw180 => (w - 1 - x, h - 1 - y),
                    Rotation::None => unreachable!(),
                };
                let di = (dy * nw + dx) * bpp;
                out[di..di + bpp].copy_from_slice(&self.bytes[si..si + bpp]);
            }
        }

        Frame {
            width: nw as u32,
            height: nh as u32,
            bytes_per_pixel: self.bytes_per_pixel,
            bgr: self.bgr,
            bytes: out,
        }
    }

    /// Pack a logical `[r,g,b]` into the frame's native pixel bytes for its bpp:
    /// 3 = RGB/BGR, 2 = 16-bit RGB565 (big-endian), 1 = luminance threshold.
    fn pack(&self, r: u8, g: u8, b: u8) -> [u8; 3] {
        match self.bytes_per_pixel {
            1 => {
                // Luminance threshold to 1-bit. Compute in u32 (u16 overflows).
                let lum = (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000;
                [if lum > 127 { 255 } else { 0 }, 0, 0]
            }
            2 => {
                // RGB565: 5 bits R, 5 bits G, 6 bits B. Big-endian (high byte first),
                // the common layout for these panels. Third slot unused. Promote to
                // u16 before shifting so the shifts can't overflow u8.
                let v = ((r as u16 & 0xF8) << 8) | ((g as u16 & 0xFC) << 3) | (b as u16 >> 3);
                [(v >> 8) as u8, v as u8, 0] // big-endian: high byte first
            }
            _ => {
                let mut px = [r, g, b];
                if self.bgr {
                    px.swap(0, 2);
                }
                px
            }
        }
    }

    /// Fill the whole frame with a single RGB color.
    pub fn fill(&mut self, r: u8, g: u8, b: u8) {
        let p = self.pack(r, g, b);
        for chunk in self.bytes.chunks_mut(self.bytes_per_pixel) {
            let n = chunk.len().min(3);
            chunk[..n].copy_from_slice(&p[..n]);
        }
    }

    /// Set one pixel to logical `[r,g,b]`. Out-of-bounds writes are ignored so the
    /// text rasterizer can run without per-pixel bounds checks.
    pub fn set(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8) {
        if x >= self.width || y >= self.height || self.bytes_per_pixel == 0 {
            return;
        }
        let p = self.pack(r, g, b);
        let i = ((y * self.width) + x) as usize * self.bytes_per_pixel;
        for (k, v) in p.iter().enumerate().take(self.bytes_per_pixel) {
            if i + k < self.bytes.len() {
                self.bytes[i + k] = *v;
            }
        }
    }

    /// Copy an RGB pixel buffer (row-major, `width*height*3`) into this frame,
    /// converting each pixel to the native bpp.
    pub fn blit_rgb(&mut self, rgb: &[u8]) {
        let w = self.width as usize;
        let h = self.height as usize;
        if rgb.len() < w * h * 3 || self.bytes_per_pixel == 0 {
            return;
        }
        for y in 0..h {
            for x in 0..w {
                let s = (y * w + x) * 3;
                self.set(x as u32, y as u32, rgb[s], rgb[s + 1], rgb[s + 2]);
            }
        }
    }

    /// Unpack a pixel into logical `[r,g,b]` (inverse of [`Self::pack`]). `None` if out of bounds.
    #[must_use]
    fn unpack(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        if x >= self.width || y >= self.height || self.bytes_per_pixel == 0 {
            return None;
        }
        let i = ((y * self.width) + x) as usize * self.bytes_per_pixel;
        match self.bytes_per_pixel {
            1 => {
                let v = self.bytes[i];
                Some([v, v, v])
            }
            2 => {
                // Inverse of the big-endian RGB565 pack.
                let v = (self.bytes[i] as u16) << 8 | self.bytes[i + 1] as u16;
                let r = (((v >> 11) & 0x1f) << 3) | (((v >> 11) & 0x1f) >> 2);
                let g = (((v >> 5) & 0x3f) << 2) | (((v >> 5) & 0x3f) >> 4);
                let b = ((v & 0x3f) << 2) | ((v & 0x3f) >> 2);
                Some([r as u8, g as u8, b as u8])
            }
            _ => {
                if self.bgr {
                    Some([self.bytes[i + 2], self.bytes[i + 1], self.bytes[i]])
                } else {
                    Some([self.bytes[i], self.bytes[i + 1], self.bytes[i + 2]])
                }
            }
        }
    }

    /// Alpha-blend an `[r,g,b]` color over a pixel with `alpha` in `0.0..=1.0`. Used for
    /// antialiased text: coverage from the glyph rasterizer drives the blend weight.
    pub fn blend_pixel(&mut self, x: u32, y: u32, rgb: [u8; 3], alpha: f64) {
        let Some(mut d) = self.unpack(x, y) else { return };
        let a = alpha.clamp(0.0, 1.0);
        for c in 0..3 {
            d[c] = (rgb[c] as f64 * a + d[c] as f64 * (1.0 - a)).round() as u8;
        }
        self.set(x, y, d[0], d[1], d[2]);
    }

    /// Fill a rounded rectangle (corner radius `r`) with a color, clipped to the frame.
    pub fn rounded_rect(&mut self, x: u32, y: u32, w: u32, h: u32, r: u32, rgb: [u8; 3]) {
        if w == 0 || h == 0 || self.bytes_per_pixel == 0 {
            return;
        }
        let p = self.pack(rgb[0], rgb[1], rgb[2]);
        // Radius clamped so the inner-rect projection is valid (avoids clamp panics).
        let rr = r.min(w / 2).min(h / 2) as i64;
        for yy in y..y.saturating_add(h).min(self.height) {
            for xx in x..x.saturating_add(w).min(self.width) {
                // Inside iff the pixel is within `rr` of its projection onto the inner rect.
                let px = xx as i64;
                let py = yy as i64;
                let ix = px.clamp(x as i64 + rr, x as i64 + (w as i64) - rr);
                let iy = py.clamp(y as i64 + rr, y as i64 + (h as i64) - rr);
                if (px - ix) * (px - ix) + (py - iy) * (py - iy) <= rr * rr {
                    let i = (yy * self.width + xx) as usize * self.bytes_per_pixel;
                    let n = self.bytes_per_pixel.min(3);
                    if let Some(dst) = self.bytes.get_mut(i..i + n) {
                        dst.copy_from_slice(&p[..n]);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelOrder, Rotation};

    #[test]
    fn rotate_cw90_swaps_dims_and_maps_pixels() {
        let spec = Spec {
            width: 2,
            height: 3,
            bytes_per_pixel: 3,
            channel_order: ChannelOrder::Rgb,
        };
        let mut f = Frame::new(spec);
        // Distinct grey value per source pixel (stored as [v,v,v]).
        for y in 0..3u32 {
            for x in 0..2u32 {
                let v = 10 * ((y * 2 + x) as u8 + 1);
                f.set(x, y, v, v, v);
            }
        }

        let r = f.rotated(Rotation::Cw90);
        assert_eq!((r.width, r.height), (3, 2)); // portrait -> landscape
        // Source (x=0,y=2) value 50 lands at dest (dx=0,dy=0).
        assert_eq!(r.bytes[0..3], [50, 50, 50]);
        // Source (x=1,y=0) value 20 lands at dest (dx=2,dy=1) -> pixel index 3*1+2 = 5.
        assert_eq!(r.bytes[15..18], [20, 20, 20]);
    }

    #[test]
    fn pack_rgb565_and_threshold() {
        let spec2 = Spec {
            width: 1,
            height: 1,
            bytes_per_pixel: 2,
            channel_order: ChannelOrder::Rgb,
        };
        let mut f = Frame::new(spec2);
        // Pure red -> RGB565 0xF800 (big-endian F8 00).
        f.set(0, 0, 255, 0, 0);
        assert_eq!(f.bytes[0..2], [0xF8, 0x00]);

        let spec1 = Spec {
            width: 1,
            height: 1,
            bytes_per_pixel: 1,
            channel_order: ChannelOrder::Rgb,
        };
        let mut m = Frame::new(spec1);
        m.set(0, 0, 255, 255, 255); // white -> 1
        assert_eq!(m.bytes[0], 255);
        m.set(0, 0, 0, 0, 0); // black -> 0
        assert_eq!(m.bytes[0], 0);
    }

    #[test]
    fn rotate_none_is_identity() {
        let spec = Spec {
            width: 4,
            height: 2,
            bytes_per_pixel: 3,
            channel_order: ChannelOrder::Rgb,
        };
        let mut f = Frame::new(spec);
        f.set(0, 0, 9, 8, 7);
        let r = f.rotated(Rotation::None);
        assert_eq!((r.width, r.height), (4, 2));
        assert_eq!(r.bytes[0..3], [9, 8, 7]);
    }
}
