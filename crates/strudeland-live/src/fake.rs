//! A fake [`Output`] driven by a pull thread with a virtual clock, for tests without hardware.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::output::{Output, OutputError, OutputInfo, OutputNotice, PlaybackTime, RenderCallback};

/// The fake clock's value before the first pull.
pub const FAKE_EPOCH_NS: u64 = 1_000_000_000;

/// `pulled` keeps at most this many samples (the first ones; 190 s of stereo at 44.1 kHz), so
/// `Pace::AsFastAsPossible` cannot exhaust memory.
pub const PULLED_CAP_SAMPLES: usize = 1 << 24;

/// Default frames per pull.
const DEFAULT_BUFFER_FRAMES: u32 = 512;

/// How the fake pulls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    /// One block every `buffer_frames / sample_rate` of wall time (the clock stays virtual).
    RealTime,
    /// A tight loop: drains the ring faster than the engine fills it.
    AsFastAsPossible,
}

struct State {
    info: Mutex<OutputInfo>,
    notice: Mutex<Option<OutputNotice>>,
    pulled: Mutex<Vec<f32>>,
    pulls: AtomicU64,
    clock_ns: AtomicU64,
    fail_start: AtomicBool,
    stop: AtomicBool,
}

/// The [`Output`] half.
pub struct FakeOutput {
    state: Arc<State>,
    pace: Pace,
    thread: Option<JoinHandle<()>>,
}

/// The test's half.
#[derive(Clone)]
pub struct FakeHandle {
    state: Arc<State>,
}

