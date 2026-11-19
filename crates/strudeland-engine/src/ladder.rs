//! superdough's ladder filter worklet (`LadderProcessor`, worklets.mjs:366-428; built by
//! `createFilter` in helpers.mjs:234-305 in place of a biquad when the raw `ftype` is the string
//! `'ladder'`): a 4-pole cascade of `fast_tanh` one-pole stages with resonance feedback taken
//! from a 4-tap FIR of the last pole's history (one sample of latency), a `drive` gain
//! `clamp(exp(drive), 0.1, 2000)` on the input and a `1 / drive` makeup.
//!
//! Every slot (lowpass, highpass, bandpass) builds the same lowpass processor: the processor has
//! no type. `frequency`, `q` and `drive` are full-range a-rate params read once per quantum at
//! its first frame (`parameters.x[0]`), so a filter envelope on `frequency` is a 128-frame
//! staircase. The state is a pair of doubles per pole (one per channel). The processor finishes
//! (`return false`) in the first quantum its input has no connection after it started; the ring
//! it had is cut there (`worklet::first_absent_after_stop` and the chain's teardown walk).
//!
//! The filter descends from Magnus Jonsson's Microtracker model (Unlicense) via
//! The BouteillacBear's webaudioworklet-wasm (MIT, copyright 2023 The BouteillacBear)
//! and Strudel's JavaScript adaptation. See the root third-party notices.

use crate::QUANTUM;
use crate::jsmath;
use crate::param::{Param, ParamInputs, Rate};
use crate::voice::zero;
use crate::worklet::{self, INVSR, Lifecycle};

/// worklets.mjs `TWO_PI = 2 * Math.PI`.
const TWO_PI: f64 = 2.0 * std::f64::consts::PI;

/// The `frequency` descriptor's default.
pub const DEFAULT_FREQUENCY: f32 = 500.0;
/// The `q` descriptor's default (also `createFilter`'s `q = 1`).
pub const DEFAULT_Q: f32 = 1.0;
/// The `drive` descriptor's default (also `createFilter`'s `drive = 0.69`).
pub const DEFAULT_DRIVE: f64 = 0.69;

/// worklets.mjs `fast_tanh(x)`: `x2 = x ** 2`, `(x * (27 + x2)) / (27 + 9 * x2)`.
pub fn fast_tanh(x: f64) -> f64 {
    let x2 = jsmath::pow(x, 2.0);
    (x * (27.0 + x2)) / (27.0 + 9.0 * x2)
}

/// The per-quantum coefficients of one `process()` call from the params' first frames:
/// `(cutoff, k, drive, makeup)`.
pub fn coefficients(frequency: f32, q: f32, drive: f32) -> (f64, f64, f64, f64) {
    let resonance = f64::from(q);
    let drive = jsmath::clamp(jsmath::exp(f64::from(drive)), 0.1, 2000.0);
    let cutoff = f64::from(frequency) * TWO_PI * INVSR;
    // `cutoff > 1 ? 1 : cutoff`: no lower clamp, and NaN stays NaN.
    let cutoff = if cutoff > 1.0 { 1.0 } else { cutoff };
    let k = jsmath::min(8.0, resonance * 0.13);
    let makeup = (1.0 / drive) * jsmath::min(1.75, 1.0 + k);
    (cutoff, k, drive, makeup)
}

/// `LadderProcessor`'s params and state.
#[derive(Debug, Clone)]
pub struct Ladder {
    pub frequency: Param,
    pub q: Param,
    pub drive: Param,
    started: bool,
    p0: [f64; 2],
    p1: [f64; 2],
    p2: [f64; 2],
    p3: [f64; 2],
    p32: [f64; 2],
    p33: [f64; 2],
    p34: [f64; 2],
}

impl Default for Ladder {
    fn default() -> Self {
        Ladder::new()
    }
}

impl Ladder {
    /// A new processor: the three full-range a-rate params at their defaults, the state zero.
    pub fn new() -> Ladder {
        Ladder {
            frequency: worklet::param(DEFAULT_FREQUENCY, Rate::A),
            q: worklet::param(DEFAULT_Q, Rate::A),
            drive: worklet::param(DEFAULT_DRIVE as f32, Rate::A),
            started: false,
            p0: [0.0; 2],
            p1: [0.0; 2],
            p2: [0.0; 2],
            p3: [0.0; 2],
            p32: [0.0; 2],
            p33: [0.0; 2],
            p34: [0.0; 2],
        }
    }

