//! The callback side of the capture: ring producers and the level and anchor accounting.
//!
//! [`Capture::push_block`] runs on the audio thread. It never blocks, allocates, locks or does
//! I/O: it touches atomics and the preallocated rings only.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::anchors::Anchor;
use crate::levels::LevelBlock;

/// Audio ring length in seconds of the device rate.
pub const AUDIO_RING_SECONDS: usize = 4;
pub const ANCHOR_RING: usize = 512;
pub const LEVEL_RING: usize = 1024;

/// State shared by the callback, the control thread and the error callback.
#[derive(Debug, Default)]
pub struct Shared {
    /// True while a take runs; the callback pushes audio only then.
    pub(crate) recording: AtomicBool,
    /// Incremented by the control thread before it sets `recording` for a new take.
    pub(crate) take_gen: AtomicU32,
    /// cpal `Xrun` errors.
    pub(crate) xruns: AtomicU32,
    /// Blocks dropped because the audio ring was full or the block too large.
    pub(crate) overflow_blocks: AtomicU32,
}

impl Shared {
    /// `xruns + overflow_blocks`, saturating.
    pub(crate) fn suspect_blocks(&self) -> u32 {
        self.xruns
            .load(Ordering::Relaxed)
            .saturating_add(self.overflow_blocks.load(Ordering::Relaxed))
    }

    pub(crate) fn count_xrun(&self) {
        self.xruns.fetch_add(1, Ordering::Relaxed);
    }
}

/// The writer's ends of the rings.
pub struct Rings {
    pub audio: Consumer<f32>,
    pub anchors: Consumer<Anchor>,
    pub levels: Consumer<LevelBlock>,
}

/// The callback's capture state: the ring producers and where the take is.
pub struct Capture {
    shared: Arc<Shared>,
    sample_rate: u32,
    audio: Producer<f32>,
    anchors: Producer<Anchor>,
    levels: Producer<LevelBlock>,
    /// Frames of the current take pushed to the ring (the file index of the next frame).
    file_frame: u64,
    last_anchor_frame: u64,
    seen_gen: u32,
    /// The take's first anchor is still owed (a full anchor ring makes the next block retry).
    need_first_anchor: bool,
}

impl Capture {
    /// Allocates the rings: audio `AUDIO_RING_SECONDS * sample_rate` slots, anchors and levels
    /// fixed. This is the only allocation of the capture side.
    pub(crate) fn new(sample_rate: u32, shared: Arc<Shared>) -> (Capture, Rings) {
        let audio_slots = AUDIO_RING_SECONDS * (sample_rate.max(1) as usize);
        let (audio, audio_rx) = RingBuffer::new(audio_slots);
        let (anchors, anchors_rx) = RingBuffer::new(ANCHOR_RING);
        let (levels, levels_rx) = RingBuffer::new(LEVEL_RING);
        let capture = Capture {
            shared,
            sample_rate,
            audio,
            anchors,
            levels,
            file_frame: 0,
            last_anchor_frame: 0,
            seen_gen: 0,
            need_first_anchor: false,
        };
        let rings = Rings {
            audio: audio_rx,
            anchors: anchors_rx,
            levels: levels_rx,
        };
        (capture, rings)
    }

    /// One callback block of the chosen channel, already converted to f32; `host_ns` is the
    /// capture time of `frames[0]`.
    pub fn push_block(&mut self, frames: &[f32], host_ns: u64) {
        let n = frames.len();
        if n == 0 {
            return;
        }
        // Levels are lossy: a full ring drops the block.
        let _ = self.levels.push(LevelBlock::of(frames));
        if !self.shared.recording.load(Ordering::Acquire) {
            return;
        }
        let take_gen = self.shared.take_gen.load(Ordering::Acquire);
        if take_gen != self.seen_gen {
            self.seen_gen = take_gen;
            self.file_frame = 0;
            self.last_anchor_frame = 0;
            self.need_first_anchor = true;
        }
        let Ok(mut chunk) = self.audio.write_chunk(n) else {
            self.count_overflow();
            return;
        };
        let (a, b) = chunk.as_mut_slices();
        let split = a.len();
        a.copy_from_slice(&frames[..split]);
        b.copy_from_slice(&frames[split..]);
        chunk.commit_all();
        let frame = self.file_frame;
        self.file_frame += n as u64;
        let due =
            self.need_first_anchor || frame - self.last_anchor_frame >= u64::from(self.sample_rate);
        if due && self.anchors.push(Anchor { frame, host_ns }).is_ok() {
            self.last_anchor_frame = frame;
            self.need_first_anchor = false;
        }
    }

    /// Counts a block the source could not deliver (larger than its conversion buffer).
    pub fn count_overflow(&self) {
        self.shared.overflow_blocks.fetch_add(1, Ordering::Relaxed);
    }

    /// The writable audio slots (all of them when the writer has drained everything).
    pub fn free_audio_slots(&self) -> usize {
        self.audio.slots()
    }

    /// The audio ring's capacity.
    pub fn audio_capacity(&self) -> usize {
        self.audio.buffer().capacity()
    }

    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn recording(rate: u32) -> (Capture, Rings, Arc<Shared>) {
        let shared = Arc::new(Shared::default());
        let (capture, rings) = Capture::new(rate, shared.clone());
        shared.take_gen.fetch_add(1, Ordering::AcqRel);
        shared.recording.store(true, Ordering::Release);
        (capture, rings, shared)
    }

    fn pop_all<T>(c: &mut Consumer<T>) -> Vec<T> {
        let mut out = Vec::new();
        while let Ok(x) = c.pop() {
            out.push(x);
        }
        out
    }

