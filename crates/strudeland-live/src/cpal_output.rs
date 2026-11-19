//! The cpal output: the system default device and its negotiated PCM layout.
//!
//! The `cpal::Stream` is `!Send` on macOS, so a thread owns it from build to drop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, ErrorKind, FromSample, SampleFormat, SizedSample, SupportedBufferSize};
use strudeland_record::clock::{StreamClock, host_now_ns};

use crate::output::{Output, OutputError, OutputInfo, OutputNotice, PlaybackTime, RenderCallback};
use crate::ring::RING_TARGET_FRAMES;

type Notice = Arc<Mutex<Option<OutputNotice>>>;

/// The default output device's stream.
pub struct CpalOutput {
    notice: Notice,
    clock_failed: Arc<AtomicBool>,
    running: Option<Running>,
}

struct Running {
    /// Dropping the sender ends the stream-owning thread.
    _release: Sender<()>,
    thread: JoinHandle<()>,
}

/// What a cpal error callback error is, as an output notice; `None` for an xrun.
pub(crate) fn notice_from_cpal(error: &cpal::Error) -> Option<OutputNotice> {
    match error.kind() {
        ErrorKind::Xrun | ErrorKind::RealtimeDenied => None,
        ErrorKind::DeviceNotAvailable | ErrorKind::StreamInvalidated | ErrorKind::DeviceChanged => {
            Some(OutputNotice::DeviceChanged)
        }
        _ => Some(OutputNotice::Failed(error.to_string())),
    }
}

fn stream_error(e: &cpal::Error) -> OutputError {
    match e.kind() {
        ErrorKind::DeviceNotAvailable => OutputError::NoDevice,
        _ => OutputError::Stream(e.to_string()),
    }
}

fn supported(format: SampleFormat, channels: u16) -> bool {
    channels > 0
        && matches!(
            format,
            SampleFormat::F32
                | SampleFormat::F64
                | SampleFormat::I16
                | SampleFormat::I24
                | SampleFormat::I32
                | SampleFormat::U16
        )
}

/// A whole device period must fit in the audio the engine keeps ready between callbacks.
fn check_period(frames: u32) -> Result<(), OutputError> {
    if frames as usize > RING_TARGET_FRAMES {
        return Err(OutputError::Unsupported(format!(
            "the device requires a {frames}-frame output period (maximum {RING_TARGET_FRAMES})"
        )));
    }
    Ok(())
}

fn requested_period(sizes: &SupportedBufferSize) -> Result<BufferSize, OutputError> {
    let SupportedBufferSize::Range { min, max } = *sizes else {
        return Ok(BufferSize::Default);
    };
    check_period(min)?;
    if max as usize > RING_TARGET_FRAMES {
        // A 512-frame period leaves several callbacks of render-ahead at common rates.
        Ok(BufferSize::Fixed(512_u32.max(min)))
    } else {
        Ok(BufferSize::Default)
    }
}

