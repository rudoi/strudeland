//! superdough's LFO worklet (`LFOProcessor`, worklets.mjs:73-192) and `getLfo` (helpers.mjs:116-161):
//! a one-channel modulation source with no input whose output is connected to an `AudioParam`
//! (tremolo's gain, a filter's frequency, the pulse's width, any target of an `lfo` modulator).
//!
//! - **Params** (12, all a-rate, full float range: the descriptors set only `defaultValue`), in
//!   descriptor order `begin, time, end, frequency, skew, depth, phaseoffset, shape, curve,
//!   dcoffset, min, max`; `min`/`max` are params that clamp the output, not param ranges. Every
//!   param is read once per quantum (`[0]`).
//! - **Start and end.** `currentTime >= end` returns `false` first (the processor finishes and
//!   its output is zeros from then on), then `currentTime <= begin` returns `true` with zeros; both
//!   compare with the float32 params ([`worklet::reached`], [`worklet::started`]).
//! - **Phase.** Set once, in the first active quantum, to `ffrac(time * frequency + phaseoffset)`
//!   (`x - (x | 0)`: negative for a negative argument): the reference is the `time` param (the
//!   hap's cycle position), not the first active frame, so the frames before that quantum are
//!   skipped, not compensated. Then `phase += frequency / sampleRate` per frame with one strict
//!   wrap (`phase > 1`); a negative frequency never wraps.
//! - **Per frame:** `pow((waveshape(phase, skew) + dcoffset) * depth, curve)` clamped to `[min,
//!   max]` with the NaN-propagating JS `clamp`, stored as float32. A NaN reaches the target param,
//!   which replaces it by its default after the sum ([`worklet::fill_block`]).
//! - **Shapes** (`waveShapeNames`: `tri, sine, ramp, saw, square, custom, sawblep`): an index that
//!   is not exactly an integer in 0..=6 makes `waveshapes[undefined](..)` throw a `TypeError`,
//!   after which the processor outputs zeros for good (a processor error, reported once);
//!   `custom` is called with the number `skew` as its `values` and returns NaN.

use crate::QUANTUM;
use crate::job::Value;
use crate::jsmath;
use crate::param::{Param, ParamError, ParamInputs, Rate, idl_float};
use crate::supersaw::saw_blep;
use crate::worklet::{self, INVSR, Lifecycle, ProcessorError};
use std::f64::consts::PI;

/// Descriptor indices (the order of the [`LfoInputs`] a render takes).
pub const BEGIN: usize = 0;
pub const TIME: usize = 1;
pub const END: usize = 2;
pub const FREQUENCY: usize = 3;
pub const SKEW: usize = 4;
pub const DEPTH: usize = 5;
pub const PHASEOFFSET: usize = 6;
pub const SHAPE: usize = 7;
pub const CURVE: usize = 8;
pub const DCOFFSET: usize = 9;
pub const MIN: usize = 10;
pub const MAX: usize = 11;
/// The number of params.
pub const PARAMS: usize = 12;

/// The descriptor defaults, in descriptor order.
const DEFAULTS: [f32; PARAMS] = [0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 0.0, 0.0, 1.0, 0.0, -1e9, 1e9];

const TWO_PI: f64 = 2.0 * PI;

/// The connection inputs of the twelve params for one quantum, in descriptor order.
pub type LfoInputs = [ParamInputs; PARAMS];

/// No param connected.
pub const NO_INPUTS: LfoInputs = [
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
];

/// The error `waveshapes[waveShapeNames[shape]]` throws for an index that names no shape.
pub const SHAPE_ERROR: ProcessorError = ProcessorError {
    processor: "lfo",
    what: "waveshapes[shape] is not a function",
};

/// `waveshapes[waveShapeNames[index]](phase, skew)`; `Err` when `index` names no shape (JS
/// indexes the array with the number's string: only `0`..`6` exactly, `-0` being `"0"`).
pub fn waveshape(index: f64, phase: f64, skew: f64) -> Result<f64, ProcessorError> {
    let shape = if index == index.trunc() && (0.0..=6.0).contains(&index) {
        index as u8
    } else {
        return Err(SHAPE_ERROR);
    };
    Ok(match shape {
        // tri(phase, skew): `x = 1 - skew; phase >= skew ? 1 / x - phase / x : phase / skew`.
        0 => {
            let x = 1.0 - skew;
            if phase >= skew {
                1.0 / x - phase / x
            } else {
                phase / skew
            }
        }
        1 => jsmath::sin(TWO_PI * phase) * 0.5 + 0.5,
        2 => phase,
        3 => 1.0 - phase,
        4 => {
            if phase >= skew {
                0.0
            } else {
                1.0
            }
        }
        // custom(phase, values = skew): `values.length` of a number is undefined, so every step
        // is NaN.
        5 => f64::NAN,
        _ => saw_blep(phase, skew),
    })
}

