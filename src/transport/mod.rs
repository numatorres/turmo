//! Transport layer: ship raw bytes to the device.
//!
//! Framing (header / size fields / pixels / footer) is deliberately *not* here —
//! it lives in [`crate::frame::framing::Framing`] so it can be unit-tested without
//! hardware and swapped independently of the wire path. The sink only owns the
//! open/handshake/write/close lifecycle for one concrete transport.

use anyhow::Result;

pub mod serial;
#[cfg(feature = "bulk")]
pub mod bulk;

/// Minimal raw byte sink. Implementations must be safe to use from a single thread.
pub trait ByteSink: Send {
    /// Open the underlying device and put it into a known state.
    fn open(&mut self) -> Result<()>;
    /// One-time setup/handshake bytes (captured in Phase 0). No-op if empty.
    fn init(&mut self, bytes: &[u8]) -> Result<()>;
    /// Write all of `buf` to the device, handling partial writes/EINTR.
    fn write_all(&mut self, buf: &[u8]) -> Result<()>;
    /// Release the device.
    fn close(&mut self);
    /// Optional hard reset of the underlying device (e.g. a USB port reset). The default
    /// implementation reports it as unsupported; transports that can reset override this.
    fn reset(&mut self) -> Result<()> {
        Err(anyhow::anyhow!("reset is not supported by this transport"))
    }
}
