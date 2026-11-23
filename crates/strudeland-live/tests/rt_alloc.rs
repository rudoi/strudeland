//! The real-time rule: the output callback the player installs never allocates (or frees).
//!
//! A counting global allocator counts every allocation, reallocation and free made while a
//! thread-local flag is set; an `Output` of this file's own sets the flag around each `render`
//! call on its audio thread, pulling 512-frame blocks in real time for 2 s as synthetic haps
//! play (priming, anchors, underrun-free play and the stop fade all run under the flag).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value as Json;
use strudeland_engine::OrbitSpec;
use strudeland_live::{
    Batch, HapEntry, Output, OutputError, OutputInfo, PlaybackTime, Player, RenderCallback,
    ValueEntry,
};

/// What the current thread is doing: 0 nothing counted, 1 in a callback, 2 the guard's own test.
const IN_CALLBACK_FLAG: u8 = 1;
const SELF_TEST_FLAG: u8 = 2;

thread_local! {
    static FLAG: Cell<u8> = const { Cell::new(0) };
}

static COUNTED: AtomicU64 = AtomicU64::new(0);
static SELF_TEST_COUNTED: AtomicU64 = AtomicU64::new(0);

struct Counting;

fn count() {
    match FLAG.try_with(Cell::get).unwrap_or(0) {
        IN_CALLBACK_FLAG => COUNTED.fetch_add(1, Ordering::SeqCst),
        SELF_TEST_FLAG => SELF_TEST_COUNTED.fetch_add(1, Ordering::SeqCst),
        _ => 0,
    };
}

// SAFETY: forwards every call to the system allocator unchanged; only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: the caller's contract is passed through.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: as above.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        // SAFETY: as above.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count();
        // SAFETY: as above.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// What the audio thread saw.
#[derive(Default)]
struct Seen {
    calls: AtomicU64,
    peak_bits: AtomicU32,
}

/// A real-time output whose audio thread flags each `render` call.
struct FlaggedOutput {
    rate: u32,
    seen: Arc<Seen>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Output for FlaggedOutput {
    fn start(&mut self, mut render: RenderCallback) -> Result<OutputInfo, OutputError> {
        self.stop();
        self.stop.store(false, Ordering::SeqCst);
        let (rate, seen, stop) = (self.rate, self.seen.clone(), self.stop.clone());
        self.thread = Some(std::thread::spawn(move || {
            let mut buf = vec![0.0f32; 1024];
            let started = Instant::now();
            let mut frames: u64 = 0;
            while !stop.load(Ordering::SeqCst) {
                let host_ns = 1_000_000_000 + frames * 1_000_000_000 / u64::from(rate);
                FLAG.with(|f| f.set(IN_CALLBACK_FLAG));
                render(&mut buf, PlaybackTime { host_ns });
                FLAG.with(|f| f.set(0));
                let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                if peak > f32::from_bits(seen.peak_bits.load(Ordering::Relaxed)) {
                    seen.peak_bits.store(peak.to_bits(), Ordering::Relaxed);
                }
                seen.calls.fetch_add(1, Ordering::SeqCst);
                frames += 512;
                let due = started + Duration::from_nanos(frames * 1_000_000_000 / u64::from(rate));
                if let Some(wait) = due.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                }
            }
        }));
        Ok(OutputInfo {
            sample_rate: self.rate,
            buffer_frames: Some(512),
        })
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The first 2 s of the live-pressure job's haps, shifted to start at 0.3 s, in one batch.
fn live_pressure_batch() -> Batch {
    let job: Json = serde_json::from_str(include_str!("data/live-pressure.job.json")).unwrap();
    let mut haps: Vec<(f64, f64, f64, u32)> = job["pauses"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["haps"].as_array().unwrap().iter())
        .map(|h| {
            (
                h["t"].as_f64().unwrap(),
                h["duration"].as_f64().unwrap(),
                h["cycle"].as_f64().unwrap(),
                h["v"].as_u64().unwrap() as u32,
            )
        })
        .collect();
    haps.sort_by(|a, b| a.0.total_cmp(&b.0));
    let t0 = haps[0].0;
    let values = job["values"].as_array().unwrap();
    let orbits: Vec<OrbitSpec> = serde_json::from_value(job["orbits"].clone()).unwrap();
    Batch {
        values: values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                ValueEntry(
                    i as u32,
                    v.clone(),
                    (i < orbits.len()).then(|| orbits[i].clone()),
                )
            })
            .collect(),
        haps: haps
            .into_iter()
            .filter(|h| h.0 < t0 + 2.0)
            .map(|(t, d, c, v)| HapEntry(t - t0 + 0.3, d, c, v, None))
            .collect(),
        cps: job["cps"].as_f64().unwrap(),
    }
}

fn no_allocation_in_the_callback(rate: u32) {
    let seen = Arc::new(Seen::default());
    let output = FlaggedOutput {
        rate,
        seen: seen.clone(),
        stop: Arc::new(AtomicBool::new(false)),
        thread: None,
    };
    let (player, _events) = Player::start(Box::new(output), 0).unwrap();
    player.schedule(live_pressure_batch());
    std::thread::sleep(Duration::from_secs(2));
    player.stop();
    let calls = seen.calls.load(Ordering::SeqCst);
    assert!(calls >= 100, "{calls} calls");
    let peak = f32::from_bits(seen.peak_bits.load(Ordering::Relaxed));
    assert!(peak > 0.01, "the haps played (peak {peak})");
    assert_eq!(
        COUNTED.load(Ordering::SeqCst),
        0,
        "allocations in the callback"
    );
}

#[test]
fn the_callback_never_allocates_at_44100() {
    no_allocation_in_the_callback(44100);
}

#[test]
fn the_callback_never_allocates_resampled_at_48000() {
    no_allocation_in_the_callback(48000);
}

#[test]
fn the_counter_counts_flagged_allocations() {
    // the guard itself works: an allocation and a free under a flag are counted
    std::thread::spawn(|| {
        FLAG.with(|f| f.set(SELF_TEST_FLAG));
        let v = std::hint::black_box(vec![1u8; 64]);
        drop(v);
        FLAG.with(|f| f.set(0));
    })
    .join()
    .unwrap();
    assert_eq!(SELF_TEST_COUNTED.load(Ordering::SeqCst), 2);
}
