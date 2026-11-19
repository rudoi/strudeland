//! Shared native host time and conversion of CPAL device stamps.

/// The mach host time in nanoseconds, shared with CPAL's CoreAudio timestamps.
#[cfg(target_vendor = "apple")]
pub fn host_now_ns() -> u64 {
    use std::sync::OnceLock;
    static TIMEBASE: OnceLock<(u32, u32)> = OnceLock::new();
    let (numer, denom) = *TIMEBASE.get_or_init(|| {
        let mut info = mach2::mach_time::mach_timebase_info::default();
        // SAFETY: `info` is a valid, writable `mach_timebase_info` for the call's duration.
        let status = unsafe { mach2::mach_time::mach_timebase_info(&mut info) };
        if status == 0 && info.denom != 0 {
            (info.numer, info.denom)
        } else {
            (1, 1)
        }
    });
    // SAFETY: `mach_absolute_time` takes no arguments and has no preconditions.
    let ticks = unsafe { mach2::mach_time::mach_absolute_time() };
    (u128::from(ticks) * u128::from(numer) / u128::from(denom)) as u64
}

/// Match the clock CPAL requests for ALSA hardware timestamps so NTP slew does not
/// move the process clock away from long-running PCM streams.
#[cfg(target_os = "linux")]
pub fn host_now_ns() -> u64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: the pointer references one writable timespec. Linux supports MONOTONIC_RAW.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut time) } == 0 {
        (time.tv_sec as u64).saturating_mul(1_000_000_000) + time.tv_nsec as u64
    } else {
        0
    }
}

/// Nanoseconds since a process-wide origin (no mach clock off Apple platforms).
#[cfg(not(any(target_vendor = "apple", target_os = "linux", windows)))]
pub fn host_now_ns() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

/// WASAPI reports both input and output instants in QPC's epoch, on a 100 ns grid.
#[cfg(windows)]
pub fn host_now_ns() -> u64 {
    use std::sync::OnceLock;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryPerformanceCounter(value: *mut i64) -> i32;
        fn QueryPerformanceFrequency(value: *mut i64) -> i32;
    }
    static FREQUENCY: OnceLock<i64> = OnceLock::new();
    let frequency = *FREQUENCY.get_or_init(|| {
        let mut frequency = 0;
        // SAFETY: a writable LARGE_INTEGER. QPC is available on all supported Windows systems.
        unsafe { QueryPerformanceFrequency(&mut frequency) };
        frequency
    });
    let mut counter = 0;
    // SAFETY: a writable LARGE_INTEGER as above.
    unsafe { QueryPerformanceCounter(&mut counter) };
    if frequency <= 0 || counter < 0 {
        return 0;
    }
    ((counter as u128 * 10_000_000 / frequency as u128) * 100) as u64
}

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

/// Offset measured by the owner of one stream; a new stream always gets a new mapping.
#[derive(Clone)]
pub struct StreamClock(Arc<AtomicI64>);

const MEASURING: i64 = i64::MIN;
const UNMEASURED: i64 = i64::MIN + 1;
const MS: u64 = 1_000_000;

impl Default for StreamClock {
    fn default() -> Self {
        // Warm the host clock outside the real-time callback.
        host_now_ns();
        let offset = if cfg!(any(target_vendor = "apple", windows)) {
            0
        } else {
            MEASURING
        };
        Self(Arc::new(AtomicI64::new(offset)))
    }
}

/// Offset intervals from consecutive observed timestamp changes. An ALSA DMA tick
/// sampled at `after` happened since the previous poll began. Intersect these
/// intervals so a brief query of an old stamp cannot masquerade as an accurate read.
#[derive(Default)]
struct Calibration {
    previous: Option<(u64, u64)>,
    range: Option<(i128, i128)>,
    changes: u8,
}

impl Calibration {
    fn observe(&mut self, before: u64, stamp: u64, after: u64) -> Option<i64> {
        let (previous_poll, old_stamp) = self.previous.replace((before, stamp))?;
        if stamp < old_stamp || after.saturating_sub(previous_poll) > 5 * MS {
            self.range = None;
            self.changes = 0;
            return None;
        }
        if old_stamp == 0 || stamp == old_stamp {
            return None;
        }
        let interval = (
            i128::from(previous_poll) - i128::from(stamp),
            i128::from(after) - i128::from(stamp),
        );
        let (low, high) = self.range.map_or(interval, |(lo, hi)| {
            (lo.max(interval.0), hi.min(interval.1))
        });
        if low > high {
            self.range = Some(interval);
            self.changes = 1;
            return None;
        }
        self.range = Some((low, high));
        self.changes += 1;
        (self.changes >= 4)
            .then(|| i64::try_from(low + (high - low) / 2).ok())
            .flatten()
    }
}

