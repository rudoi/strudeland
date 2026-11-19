//! The master stage: level frames of every rendered sample and an optional safety limiter.
//!
//! [`MasterStage::process`] runs in place on each 44.1 kHz engine quantum on the engine thread
//! (never in the output callback), before the resampler. It first measures the incoming quantum
//! (levels describe the mix as written, before the limiter), then writes the quantum delayed by
//! the lookahead ([`MASTER_DELAY_FRAMES`]) and, when the limiter is on, limited to
//! [`CEILING_DB`]. The delay line is always in the path, so toggling the limiter never moves
//! audio against the clock anchors. The gain computer runs on every frame whether or not the
//! limiter is on (`enabled` only decides whether the gain is applied and reported), so turning it
//! on limits the frames already in the delay line too: `|y| <= c` from the toggle on. Every buffer is allocated in [`MasterStage::new`]; `process`
//! allocates nothing.
//!
//! The limiter, per frame `n` with `LA` lookahead frames and the linear ceiling `c`:
//! `req[n]` is 1 or `c / max(|xL|, |xR|)`; `m[n]` the minimum of `req` over `[n - LA, n]`;
//! `h[n]` an exponential release of `m` (an instant attack); `g[n]` the mean of `h` over
//! `[n - LA, n]` (a linear ramp over the lookahead), snapped to 1 within `GAIN_SNAP`; the
//! output `y[n] = x[n - LA] * g[n]`. Every `j` in `[k, k + LA]` has `k` in its minimum window, so
//! `g[k + LA] <= req[k]` and `|y| <= c`. The gain is also capped at `req[n - LA]`, which that
//! bound makes a no-op except for f64 rounding of the box mean and a snap within `GAIN_SNAP`
//! of an over: so `|y| <= c` holds exactly.

use serde::Serialize;
use strudeland_engine::{QUANTUM, SAMPLE_RATE_F64};

/// The limiter's ceiling (sample peak).
pub(crate) const CEILING_DB: f64 = -1.0;
/// The limiter's lookahead (the offline limiter's, redeclared: this crate does not depend on
/// `strudeland-render`).
pub(crate) const LOOKAHEAD_SEC: f64 = 0.0015;
pub(crate) const RELEASE_SEC: f64 = 0.050;
/// A gain within this of 1.0 becomes exactly 1.0.
pub(crate) const GAIN_SNAP: f64 = 1e-6;
/// The lookahead in engine frames: `round(LOOKAHEAD_SEC * 44100)`; the master's delay, which
/// the anchors subtract.
pub(crate) const MASTER_DELAY_FRAMES: usize = (LOOKAHEAD_SEC * SAMPLE_RATE_F64 + 0.5) as usize;

/// Levels of the master over every sample since the previous frame. Index 0 is left, 1 right.
/// Every field is finite (a non-finite sample counts as an over of peak 1.0).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelFrame {
    /// The largest |x| (linear).
    pub peak: [f32; 2],
    /// `sqrt(mean(x^2))` (linear).
    pub rms: [f32; 2],
    /// Samples with |x| >= 1.0.
    pub overs: [u32; 2],
    /// The lowest limiter gain in dB: 0 when the limiter is off or idle, else negative.
    pub limit_db: f32,
}

/// The level accumulator between two frames.
struct Accumulator {
    peak: [f32; 2],
    sum_sq: [f64; 2],
    overs: [u32; 2],
    frames: u64,
    min_gain: f64,
}

impl Accumulator {
    const EMPTY: Accumulator = Accumulator {
        peak: [0.0; 2],
        sum_sq: [0.0; 2],
        overs: [0; 2],
        frames: 0,
        min_gain: 1.0,
    };
}