    /// One `process()` call in place on `io` (one or two channels; none when the input has no
    /// connection) with the quantum's param blocks. Returns the JS return value: `false` once the
    /// input is gone after it started (nothing written), else `true`.
    pub fn process(
        &mut self,
        io: &mut [[f32; QUANTUM]],
        frequency: &[f32; QUANTUM],
        q: &[f32; QUANTUM],
        drive: &[f32; QUANTUM],
    ) -> bool {
        let has_input = !io.is_empty();
        if self.started && !has_input {
            return false;
        }
        self.started = has_input;
        let (cutoff, k, drive, makeup) = coefficients(frequency[0], q[0], drive[0]);
        for n in 0..QUANTUM {
            for (i, ch) in io.iter_mut().enumerate() {
                let out = ((self.p3[i] * 0.360891 + self.p32[i] * 0.41729)
                    + self.p33[i] * 0.177896)
                    + self.p34[i] * 0.0439725;
                self.p34[i] = self.p33[i];
                self.p33[i] = self.p32[i];
                self.p32[i] = self.p3[i];
                let x = f64::from(ch[n]);
                self.p0[i] += (fast_tanh(x * drive - k * out) - fast_tanh(self.p0[i])) * cutoff;
                self.p1[i] += (fast_tanh(self.p0[i]) - fast_tanh(self.p1[i])) * cutoff;
                self.p2[i] += (fast_tanh(self.p1[i]) - fast_tanh(self.p2[i])) * cutoff;
                self.p3[i] += (fast_tanh(self.p2[i]) - fast_tanh(self.p3[i])) * cutoff;
                ch[n] = (out * makeup) as f32;
            }
        }
        true
    }

    /// Whether the processor has seen a connected input (its `started` flag).
    #[cfg(test)]
    pub fn started(&self) -> bool {
        self.started
    }
}

/// The ladder's `AudioWorkletNode`: the processor and its lifecycle.
#[derive(Debug, Clone, Default)]
pub struct LadderNode {
    pub proc: Ladder,
    life: Lifecycle,
}

impl LadderNode {
    pub fn new(proc: Ladder) -> LadderNode {
        LadderNode {
            proc,
            life: Lifecycle::new(),
        }
    }

    /// One quantum in place on the voice bus's channels; `connected` is whether the input has a
    /// rendering connection this quantum (the chain's teardown walk), `frequency` the
    /// connections into the `frequency` param (a filter LFO). Returns the output's silence hint:
    /// false while the processor lives (also in the quantum it returns `false`, which outputs
    /// zeros), true with zeros once it finished.
    #[cfg(test)]
    pub fn process(
        &mut self,
        q: u64,
        io: &mut [[f32; QUANTUM]],
        connected: bool,
        frequency: &ParamInputs,
    ) -> bool {
        self.process_with(
            q,
            io,
            connected,
            [frequency, &ParamInputs::NONE, &ParamInputs::NONE],
        )
    }

    /// [`LadderNode::process`] with the connections into `frequency`, `q` and `drive`.
    pub fn process_with(
        &mut self,
        q: u64,
        io: &mut [[f32; QUANTUM]],
        connected: bool,
        [frequency, q_in, drive]: [&ParamInputs; 3],
    ) -> bool {
        if !self.life.begin_quantum(connected) {
            zero(io);
            return true;
        }
        let block = |p: &mut Param, inputs: &ParamInputs| {
            let mut b = [0.0f32; QUANTUM];
            worklet::fill_block(p, q, inputs, &mut b);
            b
        };
        let f = block(&mut self.proc.frequency, frequency);
        let qb = block(&mut self.proc.q, q_in);
        let d = block(&mut self.proc.drive, drive);
        let r = if connected {
            self.proc.process(io, &f, &qb, &d)
        } else {
            zero(io);
            self.proc.process(&mut [], &f, &qb, &d)
        };
        if !r {
            // `return false` writes nothing: the zeroed output arrays.
            zero(io);
        }
        self.life.finish_quantum(Ok(r), connected);
        false
    }

