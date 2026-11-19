//! superdough's memoryless effect worklets (worklets.mjs:194-296 and 430-464; the distortion
//! curves helpers.mjs:514-608): `coarse` (sample-and-hold in phase with the quantum), `crush`
//! (bit reduction with `Math.round`), `shape` (a rational waveshaper with a clamped postgain) and
//! `distort` (nine waveshaping algorithms, `k = expm1(distort)` and a postgain per sample).
//!
//! Each is its own worklet node, built only when its control is defined, in the order `coarse`,
//! `crush`, `shape`, `distort` (superdough.mjs:799-818). The processors are ported expression by
//! expression in `f64` with `jsmath` for `Math.*` and store `f32` outputs; they are stateless
//! across quanta except `started`. Their input is never reported gone (every one
//! maps 0 to 0, so processing the zeros the upstream delivers gives Chromium's samples); the
//! processors still implement `started && !hasInput`.

use crate::QUANTUM;
use crate::job::Value;
use crate::jsmath;
use crate::param::{Param, ParamInputs, Rate};
use crate::voice::zero;
use crate::worklet::{self, Lifecycle, ProcessorError};

/// A distortion curve `f(x, k)`.
pub type Algorithm = fn(f64, f64) -> f64;

/// `Object.keys(distortionAlgorithms)`, the `distorttype` names in index order.
pub const DISTORTION_NAMES: [&str; 9] = [
    "scurve",
    "soft",
    "hard",
    "cubic",
    "diode",
    "asym",
    "fold",
    "sinefold",
    "chebyshev",
];

const ALGORITHMS: [Algorithm; 9] = [
    scurve, soft, hard, cubic, diode, asym, fold, sinefold, chebyshev,
];

/// The distortion curve called `name` (`distortionAlgorithms[name]`).
pub fn distortion_algorithm(name: &str) -> Option<Algorithm> {
    DISTORTION_NAMES
        .iter()
        .position(|n| *n == name)
        .map(|i| ALGORITHMS[i])
}

/// `x / (1 + x)`.
fn squash(x: f64) -> f64 {
    x / (1.0 + x)
}

/// `((n % m) + m) % m` (JS remainder: the sign of the dividend).
fn js_mod(n: f64, m: f64) -> f64 {
    ((n % m) + m) % m
}

fn scurve(x: f64, k: f64) -> f64 {
    ((1.0 + k) * x) / (1.0 + k * x.abs())
}

fn soft(x: f64, k: f64) -> f64 {
    jsmath::tanh(x * (1.0 + k))
}

fn hard(x: f64, k: f64) -> f64 {
    jsmath::clamp((1.0 + k) * x, -1.0, 1.0)
}

fn fold(x: f64, k: f64) -> f64 {
    let y = (1.0 + 0.5 * k) * x;
    let window = js_mod(y + 1.0, 4.0);
    1.0 - (window - 2.0).abs()
}

fn sinefold(x: f64, k: f64) -> f64 {
    jsmath::sin((std::f64::consts::PI / 2.0) * fold(x, k))
}

fn cubic(x: f64, k: f64) -> f64 {
    let t = squash(jsmath::log1p(k));
    // `(t / 3) * x * x * x` evaluates left to right.
    let c = (x - (((t / 3.0) * x) * x) * x) / (1.0 - t / 3.0);
    soft(c, k)
}

fn diode_curve(x: f64, k: f64, asym: bool) -> f64 {
    let g = 1.0 + 2.0 * k;
    let t = squash(jsmath::log1p(k));
    let bias = 0.07 * t;
    let pos = soft(x + bias, 2.0 * k);
    let neg = soft(if asym { bias } else { -x + bias }, 2.0 * k);
    let y = pos - neg;
    let sech = 1.0 / jsmath::cosh(g * bias);
    let sech2 = sech * sech;
    let c = if asym { 1.0 } else { 2.0 };
    let denom = jsmath::max(1e-8, (c * g) * sech2);
    soft(y / denom, k)
}

