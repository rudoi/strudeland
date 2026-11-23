//! The [`Player`]: the engine thread rendering ahead into the ring, the IR worker, anchors,
//! stats, scope and level frames.
//!
//! The engine thread owns the [`Output`], the `LiveEngine`, the ring producer and the resampler.
//! The audio callback ([`make_callback`]) only pops the ring, bumps atomics and pushes anchor
//! samples; everything that allocates (voices, resampling, events) happens here.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rtrb::{Consumer, Producer};
use strudeland_engine::live::{LiveEngine, LiveEvent, LiveHap};
use strudeland_engine::{QUANTUM, QUANTUM_U64, SAMPLE_RATE, SAMPLE_RATE_F64, Value};

use crate::batch::{Batch, HapEntry, ValueEntry};
use crate::error::LiveError;
use crate::event::{BatchDecodeUs, Event, Stats};
use crate::ir_worker::IrWorker;
use crate::master::{MASTER_DELAY_FRAMES, MasterStage};
use crate::output::{Output, OutputError, OutputNotice};
use crate::resample::QuantumResampler;
use crate::ring::{
    AnchorSample, RING_CAPACITY_FRAMES, RING_TARGET_FRAMES, Shared, make_anchor_ring,
    make_callback, make_ring,
};
use crate::rt::{self, TimeConstraint};

/// How long [`Player::start`] waits for the engine thread to have the output playing.
pub(crate) const START_TIMEOUT: Duration = Duration::from_secs(5);
/// How long stopping waits for the callback's fade-out.
const FADE_WAIT: Duration = Duration::from_millis(100);
/// How often a failed output is reopened.
const RETRY_EVERY: Duration = Duration::from_secs(1);
const SCOPE_EVERY: Duration = Duration::from_millis(33);
const STATS_EVERY: Duration = Duration::from_secs(1);
/// Engine frames of mono mix the scope keeps (sent as 1024 pair averages).
const SCOPE_FRAMES: usize = 2048;
/// Quanta rendered per loop pass at most: one slice is the work of one time-constraint period.
pub(crate) const SLICE_QUANTA: usize = 4;
/// The wait between slices while the ring is still below its target: a short block, so a burst
/// of rendering (catching up after a stall) never runs past the period's computation.
const SLICE_WAIT: Duration = Duration::from_micros(100);
/// The engine thread's time-constraint period: the audio time of one slice (512 engine frames,
/// 11.6 ms).
const ENGINE_PERIOD: Duration =
    Duration::from_nanos(SLICE_QUANTA as u64 * QUANTUM_U64 * 1_000_000_000 / SAMPLE_RATE as u64);
/// Half of each period, about twice the engine's measured real-time factor (0.235), so
/// a slice fits its computation with room to spare and the thread can never take more than half
/// a core away from the UI.
pub(crate) const ENGINE_CONSTRAINT: TimeConstraint =
    TimeConstraint::fraction_of(ENGINE_PERIOD, 1, 2);

/// Messages from the [`Player`] handle to its engine thread.
enum Control {
    Batch(Batch),
    Seed(i32),
    Limiter(bool),
    Stop,
}

#[cfg(not(windows))]
fn wait_control(control: &Receiver<Control>, wait: Duration) -> Result<Control, RecvTimeoutError> {
    control.recv_timeout(wait)
}

#[cfg(windows)]
fn wait_control(control: &Receiver<Control>, wait: Duration) -> Result<Control, RecvTimeoutError> {
    match control.try_recv() {
        Ok(message) => return Ok(message),
        Err(TryRecvError::Disconnected) => return Err(RecvTimeoutError::Disconnected),
        Err(TryRecvError::Empty) => {}
    }
    // The channel's Windows timeout can outlast a full 512-frame render slice. Rust's
    // Windows sleep uses a high-resolution waitable timer on the supported OS versions.
    // Check for control again after sleeping for at most one engine period (~11.6 ms).
    std::thread::sleep(wait);
    control.try_recv().map_err(|e| match e {
        TryRecvError::Empty => RecvTimeoutError::Timeout,
        TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
    })
}

