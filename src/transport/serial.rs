//! Raw CDC-serial sink using `libc` + POSIX termios (no external C library).
//!
//! The QinHeng UsbMonitor presents as a standard USB CDC ACM port, which on Linux
//! is `/dev/ttyACM*`. We open it raw (8N1) and write composed frames to it.

use std::ffi::CString;
use std::os::unix::io::RawFd;
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use libc::{c_int, tcflush, TCSANOW, TCIOFLUSH};

use super::ByteSink;

/// Serial byte sink configured for a specific device node and baud rate.
pub struct SerialSink {
    path: String,
    baud: u32,
    fd: RawFd,
}

impl SerialSink {
    pub fn new(path: &str, baud: u32) -> Self {
        Self {
            path: path.to_string(),
            baud,
            fd: -1,
        }
    }

    /// Configure a raw 8N1 line discipline at the requested speed.
    unsafe fn configure(fd: RawFd, baud: u32) -> Result<()> {
        let mut tio: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut tio) != 0 {
            bail!("tcgetattr failed: {}", std::io::Error::last_os_error());
        }

        libc::cfsetspeed(&mut tio, baud as libc::speed_t);

        // Input: disable line translation + software flow control (raw passthrough).
        tio.c_iflag &= !(libc::ICRNL | libc::INLCR | libc::IXON);
        // Output: raw.
        tio.c_oflag &= !libc::OPOST;
        // Local: no echo/canonical/signal processing.
        tio.c_lflag = 0;
        // Control: 8 data bits, no parity, one stop bit, enable receiver, ignore modem lines.
        tio.c_cflag = (tio.c_cflag & !(libc::CSIZE | libc::PARENB | libc::CSTOPB))
            | (libc::CS8 | libc::CREAD | libc::CLOCAL);

        if libc::tcsetattr(fd, TCSANOW as c_int, &tio) != 0 {
            bail!("tcsetattr failed: {}", std::io::Error::last_os_error());
        }
        tcflush(fd, TCIOFLUSH);
        Ok(())
    }

    fn fd(&self) -> Result<RawFd> {
        if self.fd < 0 {
            bail!("serial device not open");
        }
        Ok(self.fd)
    }

    /// Resolve this device's raw USB node (`/dev/bus/usb/<bus>/<dev>`) via sysfs. That is the
    /// only fd that accepts `USBDEVFS_RESET` — a CDC-ACM tty rejects it with ENOTTY.
    fn usb_reset_path(&self) -> Result<String> {
        let name = self.path.rsplit('/').next().unwrap_or("");
        if name.is_empty() {
            bail!("cannot derive USB node from path '{}'", self.path);
        }
        // /sys/class/tty/<name>/device -> .../usbN/N-x/N-x:1.0 (the interface); its parent is the
        // usb_device dir holding `busnum` + `devnum`.
        let iface = std::fs::canonicalize(format!("/sys/class/tty/{}/device", name))
            .with_context(|| format!("resolve tty '{}' in sysfs", name))?;
        let dev_dir = iface.parent().context("missing USB device dir in sysfs")?;
        let bus = read_trim(dev_dir, "busnum").context("read busnum")?;
        let dev = read_trim(dev_dir, "devnum").context("read devnum")?;
        Ok(format!("/dev/bus/usb/{:03}/{:03}", bus, dev))
    }

    /// USB port reset on the raw `/dev/bus/usb` node (preferred where the kernel allows it).
    fn usb_port_reset(&self) -> Result<()> {
        let path = self.usb_reset_path()?;
        // SAFETY: valid NUL-terminated C string for the duration of open(). A read-only handle is
        // enough for the port-reset ioctl and needs no write access to /dev/bus/usb.
        let c = std::ffi::CString::new(path.clone())?;
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY) };
        if fd < 0 {
            bail!(
                "cannot open USB node '{path}': {} (needs access to /dev/bus/usb)",
                std::io::Error::last_os_error()
            );
        }
        // <linux/usbdevice_fs.h>: USBDEVFS_RESET = _IOW('U', 21, int).
        const USBDEVFS_RESET: libc::c_ulong = 0x45_555;
        // SAFETY: fd is a valid descriptor we just opened; the ioctl performs a synchronous reset.
        let rc = unsafe { libc::ioctl(fd, USBDEVFS_RESET) };
        // SAFETY: fd is a valid descriptor we own and are about to release.
        unsafe { libc::close(fd) };
        if rc < 0 {
            bail!("USBDEVFS_RESET on '{path}': {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Unbind then rebind this device's `cdc_acm` interface instances via sysfs, forcing a full ACM
    /// re-init (pointer → home). Root-only; used when the kernel refuses to port-reset a bound device.
    fn rebind_cdc_acm(&self) -> Result<()> {
        let name = self.path.rsplit('/').next().unwrap_or("");
        if name.is_empty() {
            bail!("cannot derive USB node from path '{}'", self.path);
        }
        // .../usb1/N-x/N-x:1.0 (interface); its parent is the usb_device dir holding the interface subdirs.
        let iface = std::fs::canonicalize(format!("/sys/class/tty/{}/device", name))
            .with_context(|| format!("resolve tty '{name}' in sysfs"))?;
        let dev_dir = iface.parent().context("missing USB device dir")?.to_path_buf();

        // Collect this device's cdc_acm-bound interface devpaths (e.g. "1-2:1.0", "1-2:1.1").
        let mut ifs: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&dev_dir).with_context(|| format!("list '{}'", dev_dir.display()))? {
            let entry = entry.context("reading dir entry")?;
            let if_name = entry.file_name().to_string_lossy().into_owned();
            if driver_of(&dev_dir, &if_name).as_deref() == Some("cdc_acm") {
                ifs.push(if_name);
            }
        }
        if ifs.is_empty() {
            bail!("no cdc_acm-bound interfaces under '{}'", dev_dir.display());
        }

        const UNBIND: &str = "/sys/bus/usb/drivers/cdc_acm/unbind";
        const BIND: &str = "/sys/bus/usb/drivers/cdc_acm/bind";
        // Unbinding one interface can drop its siblings as a side effect, so "no such device" here is
        // expected and fine — it's already gone, which is exactly the reset state we want pre-rebind.
        for if_name in &ifs {
            write_sysfs(UNBIND, if_name, true)?;
        }
        std::thread::sleep(Duration::from_millis(250)); // let the kernel tear down the function

        // Rebind each interface (retrying — it may need a moment after teardown to become bindable).
        for if_name in &ifs {
            for _ in 0..10 {
                if write_sysfs(BIND, if_name, false).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }

        // Verify the tty node actually returned — a masked rebind failure must not be reported as success.
        let tty = format!("/sys/class/tty/{name}");
        for _ in 0..15 {
            if std::path::Path::new(&tty).exists() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(200)); // up to ~3 s for rebind to settle
        }
        bail!("/dev/{name} did not return after rebind — replug the USB cable to recover")
    }
}

