//! turmo — drive a QinHeng USB Monitor (1a86:5722) to show live hardware values on Linux.
//!
//! Modes: `--test` sends an asymmetric test pattern (validates transport and catches
//! B/R swaps + mirroring against the glass); default is live mode with an optional
//! background image plus a GPU temp/usage text overlay. See PROTOCOL.md for the
//! capture-first workflow.

use std::time::{Duration, Instant};

use anyhow::{bail, Context as _, Result};

mod config;
mod font;
mod frame;
mod sensor;
mod transport;

use config::{Config, TransportKind};
use frame::composer::Composer;
use frame::framing::Framing;
use image::imageops::FilterType;
use sensor::cpu::{self, CpuUsageMeter};
use sensor::mem;
use sensor::models::SystemModels;
use sensor::{GpuValues, Readout};
use transport::ByteSink;

struct Args {
    config_path: String,
    device: Option<String>,
    baud: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    bpp: Option<u8>,
    rotation: Option<String>,
    no_size_fields: bool,
    init_hex: Option<String>,
    frame_header_hex: Option<String>,
    /// One-shot home/reset probe: a hex string, `"reset"` (USB device reset), or `"none"`.
    probe_home: Option<String>,
    /// Byte count of the mid-buffer offset to create before the probe's final frame.
    offset_bytes: Option<u32>,
    test: bool,
    once: bool,
}

/// Parse a hex string like `"aa55"` (spaces/0x ok) into bytes.
fn parse_hex(s: &str) -> Result<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !cleaned.len().is_multiple_of(2) {
        bail!("hex string must have an even number of digits: '{s}'");
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).map_err(|_| anyhow::anyhow!("bad hex pair '{}' in '{s}'", &cleaned[i..i + 2])))
        .collect()
}

fn print_help() {
    println!(
        "turmo — USB monitor hardware display (Linux)\n\
         \n\
         USAGE:\n  \
         turmo [OPTIONS]\n\
         \n\
         OPTIONS:\n  \
          -c, --config <PATH>   config file (default: ./turmo.toml)\n  \
              --device <PATH>   device node override (e.g. /dev/ttyACM0)\n  \
              --baud <N>        serial baud rate override\n  \
              --width <N>       canvas width in pixels (override config)\n  \
              --height <N>      canvas height in pixels (override config)\n  \
              --bpp <N>         bytes per pixel: 1 | 2 | 3 (override config)\n  \
              --rotation <R>    none | cw90 | cw180 | cw270 (override config)\n  \
              --no-size-fields  don't prepend a width/height header before pixels\n  \
               --init <HEX>      one-time bytes sent after open (e.g. cursor-home/reset)\n  \
               --frame-header <HEX>  bytes sent before every frame's pixels\n  \
               --probe-home <V>  one-shot probe: 'hex' | 'reset' | 'none'; writes a known offset,\n  \
                                 then the candidate, then ONE full test frame (judge alignment)\n  \
               --offset-bytes <N> bytes of mid-buffer offset to create in probe mode (default: 1/4 frame)\n  \
               --test            send an asymmetric test pattern (no sensor text)\n  \
              --once            send a single frame, then exit\n  \
              -h, --help        show this help"
    );
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        config_path: "turmo.toml".to_string(),
        device: None,
        baud: None,
        width: None,
        height: None,
        bpp: None,
        rotation: None,
        no_size_fields: false,
        init_hex: None,
        frame_header_hex: None,
        probe_home: None,
        offset_bytes: None,
        test: false,
        once: false,
    };
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-c" | "--config" => {
                i += 1;
                if i >= raw.len() {
                    bail!("--config requires a value");
                }
                args.config_path = raw[i].clone();
            }
            "--device" => {
                i += 1;
                if i >= raw.len() {
                    bail!("--device requires a value");
                }
                args.device = Some(raw[i].clone());
            }
            "--baud" => {
                i += 1;
                if i >= raw.len() {
                    bail!("--baud requires a value");
                }
                args.baud = Some(raw[i].parse()?);
            }
            "--width" => {
                i += 1;
                if i >= raw.len() {
                    bail!("--width requires a value");
                }
                args.width = Some(raw[i].parse()?);
            }
            "--height" => {
                i += 1;
                if i >= raw.len() {
                    bail!("--height requires a value");
                }
                args.height = Some(raw[i].parse()?);
            }
            "--bpp" => {
                i += 1;
                if i >= raw.len() {
                    bail!("--bpp requires a value");
                }
                let v: u8 = raw[i].parse()?;
                if !(1..=3).contains(&v) {
                    bail!("--bpp must be 1, 2, or 3 (got {v})");
                }
                args.bpp = Some(v);
            }
            "--rotation" => {
                i += 1;
                if i >= raw.len() {
                    bail!("--rotation requires a value");
                }
                args.rotation = Some(raw[i].clone());
            }
            "--no-size-fields" => args.no_size_fields = true,
            "--init" => {
                i += 1;
                if i >= raw.len() {
                    bail!("--init requires a value");
                }
                args.init_hex = Some(raw[i].clone());
            }
            "--frame-header" => {
                i += 1;
                if i >= raw.len() {
                    bail!("--frame-header requires a value");
                }
                args.frame_header_hex = Some(raw[i].clone());
            }
            "--probe-home" => {
                i += 1;
                if i >= raw.len() {
                    bail!("--probe-home requires a value (hex, 'reset', or 'none')");
                }
                args.probe_home = Some(raw[i].clone());
            }
            "--offset-bytes" => {
                i += 1;
                if i >= raw.len() {
                    bail!("--offset-bytes requires a value");
                }
                args.offset_bytes = Some(raw[i].parse()?);
            }
            "--test" => args.test = true,
            "--once" => args.once = true,
            other => bail!("unknown argument '{other}' (see --help)"),
        }
        i += 1;
    }
    Ok(args)
}

