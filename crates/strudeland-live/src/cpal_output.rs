//! The cpal output: the system default device, stereo `f32` at its default rate.
//!
//! The `cpal::Stream` is `!Send` on macOS, so a thread owns it from build to drop.

use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, SampleFormat};

use crate::output::{Output, OutputError, OutputInfo, OutputNotice, PlaybackTime, RenderCallback};

type Notice = Arc<Mutex<Option<OutputNotice>>>;

/// The default output device's stream.
pub struct CpalOutput {
    notice: Notice,
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
        ErrorKind::Xrun => None,
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

/// Checks a default config can carry stereo `f32`.
pub(crate) fn check_format(format: SampleFormat, channels: u16) -> Result<(), OutputError> {
    if format != SampleFormat::F32 {
        return Err(OutputError::Unsupported(format!(
            "sample format {format} is not f32"
        )));
    }
    if channels < 2 {
        return Err(OutputError::Unsupported(format!(
            "{channels} output channel(s), need at least 2"
        )));
    }
    Ok(())
}

impl CpalOutput {
    /// The system default output device, checked for stereo `f32`.
    pub fn default_output() -> Result<CpalOutput, OutputError> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(OutputError::NoDevice)?;
        let config = device
            .default_output_config()
            .map_err(|e| stream_error(&e))?;
        check_format(config.sample_format(), config.channels())?;
        Ok(CpalOutput {
            notice: Arc::new(Mutex::new(None)),
            running: None,
        })
    }
}

impl Output for CpalOutput {
    fn start(&mut self, mut render: RenderCallback) -> Result<OutputInfo, OutputError> {
        self.stop();
        *self.notice.lock().expect("notice") = None;
        let notice = self.notice.clone();
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
                    let supported = device
                        .default_output_config()
                        .map_err(|e| stream_error(&e))?;
                    check_format(supported.sample_format(), supported.channels())?;
                    let mut config = supported.config();
                    config.channels = 2;
                    let sample_rate = config.sample_rate;
                    let data = move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                        let ns = info
                            .timestamp()
                            .playback
                            .as_nanos()
                            .min(u128::from(u64::MAX)) as u64;
                        render(data, PlaybackTime { host_ns: ns });
                    };
                    let error_notice = notice.clone();
                    let error = move |e: cpal::Error| {
                        if let Some(n) = notice_from_cpal(&e)
                            && let Ok(mut slot) = error_notice.lock()
                        {
                            *slot = Some(n);
                        }
                    };
                    let stream = device
                        .build_output_stream::<f32, _, _>(config, data, error, None)
                        .map_err(|e| stream_error(&e))?;
                    stream.play().map_err(|e| stream_error(&e))?;
                    Ok((
                        stream,
                        OutputInfo {
                            sample_rate,
                            buffer_frames: None,
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
    fn only_stereo_f32_is_accepted() {
        assert!(check_format(SampleFormat::F32, 2).is_ok());
        assert!(check_format(SampleFormat::F32, 8).is_ok());
        assert!(matches!(
            check_format(SampleFormat::I16, 2),
            Err(OutputError::Unsupported(_))
        ));
        assert!(matches!(
            check_format(SampleFormat::F32, 1),
            Err(OutputError::Unsupported(_))
        ));
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