fn diode(x: f64, k: f64) -> f64 {
    diode_curve(x, k, false)
}

fn asym(x: f64, k: f64) -> f64 {
    diode_curve(x, k, true)
}

fn chebyshev(x: f64, k: f64) -> f64 {
    let kl = 10.0 * jsmath::log1p(k);
    let mut tnm1 = 1.0;
    let mut tnm2 = x;
    let mut y = 0.0;
    for i in 1..64u32 {
        if i < 2 {
            // `y += i == 0 ? tnm1 : tnm2`: i is 1 here.
            y += tnm2;
            continue;
        }
        let tn = (2.0 * x) * tnm1 - tnm2;
        tnm2 = tnm1;
        tnm1 = tn;
        if i % 2 == 0 {
            y += jsmath::min((1.3 * kl) / f64::from(i), 2.0) * tn;
        }
    }
    soft(y, kl / 20.0)
}

/// `getDistortionAlgorithm(distorttype)`'s index: a string is looked up by name (an unknown name
/// is logged and gives 0), anything else is used as `index % 9` and must then name an entry
/// (`_algoNames[i]`): an integer in 0..=8 (`-0` is `"0"`). `null` and booleans convert to 0 and
/// 1. A negative, fractional, NaN or infinite number gives `None` (the processor then throws).
///
/// Arrays, objects and sets are unsupported signatures and never reach the engine (`None`).
pub fn algorithm_index(v: &Value) -> Option<usize> {
    let k = match v {
        Value::Str(name) => {
            return Some(DISTORTION_NAMES.iter().position(|n| n == name).unwrap_or(0));
        }
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Num(x) => *x,
        Value::Arr(_) | Value::Obj(_) | Value::Set(_) | Value::Unsupported(_) => return None,
    };
    let idx = k % 9.0;
    (idx >= 0.0 && idx.fract() == 0.0).then_some(idx as usize)
}

/// `out[ch][n] = f(ch, n)` stored as `f32` on every channel; the caller passes the per-frame value.
#[inline]
fn store(x: f64) -> f32 {
    x as f32
}

/// The `started && !hasInput` rule: `None` to go on, `Some(false)` to return.
fn gate(started: &mut bool, has_input: bool) -> Option<bool> {
    if *started && !has_input {
        return Some(false);
    }
    *started = has_input;
    None
}

/// `CoarseProcessor`: `coarse` (default 1, a-rate, read once per quantum).
#[derive(Debug, Clone)]
pub struct Coarse {
    pub coarse: Param,
    started: bool,
}

impl Default for Coarse {
    fn default() -> Self {
        Coarse::new()
    }
}

impl Coarse {
    pub fn new() -> Coarse {
        Coarse {
            coarse: worklet::param(1.0, Rate::A),
            started: false,
        }
    }

    /// One quantum in place on `io` (no channels: no input). `n % coarse < 1` takes the input,
    /// else holds the stored previous output; frame 0 always takes the input, so the hold
    /// restarts every quantum.
    pub fn process(
        &mut self,
        io: &mut [[f32; QUANTUM]],
        coarse: &[f32; QUANTUM],
    ) -> Result<bool, ProcessorError> {
        if let Some(r) = gate(&mut self.started, !io.is_empty()) {
            return Ok(r);
        }
        let c = jsmath::max(1.0, f64::from(coarse[0]));
        for n in 0..QUANTUM {
            let take = (n as f64) % c < 1.0;
            for ch in io.iter_mut() {
                if !take {
                    // `output[i][n - 1]`; at n = 0 (only reachable for a NaN coarse) it is
                    // `undefined`, stored as NaN.
                    ch[n] = if n == 0 { f32::NAN } else { ch[n - 1] };
                }
            }
        }
        Ok(true)
    }
}