/// Live playback: an engine thread rendering haps into a ring that an [`Output`] plays.
pub struct Player {
    control: Sender<Control>,
    thread: Option<JoinHandle<()>>,
}

impl Player {
    /// Starts the output (blocking until the stream plays or fails) and the engine thread.
    pub fn start(
        output: Box<dyn Output>,
        seed: i32,
    ) -> Result<(Player, Receiver<Event>), LiveError> {
        let (control_tx, control_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("strudeland-live-engine".into())
            .spawn(move || engine_main(output, seed, control_rx, event_tx, ready_tx))
            .map_err(|e| LiveError::Engine(format!("engine thread: {e}")))?;
        let mut player = Player {
            control: control_tx,
            thread: Some(thread),
        };
        let failure = match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => return Ok((player, event_rx)),
            Ok(Err(e)) => e,
            Err(RecvTimeoutError::Timeout) => LiveError::Engine("start timed out".into()),
            Err(RecvTimeoutError::Disconnected) => LiveError::Engine("engine thread ended".into()),
        };
        player.shutdown();
        Err(failure)
    }

    /// Queues a batch (non-blocking); a dead engine thread drops it.
    pub fn schedule(&self, batch: Batch) {
        let _ = self.control.send(Control::Batch(batch));
    }

    /// Changes the audio seed of haps scheduled from now.
    pub fn set_seed(&self, seed: i32) {
        let _ = self.control.send(Control::Seed(seed));
    }

    /// Turns the master safety limiter on or off (off when a player starts). The lookahead
    /// delay stays in the path either way, so toggling never moves audio against the anchors.
    pub fn set_limiter(&self, on: bool) {
        let _ = self.control.send(Control::Limiter(on));
    }

    /// Fades one quantum, stops the output and joins the engine thread and the IR worker.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = self.control.send(Control::Stop);
            let _ = thread.join();
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn engine_main(
    output: Box<dyn Output>,
    seed: i32,
    control: Receiver<Control>,
    events: Sender<Event>,
    ready: SyncSender<Result<(), LiveError>>,
) {
    rt::promote(ENGINE_CONSTRAINT);
    let mut engine = LiveEngine::new(seed);
    let (worker, requests, built) = match IrWorker::spawn() {
        Ok(w) => w,
        Err(e) => {
            let _ = ready.send(Err(LiveError::Engine(format!("ir worker: {e}"))));
            return;
        }
    };
    engine.attach_ir_worker(requests, built);
    let mut thread = EngineThread::new(output, engine, events);
    match thread.open_stream() {
        Ok(()) => {
            let _ = ready.send(Ok(()));
            thread.run(&control);
            thread.fade_and_stop();
        }
        Err(e) => {
            thread.output.stop();
            let _ = ready.send(Err(e.into()));
        }
    }
    // Dropping the engine drops the IR request sender, which ends the worker.
    drop(thread);
    worker.join();
}

/// One started output stream and what the engine thread keeps for it.
struct Stream {
    producer: Producer<f32>,
    anchors: Consumer<AnchorSample>,
    shared: Arc<Shared>,
    resampler: QuantumResampler,
    device_rate: u32,
    /// The engine frame at which this stream started (0 for the first).
    base_frame: u64,
}

/// The engine frame heard after `consumed` device frames of a stream that started at engine
/// frame `base_frame`, net of `resampler_delay` engine frames and the master's lookahead.
fn anchor_frame(base_frame: u64, consumed: u64, device_rate: u32, resampler_delay: f64) -> u64 {
    let rel = consumed as f64 * SAMPLE_RATE_F64 / f64::from(device_rate)
        - resampler_delay
        - MASTER_DELAY_FRAMES as f64;
    (base_frame as f64 + rel).round().max(0.0) as u64
}

impl Stream {
    /// The engine frame heard at an anchor sample (net of the resampler delay and the master
    /// stage's lookahead delay, which sits before the resampler). Early in a stream the delays
    /// put the heard frame before `base_frame`: `rel` is signed, so the anchor stays on the same
    /// line as every later one; only a frame before the engine's frame 0 is clamped.
    fn anchor_frame(&self, a: AnchorSample) -> u64 {
        anchor_frame(
            self.base_frame,
            a.consumed,
            self.device_rate,
            self.resampler.delay_engine_frames(),
        )
    }