    /// Whether the processor has finished (its node outputs a silent zero bus).
    #[cfg(test)]
    pub fn gone(&self) -> bool {
        self.life.gone()
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    const Q: usize = QUANTUM;

    fn bits(x: &[f32]) -> Vec<u32> {
        x.iter().map(|v| v.to_bits()).collect()
    }

    fn saw(phase: f32) -> [f32; Q] {
        let mut b = [0.0f32; Q];
        for (i, x) in b.iter_mut().enumerate() {
            *x = 0.3 * (((i as f32 * 0.013 + phase).fract()) * 2.0 - 1.0);
        }
        b
    }

    /// The JS loop transcribed literally at `f64` for one channel (an independent reference):
    /// returns the outputs and the state `[p0, p1, p2, p3, p32, p33, p34]` after it.
    fn reference(
        input: &[f32; Q],
        frequency: f64,
        q: f64,
        drive_param: f64,
        state: [f64; 7],
    ) -> ([f32; Q], [f64; 7]) {
        let ft = |x: f64| {
            let x2 = x * x;
            (x * (27.0 + x2)) / (27.0 + 9.0 * x2)
        };
        let [mut p0, mut p1, mut p2, mut p3, mut p32, mut p33, mut p34] = state;
        let drive = jsmath::clamp(jsmath::exp(drive_param), 0.1, 2000.0);
        let mut cutoff = frequency * (2.0 * std::f64::consts::PI) * (1.0 / 44100.0);
        if cutoff > 1.0 {
            cutoff = 1.0;
        }
        let k = jsmath::min(8.0, q * 0.13);
        let makeup = (1.0 / drive) * jsmath::min(1.75, 1.0 + k);
        let mut out = [0.0f32; Q];
        for n in 0..Q {
            let o = p3 * 0.360891 + p32 * 0.41729 + p33 * 0.177896 + p34 * 0.0439725;
            p34 = p33;
            p33 = p32;
            p32 = p3;
            p0 += (ft(f64::from(input[n]) * drive - k * o) - ft(p0)) * cutoff;
            p1 += (ft(p0) - ft(p1)) * cutoff;
            p2 += (ft(p1) - ft(p2)) * cutoff;
            p3 += (ft(p2) - ft(p3)) * cutoff;
            out[n] = (o * makeup) as f32;
        }
        (out, [p0, p1, p2, p3, p32, p33, p34])
    }

    fn state(l: &Ladder, ch: usize) -> [f64; 7] {
        [
            l.p0[ch], l.p1[ch], l.p2[ch], l.p3[ch], l.p32[ch], l.p33[ch], l.p34[ch],
        ]
    }

    fn blocks(f: f32, q: f32, d: f32) -> ([f32; Q], [f32; Q], [f32; Q]) {
        ([f; Q], [q; Q], [d; Q])
    }

    #[test]
    fn defaults_and_constants() {
        let l = Ladder::new();
        for (p, d) in [(&l.frequency, 500.0f32), (&l.q, 1.0), (&l.drive, 0.69)] {
            assert_eq!(p.default_value().to_bits(), d.to_bits());
            assert_eq!((p.min_value(), p.max_value()), (-f32::MAX, f32::MAX));
            assert_eq!(p.rate(), Rate::A);
            assert!(p.events().is_empty());
        }
        assert!(!l.started());
        assert_eq!(TWO_PI.to_bits(), (2.0 * std::f64::consts::PI).to_bits());
    }

    #[test]
    fn fast_tanh_matches_the_js_definition() {
        for x in [
            0.0, -0.0, 0.5, -0.5, 1.0, 3.0, -3.0, 1e-300, 1e10, 1e200, -1e200,
        ] {
            let x2 = x * x;
            let want = (x * (27.0 + x2)) / (27.0 + 9.0 * x2);
            let got = fast_tanh(x);
            assert!(
                got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()),
                "x {x}: {got} vs {want}"
            );
        }
        // -0 keeps its sign; huge inputs overflow `x2` to infinity: inf / inf is NaN.
        assert_eq!(fast_tanh(-0.0).to_bits(), (-0.0f64).to_bits());
        assert!(fast_tanh(1e200).is_nan());
        assert!(fast_tanh(f64::NAN).is_nan());
    }

    #[test]
    fn one_impulse_against_a_literal_transcription() {
        let mut l = Ladder::new();
        let mut input = [0.0f32; Q];
        input[0] = 1.0;
        input[40] = -0.5;
        let mut io = [input];
        let (f, q, d) = blocks(900.0, 8.0, 0.69);
        assert!(l.process(&mut io, &f, &q, &d));
        let (want, st) = reference(&input, 900.0, 8.0, f64::from(0.69f32), [0.0; 7]);
        assert_eq!(bits(&io[0]), bits(&want));
        for (a, b) in state(&l, 0).iter().zip(st.iter()) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
        // A second quantum continues from the state.
        let input2 = saw(0.2);
        let mut io = [input2];
        l.process(&mut io, &f, &q, &d);
        let (want2, _) = reference(&input2, 900.0, 8.0, f64::from(0.69f32), st);
        assert_eq!(bits(&io[0]), bits(&want2));
    }

