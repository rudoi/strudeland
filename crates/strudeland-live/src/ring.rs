//! The audio ring between the engine thread and the device callback, and the callback body.
//!
//! The callback runs on the audio thread: it pops the ring, bumps atomics and pushes anchor
//! samples into a second ring. It never allocates, locks, blocks or logs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::output::{PlaybackTime, RenderCallback};

/// Capacity of the audio ring in device-rate stereo frames.
pub(crate) const RING_CAPACITY_FRAMES: usize = 8192;
/// The fill the engine thread tops the ring up to, in device-rate frames.
pub(crate) const RING_TARGET_FRAMES: usize = 2048;
/// Capacity of the anchor ring (a full ring drops the sample).
pub(crate) const ANCHOR_RING_CAPACITY: usize = 16;

/// Frames of the fade-out ramp at stop.
const FADE_FRAMES: usize = 128;

/// The audio ring (interleaved stereo `f32`).
pub(crate) fn make_ring() -> (Producer<f32>, Consumer<f32>) {
    RingBuffer::<f32>::new(RING_CAPACITY_FRAMES * 2)
}

/// The anchor ring.
pub(crate) fn make_anchor_ring() -> (Producer<AnchorSample>, Consumer<AnchorSample>) {
    RingBuffer::<AnchorSample>::new(ANCHOR_RING_CAPACITY)
}

/// A mapping from ring frames consumed to host time, pushed by the callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AnchorSample {
    /// Ring frames popped before this callback (silence it padded does not count).
    pub consumed: u64,
    /// Host time at which this callback's first frame is heard.
    pub host_ns: u64,
}

/// State the callback and the engine thread share.
#[derive(Debug)]
pub(crate) struct Shared {
    /// The ring is topped up; before this the callback writes zeros and counts nothing.
    pub primed: AtomicBool,
    /// Ask the callback to ramp to silence.
    pub fade_out: AtomicBool,
    /// The ramp finished (or there was nothing to ramp).
    pub faded: AtomicBool,
    pub underruns: AtomicU64,
    /// The smallest ring fill (frames) seen at the start of a callback since the last swap.
    pub min_fill_frames: AtomicU32,
    /// The device rate, stored by the engine thread before it sets `primed` (it is known only
    /// once the output has started); sets the anchor interval.
    pub device_rate: AtomicU32,
    /// Callbacks run so far (stopping waits for one whole silent buffer after the ramp).
    pub callbacks: AtomicU64,
}

impl Default for Shared {
    fn default() -> Self {
        Shared {
            primed: AtomicBool::new(false),
            fade_out: AtomicBool::new(false),
            faded: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            min_fill_frames: AtomicU32::new(u32::MAX),
            device_rate: AtomicU32::new(0),
            callbacks: AtomicU64::new(0),
        }
    }
}