/// Prefer the advertised default without rewriting the channel layout; otherwise choose a
/// supported PCM range at the nearest rate to that default.
fn select_config(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig, OutputError> {
    let default = device
        .default_output_config()
        .map_err(|e| stream_error(&e))?;
    if supported(default.sample_format(), default.channels()) {
        return Ok(default);
    }
    let ranges = device
        .supported_output_configs()
        .map_err(|e| stream_error(&e))?;
    ranges
        .filter(|r| supported(r.sample_format(), r.channels()))
        .map(|r| {
            let rate = default
                .sample_rate()
                .clamp(r.min_sample_rate(), r.max_sample_rate());
            r.with_sample_rate(rate)
        })
        .min_by_key(|c| {
            (
                c.sample_rate().abs_diff(default.sample_rate()),
                c.channels().abs_diff(2),
            )
        })
        .ok_or_else(|| {
            OutputError::Unsupported(format!("no supported PCM configuration for {default:?}"))
        })
}

/// Write a stereo render buffer into the negotiated device layout. The first two channels
/// carry L/R, a mono device receives the average, and unused channels receive silence.
fn write_channels<T: SizedSample + FromSample<f32>>(
    out: &mut [T],
    stereo: &[f32],
    channels: usize,
) {
    for (device, frame) in out.chunks_exact_mut(channels).zip(stereo.chunks_exact(2)) {
        let convert = |x: f32| {
            T::from_sample_(if T::FORMAT.is_float() {
                x
            } else {
                x.clamp(-1.0, 1.0 - f32::EPSILON / 2.0)
            })
        };
        if channels == 1 {
            device[0] = convert(frame[0] * 0.5 + frame[1] * 0.5);
        } else {
            device[0] = convert(frame[0]);
            device[1] = convert(frame[1]);
            device[2..].fill(T::EQUILIBRIUM);
        }
    }
}

fn build_pcm<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut render: RenderCallback,
    clock: &StreamClock,
    clock_failed: Arc<AtomicBool>,
    error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, cpal::Error> {
    let channels = usize::from(config.channels);
    let rate = u64::from(config.sample_rate);
    let mut reader = clock.reader();
    let mut invalid_since = None;
    let mut seen_playback = false;
    let mut failed = false;
    // CPAL does not guarantee an upper bound on callback size.
    const CHUNK: usize = 2048;
    let mut stereo = vec![0.0; 2 * CHUNK];
    device.build_output_stream::<T, _, _>(
        config,
        move |out, info| {
            if failed {
                out.fill(T::EQUILIBRIUM);
                return;
            }
            let ts = info.timestamp();
            let Some(start) = reader.event(ts.callback, ts.playback, true) else {
                out.fill(T::EQUILIBRIUM);
                let now = host_now_ns();
                // A running engine must reopen and reanchor rather than resuming the
                // old ring after silent callbacks changed its relationship to device time.
                if seen_playback
                    || now.saturating_sub(*invalid_since.get_or_insert(now)) >= 2_000_000_000
                {
                    failed = true;
                    clock_failed.store(true, Ordering::Release);
                }
                return;
            };
            seen_playback = true;
            for (i, chunk) in out.chunks_mut(CHUNK * channels).enumerate() {
                let frames = chunk.len() / channels;
                let buf = &mut stereo[..frames * 2];
                let delta_ns = (i as u64 * CHUNK as u64) * 1_000_000_000 / rate;
                render(
                    buf,
                    PlaybackTime {
                        host_ns: start.saturating_add(delta_ns),
                    },
                );
                write_channels(chunk, buf, channels);
            }
        },
        error,
        None,
    )
}

impl CpalOutput {
    /// The system default output device, checked for a supported PCM layout.
    pub fn default_output() -> Result<CpalOutput, OutputError> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(OutputError::NoDevice)?;
        select_config(&device)?;
        Ok(CpalOutput {
            notice: Arc::new(Mutex::new(None)),
            running: None,
            clock_failed: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl Output for CpalOutput {
    fn start(&mut self, render: RenderCallback) -> Result<OutputInfo, OutputError> {
        self.stop();
        *self.notice.lock().expect("notice") = None;
        let notice = self.notice.clone();
        self.clock_failed.store(false, Ordering::Release);
        let clock_failed = self.clock_failed.clone();
        let (info_tx, info_rx) = channel::<Result<OutputInfo, OutputError>>();
        let (release_tx, release_rx) = channel::<()>();
        let thread = std::thread::Builder::new()
            .name("strudeland-live-cpal".into())
            .spawn(move || {
                // The default device is looked up on every start, so a reroute is followed.
                let built = (|| -> Result<(cpal::Stream, OutputInfo), OutputError> {
                    let device = cpal::default_host()
                        .default_output_device()
                        .ok_or(OutputError::NoDevice)?;
                    let supported = select_config(&device)?;
                    let mut config = supported.config();
                    if !cfg!(target_vendor = "apple") {
                        config.buffer_size = requested_period(supported.buffer_size())?;
                    }
                    let sample_rate = config.sample_rate;
                    let clock = StreamClock::default();
                    let error_notice = notice.clone();
                    let error = move |e: cpal::Error| {
                        if let Some(n) = notice_from_cpal(&e)
                            && let Ok(mut slot) = error_notice.lock()
                        {
                            *slot = Some(n);
                        }
                    };
                    let stream = match supported.sample_format() {
                        SampleFormat::F32 => {
                            build_pcm::<f32>(&device, config, render, &clock, clock_failed, error)
                        }
                        SampleFormat::F64 => {
                            build_pcm::<f64>(&device, config, render, &clock, clock_failed, error)
                        }
                        SampleFormat::I16 => {
                            build_pcm::<i16>(&device, config, render, &clock, clock_failed, error)
                        }
                        SampleFormat::I24 => build_pcm::<cpal::I24>(
                            &device,
                            config,
                            render,
                            &clock,
                            clock_failed,
                            error,
                        ),
                        SampleFormat::I32 => {
                            build_pcm::<i32>(&device, config, render, &clock, clock_failed, error)
                        }
                        SampleFormat::U16 => {
                            build_pcm::<u16>(&device, config, render, &clock, clock_failed, error)
                        }
                        other => {
                            return Err(OutputError::Unsupported(format!(
                                "unsupported sample format {other}"
                            )));
                        }
                    }
                    .map_err(|e| stream_error(&e))?;
                    let buffer_frames = stream.buffer_size().ok();
                    if let Some(frames) = buffer_frames {
                        check_period(frames)?;
                    }
                    stream.play().map_err(|e| stream_error(&e))?;
                    clock.calibrate(&stream);
                    Ok((
                        stream,
                        OutputInfo {
                            sample_rate,
                            buffer_frames,
                        },
                    ))
                })();
                match built {
                    Ok((stream, info)) => {
                        let _ = info_tx.send(Ok(info));
                        // Hold the stream until the output is stopped or dropped.
                        let _ = release_rx.recv();
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = info_tx.send(Err(e));
                    }
                }
            })
            .map_err(|e| OutputError::Stream(e.to_string()))?;
        match info_rx.recv() {
            Ok(Ok(info)) => {
                self.running = Some(Running {
                    _release: release_tx,
                    thread,
                });
                Ok(info)
            }
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(OutputError::Stream("output thread ended".into()))
            }
        }
    }

    fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            let Running { _release, thread } = running;
            drop(_release);
            let _ = thread.join();
        }
    }

    fn poll_notice(&mut self) -> Option<OutputNotice> {
        if self.clock_failed.swap(false, Ordering::AcqRel) {
            return Some(OutputNotice::Failed(
                "the output device stopped reporting valid playback times".into(),
            ));
        }
        self.notice.lock().ok()?.take()
    }
}

impl Drop for CpalOutput {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiates_periods_that_fit_the_live_ring_fill() {
        let range = |min, max| SupportedBufferSize::Range { min, max };
        assert_eq!(requested_period(&range(48, 512)), Ok(BufferSize::Default));
        assert_eq!(
            requested_period(&range(48, 4096)),
            Ok(BufferSize::Fixed(512))
        );
        assert_eq!(
            requested_period(&range(1024, 4096)),
            Ok(BufferSize::Fixed(1024))
        );
        assert!(requested_period(&range(4096, 8192)).is_err());
        assert!(check_period(4096).is_err());
        assert_eq!(
            requested_period(&SupportedBufferSize::Unknown),
            Ok(BufferSize::Default)
        );
    }

    #[test]
    fn device_layout_and_integer_conversion() {
        assert!(supported(SampleFormat::I24, 1));
        assert!(!supported(SampleFormat::F32, 0));
        let mut mono = [0i16; 2];
        write_channels(&mut mono, &[1.0, -1.0, 2.0, 2.0], 1);
        assert_eq!(mono, [0, i16::MAX]);
        let mut surround = [1.0; 6];
        write_channels(&mut surround, &[1.5, -0.5], 6);
        assert_eq!(surround, [1.5, -0.5, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn cpal_errors_map_to_notices() {
        let notice = |k| notice_from_cpal(&cpal::Error::new(k));
        assert_eq!(notice(ErrorKind::Xrun), None);
        for k in [
            ErrorKind::DeviceChanged,
            ErrorKind::DeviceNotAvailable,
            ErrorKind::StreamInvalidated,
        ] {
            assert_eq!(notice(k), Some(OutputNotice::DeviceChanged));
        }
        assert!(matches!(
            notice(ErrorKind::BackendError),
            Some(OutputNotice::Failed(_))
        ));
    }
}