/// The master stage on the engine thread: meter, lookahead delay line and limiter.
pub(crate) struct MasterStage {
    enabled: bool,
    /// The linear ceiling.
    ceiling: f64,
    /// The release coefficient per frame.
    release: f64,
    /// The last `MASTER_DELAY_FRAMES` input frames, interleaved; `delay_pos` is the oldest.
    delay: Box<[f32]>,
    delay_pos: usize,
    /// The sliding minimum: a monotonic deque of `(frame, req)` in a ring of `LA + 1` slots.
    deque: Box<[(u64, f64)]>,
    deque_head: usize,
    deque_len: usize,
    /// Frames seen since the last reset (the deque's frame index).
    frame: u64,
    /// The release envelope `h[n - 1]`.
    held: f64,
    /// The box filter over `h`: a ring of `LA + 1` and its running sum.
    boxed: Box<[f64]>,
    box_pos: usize,
    box_sum: f64,
    /// `req` of the last `LA + 1` frames, at `frame % (LA + 1)`: the gain is capped at the
    /// delayed frame's own requirement, which the algorithm guarantees up to f64 rounding.
    reqs: Box<[f64]>,
    acc: Accumulator,
}

impl MasterStage {
    pub(crate) fn new() -> MasterStage {
        let window = MASTER_DELAY_FRAMES + 1;
        MasterStage {
            enabled: false,
            ceiling: 10f64.powf(CEILING_DB / 20.0),
            release: (-1.0 / (RELEASE_SEC * SAMPLE_RATE_F64)).exp(),
            delay: vec![0.0; 2 * MASTER_DELAY_FRAMES].into_boxed_slice(),
            delay_pos: 0,
            deque: vec![(0, 1.0); window].into_boxed_slice(),
            deque_head: 0,
            deque_len: 0,
            frame: 0,
            held: 1.0,
            boxed: vec![1.0; window].into_boxed_slice(),
            box_pos: 0,
            box_sum: window as f64,
            reqs: vec![1.0; window].into_boxed_slice(),
            acc: Accumulator::EMPTY,
        }
    }

    /// Turns the limiter on or off. Nothing is reset: the gain computer runs on every frame
    /// either way, so the frames already in the delay line are limited from the toggle on.
    pub(crate) fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    /// A new stream: clears the delay line (the next `MASTER_DELAY_FRAMES` frames out are
    /// silence) and the limiter's state; keeps the enabled flag and the level accumulator.
    pub(crate) fn reset_stream(&mut self) {
        self.delay.fill(0.0);
        self.delay_pos = 0;
        self.reset_limiter();
    }

    fn reset_limiter(&mut self) {
        self.deque_head = 0;
        self.deque_len = 0;
        self.frame = 0;
        self.held = 1.0;
        self.boxed.fill(1.0);
        self.box_pos = 0;
        self.box_sum = self.boxed.len() as f64;
        self.reqs.fill(1.0);
    }

    /// The levels since the last call, and zeroes them; `None` when nothing was rendered.
    pub(crate) fn take_frame(&mut self) -> Option<LevelFrame> {
        let acc = std::mem::replace(&mut self.acc, Accumulator::EMPTY);
        if acc.frames == 0 {
            return None;
        }
        let n = acc.frames as f64;
        let rms = |ch: usize| (acc.sum_sq[ch] / n).sqrt() as f32;
        Some(LevelFrame {
            peak: acc.peak,
            rms: [rms(0), rms(1)],
            overs: acc.overs,
            limit_db: (20.0 * acc.min_gain.log10()) as f32,
        })
    }

    /// Measures the quantum, then replaces it with the delayed (and, when on, limited) quantum.
    pub(crate) fn process(&mut self, quantum: &mut [f32; 2 * QUANTUM]) {
        self.measure(quantum);
        for f in quantum.chunks_exact_mut(2) {
            let (l, r) = (f[0], f[1]);
            let pos = 2 * self.delay_pos;
            let (dl, dr) = (self.delay[pos], self.delay[pos + 1]);
            self.delay[pos] = l;
            self.delay[pos + 1] = r;
            self.delay_pos = (self.delay_pos + 1) % MASTER_DELAY_FRAMES;
            // the gain computer runs whether or not the limiter is on, so turning it on
            // finds the gain for the frames already in the delay line
            let g = self.gain(l, r);
            if !self.enabled {
                f[0] = dl;
                f[1] = dr;
                continue;
            }
            self.acc.min_gain = self.acc.min_gain.min(g);
            f[0] = apply(dl, g);
            f[1] = apply(dr, g);
        }
    }