    fn fill_frames(&self) -> usize {
        RING_CAPACITY_FRAMES.saturating_sub(self.producer.slots() / 2)
    }

    /// With the ring at its target: how long until it has drained a whole slice below it, at
    /// the device rate (at least `SLICE_WAIT`, at most `ENGINE_PERIOD`).
    fn idle_wait(&self) -> Duration {
        let slice =
            (SLICE_QUANTA * QUANTUM) as u64 * u64::from(self.device_rate) / u64::from(SAMPLE_RATE);
        let excess = (self.fill_frames() as u64 + slice).saturating_sub(RING_TARGET_FRAMES as u64);
        let wait = Duration::from_nanos(excess * 1_000_000_000 / u64::from(self.device_rate));
        wait.clamp(SLICE_WAIT, ENGINE_PERIOD)
    }
}

struct EngineThread {
    output: Box<dyn Output>,
    engine: LiveEngine,
    events: Sender<Event>,
    stream: Option<Stream>,
    /// When the output is down: when to try to reopen it.
    retry_at: Option<Instant>,
    /// Batch value index to engine value index (the engine wants 0, 1, 2, ... with no gaps).
    index_of: HashMap<u32, u32>,
    /// Batch value indices whose value could not be decoded (already reported).
    failed: HashSet<u32>,
    batches: u64,
    /// Underruns of streams already closed (the cumulative count spans device changes).
    closed_underruns: u64,
    decode_us: Vec<u64>,
    scope: Vec<f32>,
    scope_pos: usize,
    quantum: [f32; 2 * QUANTUM],
    /// Meter and limiter; here, not on `Stream`, so the level accumulator spans restarts.
    master: MasterStage,
    resampled: Vec<f32>,
    anchored: bool,
    next_scope: Instant,
    next_stats: Instant,
}

impl EngineThread {
    fn new(output: Box<dyn Output>, engine: LiveEngine, events: Sender<Event>) -> EngineThread {
        let now = Instant::now();
        EngineThread {
            output,
            engine,
            events,
            stream: None,
            retry_at: None,
            index_of: HashMap::new(),
            failed: HashSet::new(),
            batches: 0,
            closed_underruns: 0,
            decode_us: Vec::with_capacity(64),
            scope: vec![0.0; SCOPE_FRAMES],
            scope_pos: 0,
            quantum: [0.0; 2 * QUANTUM],
            master: MasterStage::new(),
            resampled: Vec::new(),
            anchored: false,
            next_scope: now + SCOPE_EVERY,
            next_stats: now + STATS_EVERY,
        }
    }

    fn send(&self, event: Event) {
        // A dropped receiver is fine: the player keeps playing.
        let _ = self.events.send(event);
    }

    /// Opens a stream with a new ring at the device's current rate, tops it up and primes it.
    fn open_stream(&mut self) -> Result<(), OutputError> {
        let (producer, consumer) = make_ring();
        let (anchor_tx, anchor_rx) = make_anchor_ring();
        let shared = Arc::new(Shared::default());
        let info = self
            .output
            .start(make_callback(consumer, anchor_tx, shared.clone()))?;
        let resampler = match QuantumResampler::new(info.sample_rate) {
            Ok(r) => r,
            Err(e) => {
                self.output.stop();
                return Err(OutputError::Unsupported(e));
            }
        };
        shared
            .device_rate
            .store(info.sample_rate, Ordering::Release);
        self.resampled = Vec::with_capacity(resampler.output_frames_max() * 2);
        self.stream = Some(Stream {
            producer,
            anchors: anchor_rx,
            shared: shared.clone(),
            resampler,
            device_rate: info.sample_rate,
            base_frame: self.engine.now_frame(),
        });
        // The first MASTER_DELAY_FRAMES frames out of the new stream are silence.
        self.master.reset_stream();
        self.retry_at = None;
        // Priming fills the whole target in one go (the callback plays zeros meanwhile).
        while self.fill() {}
        shared.primed.store(true, Ordering::Release);
        Ok(())
    }