/// The processor's state: the phase, unset until the first active quantum.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LfoProc {
    phase: Option<f64>,
}

impl LfoProc {
    /// One `process()` call at `currentTime` `ct` with the twelve param blocks, writing `out`
    /// (zeroed by the caller). The throw comes before the first sample is written.
    pub fn process(
        &mut self,
        ct: f64,
        p: &[[f32; QUANTUM]; PARAMS],
        out: &mut [f32; QUANTUM],
    ) -> Result<bool, ProcessorError> {
        if worklet::reached(ct, p[END][0]) {
            return Ok(false);
        }
        if !worklet::started(ct, p[BEGIN][0]) {
            return Ok(true);
        }
        let v = |k: usize| f64::from(p[k][0]);
        let (frequency, time, depth, skew) = (v(FREQUENCY), v(TIME), v(DEPTH), v(SKEW));
        let (phaseoffset, curve, dcoffset) = (v(PHASEOFFSET), v(CURVE), v(DCOFFSET));
        let (min, max, shape) = (v(MIN), v(MAX), v(SHAPE));
        let mut phase = match self.phase {
            Some(x) => x,
            None => jsmath::ffrac(time * frequency + phaseoffset),
        };
        self.phase = Some(phase);
        let dt = frequency * INVSR;
        // `Math.pow(x, 1)` is `x` (also -0 and the infinities), so the default curve skips it;
        // NaN still goes through it.
        let linear = curve == 1.0;
        for o in out.iter_mut() {
            let mut modval = (waveshape(shape, phase, skew)? + dcoffset) * depth;
            if !linear || modval.is_nan() {
                modval = jsmath::pow(modval, curve);
            }
            *o = jsmath::clamp(modval, min, max) as f32;
            phase += dt;
            if phase > 1.0 {
                phase -= 1.0;
            }
            self.phase = Some(phase);
        }
        Ok(true)
    }

    /// The phase (`None` before the first active quantum).
    #[cfg(test)]
    pub fn phase(&self) -> Option<f64> {
        self.phase
    }
}

/// An `lfo-processor` `AudioWorkletNode`: the processor, its twelve params and its lifecycle (no
/// input, so it finishes at its first `false`).
#[derive(Debug, Clone)]
pub struct Lfo {
    pub proc: LfoProc,
    /// In descriptor order (see [`BEGIN`] .. [`MAX`]).
    pub params: [Param; PARAMS],
    life: Lifecycle,
    /// A processor error not yet reported.
    error: Option<ProcessorError>,
}

impl Default for Lfo {
    fn default() -> Self {
        Lfo::new()
    }
}

impl Lfo {
    /// `new AudioWorkletNode(ac, 'lfo-processor')`: every param at its descriptor default.
    pub fn new() -> Lfo {
        Lfo {
            proc: LfoProc::default(),
            params: DEFAULTS.map(|d| worklet::param(d, Rate::A)),
            life: Lifecycle::new(),
            error: None,
        }
    }

    /// One quantum starting at frame `q` into `out` (overwritten), its params pulling `inputs`.
    /// Returns the output's silence hint: false while the processor lives (also before `begin`
    /// and in the quantum it returns `false` or throws, all zeros), true once it finished.
    pub fn render(&mut self, q: u64, inputs: &LfoInputs, out: &mut [f32; QUANTUM]) -> bool {
        *out = [0.0; QUANTUM];
        if !self.life.begin_quantum(false) {
            return true;
        }
        let mut blocks = [[0.0f32; QUANTUM]; PARAMS];
        for ((p, input), block) in self.params.iter_mut().zip(inputs).zip(blocks.iter_mut()) {
            worklet::fill_block(p, q, input, block);
        }
        let r = self.proc.process(worklet::current_time(q), &blocks, out);
        if let Err(e) = r {
            *out = [0.0; QUANTUM];
            self.error = Some(e);
        }
        self.life.finish_quantum(r, false);
        false
    }

