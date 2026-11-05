//! Wire framing: turn a [`crate::frame::Frame`] into the exact byte sequence to
//! send. This is pure (no I/O), so it can be unit-tested without hardware and the
//! protocol knobs are trivially adjustable once Phase 0 capture fills in the real
//! values.

use crate::frame::Frame;

#[derive(Debug, Clone)]
pub struct Framing {
    /// Per-frame preamble/header bytes (from config `frame_header`).
    pub header: Vec<u8>,
    /// Per-frame trailer/footer bytes (from config `frame_footer`).
    pub footer: Vec<u8>,
    /// Prepend width/height as little-endian u16 before the pixel payload.
    pub include_size_fields: bool,
}

impl Framing {
    pub fn new(header: Vec<u8>, footer: Vec<u8>, include_size_fields: bool) -> Self {
        Self {
            header,
            footer,
            include_size_fields,
        }
    }

    /// Estimated byte length of an encoded frame (for buffer sizing).
    pub fn frame_len(&self, f: &Frame) -> usize {
        self.header.len() + (if self.include_size_fields { 4 } else { 0 }) + f.bytes.len()
            + self.footer.len()
    }

    /// Encode `f` into the on-wire byte sequence.
    pub fn encode(&self, f: &Frame) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.frame_len(f));
        out.extend_from_slice(&self.header);
        if self.include_size_fields {
            out.extend_from_slice(&(f.width as u16).to_le_bytes());
            out.extend_from_slice(&(f.height as u16).to_le_bytes());
        }
        out.extend_from_slice(&f.bytes);
        out.extend_from_slice(&self.footer);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelOrder, Spec};

    #[test]
    fn encodes_header_size_pixels_footer() {
        let spec = Spec {
            width: 2,
            height: 1,
            bytes_per_pixel: 3,
            channel_order: ChannelOrder::Rgb,
        };
        let mut f = Frame::new(spec);
        // Two pixels: red then green.
        f.set(0, 0, 255, 0, 0);
        f.set(1, 0, 0, 255, 0);

        let fr = Framing::new(vec![0xAA, 0x55], vec![0x0D], true);
        let out = fr.encode(&f);

        // AA 55 | 02 00 (w) | 01 00 (h) | FF0000 00FF00 | 0D
        let expected: Vec<u8> = vec![
            0xAA, 0x55, 0x02, 0x00, 0x01, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x0D,
        ];
        assert_eq!(out, expected);
    }

    #[test]
    fn omits_size_fields_when_disabled() {
        let spec = Spec {
            width: 1,
            height: 1,
            bytes_per_pixel: 3,
            channel_order: ChannelOrder::Rgb,
        };
        let mut f = Frame::new(spec);
        f.set(0, 0, 1, 2, 3);

        let fr = Framing::new(Vec::new(), Vec::new(), false);
        assert_eq!(fr.encode(&f), vec![1, 2, 3]);
    }
}