fn load_background(cfg: &Config, composer: &mut Composer) {
    let Some(path) = cfg.bg_image.clone() else {
        return;
    };
    match image::open(&path) {
        Ok(img) => {
            let rgb = img.resize_exact(cfg.width, cfg.height, FilterType::Lanczos3).to_rgb8();
            composer.set_bg(rgb.into_raw());
            eprintln!("loaded background '{path}'");
        }
        Err(e) => {
            eprintln!(
                "warning: could not load background '{path}': {e}; using dark fill"
            );
        }
    }
}

/// One-shot hardware probe for a per-frame home/reset candidate. Creates a known mid-buffer
/// write-pointer offset, applies the candidate (`hex` bytes, `"reset"` for a USB device reset, or
/// `"none"`), then sends one full asymmetric test-pattern frame so any shift is immediately
/// visible on glass. Used to find which sequence re-homes the panel after an interrupted frame.
fn run_probe(
    sink: &mut Box<dyn ByteSink>,
    composer: &Composer,
    framing: &Framing,
    rotation: config::Rotation,
    home: &str,
    offset_bytes: Option<u32>,
) -> Result<()> {
    let pattern = composer.test_pattern().rotated(rotation);
    let full_len = pattern.bytes.len() as u32;
    let n = offset_bytes.unwrap_or(full_len / 4).min(full_len);

    // 1) Advance the device's write pointer to a known mid-buffer position.
    eprintln!("probe: writing {n} byte(s) of pixel data to create an offset");
    sink.write_all(&pattern.bytes[..n as usize]).context("offset write")?;

    // 2) Apply the candidate home/reset before the real frame.
    match home {
        "none" => eprintln!("probe: no home command (baseline — expect a visible shift)"),
        "reset" => {
            eprintln!("probe: USB device reset");
            sink.reset().context("usb reset")?;
            // Device re-enumerates: /dev/ttyACM* disappears then returns. Retry the reopen briefly.
            sink.close();
            let mut reopened = false;
            for _ in 0..10 {
                if sink.open().is_ok() {
                    reopened = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(200)); // up to ~2 s for rebind to settle
            }
            if !reopened {
                bail!("device did not come back after reset — replug the USB cable and retry");
            }
        }
        hex => {
            let bytes = parse_hex(hex)?;
            let shown: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
            eprintln!("probe: sending {} home byte(s): {}", bytes.len(), shown.join(" "));
            if !bytes.is_empty() {
                sink.write_all(&bytes).context("home write")?;
                std::thread::sleep(Duration::from_millis(30)); // settle before pixels
            }
        }
    }

    // 3) Send one full framed test-pattern frame — alignment is judged against this.
    let frame = framing.encode(&pattern);
    eprintln!("probe: sending {} byte full test-pattern frame", frame.len());
    sink.write_all(&frame).context("final frame")?;
    Ok(())
}

