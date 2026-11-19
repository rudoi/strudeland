//! The capture-timestamp probe: prints cpal's raw input timestamps next to `host_now_ns()` so
//! the `CaptureTime` candidates can be compared on real hardware. Run by hand only.
//!
//! ```text
//! cargo run -p strudeland-record --example probe -- [--list] [--device ID] [--seconds N]
//! ```
//!
//! Exit codes: 0 ok, 1 microphone permission denied (or the stream failed), 2 bad arguments.

use std::process::ExitCode;
use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, InputCallbackInfo, SampleFormat, SizedSample};
use strudeland_record::clock::{ClockReader, StreamClock};
use strudeland_record::{RecordError, find_input_device, host_now_ns, list_input_devices};

const USAGE: &str = "usage: probe [--list] [--device ID] [--seconds N]";

/// One callback, as recorded on the audio thread.
#[derive(Debug, Clone, Copy, Default)]
struct Row {
    frames: u32,
    now_ns: u64,
    callback_ns: u64,
    capture_ns: u64,
    mapped_ns: Option<u64>,
    peak: f32,
}

struct Args {
    list: bool,
    device: Option<String>,
    seconds: f64,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        list: false,
        device: None,
        seconds: 5.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--list" => args.list = true,
            "--device" => args.device = Some(it.next().ok_or("--device needs an id")?),
            "--seconds" => {
                let value = it.next().ok_or("--seconds needs a number")?;
                args.seconds = value
                    .parse::<f64>()
                    .ok()
                    .filter(|s| s.is_finite() && *s > 0.0 && *s <= 3_600.0)
                    .ok_or_else(|| format!("bad --seconds: {value}"))?;
            }
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.list {
        return list();
    }
    match probe(args.device.as_deref(), args.seconds) {
        Ok(()) => ExitCode::SUCCESS,
        Err(RecordError::NoDevice) => {
            eprintln!("no such input device");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("probe failed: {e}");
            ExitCode::from(1)
        }
    }
}

fn list() -> ExitCode {
    match list_input_devices() {
        Ok(devices) => {
            for d in devices {
                let default = if d.is_default { "default" } else { "" };
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    d.id, d.name, d.channels, d.sample_rate, default
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("listing failed: {e}");
            ExitCode::from(1)
        }
    }
}

fn probe(device_id: Option<&str>, seconds: f64) -> Result<(), RecordError> {
    let device = find_input_device(device_id)?;
    let config = device
        .default_input_config()
        .map_err(|e| RecordError::from_cpal(&e))?;
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_default();
    let id = device.id().map(|i| i.to_string()).unwrap_or_default();
    let rate = config.sample_rate();
    let channels = config.channels();
    eprintln!(
        "device {id} ({name}): {channels} ch, {rate} Hz, {}",
        config.sample_format()
    );
    // Room for every callback of the run at 32-frame blocks, so the callback never allocates.
    let capacity = (seconds * f64::from(rate) / 32.0).ceil() as usize + 64;
    let (tx, mut rx) = rtrb::RingBuffer::<Row>::new(capacity);
    let (err_tx, err_rx) = std::sync::mpsc::channel::<cpal::Error>();
    let error = move |e: cpal::Error| {
        let _ = err_tx.send(e);
    };
    let clock = StreamClock::default();
    let reader = clock.reader();
    let stream = match config.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, reader, tx, error),
        SampleFormat::I16 => build::<i16>(&device, &config, reader, tx, error),
        SampleFormat::I24 => build::<cpal::I24>(&device, &config, reader, tx, error),
        SampleFormat::U16 => build::<u16>(&device, &config, reader, tx, error),
        SampleFormat::I32 => build::<i32>(&device, &config, reader, tx, error),
        other => {
            return Err(RecordError::UnsupportedFormat(format!(
                "sample format {other}"
            )));
        }
    }?;
    stream.play().map_err(|e| RecordError::from_cpal(&e))?;
    clock.calibrate(&stream);
    std::thread::sleep(Duration::from_secs_f64(seconds));
    drop(stream);

    let mut xruns = 0;
    for e in err_rx.try_iter() {
        if e.kind() == cpal::ErrorKind::Xrun {
            xruns += 1;
        } else {
            return Err(RecordError::from_cpal(&e));
        }
    }
    let mut rows = Vec::new();
    while let Ok(row) = rx.pop() {
        rows.push(row);
    }
    report(&rows, rate, xruns);
    Ok(())
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    mut reader: ClockReader,
    mut tx: rtrb::Producer<Row>,
    error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, RecordError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels().max(1));
    let data = move |data: &[T], info: &InputCallbackInfo| {
        let now_ns = host_now_ns();
        let ts = info.timestamp();
        let mut peak = 0.0f32;
        for frame in data.chunks(channels) {
            peak = peak.max(f32::from_sample_(frame[0]).abs());
        }
        let _ = tx.push(Row {
            frames: (data.len() / channels) as u32,
            now_ns,
            callback_ns: ts.callback.as_nanos() as u64,
            capture_ns: ts.capture.as_nanos() as u64,
            mapped_ns: reader.event(ts.callback, ts.capture, false),
            peak,
        });
    };
    device
        .build_input_stream::<T, _, _>(config.config(), data, error, None)
        .map_err(|e| RecordError::from_cpal(&e))
}