    fn measure(&mut self, quantum: &[f32; 2 * QUANTUM]) {
        for f in quantum.chunks_exact(2) {
            for (ch, &x) in f.iter().enumerate() {
                let a = if x.is_finite() { x.abs() } else { 1.0 };
                self.acc.peak[ch] = self.acc.peak[ch].max(a);
                self.acc.sum_sq[ch] += f64::from(a) * f64::from(a);
                if a >= 1.0 {
                    self.acc.overs[ch] = self.acc.overs[ch].saturating_add(1);
                }
            }
        }
        self.acc.frames += QUANTUM as u64;
    }

    /// One frame of the limiter: the gain `g[n]` for input frame `(l, r)`.
    fn gain(&mut self, l: f32, r: f32) -> f64 {
        let finite = |x: f32| {
            if x.is_finite() {
                f64::from(x.abs())
            } else {
                0.0
            }
        };
        let peak = finite(l).max(finite(r));
        let req = if peak <= self.ceiling {
            1.0
        } else {
            self.ceiling / peak
        };
        let window = self.boxed.len();
        let slot = (self.frame % window as u64) as usize;
        self.reqs[slot] = req;
        // the frame output now, `n - LA`, sits in the slot after this one
        let delayed_req = self.reqs[(slot + 1) % window];
        let m = self.sliding_min(req);
        self.held = if m < self.held {
            m
        } else {
            m - (m - self.held) * self.release
        };
        // the box filter: replace the oldest h, refresh the sum exactly once per wrap
        self.box_sum += self.held - self.boxed[self.box_pos];
        self.boxed[self.box_pos] = self.held;
        self.box_pos = (self.box_pos + 1) % window;
        if self.box_pos == 0 {
            self.box_sum = self.boxed.iter().sum();
        }
        let g = self.box_sum / window as f64;
        let g = if 1.0 - g < GAIN_SNAP { 1.0 } else { g };
        g.min(delayed_req)
    }

    /// Pushes `req` for the current frame and returns the minimum over the last `LA + 1`.
    fn sliding_min(&mut self, req: f64) -> f64 {
        let cap = self.deque.len();
        let n = self.frame;
        self.frame += 1;
        while self.deque_len > 0 {
            let back = (self.deque_head + self.deque_len - 1) % cap;
            if self.deque[back].1 >= req {
                self.deque_len -= 1;
            } else {
                break;
            }
        }
        while self.deque_len > 0 && self.deque[self.deque_head].0 + (MASTER_DELAY_FRAMES as u64) < n
        {
            self.deque_head = (self.deque_head + 1) % cap;
            self.deque_len -= 1;
        }
        // at most LA + 1 entries: every one left is within the window [n - LA, n]
        let at = (self.deque_head + self.deque_len) % cap;
        self.deque[at] = (n, req);
        self.deque_len += 1;
        self.deque[self.deque_head].1
    }
}