    #[test]
    fn not_recording_pushes_levels_only() {
        let shared = Arc::new(Shared::default());
        let (mut capture, mut rings) = Capture::new(100, shared);
        capture.push_block(&[0.5; 10], 7);
        assert_eq!(rings.levels.slots(), 1);
        assert_eq!(rings.audio.slots(), 0);
        assert_eq!(rings.anchors.slots(), 0);
        assert_eq!(pop_all(&mut rings.levels)[0].frames, 10);
    }

    #[test]
    fn an_empty_block_is_ignored() {
        let (mut capture, rings, _) = recording(100);
        capture.push_block(&[], 7);
        assert_eq!(rings.levels.slots(), 0);
        assert_eq!(rings.anchors.slots(), 0);
        assert_eq!(capture.file_frame, 0);
    }

    #[test]
    fn the_first_block_of_a_take_is_anchored_then_once_per_second() {
        let (mut capture, mut rings, _) = recording(100);
        // Blocks of 30 frames at 10 ns per frame.
        for i in 0..10u64 {
            capture.push_block(&[0.1; 30], 1_000 + i * 300);
        }
        let anchors = pop_all(&mut rings.anchors);
        // Frames 0, 30, …, 270: the first at or past 100 after 0 is 120, then 240.
        assert_eq!(
            anchors,
            vec![
                Anchor {
                    frame: 0,
                    host_ns: 1_000
                },
                Anchor {
                    frame: 120,
                    host_ns: 2_200
                },
                Anchor {
                    frame: 240,
                    host_ns: 3_400
                },
            ]
        );
        assert_eq!(pop_all(&mut rings.audio).len(), 300);
    }

    #[test]
    fn a_new_take_restarts_the_frame_count_and_anchors_at_once() {
        let (mut capture, mut rings, shared) = recording(100);
        capture.push_block(&[0.1; 50], 10);
        capture.push_block(&[0.1; 20], 20);
        shared.recording.store(false, Ordering::Release);
        capture.push_block(&[0.1; 20], 30);
        shared.take_gen.fetch_add(1, Ordering::AcqRel);
        shared.recording.store(true, Ordering::Release);
        capture.push_block(&[0.1; 20], 40);
        assert_eq!(
            pop_all(&mut rings.anchors),
            vec![
                Anchor {
                    frame: 0,
                    host_ns: 10
                },
                Anchor {
                    frame: 0,
                    host_ns: 40
                }
            ]
        );
        assert_eq!(capture.file_frame, 20);
    }

    #[test]
    fn a_full_audio_ring_drops_the_block_without_blocking() {
        let (mut capture, mut rings, shared) = recording(100);
        assert_eq!(capture.audio_capacity(), 400);
        let started = Instant::now();
        for i in 0..6u64 {
            capture.push_block(&[0.1; 100], i);
        }
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(shared.overflow_blocks.load(Ordering::Relaxed), 2);
        assert_eq!(capture.file_frame, 400);
        assert_eq!(shared.suspect_blocks(), 2);
        // A block larger than the whole ring is dropped too.
        pop_all(&mut rings.audio);
        capture.push_block(&[0.1; 401], 9);
        assert_eq!(shared.overflow_blocks.load(Ordering::Relaxed), 3);
        assert_eq!(capture.file_frame, 400);
        assert_eq!(capture.free_audio_slots(), 400);
    }

    #[test]
    fn a_full_anchor_ring_skips_and_the_next_block_retries() {
        let (mut capture, mut rings, _) = recording(1);
        // At 1 Hz every block is due an anchor.
        for i in 0..(ANCHOR_RING as u64 + 3) {
            capture.push_block(&[0.0], 100 + i);
            pop_all(&mut rings.audio);
        }
        assert_eq!(rings.anchors.slots(), ANCHOR_RING);
        let first = pop_all(&mut rings.anchors);
        assert_eq!(
            first[0],
            Anchor {
                frame: 0,
                host_ns: 100
            }
        );
        assert_eq!(first[ANCHOR_RING - 1].frame, ANCHOR_RING as u64 - 1);
        capture.push_block(&[0.0], 9_000);
        assert_eq!(
            pop_all(&mut rings.anchors),
            vec![Anchor {
                frame: ANCHOR_RING as u64 + 3,
                host_ns: 9_000
            }]
        );
    }

    #[test]
    fn a_skipped_first_anchor_is_still_forced_on_the_next_block() {
        let (mut capture, mut rings, shared) = recording(1_000);
        // Fill the anchor ring with a take's first anchors.
        for i in 0..ANCHOR_RING as u32 {
            shared.take_gen.store(100 + i, Ordering::Release);
            capture.push_block(&[0.0], u64::from(i));
            pop_all(&mut rings.audio);
        }
        shared.take_gen.store(5_000, Ordering::Release);
        capture.push_block(&[0.0; 10], 77);
        pop_all(&mut rings.anchors);
        capture.push_block(&[0.0; 10], 88);
        assert_eq!(
            pop_all(&mut rings.anchors),
            vec![Anchor {
                frame: 10,
                host_ns: 88
            }]
        );
    }

    #[test]
    fn audio_wraps_around_the_ring_in_order() {
        let (mut capture, mut rings, _) = recording(1);
        // Ring of 4 slots.
        capture.push_block(&[1.0, 2.0, 3.0], 0);
        assert_eq!(pop_all(&mut rings.audio), vec![1.0, 2.0, 3.0]);
        capture.push_block(&[4.0, 5.0, 6.0], 1);
        assert_eq!(pop_all(&mut rings.audio), vec![4.0, 5.0, 6.0]);
    }
}