    /// The first quantum start at which the processor is gone: the first whose `currentTime`
    /// reaches the float32 `end` (`u64::MAX` when none does).
    pub fn end_quantum(&self) -> u64 {
        worklet::first_quantum_reaching(f64::from(self.params[END].value())).unwrap_or(u64::MAX)
    }

    /// The processor error to report, once.
    pub fn take_error(&mut self) -> Option<ProcessorError> {
        self.error.take()
    }

    /// Whether the processor has finished.
    #[cfg(test)]
    pub fn gone(&self) -> bool {
        self.life.gone()
    }
}

/// The `shape` argument of `getLfo`: a number or a name (`getModulationShapeInput`'s two cases).
#[derive(Debug, Clone, PartialEq)]
pub enum ShapeInput {
    Num(f64),
    Name(String),
}

impl ShapeInput {
    /// A control value as `getModulationShapeInput` sees it: a JS number, else (strings, `null`,
    /// booleans, lists) the name lookup, which only strings can satisfy.
    pub fn from_value(v: &Value) -> ShapeInput {
        match v {
            Value::Num(x) => ShapeInput::Num(*x),
            Value::Str(s) => ShapeInput::Name(s.clone()),
            _ => ShapeInput::Name(String::new()),
        }
    }
}

/// `getModulationShapeInput(val)` (helpers.mjs:116-121): a number gives `val % 5` as is (negative
/// and fractional results are kept), a name `tri`/`triangle` 0, `sine` 1, `ramp` 2, `saw` 3,
/// `square` 4, any other name 0.
pub fn modulation_shape(shape: &ShapeInput) -> f64 {
    match shape {
        ShapeInput::Num(x) => x % 5.0,
        ShapeInput::Name(s) => match s.as_str() {
            "tri" | "triangle" => 0.0,
            "sine" => 1.0,
            "ramp" => 2.0,
            "saw" => 3.0,
            "square" => 4.0,
            _ => 0.0,
        },
    }
}

/// `getLfo`'s properties; `None` is `undefined` (the destructuring default applies).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LfoProps {
    pub begin: Option<f64>,
    pub end: Option<f64>,
    pub time: Option<f64>,
    pub depth: Option<f64>,
    pub dcoffset: Option<f64>,
    pub frequency: Option<f64>,
    pub skew: Option<f64>,
    pub phaseoffset: Option<f64>,
    pub curve: Option<f64>,
    pub shape: Option<ShapeInput>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// `getLfo(ac, props)` scheduled at frame `now`: the defaults (`shape 0, begin 0, end 0, time =