/// `x * g` in f64, rounded to f32 (`x` bit for bit at `g = 1`); a non-finite `x` is 0.
fn apply(x: f32, g: f64) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    if g == 1.0 {
        return x;
    }
    (f64::from(x) * g) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const LA: usize = MASTER_DELAY_FRAMES;

    /// Runs `input` (interleaved stereo, padded to whole quanta) through `stage` and returns
    /// the output, `extra` quanta of silence after it included.
    fn run(stage: &mut MasterStage, input: &[f32], extra: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let mut q = [0.0f32; 2 * QUANTUM];
        let quanta = input.len().div_ceil(2 * QUANTUM) + extra;
        for k in 0..quanta {
            q.fill(0.0);
            let from = (k * 2 * QUANTUM).min(input.len());
            let to = ((k + 1) * 2 * QUANTUM).min(input.len());
            q[..to - from].copy_from_slice(&input[from..to]);
            stage.process(&mut q);
            out.extend_from_slice(&q);
        }
        out
    }

    fn stereo(frames: usize, f: impl Fn(usize) -> (f32, f32)) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let (l, r) = f(i);
                [l, r]
            })
            .collect()
    }

    fn sine(frames: usize, amp: f32) -> Vec<f32> {
        stereo(frames, |i| {
            let v = amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 44100.0).sin();
            (v, -0.5 * v)
        })
    }

    fn ceiling() -> f32 {
        10f64.powf(CEILING_DB / 20.0) as f32
    }

    #[test]
    fn the_constants() {
        assert_eq!(
            MASTER_DELAY_FRAMES,
            (LOOKAHEAD_SEC * SAMPLE_RATE_F64).round() as usize
        );
        assert_eq!(MASTER_DELAY_FRAMES, 66);
        assert!((MasterStage::new().ceiling - 0.891251).abs() < 1e-6);
    }

    #[test]
    fn a_frame_measures_peak_rms_and_overs() {
        let mut m = MasterStage::new();
        // left alternates 0.5 and -1.0; right is a constant 0.25
        let mut q = [0.0f32; 2 * QUANTUM];
        for (i, f) in q.chunks_exact_mut(2).enumerate() {
            f[0] = if i % 2 == 0 { 0.5 } else { -1.0 };
            f[1] = 0.25;
        }
        m.process(&mut q);
        let f = m.take_frame().expect("a frame");
        assert_eq!(f.peak, [1.0, 0.25]);
        let want_l = ((0.25 + 1.0) / 2.0f64).sqrt() as f32;
        assert_eq!(f.rms, [want_l, 0.25]);
        assert_eq!(f.overs, [QUANTUM as u32 / 2, 0]);
        assert_eq!(f.limit_db, 0.0);
    }

    #[test]
    fn a_frame_covers_every_quantum_since_the_last() {
        let mut m = MasterStage::new();
        let mut q = [0.0f32; 2 * QUANTUM];
        q[0] = 0.5; // first quantum: one loud left sample
        m.process(&mut q);
        let mut q = [0.0f32; 2 * QUANTUM];
        q[1] = 2.0; // second quantum: one over on the right
        m.process(&mut q);
        let f = m.take_frame().unwrap();
        assert_eq!(f.peak, [0.5, 2.0]);
        assert_eq!(f.overs, [0, 1]);
        let n = (2 * QUANTUM) as f64;
        assert_eq!(f.rms, [(0.25 / n).sqrt() as f32, (4.0 / n).sqrt() as f32]);
    }

    #[test]
    fn identical_channels_give_identical_levels() {
        // the engine always renders stereo; a mono mix has the same values in both
        let mut m = MasterStage::new();
        let x = stereo(QUANTUM, |i| {
            let v = (i as f32 / 40.0).sin();
            (v, v)
        });
        run(&mut m, &x, 0);
        let f = m.take_frame().unwrap();
        assert_eq!(f.peak[0], f.peak[1]);
        assert_eq!(f.rms[0], f.rms[1]);
        assert_eq!(f.overs[0], f.overs[1]);
    }

    #[test]
    fn silence_is_a_frame_of_zeros() {
        let mut m = MasterStage::new();
        let mut q = [0.0f32; 2 * QUANTUM];
        m.process(&mut q);
        assert_eq!(
            m.take_frame(),
            Some(LevelFrame {
                peak: [0.0; 2],
                rms: [0.0; 2],
                overs: [0; 2],
                limit_db: 0.0
            })
        );
    }

    #[test]
    fn non_finite_samples_are_overs_of_peak_one() {
        for on in [false, true] {
            let mut m = MasterStage::new();
            m.set_enabled(on);
            let mut q = [0.0f32; 2 * QUANTUM];
            q[0] = f32::NAN;
            q[1] = f32::INFINITY;
            q[3] = f32::NEG_INFINITY;
            m.process(&mut q);
            let f = m.take_frame().unwrap();
            assert_eq!(f.peak, [1.0, 1.0]);
            assert_eq!(f.overs, [1, 2]);
            let n = QUANTUM as f32;
            assert_eq!(f.rms, [(1.0 / n).sqrt(), (2.0 / n).sqrt()]);
            assert!(f.limit_db.is_finite());
            let v = serde_json::to_value(f).unwrap();
            assert!(!v.to_string().contains("null"), "{v}");
        }
    }

    #[test]
    fn take_frame_drains_and_is_none_when_nothing_rendered() {
        let mut m = MasterStage::new();
        assert_eq!(m.take_frame(), None);
        let mut q = [0.5f32; 2 * QUANTUM];
        m.process(&mut q);
        assert!(m.take_frame().is_some());
        assert_eq!(m.take_frame(), None);
    }

    #[test]
    fn levels_are_measured_before_the_limiter() {
        let mut m = MasterStage::new();
        m.set_enabled(true);
        run(&mut m, &sine(4 * QUANTUM, 2.0), 0);
        let f = m.take_frame().unwrap();
        assert!(f.peak[0] > 1.99, "{f:?}");
        assert!(f.overs[0] > 0);
        assert!(f.limit_db < -6.0, "{f:?}");
    }

    #[test]
    fn below_the_ceiling_the_output_is_the_input_delayed_bit_for_bit() {
        for on in [false, true] {
            let mut m = MasterStage::new();
            m.set_enabled(on);
            let x = sine(10 * QUANTUM, 0.8);
            let y = run(&mut m, &x, 1);
            assert!(y[..2 * LA].iter().all(|&s| s == 0.0), "silence first");
            assert_eq!(&y[2 * LA..2 * LA + x.len()], &x[..], "limiter {on}");
            assert!(m.take_frame().unwrap().limit_db == 0.0);
        }
    }

    #[test]
    fn disabled_passes_a_nan_through() {
        let mut m = MasterStage::new();
        let mut x = vec![0.0f32; 2 * QUANTUM];
        x[10] = f32::NAN;
        let y = run(&mut m, &x, 1);
        assert!(y[10 + 2 * LA].is_nan());
    }

    #[test]
    fn enabled_replaces_a_nan_with_zero_and_keeps_the_gain_finite() {
        let mut m = MasterStage::new();
        m.set_enabled(true);
        let mut x = sine(4 * QUANTUM, 0.5);
        x[10] = f32::NAN;
        let y = run(&mut m, &x, 1);
        assert_eq!(y[10 + 2 * LA], 0.0);
        assert!(y.iter().all(|s| s.is_finite()));
        // the rest passes unchanged: the NaN asked for no gain reduction
        assert_eq!(y[12 + 2 * LA], x[12]);
    }

    #[test]
    fn enabled_output_never_exceeds_the_ceiling() {
        let c = ceiling();
        let loud = sine(40 * QUANTUM, 4.0);
        let mut impulse = vec![0.0f32; 40 * QUANTUM];
        impulse[2 * 1000] = 8.0;
        impulse[2 * 1001 + 1] = -3.0;
        impulse[2 * 1500] = 1.2;
        for x in [loud, impulse] {
            let mut m = MasterStage::new();
            m.set_enabled(true);
            let y = run(&mut m, &x, 2);
            let peak = y.iter().fold(0.0f32, |p, s| p.max(s.abs()));
            assert!(peak <= c, "peak {peak} over {c}");
            assert!(peak > 0.5 * c, "peak {peak}");
        }
    }

    #[test]
    fn the_gain_stays_under_every_frames_requirement() {
        // g[k + LA] <= req[k] for every k, on a signal with sudden peaks
        let c = 10f64.powf(CEILING_DB / 20.0);
        let x = stereo(30 * QUANTUM, |i| {
            let v = 0.6 * (i as f32 / 7.0).sin() + if i % 397 == 0 { 3.0 } else { 0.0 };
            (v, 0.2 * v)
        });
        let mut m = MasterStage::new();
        m.set_enabled(true);
        let gains: Vec<f64> = x.chunks_exact(2).map(|f| m.gain(f[0], f[1])).collect();
        for k in 0..gains.len() - LA {
            let p = f64::from(x[2 * k].abs().max(x[2 * k + 1].abs()));
            let req = if p <= c { 1.0 } else { c / p };
            assert!(gains[k + LA] <= req, "frame {k}: {} > {req}", gains[k + LA]);
        }
    }

    #[test]
    fn the_sliding_minimum_covers_the_last_la_plus_one_frames() {
        let mut m = MasterStage::new();
        let reqs: Vec<f64> = (0..500)
            .map(|i| 0.5 + 0.5 * ((i * 37) % 101) as f64 / 101.0)
            .collect();
        for (n, &r) in reqs.iter().enumerate() {
            let got = m.sliding_min(r);
            let from = n.saturating_sub(LA);
            let want = reqs[from..=n].iter().copied().fold(f64::INFINITY, f64::min);
            assert_eq!(got, want, "frame {n}");
        }
    }

    #[test]
    fn the_release_is_monotone_after_a_peak_and_returns_to_exactly_one() {
        let mut m = MasterStage::new();
        m.set_enabled(true);
        let mut gains = Vec::new();
        gains.push(m.gain(4.0, 0.0));
        for _ in 0..44100 {
            gains.push(m.gain(0.1, 0.1));
        }
        let lowest = gains
            .iter()
            .enumerate()
            .fold((0, 1.0), |b, (i, &g)| if g < b.1 { (i, g) } else { b });
        assert_eq!(
            lowest.0, LA,
            "the ramp reaches its floor after the lookahead"
        );
        assert!(lowest.1 < 0.25);
        for w in gains[LA..].windows(2) {
            assert!(w[1] >= w[0], "{} then {}", w[0], w[1]);
        }
        assert_eq!(*gains.last().unwrap(), 1.0);
        // the attack is a ramp, not a step
        for w in gains[..=LA].windows(2) {
            assert!(w[0] - w[1] < 0.02, "{} then {}", w[0], w[1]);
        }
    }

    #[test]
    fn the_gain_snaps_to_one() {
        let mut m = MasterStage::new();
        m.set_enabled(true);
        // just over the ceiling: the box mean comes within GAIN_SNAP of 1 and snaps, except
        // at the over's own output frame, which the cap holds to its requirement
        let c = 10f64.powf(CEILING_DB / 20.0);
        let tiny = (c * (1.0 + 5e-7)) as f32;
        assert!(f64::from(tiny) > c);
        let req = c / f64::from(tiny);
        assert!(1.0 - req < GAIN_SNAP);
        let gains: Vec<f64> = (0..300)
            .map(|i| m.gain(if i == 0 { tiny } else { 0.0 }, 0.0))
            .collect();
        for (n, &g) in gains.iter().enumerate() {
            let want = if n == LA { req } else { 1.0 };
            assert_eq!(g, want, "frame {n}");
        }
    }

    #[test]
    fn toggling_does_not_move_audio() {
        let x = stereo(12 * QUANTUM, |i| {
            if i % 300 == 5 {
                (0.5, -0.5)
            } else {
                (0.0, 0.0)
            }
        });
        let mut plain = MasterStage::new();
        let want = run(&mut plain, &x, 1);
        let mut toggled = MasterStage::new();
        let mut got = Vec::new();
        let mut q = [0.0f32; 2 * QUANTUM];
        for (k, chunk) in x.chunks(2 * QUANTUM).chain([&[][..]]).enumerate() {
            toggled.set_enabled(k % 2 == 1);
            q.fill(0.0);
            q[..chunk.len()].copy_from_slice(chunk);
            toggled.process(&mut q);
            got.extend_from_slice(&q);
        }
        assert_eq!(got, want);
    }

    #[test]
    fn enabling_mid_stream_limits_from_the_toggle_on() {
        // a loud steady signal, the limiter turned on mid-quantum-stream: the frames already
        // in the delay line went in unlimited, yet every sample out from the toggle on is
        // under the ceiling
        let c = ceiling();
        let x = sine(20 * QUANTUM, 3.0);
        for at in [1, 5, 9] {
            let mut m = MasterStage::new();
            let mut q = [0.0f32; 2 * QUANTUM];
            let mut after = Vec::new();
            for (k, chunk) in x.chunks(2 * QUANTUM).enumerate() {
                if k == at {
                    m.set_enabled(true);
                }
                q.copy_from_slice(chunk);
                m.process(&mut q);
                if k >= at {
                    after.extend_from_slice(&q);
                } else {
                    assert!(q.iter().any(|s| s.abs() > c), "unlimited before the toggle");
                }
            }
            let peak = after.iter().fold(0.0f32, |p, s| p.max(s.abs()));
            assert!(peak <= c, "toggle at quantum {at}: peak {peak} over {c}");
            assert!(peak > 0.5 * c, "peak {peak}");
        }
    }

    #[test]
    fn disabled_output_is_the_delayed_input_even_over_the_ceiling() {
        let x = sine(10 * QUANTUM, 3.0);
        let mut m = MasterStage::new();
        let y = run(&mut m, &x, 1);
        assert_eq!(&y[2 * LA..2 * LA + x.len()], &x[..]);
        assert_eq!(
            m.take_frame().unwrap().limit_db,
            0.0,
            "off reports no limiting"
        );
    }

    #[test]
    fn reset_stream_clears_the_delay_line_and_keeps_the_rest() {
        let mut m = MasterStage::new();
        m.set_enabled(true);
        let mut q = [0.5f32; 2 * QUANTUM];
        m.process(&mut q);
        m.reset_stream();
        let mut q = [0.0f32; 2 * QUANTUM];
        m.process(&mut q);
        assert!(q.iter().all(|&s| s == 0.0), "the delayed frames are gone");
        assert!(m.enabled);
        let f = m.take_frame().unwrap();
        assert_eq!(f.peak, [0.5, 0.5], "the accumulator survives");
    }

    #[test]
    fn process_allocates_nothing() {
        let mut m = MasterStage::new();
        m.set_enabled(true);
        let x = sine(QUANTUM, 3.0);
        let mut q = [0.0f32; 2 * QUANTUM];
        q.copy_from_slice(&x);
        let n = alloc_count::count_in(|| {
            for _ in 0..200 {
                let mut r = q;
                m.process(&mut r);
            }
            m.set_enabled(false);
            m.process(&mut q.clone());
            m.reset_stream();
        });
        assert_eq!(n, 0);
        // the counter works
        assert!(alloc_count::count_in(|| drop(std::hint::black_box(vec![1u8; 64]))) > 0);
    }

    /// A counting global allocator for this crate's unit tests (the `tests/rt_alloc.rs`
    /// pattern): counts only on the thread inside `count_in`.
    mod alloc_count {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::cell::Cell;

        thread_local! {
            static ON: Cell<bool> = const { Cell::new(false) };
            static COUNT: Cell<u64> = const { Cell::new(0) };
        }

        struct Counting;

        fn count() {
            if ON.try_with(Cell::get).unwrap_or(false) {
                let _ = COUNT.try_with(|c| c.set(c.get() + 1));
            }
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

        /// Allocations (and frees) made by `f` on this thread.
        pub(super) fn count_in(f: impl FnOnce()) -> u64 {
            COUNT.with(|c| c.set(0));
            ON.with(|o| o.set(true));
            f();
            ON.with(|o| o.set(false));
            COUNT.with(Cell::get)
        }
    }
}