impl StreamClock {
    /// Called on the stream owner after play. ALSA can expose elapsed time at the most
    /// recent DMA interrupt. Bracket *changes* in the stream clock over several periods;
    /// the host polls bound the age of each newly observed tick.
    pub fn calibrate(&self, stream: &cpal::Stream) {
        if cfg!(any(target_vendor = "apple", windows)) {
            return;
        }
        use cpal::traits::StreamTrait;
        let begin = std::time::Instant::now();
        let mut calibration = Calibration::default();
        while begin.elapsed() < std::time::Duration::from_millis(250) {
            let before = host_now_ns();
            let instant = stream.now().as_nanos();
            let after = host_now_ns();
            if before != 0
                && let Ok(stamp) = u64::try_from(instant)
                && let Some(offset) = calibration.observe(before, stamp, after)
            {
                self.0.store(offset, Ordering::Release);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        self.0.store(UNMEASURED, Ordering::Release);
    }

    pub fn reader(&self) -> ClockReader {
        ClockReader {
            clock: self.clone(),
            last_event: None,
            previous_callback: None,
            fallback: None,
            samples: 0,
        }
    }
}

/// Callback-local continuity and a fallback offset derived from successive device callbacks.
/// Only the owner samples Stream::now(). The audio thread reads one atomic and its host clock.
pub struct ClockReader {
    clock: StreamClock,
    last_event: Option<u64>,
    previous_callback: Option<(u64, u64)>,
    fallback: Option<i128>,
    samples: u8,
}

impl ClockReader {
    /// Validate the first-frame device event and preserve its signed displacement from
    /// the callback stamp (in particular, playout can legitimately be in the future).
    pub fn event(
        &mut self,
        callback: cpal::StreamInstant,
        event: cpal::StreamInstant,
        output: bool,
    ) -> Option<u64> {
        let event = u64::try_from(event.as_nanos()).ok()?;
        if cfg!(target_vendor = "apple") {
            // CoreAudio already delivers the event in mach host time. In particular its
            // output callback stamp may describe a future quantum.
            return (event != 0).then_some(event);
        }
        self.map(
            u64::try_from(callback.as_nanos()).ok()?,
            event,
            host_now_ns(),
            output,
        )
    }

    fn map(&mut self, callback: u64, event: u64, host: u64, output: bool) -> Option<u64> {
        if callback == 0
            || event == 0
            || (output && event < callback)
            || (!output && event > callback)
            || event.abs_diff(callback) > 2_000 * MS
        {
            return None;
        }
        let measured = self.clock.0.load(Ordering::Acquire);
        if measured == MEASURING {
            return None;
        }
        let previous = self.previous_callback.replace((callback, host));
        if let Some((old_callback, old_host)) = previous
            && (callback < old_callback
                || callback
                    .saturating_sub(old_callback)
                    .abs_diff(host.saturating_sub(old_host))
                    > 250 * MS)
        {
            self.fallback = None;
            self.samples = 0;
            self.last_event = None;
            return None;
        }
        let observed = i128::from(host) - i128::from(callback);
        let offset = if measured != UNMEASURED {
            i128::from(measured)
        } else {
            // Only distinct, advancing callbacks qualify. Pool their observed offsets so
            // scheduling delays in a single callback do not shift its first-frame time.
            if previous.is_none_or(|(old, _)| callback > old) {
                self.samples = self.samples.saturating_add(1);
                self.fallback = Some(self.fallback.map_or(observed, |old| old.min(observed)));
            }
            if self.samples < 4 {
                return None;
            }
            self.fallback?
        };
        // Reject an origin that no longer tracks the physical stream. Allow driver
        // scheduling latency and a few ms of measurement uncertainty.
        let age = observed - offset;
        if !(-5 * i128::from(MS)..=250 * i128::from(MS)).contains(&age) {
            return None;
        }
        let result = u64::try_from(i128::from(event) + offset).ok()?;
        if self.last_event.is_some_and(|last| result <= last) {
            return None;
        }
        self.last_event = Some(result);
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reader(offset: i64) -> ClockReader {
        StreamClock(Arc::new(AtomicI64::new(offset))).reader()
    }

    #[test]
    fn calibration_needs_consistent_advancing_ticks_and_accounts_for_the_previous_poll() {
        let mut c = Calibration::default();
        for host in 100..=108 {
            assert_eq!(c.observe(host * MS, 2 * MS, host * MS + 100), None);
        }
        // Stream epoch is 100 ms. Each tick may have fired up to one poll ago.
        for host in 109..=111 {
            assert_eq!(
                c.observe(host * MS, (host - 100) * MS, host * MS + 100),
                None
            );
        }
        let offset = c.observe(112 * MS, 12 * MS, 112 * MS + 100).unwrap();
        assert!((99_000_000..=100_000_100).contains(&offset));
        assert_eq!(c.observe(113 * MS, 9 * MS, 113 * MS), None);
        assert_eq!(c.observe(150 * MS, 50 * MS, 150 * MS), None);
    }

    #[test]
    fn offsets_preserve_capture_and_playout_deltas_across_epochs() {
        assert_eq!(reader(90).map(20, 15, 111, false), Some(105));
        assert_eq!(reader(-90).map(120, 135, 31, true), Some(45));
        assert_eq!(reader(90).map(20, 0, 111, false), None);
        assert_eq!(reader(90).map(20, 21, 111, false), None);
        assert_eq!(reader(0).map(1, 1, 500 * MS, true), None);
        assert_eq!(reader(-90).map(91, 10, 1, false), None);
        assert_eq!(reader(1).map(u64::MAX, u64::MAX, u64::MAX, true), None);
    }

    #[test]
    fn stale_observations_do_not_establish_an_epoch_and_regressions_are_rejected() {
        let mut pending = reader(MEASURING);
        for t in 10..20 {
            assert_eq!(pending.map(t, t - 2, 300 + t, false), None);
        }
        let mut r = reader(UNMEASURED);
        for t in 1..=12 {
            assert_eq!(r.map(10, 8, 300 + t, false), None);
        }
        for t in [20, 30] {
            assert_eq!(r.map(t, t - 2, 300 + t, false), None);
        }
        assert!(r.map(40, 38, 340, false).is_some());
        assert_eq!(r.map(40, 38, 341, false), None);
        assert_eq!(r.map(5, 3, 350, false), None);
    }
}