/// `CrushProcessor`: `crush` (default 0, a-rate, read once per quantum).
#[derive(Debug, Clone)]
pub struct Crush {
    pub crush: Param,
    started: bool,
}

impl Default for Crush {
    fn default() -> Self {
        Crush::new()
    }
}

impl Crush {
    pub fn new() -> Crush {
        Crush {
            crush: worklet::param(0.0, Rate::A),
            started: false,
        }
    }

    /// `x = pow(2, max(1, crush) - 1)` (the same each sample), `out = round(in * x) / x`.
    pub fn process(
        &mut self,
        io: &mut [[f32; QUANTUM]],
        crush: &[f32; QUANTUM],
    ) -> Result<bool, ProcessorError> {
        if let Some(r) = gate(&mut self.started, !io.is_empty()) {
            return Ok(r);
        }
        let c = jsmath::max(1.0, f64::from(crush[0]));
        let x = jsmath::pow(2.0, c - 1.0);
        for ch in io.iter_mut() {
            for s in ch.iter_mut() {
                *s = store(jsmath::round(f64::from(*s) * x) / x);
            }
        }
        Ok(true)
    }
}

/// `ShapeProcessor`: `shape` (default 0) and `postgain` (default 1), a-rate, read once per
/// quantum.
#[derive(Debug, Clone)]
pub struct Shape {
    pub shape: Param,
    pub postgain: Param,
    started: bool,
}

impl Default for Shape {
    fn default() -> Self {
        Shape::new()
    }
}

impl Shape {
    pub fn new() -> Shape {
        Shape {
            shape: worklet::param(0.0, Rate::A),
            postgain: worklet::param(1.0, Rate::A),
            started: false,
        }
    }

    /// `s = shape < 1 ? shape : 1 - 4e-10; s = 2s / (1 - s)`, `post = max(0.001, min(1,
    /// postgain))`, `out = ((1 + s) * x) / (1 + s * |x|) * post`.
    pub fn process(
        &mut self,
        io: &mut [[f32; QUANTUM]],
        shape: &[f32; QUANTUM],
        postgain: &[f32; QUANTUM],
    ) -> Result<bool, ProcessorError> {
        if let Some(r) = gate(&mut self.started, !io.is_empty()) {
            return Ok(r);
        }
        let s0 = f64::from(shape[0]);
        let s0 = if s0 < 1.0 { s0 } else { 1.0 - 4e-10 };
        let s = (2.0 * s0) / (1.0 - s0);
        let post = jsmath::max(0.001, jsmath::min(1.0, f64::from(postgain[0])));
        for ch in io.iter_mut() {
            for v in ch.iter_mut() {
                let x = f64::from(*v);
                *v = store((((1.0 + s) * x) / (1.0 + s * x.abs())) * post);
            }
        }
        Ok(true)
    }
}

/// `DistortProcessor`: `distort` (default 0) and `postgain` (default 1), a-rate, read per
/// sample; the algorithm from `processorOptions.algorithm`.
#[derive(Debug, Clone)]
pub struct Distort {
    pub distort: Param,
    pub postgain: Param,
    algorithm: Option<Algorithm>,
    started: bool,
}

impl Distort {
    /// The constructor: `getDistortionAlgorithm(algorithm)`, never failing (an invalid index
    /// leaves `undefined`, which throws at the first call with input).
    pub fn new(algorithm: &Value) -> Distort {
        Distort {
            distort: worklet::param(0.0, Rate::A),
            postgain: worklet::param(1.0, Rate::A),
            algorithm: algorithm_index(algorithm).map(|i| ALGORITHMS[i]),
            started: false,
        }
    }