    #[test]
    fn one_sample_latency() {
        // The first output frame is computed from the zero history: 0, whatever the input.
        let mut l = Ladder::new();
        let mut io = [[0.9f32; Q]];
        let (f, q, d) = blocks(20000.0, 0.0, 0.0);
        l.process(&mut io, &f, &q, &d);
        assert_eq!(io[0][0].to_bits(), 0);
        // Frame 1 reads p3 after one update (non-zero with the cutoff clamped to 1).
        assert_ne!(io[0][1], 0.0);
        // The feedback of frame n uses that frame's `out`: k = 0 makes it independent of q.
        let (want, _) = reference(&[0.9; Q], 20000.0, 0.0, 0.0, [0.0; 7]);
        assert_eq!(bits(&io[0]), bits(&want));
    }

    #[test]
    fn cutoff_clamps_above_one_only() {
        // 20 kHz: 2 pi 20000 / 44100 = 2.85 > 1, clamped.
        let (c, ..) = coefficients(20000.0, 1.0, 0.69);
        assert_eq!(c, 1.0);
        // Just below 1: kept (`> 1` only; 7000 Hz gives 0.9973).
        let (c, ..) = coefficients(7000.0, 1.0, 0.69);
        assert_eq!(c.to_bits(), (7000.0f64 * TWO_PI * INVSR).to_bits());
        assert!(c < 1.0);
        // A negative frequency gives a negative cutoff, still finite for a small input.
        let (c, ..) = coefficients(-1000.0, 1.0, 0.69);
        assert_eq!(
            c.to_bits(),
            (-1000.0f64 * TWO_PI * INVSR).to_bits(),
            "no lower clamp"
        );
        let mut l = Ladder::new();
        let mut io = [saw(0.0)];
        let (f, q, d) = blocks(-1000.0, 1.0, 0.69);
        l.process(&mut io, &f, &q, &d);
        assert!(io[0].iter().all(|x| x.is_finite()));
        // NaN frequency: the cutoff stays NaN (`NaN > 1` is false).
        let (c, ..) = coefficients(f32::NAN, 1.0, 0.69);
        assert!(c.is_nan());
    }

    #[test]
    fn drive_clamp_and_makeup() {
        // exp(0.69) inside the range; makeup 1/drive * min(1.75, 1 + k).
        let (_, k, d, m) = coefficients(500.0, 1.0, 0.69);
        let want_d = jsmath::exp(f64::from(0.69f32));
        assert_eq!(d.to_bits(), want_d.to_bits());
        assert_eq!(k.to_bits(), 0.13f64.to_bits());
        assert_eq!(m.to_bits(), ((1.0 / want_d) * (1.0 + k)).to_bits());
        // drive = 100: exp(100) clamps to 2000; drive = -100: exp(-100) clamps to 0.1.
        let (_, k, d, m) = coefficients(500.0, 1.0, 100.0);
        assert_eq!(d, 2000.0);
        assert_eq!(m.to_bits(), ((1.0 / 2000.0) * (1.0 + k)).to_bits());
        let (_, _, d, _) = coefficients(500.0, 1.0, -100.0);
        assert_eq!(d, 0.1);
        // drive 3 (ladder-drive): exp(3) = 20.08, makeup 1/20.08.
        let (_, _, d, _) = coefficients(500.0, 0.0, 3.0);
        assert_eq!(d.to_bits(), jsmath::exp(3.0).to_bits());
        // A NaN drive is NaN through the clamp.
        let (_, _, d, m) = coefficients(500.0, 1.0, f32::NAN);
        assert!(d.is_nan() && m.is_nan());
    }