/// time ?? begin, depth 1, dcoffset -0.5, frequency 1, skew 0.5, phaseoffset 0, curve 1, min =
/// min ?? dcoffset * depth, max = max ?? dcoffset * depth + depth`), then `.value =` in the order
/// `begin, end, time, depth, dcoffset, frequency, skew, phaseoffset, curve, shape, min, max`; a
/// non-finite value (or one overflowing float32) throws there.
pub fn get_lfo(props: &LfoProps, now: u64) -> Result<Lfo, ParamError> {
    let begin = props.begin.unwrap_or(0.0);
    let end = props.end.unwrap_or(0.0);
    let time = props.time.unwrap_or(begin);
    let depth = props.depth.unwrap_or(1.0);
    let dcoffset = props.dcoffset.unwrap_or(-0.5);
    let frequency = props.frequency.unwrap_or(1.0);
    let skew = props.skew.unwrap_or(0.5);
    let phaseoffset = props.phaseoffset.unwrap_or(0.0);
    let curve = props.curve.unwrap_or(1.0);
    let shape = props.shape.as_ref().map_or(0.0, modulation_shape);
    let min = props.min.unwrap_or(dcoffset * depth);
    let max = props.max.unwrap_or(dcoffset * depth + depth);
    let mut lfo = Lfo::new();
    let sets = [
        (BEGIN, begin),
        (END, end),
        (TIME, time),
        (DEPTH, depth),
        (DCOFFSET, dcoffset),
        (FREQUENCY, frequency),
        (SKEW, skew),
        (PHASEOFFSET, phaseoffset),
        (CURVE, curve),
        (SHAPE, shape),
        (MIN, min),
        (MAX, max),
    ];
    for (k, v) in sets {
        lfo.params[k].set_value(idl_float(v, "value")?, now)?;
    }
    Ok(lfo)
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    use crate::QUANTUM_U64 as Q;

    fn bits(x: f64) -> u64 {
        x.to_bits()
    }

    /// Param blocks: every param at `v[k]` for the whole quantum.
    fn blocks(v: [f32; PARAMS]) -> [[f32; QUANTUM]; PARAMS] {
        v.map(|x| [x; QUANTUM])
    }

    /// A running LFO: begin 0, end 10 s, the getLfo defaults otherwise.
    fn running(shape: f32, frequency: f32, time: f32, skew: f32) -> [f32; PARAMS] {
        let mut v = DEFAULTS;
        v[END] = 10.0;
        v[SHAPE] = shape;
        v[FREQUENCY] = frequency;
        v[TIME] = time;
        v[SKEW] = skew;
        v
    }

    #[test]
    fn waveshapes_at_the_quarter_points() {
        let ps = [0.0, 0.25, 0.5, 0.75, 1.0];
        // (shape, skew, expected at each p): NaN where JS computes Infinity - Infinity or 0 / 0.
        let nan = f64::NAN;
        let table: [(f64, f64, [f64; 5]); 9] = [
            (0.0, 0.5, [0.0, 0.5, 1.0, 0.5, 0.0]),
            // skew 0: every phase is on the falling side, 1 - p.
            (0.0, 0.0, [1.0, 0.75, 0.5, 0.25, 0.0]),
            // skew 1: the rising ramp, and 1 / 0 - 1 / 0 at p = 1.
            (0.0, 1.0, [0.0, 0.25, 0.5, 0.75, nan]),
            (1.0, 0.5, [0.5, 1.0, 0.5, 0.0, 0.5]),
            (2.0, 0.5, ps),
            (3.0, 0.5, [1.0, 0.75, 0.5, 0.25, 0.0]),
            (4.0, 0.5, [1.0, 1.0, 0.0, 0.0, 0.0]),
            (4.0, 0.0, [0.0; 5]),
            (4.0, 1.0, [1.0, 1.0, 1.0, 1.0, 0.0]),
        ];
        for (shape, skew, want) in table {
            for (p, w) in ps.iter().zip(want) {
                let got = waveshape(shape, *p, skew).unwrap();
                if w.is_nan() {
                    assert!(got.is_nan(), "shape {shape} skew {skew} p {p}: {got}");
                } else if shape == 1.0 {
                    // sin(2 pi p) is not exactly 0 at p = 0.5 or 1.
                    let exact = jsmath::sin(TWO_PI * p) * 0.5 + 0.5;
                    assert_eq!(bits(got), bits(exact), "sine p {p}");
                    assert!((got - w).abs() < 1e-15, "sine p {p}: {got}");
                } else {
                    assert_eq!(bits(got), bits(w), "shape {shape} skew {skew} p {p}");
                }
            }
        }
        // tri with skew 0 at p = 0: 0 >= 0 takes the falling side, 1 / 1 - 0.
        assert_eq!(waveshape(0.0, 0.0, 0.0), Ok(1.0));
        // custom: NaN without an error, whatever the phase.
        for p in ps {
            assert!(waveshape(5.0, p, 0.5).unwrap().is_nan());
        }
        // sawblep: (2p - 1) - polyBLEP(p, dt = skew).
        for (p, dt) in [(0.0, 0.1), (0.05, 0.1), (0.5, 0.1), (0.95, 0.1), (0.3, 0.5)] {
            assert_eq!(
                bits(waveshape(6.0, p, dt).unwrap()),
                bits(saw_blep(p, dt)),
                "sawblep {p} {dt}"
            );
        }
        assert_eq!(waveshape(6.0, 0.5, 0.1), Ok(0.0));
    }

    #[test]
    fn shape_indices_that_name_no_shape_throw() {
        for bad in [-1.0, 1.5, 7.0, 6.5, f64::NAN, f64::INFINITY, -0.5] {
            assert_eq!(waveshape(bad, 0.3, 0.5), Err(SHAPE_ERROR), "{bad}");
        }
        // -0 is the key "0": tri.
        assert_eq!(waveshape(-0.0, 0.25, 0.5), Ok(0.5));
        assert_eq!(waveshape(6.0, 0.5, 0.25), Ok(0.0));
    }

    #[test]
    fn the_first_active_quantum_sets_the_phase_from_time() {
        // time 0.3, frequency 2, phaseoffset 0.1: ffrac(0.6 + 0.1) with the float32 values.
        let mut v = running(2.0, 2.0, 0.3, 0.5);
        v[PHASEOFFSET] = 0.1;
        v[CURVE] = 1.0;
        v[DCOFFSET] = 0.0;
        let p = blocks(v);
        let mut l = LfoProc::default();
        let mut out = [0.0f32; QUANTUM];
        assert_eq!(l.process(worklet::current_time(Q), &p, &mut out), Ok(true));
        let phase0 = jsmath::ffrac(f64::from(0.3f32) * 2.0 + f64::from(0.1f32));
        assert_ne!(
            bits(phase0),
            bits(0.7),
            "the float32 time makes a difference"
        );
        // ramp: out[n] = phase after n increments, as float32.
        let dt = 2.0 * INVSR;
        let mut ph = phase0;
        for n in 0..QUANTUM {
            assert_eq!(out[n].to_bits(), (ph as f32).to_bits(), "frame {n}");
            ph += dt;
        }
        // The phase is not reset by a later quantum (a changed time is ignored).
        let mut v2 = v;
        v2[TIME] = 100.25;
        let mut out = [0.0f32; QUANTUM];
        l.process(worklet::current_time(2 * Q), &blocks(v2), &mut out)
            .unwrap();
        assert_eq!(out[0].to_bits(), (ph as f32).to_bits());
    }

    #[test]
    fn a_negative_start_phase_keeps_its_sign_and_a_negative_frequency_never_wraps() {
        // time * frequency = -1.25: ffrac gives -0.25 (not frac's 0.75).
        let mut v = running(2.0, -2.5, 0.5, 0.5);
        v[DCOFFSET] = 0.0;
        v[MIN] = -1e9;
        let mut l = LfoProc::default();
        let mut out = [0.0f32; QUANTUM];
        l.process(worklet::current_time(Q), &blocks(v), &mut out)
            .unwrap();
        assert_eq!(l.phase().map(bits), {
            let mut ph = -0.25f64;
            for _ in 0..QUANTUM {
                ph += -2.5 * INVSR;
            }
            Some(bits(ph))
        });
        assert_eq!(out[0], -0.25);
        assert!(out.iter().all(|x| *x < 0.0));
        // Many quanta later the phase is still falling, below -1.
        for k in 2..400u64 {
            l.process(worklet::current_time(k * Q), &blocks(v), &mut out)
                .unwrap();
        }
        assert!(l.phase().unwrap() < -1.0);
    }

    #[test]
    fn the_phase_wraps_strictly_above_one_once() {
        // A phase that lands exactly on 1.0 is not wrapped (tri with skew 1 is then NaN); one
        // above it is, by a single subtraction.
        let mut l = LfoProc { phase: Some(1.0) };
        let mut v = running(2.0, 0.0, 0.0, 0.5);
        v[DCOFFSET] = 0.0;
        let mut out = [0.0f32; QUANTUM];
        l.process(worklet::current_time(Q), &blocks(v), &mut out)
            .unwrap();
        assert_eq!(out[0], 1.0, "frequency 0: the phase stays at exactly 1");
        assert_eq!(l.phase(), Some(1.0));
        // frequency such that dt is 0.75: 0.5 -> 1.25 -> 0.25 -> 1.0 -> 1.75 -> 0.75.
        let mut l = LfoProc { phase: Some(0.5) };
        let mut v = running(2.0, (0.75 * 44100.0) as f32, 0.0, 0.5);
        v[DCOFFSET] = 0.0;
        l.process(worklet::current_time(Q), &blocks(v), &mut out)
            .unwrap();
        let dt = f64::from((0.75 * 44100.0) as f32) * INVSR;
        let mut ph = 0.5f64;
        for n in 0..6 {
            assert_eq!(out[n].to_bits(), (ph as f32).to_bits(), "frame {n}");
            ph += dt;
            if ph > 1.0 {
                ph -= 1.0;
            }
        }
        // A frequency of 3 x the rate: one subtraction leaves the phase above 1.
        let mut l = LfoProc { phase: Some(0.0) };
        let mut v = running(2.0, 3.0 * 44100.0, 0.0, 0.5);
        v[DCOFFSET] = 0.0;
        l.process(worklet::current_time(Q), &blocks(v), &mut out)
            .unwrap();
        assert_eq!(out[1], 2.0);
    }

    #[test]
    fn shape_errors_are_processor_errors_and_shape_five_is_nan() {
        for bad in [-1.0f32, 1.5, 7.0, f32::NAN] {
            let mut lfo = Lfo::new();
            lfo.params[END].set_value(10.0, 0).unwrap();
            lfo.params[SHAPE] = worklet::param(bad, Rate::A);
            let mut out = [1.0f32; QUANTUM];
            assert!(
                !lfo.render(Q, &NO_INPUTS, &mut out),
                "{bad}: live in the throwing quantum"
            );
            assert!(out.iter().all(|x| x.to_bits() == 0), "{bad}");
            assert_eq!(lfo.take_error(), Some(SHAPE_ERROR), "{bad}");
            assert_eq!(lfo.take_error(), None, "{bad}: reported once");
            assert!(lfo.gone(), "{bad}");
            assert!(lfo.render(2 * Q, &NO_INPUTS, &mut out), "{bad}: gone");
            assert!(out.iter().all(|x| x.to_bits() == 0));
        }
        // -0 is tri; 5 outputs NaN without an error.
        for (shape, nan) in [(-0.0f32, false), (5.0, true)] {
            let mut lfo = Lfo::new();
            lfo.params[END].set_value(10.0, 0).unwrap();
            lfo.params[SHAPE] = worklet::param(shape, Rate::A);
            let mut out = [0.0f32; QUANTUM];
            assert!(!lfo.render(Q, &NO_INPUTS, &mut out));
            assert_eq!(lfo.take_error(), None, "{shape}");
            assert_eq!(out.iter().all(|x| x.is_nan()), nan, "{shape}");
            assert!(!lfo.gone());
        }
    }

    #[test]
    fn a_nan_output_through_a_connected_param_gives_the_default() {
        // tri with skew 1 at phase 1: NaN, then the target (a gain) replaces the sum by 1.
        let mut l = LfoProc { phase: Some(1.0) };
        let mut v = running(0.0, 0.0, 0.0, 1.0);
        v[DCOFFSET] = 0.0;
        let mut out = [0.0f32; QUANTUM];
        l.process(worklet::current_time(Q), &blocks(v), &mut out)
            .unwrap();
        assert!(out[0].is_nan());
        let mut gain = Param::gain(1.0);
        let inputs = ParamInputs {
            bufs: vec![out],
            connected: true,
        };
        let mut values = [0.0f32; QUANTUM];
        worklet::fill_block(&mut gain, Q, &inputs, &mut values);
        assert_eq!(values[0], 1.0);
    }

    #[test]
    fn gating_on_exact_float32_boundaries_end_first() {
        let lfo_at = |begin: f32, end: f32| {
            let mut v = running(2.0, 1.0, 0.0, 0.5);
            v[BEGIN] = begin;
            v[END] = end;
            v[DCOFFSET] = 0.0;
            v[MIN] = -1e9;
            blocks(v)
        };
        let q128 = worklet::current_time(Q);
        // begin exactly at the quantum start (as float32 widened): zeros, alive.
        let b = q128 as f32;
        let mut l = LfoProc::default();
        let mut out = [0.0f32; QUANTUM];
        if f64::from(b) == q128 {
            assert_eq!(l.process(q128, &lfo_at(b, 10.0), &mut out), Ok(true));
            assert!(l.phase().is_none());
        }
        // An exact boundary: begin 0 at the quantum starting at 0 is `ct <= begin`.
        assert_eq!(l.process(0.0, &lfo_at(0.0, 10.0), &mut out), Ok(true));
        assert!(l.phase().is_none());
        assert!(out.iter().all(|x| x.to_bits() == 0));
        // f32(begin) just below the start: active.
        let below = f32::from_bits((q128 as f32).to_bits() - 1);
        assert!(f64::from(below) < q128);
        assert_eq!(l.process(q128, &lfo_at(below, 10.0), &mut out), Ok(true));
        assert!(l.phase().is_some());
        // f32(begin) just above: not active.
        let above = f32::from_bits((q128 as f32).to_bits() + 1);
        let mut l = LfoProc::default();
        assert_eq!(l.process(q128, &lfo_at(above, 10.0), &mut out), Ok(true));
        assert!(l.phase().is_none());
        // end reached exactly or below: false; end first even when also before begin.
        for end in [below, q128 as f32] {
            if f64::from(end) <= q128 {
                let mut l = LfoProc::default();
                assert_eq!(l.process(q128, &lfo_at(above, end), &mut out), Ok(false));
                assert!(l.phase().is_none());
            }
        }
        let mut l = LfoProc::default();
        assert_eq!(l.process(q128, &lfo_at(0.0, above), &mut out), Ok(true));
        assert!(l.phase().is_some(), "end just above: still active");
    }

    #[test]
    fn clamp_and_curve() {
        // ramp at phase 0.5, dcoffset 0, depth 1: pow(0.5, curve).
        let out_for = |curve: f32, depth: f32, min: f32, max: f32| {
            let mut v = running(2.0, 0.0, 0.0, 0.5);
            v[DCOFFSET] = 0.0;
            v[CURVE] = curve;
            v[DEPTH] = depth;
            v[MIN] = min;
            v[MAX] = max;
            let mut l = LfoProc { phase: Some(0.5) };
            let mut out = [0.0f32; QUANTUM];
            l.process(worklet::current_time(Q), &blocks(v), &mut out)
                .unwrap();
            out[0]
        };
        for curve in [0.25f32, 0.5, 1.0, 1.5, 2.0, 3.0] {
            let want = jsmath::pow(0.5, f64::from(curve)) as f32;
            assert_eq!(
                out_for(curve, 1.0, -1e9, 1e9).to_bits(),
                want.to_bits(),
                "{curve}"
            );
        }
        // curve 1 is the identity bit for bit.
        assert_eq!(
            out_for(1.0, 0.3, -1e9, 1e9),
            (0.5 * f64::from(0.3f32)) as f32
        );
        // curve 1.5 of a negative value: NaN, kept by the clamp.
        assert!(out_for(1.5, -1.0, 0.0, 1.0).is_nan());
        // min > max: Math.min(Math.max(v, min), max) is max.
        assert_eq!(out_for(1.0, 1.0, 0.8, 0.2), 0.2);
        // Clamped to [0, 1] after the pow.
        assert_eq!(out_for(1.0, 4.0, 0.0, 1.0), 1.0);
    }

    #[test]
    fn render_silence_hint_and_end_quantum() {
        // begin 0.001 s (frame 44.1), end 0.01 s (frame 441): zeros and live in quantum 0,
        // active in 128..384, gone from 512 (the first start reaching 0.01).
        let props = LfoProps {
            begin: Some(0.001),
            end: Some(0.01),
            frequency: Some(5.0),
            ..LfoProps::default()
        };
        let mut lfo = get_lfo(&props, 0).unwrap();
        assert_eq!(lfo.end_quantum(), 512);
        let mut out = [0.0f32; QUANTUM];
        assert!(!lfo.render(0, &NO_INPUTS, &mut out));
        assert!(out.iter().all(|x| x.to_bits() == 0));
        for k in 1..4u64 {
            assert!(!lfo.render(k * Q, &NO_INPUTS, &mut out), "quantum {k}");
            assert!(out.iter().any(|x| *x != 0.0), "quantum {k}");
        }
        assert!(
            !lfo.render(4 * Q, &NO_INPUTS, &mut out),
            "returns false: still live"
        );
        assert!(out.iter().all(|x| x.to_bits() == 0));
        assert!(lfo.gone());
        assert!(lfo.render(5 * Q, &NO_INPUTS, &mut out));
        assert_eq!(lfo.take_error(), None);
        // A far end: no quantum reaches it.
        let lfo = get_lfo(
            &LfoProps {
                end: Some(1e30),
                ..LfoProps::default()
            },
            0,
        )
        .unwrap();
        assert_eq!(lfo.end_quantum(), u64::MAX);
        // end 0 (the default): gone in quantum 0.
        let lfo = get_lfo(&LfoProps::default(), 0).unwrap();
        assert_eq!(lfo.end_quantum(), 0);
    }

    #[test]
    fn get_lfo_defaults_and_set_order() {
        let lfo = get_lfo(
            &LfoProps {
                begin: Some(0.25),
                ..LfoProps::default()
            },
            256,
        )
        .unwrap();
        let v: Vec<f32> = lfo.params.iter().map(Param::value).collect();
        // begin, time = begin, end 0, frequency 1, skew 0.5, depth 1, phaseoffset 0, shape 0,
        // curve 1, dcoffset -0.5, min = -0.5, max = 0.5.
        assert_eq!(
            v,
            [
                0.25, 0.25, 0.0, 1.0, 0.5, 1.0, 0.0, 0.0, 1.0, -0.5, -0.5, 0.5
            ]
        );
        for p in &lfo.params {
            assert_eq!(p.events().len(), 1, "every .value = schedules at now");
            assert_eq!(p.events()[0].time, 256.0 / 44100.0);
        }
        // min/max from dcoffset and depth in doubles.
        let lfo = get_lfo(
            &LfoProps {
                depth: Some(0.3),
                dcoffset: Some(-0.25),
                ..LfoProps::default()
            },
            0,
        )
        .unwrap();
        assert_eq!(lfo.params[MIN].value(), (-0.25f64 * 0.3) as f32);
        assert_eq!(lfo.params[MAX].value(), (-0.25f64 * 0.3 + 0.3) as f32);
        // Given min/max win; a given time wins over begin; shapes map.
        let lfo = get_lfo(
            &LfoProps {
                begin: Some(1.0),
                time: Some(3.5),
                min: Some(0.0),
                max: Some(1.0),
                shape: Some(ShapeInput::Name("square".into())),
                ..LfoProps::default()
            },
            0,
        )
        .unwrap();
        assert_eq!(lfo.params[TIME].value(), 3.5);
        assert_eq!(
            (lfo.params[MIN].value(), lfo.params[MAX].value()),
            (0.0, 1.0)
        );
        assert_eq!(lfo.params[SHAPE].value(), 4.0);
        // A non-finite value throws at its `.value =`: the earlier params are set.
        let e = get_lfo(
            &LfoProps {
                frequency: Some(f64::NAN),
                ..LfoProps::default()
            },
            0,
        )
        .unwrap_err();
        assert_eq!(e, ParamError::NonFiniteValue { method: "value" });
        assert_eq!(
            get_lfo(
                &LfoProps {
                    depth: Some(1e39),
                    ..LfoProps::default()
                },
                0
            )
            .unwrap_err(),
            ParamError::NonFiniteValue { method: "value" },
            "overflows float32"
        );
        // min derived from an infinite product throws at `min`.
        assert!(
            get_lfo(
                &LfoProps {
                    depth: Some(1e38),
                    dcoffset: Some(-1e38),
                    ..LfoProps::default()
                },
                0
            )
            .is_err()
        );
    }

    #[test]
    fn modulation_shape_inputs() {
        let n = |x: f64| modulation_shape(&ShapeInput::Num(x));
        assert_eq!(n(3.0), 3.0);
        assert_eq!(n(7.0), 2.0);
        assert_eq!(n(-1.0), -1.0, "not wrapped to non-negative");
        assert_eq!(n(6.5), 1.5, "not made integral");
        assert!(n(f64::NAN).is_nan());
        assert_eq!(n(-5.0).to_bits(), (-0.0f64).to_bits());
        let s = |x: &str| modulation_shape(&ShapeInput::Name(x.into()));
        for (name, want) in [
            ("tri", 0.0),
            ("triangle", 0.0),
            ("sine", 1.0),
            ("ramp", 2.0),
            ("saw", 3.0),
            ("square", 4.0),
            ("sawblep", 0.0),
            ("", 0.0),
            ("Sine", 0.0),
        ] {
            assert_eq!(s(name), want, "{name}");
        }
        assert_eq!(
            ShapeInput::from_value(&Value::Num(2.0)),
            ShapeInput::Num(2.0)
        );
        assert_eq!(
            ShapeInput::from_value(&Value::Str("saw".into())),
            ShapeInput::Name("saw".into())
        );
        assert_eq!(modulation_shape(&ShapeInput::from_value(&Value::Null)), 0.0);
        assert_eq!(
            modulation_shape(&ShapeInput::from_value(&Value::Bool(true))),
            0.0
        );
    }

    #[test]
    fn idl_float_failures_of_set_values() {
        for bad in [f64::INFINITY, f64::NEG_INFINITY, 3.5e38, -3.5e38] {
            for field in 0..PARAMS {
                let mut p = LfoProps::default();
                let slot = match field {
                    0 => &mut p.begin,
                    1 => &mut p.end,
                    2 => &mut p.time,
                    3 => &mut p.depth,
                    4 => &mut p.dcoffset,
                    5 => &mut p.frequency,
                    6 => &mut p.skew,
                    7 => &mut p.phaseoffset,
                    8 => &mut p.curve,
                    9 => &mut p.min,
                    10 => &mut p.max,
                    _ => {
                        // `bad % 5`: NaN for the infinities, finite (and accepted) otherwise.
                        p.shape = Some(ShapeInput::Num(bad));
                        assert_eq!(get_lfo(&p, 0).is_err(), bad.is_infinite(), "shape {bad}");
                        continue;
                    }
                };
                *slot = Some(bad);
                assert!(get_lfo(&p, 0).is_err(), "field {field} {bad}");
            }
        }
    }
}