    /// Per sample `n`: `post = clamp(postgain[n], 0.001, 1)`, `k = expm1(distort[n])`, then for
    /// each channel `out = post * algorithm(x, k)`. Without an algorithm the first call with an
    /// input channel throws (`this.algorithm is not a function`) before writing anything.
    pub fn process(
        &mut self,
        io: &mut [[f32; QUANTUM]],
        distort: &[f32; QUANTUM],
        postgain: &[f32; QUANTUM],
    ) -> Result<bool, ProcessorError> {
        if let Some(r) = gate(&mut self.started, !io.is_empty()) {
            return Ok(r);
        }
        if io.is_empty() {
            return Ok(true);
        }
        let Some(f) = self.algorithm else {
            return Err(ProcessorError {
                processor: "distort",
                what: "algorithm is not a function",
            });
        };
        for n in 0..QUANTUM {
            let post = jsmath::clamp(f64::from(postgain[n]), 0.001, 1.0);
            let k = jsmath::expm1(f64::from(distort[n]));
            for ch in io.iter_mut() {
                ch[n] = store(post * f(f64::from(ch[n]), k));
            }
        }
        Ok(true)
    }
}

/// Which shaper a node runs.
#[derive(Debug, Clone)]
pub enum ShaperKind {
    Coarse(Coarse),
    Crush(Crush),
    Shape(Shape),
    Distort(Box<Distort>),
}

impl ShaperKind {
    /// One `process()` call at the quantum starting at `q`: the param blocks, then the
    /// processor, in place.
    fn run(
        &mut self,
        q: u64,
        io: &mut [[f32; QUANTUM]],
        inputs: [&ParamInputs; 2],
    ) -> Result<bool, ProcessorError> {
        let block = |p: &mut Param, input: &ParamInputs| {
            let mut b = [0.0f32; QUANTUM];
            worklet::fill_block(p, q, input, &mut b);
            b
        };
        match self {
            ShaperKind::Coarse(c) => {
                let b = block(&mut c.coarse, inputs[0]);
                c.process(io, &b)
            }
            ShaperKind::Crush(c) => {
                let b = block(&mut c.crush, inputs[0]);
                c.process(io, &b)
            }
            ShaperKind::Shape(s) => {
                let (a, b) = (
                    block(&mut s.shape, inputs[0]),
                    block(&mut s.postgain, inputs[1]),
                );
                s.process(io, &a, &b)
            }
            ShaperKind::Distort(d) => {
                let (a, b) = (
                    block(&mut d.distort, inputs[0]),
                    block(&mut d.postgain, inputs[1]),
                );
                d.process(io, &a, &b)
            }
        }
    }
}

/// A shaper's `AudioWorkletNode`: the processor and its lifecycle.
#[derive(Debug, Clone)]
pub struct Shaper {
    pub kind: ShaperKind,
    life: Lifecycle,
    /// A processor error not yet reported.
    error: Option<ProcessorError>,
}

impl Shaper {
    pub fn new(kind: ShaperKind) -> Shaper {
        Shaper {
            kind,
            life: Lifecycle::new(),
            error: None,
        }
    }

    /// One quantum in place on the voice bus's channels; returns the output's silence hint. The
    /// input is always connected (see the module docs): a live processor outputs a non-silent
    /// bus, a finished one (after an error) a silent zero bus. The quantum that throws outputs
    /// zeros.
    #[cfg(test)]
    pub fn process(&mut self, q: u64, io: &mut [[f32; QUANTUM]]) -> bool {
        self.process_with(q, io, [&ParamInputs::NONE, &ParamInputs::NONE])
    }

    /// [`Shaper::process`] with the connections into its params (slot 0 the shaper's own:
    /// `coarse`, `crush`, `shape` or `distort`; slot 1 `postgain`).
    pub fn process_with(
        &mut self,
        q: u64,
        io: &mut [[f32; QUANTUM]],
        inputs: [&ParamInputs; 2],
    ) -> bool {
        if !self.life.begin_quantum(true) {
            zero(io);
            return true;
        }
        let r = self.kind.run(q, io, inputs);
        if let Err(e) = r {
            zero(io);
            self.error = Some(e);
        }
        self.life.finish_quantum(r, true);
        false
    }