fn ms(ns: i128) -> f64 {
    ns as f64 / 1e6
}

fn report(rows: &[Row], rate: u32, xruns: usize) {
    println!(
        "n\tframes\tnow_ns\tcallback_ns\tcapture_ns\tnow_minus_callback_ms\tcallback_minus_capture_ms\tbuffer_ms\textra_ms\tmapped_capture_ns"
    );
    let mut cols: [Vec<f64>; 4] = Default::default();
    for (n, r) in rows.iter().enumerate() {
        let now_cb = ms(i128::from(r.now_ns) - i128::from(r.callback_ns));
        let cb_cap = ms(i128::from(r.callback_ns) - i128::from(r.capture_ns));
        let buffer = f64::from(r.frames) * 1e3 / f64::from(rate);
        let extra = cb_cap - buffer;
        let mapped = r
            .mapped_ns
            .map_or_else(|| "unknown".into(), |t| t.to_string());
        println!(
            "{n}\t{}\t{}\t{}\t{}\t{now_cb:.3}\t{cb_cap:.3}\t{buffer:.3}\t{extra:.3}\t{mapped}",
            r.frames, r.now_ns, r.callback_ns, r.capture_ns
        );
        for (col, v) in cols.iter_mut().zip([now_cb, cb_cap, buffer, extra]) {
            col.push(v);
        }
    }
    println!();
    println!("# summary");
    println!("callbacks\t{}", rows.len());
    println!("xruns\t{xruns}");
    println!("sample_rate\t{rate}");
    let mut frames: Vec<u32> = rows.iter().map(|r| r.frames).collect();
    frames.sort_unstable();
    println!(
        "buffer_frames\tmedian {}\tmin {}\tmax {}",
        frames.get(frames.len() / 2).copied().unwrap_or(0),
        frames.first().copied().unwrap_or(0),
        frames.last().copied().unwrap_or(0)
    );
    let peak = rows.iter().map(|r| r.peak).fold(0.0f32, f32::max);
    println!("peak_abs_sample_ch0\t{peak}");
    if peak == 0.0 {
        println!("# every sample was zero: microphone access may be denied");
    }
    let names = [
        "now_minus_callback_ms",
        "callback_minus_capture_ms",
        "buffer_ms",
        "extra_ms",
    ];
    let mut medians = [0.0; 4];
    for (i, (name, col)) in names.iter().zip(cols.iter_mut()).enumerate() {
        col.sort_by(f64::total_cmp);
        let median = col.get(col.len() / 2).copied().unwrap_or(f64::NAN);
        let max = col.last().copied().unwrap_or(f64::NAN);
        medians[i] = median;
        println!("{name}\tmedian {median:.3}\tmax {max:.3}");
    }
    println!("# candidate shifts relative to capture_ns (median)");
    println!("Capture\t+0.000 ms");
    println!("Callback\t+{:.3} ms", medians[1]);
    println!("CallbackMinusExtra\t+{:.3} ms", medians[2]);
}