/// The callback the player installs on the output. `shared.device_rate` (read on the first
/// primed call) sets the anchor interval.
pub(crate) fn make_callback(
    mut consumer: Consumer<f32>,
    mut anchors: Producer<AnchorSample>,
    shared: Arc<Shared>,
) -> RenderCallback {
    let mut anchor_every: u64 = 0;
    let mut consumed: u64 = 0;
    let mut since_anchor: u64 = 0;
    let mut first = true;
    let mut after_underrun = false;
    let mut fade_pos: usize = 0;
    Box::new(move |buf: &mut [f32], time: PlaybackTime| {
        shared.callbacks.fetch_add(1, Ordering::Release);
        let fading = shared.fade_out.load(Ordering::Acquire);
        if shared.faded.load(Ordering::Relaxed) {
            buf.fill(0.0);
            return;
        }
        if !shared.primed.load(Ordering::Acquire) {
            buf.fill(0.0);
            if fading {
                shared.faded.store(true, Ordering::Release);
            }
            return;
        }
        if anchor_every == 0 {
            anchor_every = u64::from(shared.device_rate.load(Ordering::Acquire) / 4).max(1);
        }
        let want = buf.len() / 2;
        let have = consumer.slots() / 2;
        shared
            .min_fill_frames
            .fetch_min(have.min(u32::MAX as usize) as u32, Ordering::Relaxed);
        if first || after_underrun || since_anchor >= anchor_every {
            // A full anchor ring drops the sample.
            let _ = anchors.push(AnchorSample {
                consumed,
                host_ns: time.host_ns,
            });
            first = false;
            after_underrun = false;
            since_anchor = 0;
        }
        let n = want.min(have);
        if n > 0
            && let Ok(chunk) = consumer.read_chunk(n * 2)
        {
            let (a, b) = chunk.as_slices();
            buf[..a.len()].copy_from_slice(a);
            buf[a.len()..a.len() + b.len()].copy_from_slice(b);
            chunk.commit_all();
        }
        buf[n * 2..].fill(0.0);
        consumed += n as u64;
        since_anchor += n as u64;
        if have < want {
            shared.underruns.fetch_add(1, Ordering::Relaxed);
            shared.min_fill_frames.fetch_min(0, Ordering::Relaxed);
            after_underrun = true;
        }
        if fading {
            for frame in buf.chunks_exact_mut(2) {
                let gain = if fade_pos < FADE_FRAMES {
                    1.0 - fade_pos as f32 / (FADE_FRAMES - 1) as f32
                } else {
                    0.0
                };
                frame[0] *= gain;
                frame[1] *= gain;
                fade_pos += 1;
            }
            if fade_pos >= FADE_FRAMES {
                shared.faded.store(true, Ordering::Release);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    struct Rig {
        tx: Producer<f32>,
        anchors: Consumer<AnchorSample>,
        shared: Arc<Shared>,
        cb: RenderCallback,
    }

    fn rig() -> Rig {
        let (tx, rx) = make_ring();
        let (atx, arx) = make_anchor_ring();
        let shared = Arc::new(Shared::default());
        shared.device_rate.store(RATE, Ordering::Release);
        let cb = make_callback(rx, atx, shared.clone());
        Rig {
            tx,
            anchors: arx,
            shared,
            cb,
        }
    }

    impl Rig {
        fn fill(&mut self, frames: usize, value: f32) {
            for _ in 0..frames * 2 {
                self.tx.push(value).unwrap();
            }
        }
        fn pull(&mut self, frames: usize, host_ns: u64) -> Vec<f32> {
            let mut buf = vec![9.0; frames * 2];
            (self.cb)(&mut buf, PlaybackTime { host_ns });
            buf
        }
        fn drain_anchors(&mut self) -> Vec<AnchorSample> {
            std::iter::from_fn(|| self.anchors.pop().ok()).collect()
        }
    }

    #[test]
    fn constants_are_the_documented_ones() {
        assert_eq!(RING_CAPACITY_FRAMES, 8192);
        assert_eq!(RING_TARGET_FRAMES, 2048);
        assert_eq!(ANCHOR_RING_CAPACITY, 16);
        let (tx, _rx) = make_ring();
        assert_eq!(tx.buffer().capacity(), RING_CAPACITY_FRAMES * 2);
    }

    #[test]
    fn not_primed_writes_zeros_and_counts_nothing() {
        let mut r = rig();
        r.fill(512, 0.5);
        let out = r.pull(256, 10);
        assert!(out.iter().all(|&s| s == 0.0));
        assert_eq!(r.shared.underruns.load(Ordering::Relaxed), 0);
        assert_eq!(r.shared.min_fill_frames.load(Ordering::Relaxed), u32::MAX);
        assert!(r.drain_anchors().is_empty());
    }

    #[test]
    fn first_primed_call_pushes_an_anchor_and_pops_frames() {
        let mut r = rig();
        r.fill(512, 0.5);
        r.shared.primed.store(true, Ordering::Release);
        let out = r.pull(256, 1234);
        assert!(out.iter().all(|&s| s == 0.5));
        assert_eq!(
            r.drain_anchors(),
            vec![AnchorSample {
                consumed: 0,
                host_ns: 1234
            }]
        );
        assert_eq!(r.shared.min_fill_frames.load(Ordering::Relaxed), 512);
        r.pull(256, 2000);
        assert!(r.drain_anchors().is_empty());
        assert_eq!(r.shared.underruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_short_ring_zero_fills_counts_an_underrun_and_anchors_next_call() {
        let mut r = rig();
        r.fill(100, 0.25);
        r.shared.primed.store(true, Ordering::Release);
        let out = r.pull(256, 1);
        assert!(out[..200].iter().all(|&s| s == 0.25));
        assert!(out[200..].iter().all(|&s| s == 0.0));
        assert_eq!(r.shared.underruns.load(Ordering::Relaxed), 1);
        assert_eq!(r.shared.min_fill_frames.load(Ordering::Relaxed), 0);
        assert_eq!(r.drain_anchors().len(), 1); // the first-call anchor
        r.fill(512, 0.5);
        r.pull(256, 99);
        // consumed counts popped frames only, not the padding
        assert_eq!(
            r.drain_anchors(),
            vec![AnchorSample {
                consumed: 100,
                host_ns: 99
            }]
        );
    }

    #[test]
    fn one_anchor_per_250_ms_of_consumed_frames() {
        let mut r = rig();
        r.shared.primed.store(true, Ordering::Release);
        let block = 480; // 10 ms at 48 kHz
        let mut total = Vec::new();
        for i in 0..100 {
            r.fill(block, 0.1);
            r.pull(block, i);
            total.extend(r.drain_anchors());
        }
        // 1 s of frames: the first call, then at 250, 500 and 750 ms
        assert_eq!(total.len(), 4, "{total:?}");
        let consumed: Vec<u64> = total.iter().map(|a| a.consumed).collect();
        assert_eq!(consumed, vec![0, 12_000, 24_000, 36_000]);
    }

    #[test]
    fn fade_out_ramps_over_128_frames_then_zeros() {
        let mut r = rig();
        r.fill(1024, 1.0);
        r.shared.primed.store(true, Ordering::Release);
        r.shared.fade_out.store(true, Ordering::Release);
        let out = r.pull(512, 1);
        assert_eq!(out[0], 1.0);
        assert!(out[1] > 0.99);
        assert!(out[2 * 64] < 0.6 && out[2 * 64] > 0.4);
        assert_eq!(out[2 * 127], 0.0);
        assert!(out[2 * 128..].iter().all(|&s| s == 0.0));
        assert!(
            out[..2 * 127]
                .chunks(2)
                .zip(out[2..2 * 127].chunks(2))
                .all(|(a, b)| a[0] >= b[0])
        );
        assert!(r.shared.faded.load(Ordering::Acquire));
        // from then on zeros
        let again = r.pull(64, 2);
        assert!(again.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn a_short_callback_continues_the_ramp() {
        let mut r = rig();
        r.fill(512, 1.0);
        r.shared.primed.store(true, Ordering::Release);
        r.shared.fade_out.store(true, Ordering::Release);
        r.pull(64, 1);
        assert!(!r.shared.faded.load(Ordering::Acquire));
        r.pull(64, 2);
        assert!(r.shared.faded.load(Ordering::Acquire));
    }

    #[test]
    fn fade_out_before_priming_just_marks_faded() {
        let mut r = rig();
        r.shared.fade_out.store(true, Ordering::Release);
        let out = r.pull(32, 1);
        assert!(out.iter().all(|&s| s == 0.0));
        assert!(r.shared.faded.load(Ordering::Acquire));
    }

    #[test]
    fn a_full_anchor_ring_drops_the_sample_without_blocking() {
        let mut r = rig();
        r.shared.primed.store(true, Ordering::Release);
        // every call underruns, so every following call wants an anchor
        for i in 0..(ANCHOR_RING_CAPACITY as u64 * 3) {
            r.pull(64, i);
        }
        let anchors = r.drain_anchors();
        assert_eq!(anchors.len(), ANCHOR_RING_CAPACITY);
        assert_eq!(anchors[0].host_ns, 0);
    }
}