    /// The processor error to report, once.
    pub fn take_error(&mut self) -> Option<ProcessorError> {
        self.error.take()
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    const Q: usize = QUANTUM;

    fn ramp(offset: f32) -> [f32; Q] {
        let mut b = [0.0f32; Q];
        for (i, x) in b.iter_mut().enumerate() {
            *x = (i as f32 + offset) * 0.01 - 0.6;
        }
        b
    }

    fn bits(x: &[f32]) -> Vec<u32> {
        x.iter().map(|v| v.to_bits()).collect()
    }

    /// The frames `coarse(c)` takes from the input in one quantum.
    fn taken(c: f32) -> Vec<usize> {
        let mut p = Coarse::new();
        let input = ramp(0.0);
        let mut io = [input];
        assert_eq!(p.process(&mut io, &[c; Q]), Ok(true));
        let mut taken = Vec::new();
        for n in 0..Q {
            if io[0][n] == input[n] {
                taken.push(n);
            } else {
                assert_eq!(io[0][n].to_bits(), io[0][n - 1].to_bits(), "held at {n}");
            }
        }
        taken
    }

    #[test]
    fn coarse_hold_patterns() {
        assert_eq!(taken(8.0), (0..Q).step_by(8).collect::<Vec<_>>());
        assert_eq!(taken(3.0), (0..Q).step_by(3).collect::<Vec<_>>());
        assert_eq!(
            *taken(3.0).last().unwrap(),
            126,
            "128 mod 3: the last hold is 2 frames"
        );
        // 2.5: n % 2.5 < 1 at 0, 3, 5, 8, 10, ...
        let t = taken(2.5);
        assert_eq!(&t[..6], &[0, 3, 5, 8, 10, 13]);
        assert!(t.iter().all(|n| (*n as f64) % 2.5 < 1.0));
        assert_eq!(taken(1.0), (0..Q).collect::<Vec<_>>());
        assert_eq!(taken(0.5), (0..Q).collect::<Vec<_>>(), "clamped to 1");
        assert_eq!(taken(1e9), vec![0], "only frame 0");
        assert_eq!(
            taken(-3.0),
            (0..Q).collect::<Vec<_>>(),
            "negative clamps to 1"
        );
        // The hold restarts every quantum and is exact on stereo input.
        let mut p = Coarse::new();
        let (a, b) = (ramp(0.0), ramp(7.0));
        let mut io = [a, b];
        p.process(&mut io, &[8.0; Q]).unwrap();
        for n in 0..Q {
            assert_eq!(io[0][n].to_bits(), a[n - n % 8].to_bits());
            assert_eq!(io[1][n].to_bits(), b[n - n % 8].to_bits());
        }
    }

    #[test]
    fn shapers_implement_the_started_rule() {
        let mut p = Coarse::new();
        // No input before any: started stays false, still true.
        assert_eq!(p.process(&mut [], &[1.0; Q]), Ok(true));
        let mut io = [ramp(0.0)];
        assert_eq!(p.process(&mut io, &[1.0; Q]), Ok(true));
        assert_eq!(p.process(&mut [], &[1.0; Q]), Ok(false), "input gone");
        let mut c = Crush::new();
        c.process(&mut io, &[4.0; Q]).unwrap();
        assert_eq!(c.process(&mut [], &[4.0; Q]), Ok(false));
        let mut s = Shape::new();
        s.process(&mut io, &[0.5; Q], &[1.0; Q]).unwrap();
        assert_eq!(s.process(&mut [], &[0.5; Q], &[1.0; Q]), Ok(false));
        let mut d = Distort::new(&Value::Num(0.0));
        d.process(&mut io, &[1.0; Q], &[1.0; Q]).unwrap();
        assert_eq!(d.process(&mut [], &[1.0; Q], &[1.0; Q]), Ok(false));
    }

    fn crush_one(c: f32, x: f32) -> f32 {
        let mut p = Crush::new();
        let mut io = [[x; Q]];
        p.process(&mut io, &[c; Q]).unwrap();
        io[0][5]
    }

    #[test]
    fn crush_rounds_halves_toward_positive_infinity() {
        // crush(1): x = 1, outputs in {-1, 0, 1}.
        assert_eq!(crush_one(1.0, 0.5), 1.0);
        assert_eq!(crush_one(1.0, -0.5).to_bits(), (-0.0f32).to_bits());
        assert_eq!(crush_one(1.0, 1.5), 2.0);
        assert_eq!(crush_one(1.0, -1.5), -1.0);
        assert_eq!(crush_one(1.0, 0.49), 0.0);
        // crush(4): x = 8.
        assert_eq!(crush_one(4.0, 0.0625), 0.125);
        assert_eq!(crush_one(4.0, -0.0625).to_bits(), (-0.0f32).to_bits());
        assert_eq!(crush_one(4.0, 0.1875), 0.25);
        assert_eq!(crush_one(4.0, -0.1875), -0.125);
        // crush(0) clamps to 1.
        assert_eq!(crush_one(0.0, 0.5), crush_one(1.0, 0.5));
        assert_eq!(crush_one(-5.0, -0.7), -1.0);
        // crush(3.5): x = pow(2, 2.5), not dyadic.
        let x = jsmath::pow(2.0, 2.5);
        for v in [0.3f32, -0.7, 0.123_456_7, 0.99] {
            let want = (jsmath::round(f64::from(v) * x) / x) as f32;
            assert_eq!(crush_one(3.5, v).to_bits(), want.to_bits(), "{v}");
        }
        // 0 and -0 stay.
        assert_eq!(crush_one(4.0, 0.0).to_bits(), 0);
        assert_eq!(crush_one(4.0, -0.0).to_bits(), (-0.0f32).to_bits());
    }

    fn shape_one(shape: f32, post: f32, x: f32) -> f32 {
        let mut p = Shape::new();
        let mut io = [[x; Q]];
        p.process(&mut io, &[shape; Q], &[post; Q]).unwrap();
        io[0][0]
    }

    #[test]
    fn shape_curve_and_postgain_clamp() {
        // shape 0: identity times post.
        for x in [0.3f32, -0.9, 1.0] {
            assert_eq!(shape_one(0.0, 1.0, x), x);
            assert_eq!(shape_one(0.0, 0.5, x), x * 0.5);
        }
        // shape 0.6: s = 1.2 / 0.4.
        let s = (2.0 * f64::from(0.6f32)) / (1.0 - f64::from(0.6f32));
        let x = 0.25f64;
        let want = ((((1.0 + s) * x) / (1.0 + s * x.abs())) * 0.5) as f32;
        assert_eq!(shape_one(0.6, 0.5, 0.25).to_bits(), want.to_bits());
        // shape 1 (and above) is clamped to 1 - 4e-10: s is about 5e9.
        let s1 = (2.0 * (1.0 - 4e-10)) / (1.0 - (1.0 - 4e-10));
        let want = ((((1.0 + s1) * x) / (1.0 + s1 * x)) * 1.0) as f32;
        assert_eq!(shape_one(1.0, 1.0, 0.25).to_bits(), want.to_bits());
        assert_eq!(shape_one(7.0, 1.0, 0.25).to_bits(), want.to_bits());
        assert!((s1 - 5e9).abs() < 1e3);
        // shape -1: s = -1, a zero denominator at |x| = 1 (not special-cased).
        assert!(!shape_one(-1.0, 1.0, 1.0).is_finite());
        assert!(shape_one(-1.0, 1.0, 0.5).is_finite());
        // shapevol above 1 is clamped to 1, 0 to 0.001.
        assert_eq!(shape_one(0.0, 2.0, 0.5), 0.5);
        assert_eq!(shape_one(0.0, 0.0, 0.5), (0.5f64 * 0.001) as f32);
    }

    #[test]
    fn every_algorithm_maps_zero_to_zero() {
        for (name, f) in DISTORTION_NAMES.iter().zip(ALGORITHMS) {
            for k in [0.0, jsmath::expm1(2.0), 100.0] {
                let p = f(0.0, k);
                assert_eq!(p, 0.0, "{name} at +0, k {k}");
                let n = f(-0.0, k);
                assert_eq!(n, 0.0, "{name} at -0, k {k}");
            }
        }
        // The signs at -0: scurve, soft and hard keep it; cubic (`-0 - -0`), the folds
        // (`1 - 1`), the diodes (`pos - neg` of equal values) and chebyshev (`0 + -0`) give +0.
        for (name, f) in DISTORTION_NAMES.iter().zip(ALGORITHMS) {
            let negative = matches!(*name, "scurve" | "soft" | "hard");
            assert_eq!(f(-0.0, 1.0).is_sign_negative(), negative, "{name}");
            assert!(f(0.0, 1.0).is_sign_positive(), "{name}");
        }
    }

    #[test]
    fn algorithms_at_k_zero() {
        for x in [-2.0, -1.0, -0.75, -0.25, 0.0, 0.5, 1.0, 1.5] {
            assert_eq!(scurve(x, 0.0), x, "scurve {x}");
            assert_eq!(soft(x, 0.0), jsmath::tanh(x), "soft {x}");
            assert_eq!(hard(x, 0.0), x.clamp(-1.0, 1.0), "hard {x}");
            if (-1.0..=1.0).contains(&x) {
                // Exact for these dyadic values.
                assert_eq!(fold(x, 0.0), x, "fold {x}");
            }
        }
        // Each is odd at k = 0 on dyadic points.
        for (name, f) in DISTORTION_NAMES.iter().zip(ALGORITHMS) {
            for x in [0.25, 0.5, 0.75] {
                assert_eq!(f(-x, 0.0), -f(x, 0.0), "{name} odd at {x}");
            }
        }
        // fold folds: past 1 it comes back down.
        assert_eq!(fold(1.5, 0.0), 0.5);
        assert_eq!(fold(3.0, 0.0), -1.0);
    }

    #[test]
    fn algorithm_index_mapping() {
        let num = |x: f64| algorithm_index(&Value::Num(x));
        assert_eq!(num(0.0), Some(0));
        assert_eq!(num(8.0), Some(8));
        assert_eq!(num(9.0), Some(0));
        assert_eq!(num(10.0), Some(1));
        assert_eq!(num(-0.0), Some(0));
        assert_eq!(num(-9.0), Some(0), "-9 % 9 is -0, which names entry 0");
        assert_eq!(num(-1.0), None);
        assert_eq!(num(1.5), None);
        assert_eq!(num(f64::NAN), None);
        assert_eq!(num(f64::INFINITY), None);
        assert_eq!(num(1e300), Some((1e300f64 % 9.0) as usize));
        for (i, n) in DISTORTION_NAMES.iter().enumerate() {
            assert_eq!(algorithm_index(&Value::Str((*n).into())), Some(i));
        }
        assert_eq!(algorithm_index(&Value::Str("nope".into())), Some(0));
        assert_eq!(
            algorithm_index(&Value::Str("3".into())),
            Some(0),
            "by name only"
        );
        assert_eq!(algorithm_index(&Value::Null), Some(0));
        assert_eq!(algorithm_index(&Value::Bool(true)), Some(1));
        assert_eq!(algorithm_index(&Value::Bool(false)), Some(0));
        assert_eq!(algorithm_index(&Value::Arr(vec![Value::Num(3.0)])), None);
        assert_eq!(distortion_algorithm("fold").map(|f| f(1.5, 0.0)), Some(0.5));
        assert!(distortion_algorithm("Fold").is_none());
    }

    #[test]
    fn distort_per_sample_postgain_and_k() {
        let mut d = Distort::new(&Value::Num(0.0));
        let input = ramp(0.0);
        let mut io = [input];
        let mut dist = [2.0f32; Q];
        dist[3] = 0.5;
        let mut post = [0.4f32; Q];
        post[3] = 7.0;
        post[4] = -1.0;
        d.process(&mut io, &dist, &post).unwrap();
        for n in 0..Q {
            let p = jsmath::clamp(f64::from(post[n]), 0.001, 1.0);
            let k = jsmath::expm1(f64::from(dist[n]));
            let want = (p * scurve(f64::from(input[n]), k)) as f32;
            assert_eq!(io[0][n].to_bits(), want.to_bits(), "frame {n}");
        }
        assert_eq!(
            io[0][4],
            (0.001 * scurve(f64::from(input[4]), jsmath::expm1(2.0))) as f32
        );
    }

    #[test]
    fn an_invalid_distorttype_throws_at_the_first_call_with_input() {
        for v in [Value::Num(-1.0), Value::Num(1.5), Value::Num(f64::NAN)] {
            let mut d = Distort::new(&v);
            // No input: the loop body never runs, nothing throws.
            assert_eq!(d.process(&mut [], &[0.0; Q], &[1.0; Q]), Ok(true));
            let mut io = [ramp(0.0)];
            let before = io;
            let e = d.process(&mut io, &[1.0; Q], &[1.0; Q]).unwrap_err();
            assert_eq!(e.processor, "distort");
            assert_eq!(
                bits(&io[0]),
                bits(&before[0]),
                "nothing written before the throw"
            );
        }
        // Through the node: the throwing quantum and every later one output zeros, the error is
        // reported once, the processor is not called again.
        let mut node = Shaper::new(ShaperKind::Distort(Box::new(Distort::new(&Value::Num(
            -1.0,
        )))));
        let mut io = [ramp(0.0)];
        assert!(
            !node.process(0, &mut io),
            "the throwing quantum is not silent"
        );
        assert!(io[0].iter().all(|x| x.to_bits() == 0));
        assert_eq!(
            node.take_error().map(|e| e.message()),
            Some("distort: processor error; Chromium outputs silence".into())
        );
        assert_eq!(node.take_error(), None);
        let mut io = [ramp(0.0)];
        assert!(node.process(128, &mut io), "gone: silent");
        assert!(io[0].iter().all(|x| x.to_bits() == 0));
        assert_eq!(node.take_error(), None);
    }

    #[test]
    fn a_shaper_node_reads_its_param_timeline() {
        let mut c = Crush::new();
        c.crush.set_value(4.0, 0).unwrap();
        let mut node = Shaper::new(ShaperKind::Crush(c));
        let mut io = [[0.0625f32; Q], [-0.0625f32; Q]];
        assert!(!node.process(0, &mut io));
        assert!(io[0].iter().all(|x| *x == 0.125));
        assert!(io[1].iter().all(|x| x.to_bits() == (-0.0f32).to_bits()));
        // A silent zero input is processed (zeros out, non-silent: the worklet rule).
        let mut io = [[0.0f32; Q]];
        assert!(!node.process(128, &mut io));
        assert!(io[0].iter().all(|x| x.to_bits() == 0));
        // The default postgain of shape and distort is 1.
        let s = Shape::new();
        assert_eq!((s.shape.value(), s.postgain.value()), (0.0, 1.0));
        let d = Distort::new(&Value::Null);
        assert_eq!((d.distort.value(), d.postgain.value()), (0.0, 1.0));
        assert_eq!(Coarse::new().coarse.value(), 1.0);
        assert_eq!(Crush::new().crush.value(), 0.0);
    }
}