fn run(args: Args) -> Result<()> {
    let mut cfg = Config::load(&args.config_path)?;
    if let Some(d) = &args.device {
        cfg.device = d.clone();
    }
    if let Some(b) = args.baud {
        cfg.baud = b;
    }
    // Geometry overrides — handy for sweeping the native panel size without editing config.
    if let Some(w) = args.width {
        cfg.width = w;
    }
    if let Some(h) = args.height {
        cfg.height = h;
    }
    if let Some(b) = args.bpp {
        cfg.bytes_per_pixel = b;
    }
    if let Some(r) = &args.rotation {
        cfg.rotation = match r.as_str() {
            "none" => config::Rotation::None,
            "cw90" => config::Rotation::Cw90,
            "cw180" => config::Rotation::Cw180,
            "cw270" => config::Rotation::Cw270,
            other => bail!("invalid --rotation '{other}' (use none|cw90|cw180|cw270)"),
        };
    }
    if args.no_size_fields {
        cfg.include_size_fields = false;
    }
    if let Some(h) = &args.init_hex {
        cfg.init_bytes = parse_hex(h)?;
    }
    if let Some(h) = &args.frame_header_hex {
        cfg.frame_header = parse_hex(h)?;
    }

    // Only sample sensors in live mode (v1 always means the GPU readout).
    let want_sensors = !args.test && !cfg.sensors.is_empty();

    let spec = cfg.spec();
    // Output (panel) size after applying the configured rotation to the canvas.
    let out_dims = if matches!(cfg.rotation, config::Rotation::Cw90 | config::Rotation::Cw270) {
        (spec.height, spec.width)
    } else {
        (spec.width, spec.height)
    };
    eprintln!(
        "turmo: transport={:?} device={} canvas={}x{} rotation={:?} output={}x{} \
         bpp={} order={:?} mode={} sensors={}",
        cfg.transport,
        cfg.device,
        spec.width,
        spec.height,
        cfg.rotation,
        out_dims.0,
        out_dims.1,
        spec.bytes_per_pixel,
        cfg.channel_order,
        if args.test { "test" } else { "live" },
        want_sensors
    );

    let mut sink: Box<dyn ByteSink> = match cfg.transport {
        TransportKind::Serial => Box::new(transport::serial::SerialSink::new(&cfg.device, cfg.baud)),
        #[cfg(feature = "bulk")]
        TransportKind::Bulk => Box::new(transport::bulk::BulkSink::new(
            cfg.usb_vid.unwrap_or(0x1a86),
            cfg.usb_pid.unwrap_or(0x5722),
            cfg.bulk_out_endpoint,
        )),
        #[cfg(not(feature = "bulk"))]
        TransportKind::Bulk => {
            bail!(
                "transport 'bulk' requires building with `--features bulk` and a system libusb"
            )
        }
    };

    sink.open().with_context(|| format!("open '{}'", cfg.device))?;
    if !cfg.init_bytes.is_empty() {
        eprintln!("sending {} init/handshake byte(s)", cfg.init_bytes.len());
        sink.init(&cfg.init_bytes)?;
    }

    let framing = Framing::new(
        cfg.frame_header.clone(),
        cfg.frame_footer.clone(),
        cfg.include_size_fields,
    );

    let mut composer = Composer::new(spec, cfg.font_scale);
    if !args.test {
        load_background(&cfg, &mut composer);
        // Static identity + thermal-alarm limits, detected once at startup (not per frame).
        let models = SystemModels::detect();
        eprintln!(
            "turmo: gpu_model='{}' cpu_model='{}'",
            models.gpu_model, models.cpu_model
        );
        composer.set_context(&models, cfg.gpu_temp_limit, cfg.cpu_temp_limit);
    }

    // One-shot home/reset probe: build a known offset, apply the candidate, send one full frame.
    if let Some(home) = &args.probe_home {
        eprintln!("turmo: probing home candidate '{home}'");
        run_probe(&mut sink, &composer, &framing, cfg.rotation, home, args.offset_bytes)?;
        sink.close();
        return Ok(());
    }

    // Stateful CPU usage meter held across frames (needs a previous sample for the delta).
    let mut cpu_meter = CpuUsageMeter::new();

    let refresh = Duration::from_millis(cfg.refresh_ms.max(1));
    let mut start = Instant::now();
    loop {
        let frame = if args.test {
            composer.test_pattern()
        } else {
            let gpu: GpuValues = if want_sensors {
                GpuValues::sample()
            } else {
                GpuValues::default()
            };
            // CPU sensors are sampled only in live mode; the meter yields None until it has a delta.
            let cpu_temp = if want_sensors { cpu::temp_c() } else { None };
            let cpu_usage = if want_sensors { cpu_meter.sample() } else { None };
            let ram = if want_sensors { mem::ram_pct() } else { None };
            composer.compose(&Readout::new(&gpu, cpu_temp, cpu_usage, ram))
        };

        // Rotate the composed canvas into the panel's native orientation before sending.
        let frame = frame.rotated(cfg.rotation);

        let bytes = framing.encode(&frame);
        sink.write_all(&bytes).context("send frame")?;

        if args.once {
            break;
        }
        // Sleep for the remainder of this refresh period (avoids drift + double delay on t=0).
        let elapsed = start.elapsed();
        if elapsed < refresh {
            std::thread::sleep(refresh - elapsed);
        }
        start = Instant::now();
    }

    sink.close();
    Ok(())
}

fn main() -> Result<()> {
    run(parse_args()?)
}
