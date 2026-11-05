//! Raw USB *bulk* sink, built with `--features bulk` and requiring the system
//! `libusb` library.
//!
//! This exists because the bundled Windows build also ships an Indirect-Display
//! driver that hints frames *may* travel over a separate bulk interface rather
//! than the CDC serial port. It is **off by default** so the primary (serial)
//! path builds everywhere. Enable it only after Phase 0 traffic capture confirms
//! a bulk endpoint carries the pixel data, and fill in `bulk_out_endpoint` /
//! VID/PID from `lsusb -v`.

use anyhow::{bail, Context as _, Result};
use libusb::prelude::*;

use super::ByteSink;

pub struct BulkSink {
    vid: u16,
    pid: u16,
    ep_out: u8,
    ctx: Option<Context>,
    handle: Option<DeviceHandle>,
}

impl BulkSink {
    pub fn new(vid: u16, pid: u16, ep_out: u8) -> Self {
        Self {
            vid,
            pid,
            ep_out,
            ctx: None,
            handle: None,
        }
    }

    fn find_device(&self, ctx: &Context) -> Result<Device> {
        let list = ctx.device_list();
        for dev in list.devices() {
            if dev.vendor_id() == self.vid && dev.product_id() == self.pid {
                return Ok(dev);
            }
        }
        bail!(
            "no USB device with VID {:04x} / PID {:04x} found (is the monitor plugged in?)",
            self.vid,
            self.pid
        );
    }
}

impl super::ByteSink for BulkSink {
    fn open(&mut self) -> Result<()> {
        let ctx = Context::new().context("failed to initialize libusb")?;
        // SAFETY: the crate manages USB device lifetimes internally.
        let dev = unsafe { self.find_device(&ctx)? };
        let handle = dev
            .open_handle()
            .with_context(|| format!("open handle for {:04x}:{:04x}", self.vid, self.pid))?;
        self.ctx = Some(ctx);
        self.handle = Some(handle);
        Ok(())
    }

    fn init(&mut self, _bytes: &[u8]) -> Result<()> {
        // Bulk path has no serial handshake to replay in v1; left as a hook.
        Ok(())
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        let handle = self
            .handle
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("bulk device not open"))?;
        // SAFETY: the crate transfers `buf` to the endpoint for us.
        unsafe { handle.bulk_transfer_out(buf, 1000) }
            .with_context(|| "bulk transfer to out endpoint failed")?;
        Ok(())
    }

    fn close(&mut self) {
        if let Some(h) = self.handle.take() {
            // SAFETY: we own this handle and are releasing it.
            unsafe { h.close_device() };
        }
        self.ctx = None;
    }
}

impl Drop for BulkSink {
    fn drop(&mut self) {
        self.close();
    }
}