/// Read a sysfs attribute as a trimmed `u32`.
fn read_trim(dir: &std::path::Path, name: &str) -> Result<u32> {
    let p = dir.join(name);
    let s = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    s.trim().parse::<u32>().with_context(|| format!("parse '{}' as a number", name))
}

/// Basename of the driver bound to a sysfs device/interface dir, if any.
fn driver_of(dir: &std::path::Path, name: &str) -> Option<String> {
    std::fs::read_link(dir.join(name).join("driver"))
        .ok()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
}

/// Write a value to a sysfs control file (root-only). With `lenient_enodev`, "no such device" is
/// treated as success — unbinding one interface can drop its siblings, and rebinding may find them back.
fn write_sysfs(path: &str, value: &str, lenient_enodev: bool) -> Result<()> {
    match std::fs::write(path, value) {
        Ok(()) => Ok(()),
        Err(e) if lenient_enodev && e.raw_os_error() == Some(libc::ENODEV) => Ok(()),
        Err(e) => bail!("write '{value}' to {path} (needs root): {e}"),
    }
}

impl super::ByteSink for SerialSink {
    fn open(&mut self) -> Result<()> {
        let c = CString::new(self.path.clone())
            .with_context(|| format!("invalid device path '{}'", self.path))?;
        // SAFETY: `c` is a valid NUL-terminated C string for the duration of this call.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
        if fd < 0 {
            let e = std::io::Error::last_os_error();
            bail!(
                "cannot open '{}': {} (is it present? try `ls -l /dev/ttyACM*`; \
                 permissions: add your user to the dialout group or install udev/99-turmo.rules)",
                self.path, e
            );
        }
        // SAFETY: fd is a valid descriptor we just opened; configure uses it synchronously.
        unsafe { Self::configure(fd, self.baud) }.with_context(|| format!("configure '{}'", self.path))?;
        self.fd = fd;
        Ok(())
    }

    fn init(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let fd = self.fd()?;
        // SAFETY: write to a valid descriptor we own.
        let n = unsafe { libc::write(fd, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
        if n < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            bail!("init write failed: {}", std::io::Error::last_os_error());
        }
        // Brief settle so the device can latch any handshake state.
        std::thread::sleep(Duration::from_millis(20));
        Ok(())
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        let fd = self.fd()?;
        let mut off = 0usize;
        while off < buf.len() {
            // SAFETY: pointer into `buf` for the remaining slice; fd is valid.
            let n = unsafe {
                libc::write(
                    fd,
                    buf[off..].as_ptr() as *const libc::c_void,
                    buf.len() - off,
                )
            };
            if n < 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                bail!("write failed: {e}");
            }
            off += n as usize;
        }
        Ok(())
    }

    fn close(&mut self) {
        if self.fd >= 0 {
            // SAFETY: fd is a valid descriptor we opened and have not closed yet.
            unsafe { libc::close(self.fd) };
            self.fd = -1;
        }
    }

    /// Re-home the device. Tries a USB port reset first (works where permitted); if refused — as on
    /// driver-bound CDC-ACM devices here — falls back to unbinding/rebinding the `cdc_acm` interfaces
    /// via sysfs, which forces a full re-init (pointer → home) and needs root. After this call the
    /// caller must `close()` + `open()`: /dev/ttyACM* disappears and returns during the reset.
    fn reset(&mut self) -> Result<()> {
        match self.usb_port_reset() {
            Ok(()) => return Ok(()),
            Err(e) => eprintln!("turmo: USB port reset failed ({e}); trying cdc_acm rebind"),
        }
        self.rebind_cdc_acm().context("re-home the panel")?;
        Ok(())
    }
}

impl Drop for SerialSink {
    fn drop(&mut self) {
        self.close();
    }
}