impl FakeOutput {
    pub fn new(info: OutputInfo, pace: Pace) -> (FakeOutput, FakeHandle) {
        let state = Arc::new(State {
            info: Mutex::new(info),
            notice: Mutex::new(None),
            pulled: Mutex::new(Vec::new()),
            pulls: AtomicU64::new(0),
            clock_ns: AtomicU64::new(FAKE_EPOCH_NS),
            fail_start: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        (
            FakeOutput {
                state: state.clone(),
                pace,
                thread: None,
            },
            FakeHandle { state },
        )
    }
}

impl Output for FakeOutput {
    fn start(&mut self, mut render: RenderCallback) -> Result<OutputInfo, OutputError> {
        self.stop();
        if self.state.fail_start.load(Ordering::SeqCst) {
            return Err(OutputError::Stream("fake start failure".into()));
        }
        let info = *self.state.info.lock().expect("fake info");
        let state = self.state.clone();
        state.stop.store(false, Ordering::SeqCst);
        let pace = self.pace;
        let block_frames = info.buffer_frames.unwrap_or(DEFAULT_BUFFER_FRAMES).max(1) as usize;
        let rate = u64::from(info.sample_rate.max(1));
        self.thread = Some(
            std::thread::Builder::new()
                .name("strudeland-fake-output".into())
                .spawn(move || {
                    // The device's I/O thread runs under the time-constraint policy; so does
                    // its stand-in: a period of one block, a quarter of it to compute.
                    crate::rt::promote(crate::rt::TimeConstraint::fraction_of(
                        Duration::from_nanos(block_frames as u64 * 1_000_000_000 / rate),
                        1,
                        4,
                    ));
                    let base_ns = state.clock_ns.load(Ordering::SeqCst);
                    let started = Instant::now();
                    let mut frames: u64 = 0;
                    let mut buf = vec![0.0f32; block_frames * 2];
                    while !state.stop.load(Ordering::SeqCst) {
                        let host_ns = base_ns + frames * 1_000_000_000 / rate;
                        render(&mut buf, PlaybackTime { host_ns });
                        {
                            let mut pulled = state.pulled.lock().expect("fake pulled");
                            let room = PULLED_CAP_SAMPLES.saturating_sub(pulled.len());
                            pulled.extend_from_slice(&buf[..room.min(buf.len())]);
                        }
                        frames += block_frames as u64;
                        state
                            .clock_ns
                            .store(base_ns + frames * 1_000_000_000 / rate, Ordering::SeqCst);
                        state.pulls.fetch_add(1, Ordering::SeqCst);
                        if pace == Pace::RealTime {
                            let due = started + Duration::from_nanos(frames * 1_000_000_000 / rate);
                            let now = Instant::now();
                            if due > now {
                                std::thread::sleep(due - now);
                            }
                        } else {
                            std::thread::yield_now();
                        }
                    }
                })
                .expect("spawn fake output"),
        );
        Ok(info)
    }

    fn stop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    fn poll_notice(&mut self) -> Option<OutputNotice> {
        self.state.notice.lock().expect("fake notice").take()
    }
}

impl Drop for FakeOutput {
    fn drop(&mut self) {
        self.stop();
    }
}

impl FakeHandle {
    /// Every sample pulled so far (interleaved stereo), up to [`PULLED_CAP_SAMPLES`].
    pub fn pulled(&self) -> Vec<f32> {
        self.state.pulled.lock().expect("fake pulled").clone()
    }

    /// The device changes: the next `start` runs at `info`, and the output raises
    /// [`OutputNotice::DeviceChanged`].
    pub fn change_device(&self, info: OutputInfo) {
        *self.state.info.lock().expect("fake info") = info;
        *self.state.notice.lock().expect("fake notice") = Some(OutputNotice::DeviceChanged);
    }

    /// The virtual host clock: `FAKE_EPOCH_NS` plus the time of the frames pulled.
    pub fn host_now_ns(&self) -> u64 {
        self.state.clock_ns.load(Ordering::SeqCst)
    }

    /// Callbacks made so far.
    pub fn pulls(&self) -> u64 {
        self.state.pulls.load(Ordering::SeqCst)
    }

    /// Makes `start` fail (or work again).
    pub fn fail_starts(&self, fail: bool) {
        self.state.fail_start.store(fail, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn info(rate: u32) -> OutputInfo {
        OutputInfo {
            sample_rate: rate,
            buffer_frames: None,
        }
    }

    fn wait_for(mut f: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < end, "timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn pulls_blocks_with_a_virtual_clock() {
        let (mut out, handle) = FakeOutput::new(info(48000), Pace::AsFastAsPossible);
        let (tx, rx) = mpsc::channel();
        let got = out
            .start(Box::new(move |buf, t| {
                assert_eq!(buf.len(), 1024);
                let _ = tx.send(t.host_ns);
                buf.fill(0.25);
            }))
            .unwrap();
        assert_eq!(got, info(48000));
        wait_for(|| handle.pulls() >= 3);
        out.stop();
        let times: Vec<u64> = rx.try_iter().take(3).collect();
        assert_eq!(times[0], FAKE_EPOCH_NS);
        assert_eq!(times[1], FAKE_EPOCH_NS + 512 * 1_000_000_000 / 48000);
        assert_eq!(times[2], FAKE_EPOCH_NS + 1024 * 1_000_000_000 / 48000);
        let pulled = handle.pulled();
        let expected = (handle.pulls() * 1024).min(PULLED_CAP_SAMPLES as u64);
        assert_eq!(pulled.len() as u64, expected);
        assert!(pulled.iter().all(|&s| s == 0.25));
        assert_eq!(
            handle.host_now_ns(),
            FAKE_EPOCH_NS + handle.pulls() * 512 * 1_000_000_000 / 48000
        );
    }

    #[test]
    fn real_time_pacing_follows_wall_time() {
        let (mut out, handle) = FakeOutput::new(info(44100), Pace::RealTime);
        out.start(Box::new(|_, _| {})).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        out.stop();
        // 512 / 44100 s per pull, so about 25 pulls in 300 ms
        let pulls = handle.pulls();
        assert!((15..=35).contains(&pulls), "pulls {pulls}");
    }

    #[test]
    fn device_change_raises_a_notice_and_restarts_at_the_new_rate() {
        let (mut out, handle) = FakeOutput::new(info(44100), Pace::AsFastAsPossible);
        out.start(Box::new(|_, _| {})).unwrap();
        assert_eq!(out.poll_notice(), None);
        handle.change_device(info(96000));
        assert_eq!(out.poll_notice(), Some(OutputNotice::DeviceChanged));
        assert_eq!(out.poll_notice(), None);
        out.stop();
        let before = handle.host_now_ns();
        let again = out.start(Box::new(|_, _| {})).unwrap();
        assert_eq!(again.sample_rate, 96000);
        wait_for(|| handle.host_now_ns() > before);
        out.stop();
        assert!(handle.host_now_ns() >= before);
    }

    #[test]
    fn start_can_be_made_to_fail() {
        let (mut out, handle) = FakeOutput::new(info(44100), Pace::RealTime);
        handle.fail_starts(true);
        assert!(matches!(
            out.start(Box::new(|_, _| {})),
            Err(OutputError::Stream(_))
        ));
        handle.fail_starts(false);
        assert!(out.start(Box::new(|_, _| {})).is_ok());
    }

    #[test]
    fn pulled_is_capped() {
        let (mut out, handle) = FakeOutput::new(info(44100), Pace::AsFastAsPossible);
        out.start(Box::new(|buf, _| buf.fill(0.5))).unwrap();
        wait_for(|| handle.pulls() as usize * 1024 > PULLED_CAP_SAMPLES + 4096);
        out.stop();
        assert_eq!(handle.pulled().len(), PULLED_CAP_SAMPLES);
    }

    #[test]
    fn stop_is_idempotent() {
        let (mut out, _h) = FakeOutput::new(info(44100), Pace::RealTime);
        out.stop();
        out.start(Box::new(|_, _| {})).unwrap();
        out.stop();
        out.stop();
    }
}