    /// Stops the output and forgets the stream (its unplayed frames are discarded).
    fn close_stream(&mut self) {
        self.output.stop();
        if let Some(s) = self.stream.take() {
            self.closed_underruns += s.shared.underruns.load(Ordering::Relaxed);
        }
    }

    fn run(&mut self, control: &Receiver<Control>) {
        let mut pending: Option<Control> = None;
        loop {
            // 1. Control messages.
            loop {
                let msg = match pending.take() {
                    Some(m) => m,
                    None => match control.try_recv() {
                        Ok(m) => m,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    },
                };
                match msg {
                    Control::Batch(b) => {
                        self.batches += 1;
                        // While the output is down the haps are dropped, but the values are kept:
                        // each crosses once per session, so later haps still name them.
                        let play = self.stream.is_some();
                        self.apply_batch(b, play);
                    }
                    Control::Seed(s) => self.engine.set_seed(s),
                    Control::Limiter(on) => self.master.set_enabled(on),
                    Control::Stop => return,
                }
            }
            // 2. The output.
            self.poll_output();
            // 3. Engine events.
            for e in self.engine.take_events() {
                self.send(match e {
                    LiveEvent::Warning {
                        signature,
                        keys,
                        first_cycle,
                    } => Event::Warning {
                        signature,
                        keys,
                        first_cycle,
                    },
                    LiveEvent::Error { message } => Event::Error { message },
                });
            }
            // 4. Anchors.
            self.emit_anchors();
            // 5. Render ahead, one slice.
            let behind = self.fill();
            // 6. Levels, scope and stats. Levels go out anchored or not.
            let now = Instant::now();
            if now >= self.next_scope {
                if let Some(frame) = self.master.take_frame() {
                    self.send(Event::Levels(frame));
                }
                if self.anchored {
                    self.send_scope();
                }
                self.next_scope = now + SCOPE_EVERY;
            }
            if now >= self.next_stats {
                let stats = self.stats();
                self.send(Event::Stats(stats));
                self.decode_us.clear();
                self.next_stats = now + STATS_EVERY;
            }
            // 7. Wait: briefly between slices, else until a message, the ring has drained a
            // slice below its target, or the next scope or stats event is due.
            let wait = if behind {
                SLICE_WAIT
            } else {
                let idle = self
                    .stream
                    .as_ref()
                    .map_or(ENGINE_PERIOD, Stream::idle_wait);
                let due = self.next_scope.min(self.next_stats);
                idle.min(due.saturating_duration_since(Instant::now()))
            };
            match wait_control(control, wait) {
                Ok(m) => pending = Some(m),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn poll_output(&mut self) {
        match self.output.poll_notice() {
            Some(OutputNotice::DeviceChanged) => self.device_changed(),
            Some(OutputNotice::Failed(message)) => {
                self.send(Event::Error { message });
                self.output_down();
            }
            None => {}
        }
        if let Some(at) = self.retry_at
            && Instant::now() >= at
        {
            self.output.stop();
            if self.open_stream().is_err() {
                // No further events on repeated failure.
                self.retry_at = Some(Instant::now() + RETRY_EVERY);
            }
        }
    }

    /// The device changed: a new stream at the new rate; the old ring's frames are discarded.
    fn device_changed(&mut self) {
        self.emit_anchors();
        self.close_stream();
        if let Err(e) = self.open_stream() {
            self.send(Event::Error {
                message: e.to_string(),
            });
            self.output_down();
        }
    }

    fn output_down(&mut self) {
        self.close_stream();
        self.retry_at = Some(Instant::now() + RETRY_EVERY);
    }

    fn emit_anchors(&mut self) {
        let Some(s) = &mut self.stream else {
            return;
        };
        while let Ok(a) = s.anchors.pop() {
            let frame = s.anchor_frame(a);
            let _ = self.events.send(Event::Anchor {
                frame,
                host_ns: a.host_ns,
            });
            self.anchored = true;
        }
    }

    /// Renders one slice: quanta into the ring until it holds `RING_TARGET_FRAMES` (or has no
    /// room for another resampled quantum), at most `SLICE_QUANTA`. True when the slice ended
    /// with the ring still below its target.
    fn fill(&mut self) -> bool {
        let Some(s) = &mut self.stream else {
            return false;
        };
        let room_needed = s.resampler.output_frames_max();
        let wants = |s: &Stream| {
            s.producer.slots() / 2 >= room_needed && s.fill_frames() < RING_TARGET_FRAMES
        };
        for _ in 0..SLICE_QUANTA {
            if !wants(s) {
                return false;
            }
            self.engine.render_quantum(&mut self.quantum);
            for f in self.quantum.chunks_exact(2) {
                self.scope[self.scope_pos] = (f[0] + f[1]) * 0.5;
                self.scope_pos = (self.scope_pos + 1) % SCOPE_FRAMES;
            }
            self.master.process(&mut self.quantum);
            s.resampler.process(&self.quantum, &mut self.resampled);
            let n = self.resampled.len();
            if let Ok(mut chunk) = s.producer.write_chunk(n) {
                let (a, b) = chunk.as_mut_slices();
                let split = a.len();
                a.copy_from_slice(&self.resampled[..split]);
                b.copy_from_slice(&self.resampled[split..]);
                chunk.commit_all();
            }
        }
        wants(s)
    }

    /// Adds the batch's values (and orbit specs); schedules its haps only when `play`.
    fn apply_batch(&mut self, batch: Batch, play: bool) {
        let started = Instant::now();
        let cps = batch.cps;
        for ValueEntry(index, json, orbit) in batch.values {
            if self.index_of.contains_key(&index) || self.failed.contains(&index) {
                self.send(Event::Error {
                    message: format!("value {index}: sent twice"),
                });
                continue;
            }
            match Value::from_json(json) {
                Ok(Value::Obj(map)) => {
                    let engine_index = self.index_of.len() as u32;
                    self.engine.add_value(engine_index, map, orbit);
                    self.index_of.insert(index, engine_index);
                }
                _ => {
                    self.failed.insert(index);
                    self.send(Event::Error {
                        message: format!("value {index}: not a control map"),
                    });
                }
            }
        }
        let mut unknown: Vec<u32> = Vec::new();
        let haps: &[HapEntry] = if play { &batch.haps } else { &[] };
        for HapEntry(t, duration, cycle, index, phase_sec) in haps {
            let Some(&value) = self.index_of.get(index) else {
                // A value that failed to decode was reported already; an unknown one once per batch.
                if !self.failed.contains(index) && !unknown.contains(index) {
                    unknown.push(*index);
                    self.send(Event::Error {
                        message: format!("hap names unknown value {index}"),
                    });
                }
                continue;
            };
            self.engine.schedule(LiveHap {
                frame: (t * SAMPLE_RATE_F64).floor().max(0.0) as u64,
                t_sec: *t,
                duration_sec: *duration,
                cycle: *cycle,
                phase_sec: *phase_sec,
                value,
                cps,
            });
        }
        let ns = started.elapsed().as_nanos();
        self.decode_us
            .push(ns.div_ceil(1000).min(u128::from(u64::MAX)) as u64);
    }

    fn send_scope(&self) {
        let samples: Vec<f32> = (0..SCOPE_FRAMES / 2)
            .map(|k| {
                let a = self.scope[(self.scope_pos + 2 * k) % SCOPE_FRAMES];
                let b = self.scope[(self.scope_pos + 2 * k + 1) % SCOPE_FRAMES];
                (a + b) * 0.5
            })
            .collect();
        self.send(Event::Scope { samples });
    }

    fn stats(&mut self) -> Stats {
        let counters = self.engine.counters();
        let (underruns, headroom) = match &self.stream {
            Some(s) => {
                let min = s.shared.min_fill_frames.swap(u32::MAX, Ordering::Relaxed);
                let ms = if min == u32::MAX {
                    0.0
                } else {
                    f64::from(min) * 1000.0 / f64::from(s.device_rate)
                };
                (s.shared.underruns.load(Ordering::Relaxed), ms)
            }
            None => (0, 0.0),
        };
        let mut decode = self.decode_us.clone();
        decode.sort_unstable();
        let batch_decode_us = match decode.last() {
            Some(&max) => BatchDecodeUs {
                p50: decode[decode.len() / 2],
                max,
            },
            None => BatchDecodeUs::default(),
        };
        Stats {
            underruns: self.closed_underruns + underruns,
            late_haps: counters.late_haps,
            haps: counters.haps,
            batches: self.batches,
            skipped_haps: counters.skipped_haps,
            render_headroom_ms: headroom,
            batch_decode_us,
        }
    }

    /// Stopping: ramp to silence, wait for one silent buffer after the ramp (at most
    /// `FADE_WAIT` in all), stop the output.
    fn fade_and_stop(&mut self) {
        if let Some(s) = &self.stream {
            s.shared.fade_out.store(true, Ordering::Release);
            let deadline = Instant::now() + FADE_WAIT;
            let mut faded_at: Option<u64> = None;
            while Instant::now() < deadline {
                let calls = s.shared.callbacks.load(Ordering::Acquire);
                if s.shared.faded.load(Ordering::Acquire) {
                    match faded_at {
                        None => faded_at = Some(calls),
                        Some(c) if calls > c => break,
                        Some(_) => {}
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        self.close_stream();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn the_slice_wait_checks_messages_and_disconnection() {
        let (tx, rx) = mpsc::channel();
        assert!(matches!(
            wait_control(&rx, SLICE_WAIT),
            Err(RecvTimeoutError::Timeout)
        ));
        assert!(tx.send(Control::Stop).is_ok());
        drop(tx);
        assert!(matches!(wait_control(&rx, SLICE_WAIT), Ok(Control::Stop)));
        assert!(matches!(
            wait_control(&rx, SLICE_WAIT),
            Err(RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn an_anchor_before_the_streams_base_is_not_clamped_to_it() {
        let d = MASTER_DELAY_FRAMES as u64;
        // a stream that started at engine frame 10000: the first anchor is heard 66 frames
        // (plus the resampler delay) before it
        assert_eq!(anchor_frame(10_000, 0, 44100, 0.0), 10_000 - d);
        assert_eq!(anchor_frame(10_000, 0, 48000, 20.4), 10_000 - d - 20);
        assert_eq!(anchor_frame(10_000, 4410, 44100, 0.0), 10_000 + 4410 - d);
        // only a frame before the engine's frame 0 is clamped
        assert_eq!(anchor_frame(0, 0, 44100, 0.0), 0);
        assert_eq!(anchor_frame(10, 0, 44100, 0.0), 0);
        assert_eq!(anchor_frame(0, 100, 44100, 0.0), 100 - d);
    }

    #[test]
    fn the_engine_period_is_one_slice_of_audio() {
        // 4 quanta of 128 frames at 44.1 kHz
        assert_eq!(SLICE_QUANTA * QUANTUM, 512);
        assert_eq!(ENGINE_PERIOD, Duration::from_nanos(11_609_977));
        assert_eq!(ENGINE_CONSTRAINT.period, ENGINE_PERIOD);
        assert_eq!(ENGINE_CONSTRAINT.constraint, ENGINE_PERIOD);
        assert_eq!(ENGINE_CONSTRAINT.computation, ENGINE_PERIOD / 2);
    }

    #[test]
    fn a_slice_fits_its_computation_at_twice_the_measured_real_time_factor() {
        // The earlier benchmark rendered at 0.235 of real time on one thread.
        let slice_work = ENGINE_PERIOD.as_secs_f64() * 0.235 * 2.0;
        assert!(slice_work < ENGINE_CONSTRAINT.computation.as_secs_f64());
    }
}