    #[test]
    fn k_and_makeup_clamps() {
        // q 61.5 gives k = 7.995 < 8; q = 100 and q = 1e6 give k = 8.
        let (_, k, ..) = coefficients(500.0, 61.5, 0.69);
        assert_eq!(k.to_bits(), (61.5f64 * 0.13).to_bits());
        assert!(k < 8.0);
        for q in [100.0f32, 1e6, f32::MAX] {
            let (_, k, _, m) = coefficients(500.0, q, 0.0);
            assert_eq!(k, 8.0, "q {q}");
            assert_eq!(m, 1.75, "q {q}: min(1.75, 9)");
        }
        // A negative q gives a negative k (more feedback sign flip), makeup below 1.
        let (_, k, _, m) = coefficients(500.0, -10.0, 0.0);
        assert_eq!(k.to_bits(), (-10.0f64 * 0.13).to_bits());
        assert_eq!(m.to_bits(), (1.0 + k).to_bits());
        // q = 1e6 renders without panicking and stays finite on a bounded input.
        let mut l = Ladder::new();
        for k in 0..20 {
            let mut io = [saw(k as f32 * 0.1)];
            let (f, q, d) = blocks(900.0, 1e6, 0.69);
            l.process(&mut io, &f, &q, &d);
            assert!(io[0].iter().all(|x| x.is_finite()), "quantum {k}");
        }
    }

    #[test]
    fn the_params_are_read_at_each_quantums_first_frame() {
        // A block whose frames differ: only frame 0 counts.
        let mut a = Ladder::new();
        let mut b = Ladder::new();
        let mut f = [900.0f32; Q];
        for (i, x) in f.iter_mut().enumerate().skip(1) {
            *x = 100.0 + i as f32;
        }
        let input = saw(0.3);
        let (mut ia, mut ib) = ([input], [input]);
        a.process(&mut ia, &f, &[2.0; Q], &[0.5; Q]);
        b.process(&mut ib, &[900.0; Q], &[2.0; Q], &[0.5; Q]);
        assert_eq!(bits(&ia[0]), bits(&ib[0]));
    }

    #[test]
    fn started_rule() {
        // A first quantum with no input does not set `started` and returns true.
        let mut l = Ladder::new();
        let (f, q, d) = blocks(500.0, 1.0, 0.69);
        assert!(l.process(&mut [], &f, &q, &d));
        assert!(!l.started());
        assert!(l.process(&mut [], &f, &q, &d));
        // With input it starts; then a missing input returns false without touching the state.
        let mut io = [saw(0.0)];
        assert!(l.process(&mut io, &f, &q, &d));
        assert!(l.started());
        let before = state(&l, 0);
        assert!(!l.process(&mut [], &f, &q, &d));
        assert_eq!(state(&l, 0), before);
    }

    #[test]
    fn stereo_channels_are_independent() {
        let mut l = Ladder::new();
        let mut m0 = Ladder::new();
        let mut m1 = Ladder::new();
        let (f, q, d) = blocks(700.0, 20.0, 1.5);
        for k in 0..4 {
            let (a, b) = (saw(k as f32 * 0.2), saw(0.5 + k as f32 * 0.3));
            let mut io = [a, b];
            l.process(&mut io, &f, &q, &d);
            let (mut x, mut y) = ([a], [b]);
            m0.process(&mut x, &f, &q, &d);
            m1.process(&mut y, &f, &q, &d);
            assert_eq!(bits(&io[0]), bits(&x[0]), "left, quantum {k}");
            assert_eq!(bits(&io[1]), bits(&y[0]), "right, quantum {k}");
        }
    }

    #[test]
    fn node_teardown_at_the_first_absent_quantum() {
        // A self-oscillating ladder (k = 5.2) rings on zeros; the quantum the input goes, the
        // output is zero (non-silent hint: the processor returned false in it), then zero and
        // silent for good.
        let mut n = LadderNode::new(Ladder::new());
        n.proc.frequency.set_value(300.0, 0).unwrap();
        n.proc.q.set_value(40.0, 0).unwrap();
        let cut = worklet::first_absent_after_stop(13671);
        assert_eq!(cut, 13696);
        let mut q = 0u64;
        while q < cut {
            let mut io = if q < 13568 {
                [saw(q as f32)]
            } else {
                [[0.0; Q]]
            };
            assert!(
                !n.process(q, &mut io, true, &ParamInputs::NONE),
                "live at {q}"
            );
            if q == cut - 128 {
                assert!(
                    io[0].iter().any(|x| *x != 0.0),
                    "the quantum before still rings"
                );
            }
            q += 128;
        }
        let mut io = [[0.0f32; Q]];
        assert!(
            !n.process(cut, &mut io, false, &ParamInputs::NONE),
            "the cut quantum is not silent"
        );
        assert!(io[0].iter().all(|x| x.to_bits() == 0));
        assert!(n.gone());
        let mut io = [[0.5f32; Q]];
        assert!(
            n.process(cut + 128, &mut io, true, &ParamInputs::NONE),
            "gone: silent for good"
        );
        assert!(io[0].iter().all(|x| x.to_bits() == 0));
    }
}
