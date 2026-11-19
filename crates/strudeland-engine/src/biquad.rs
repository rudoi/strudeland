// Port of Blink (Chromium 156.0.8078.4) platform/audio/biquad.cc, modules/webaudio/biquad_filter_handler.cc, modules/webaudio/biquad_filter_node.cc, modules/webaudio/audio_handler.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! The `BiquadFilterNode` superdough's voices use (`lowpass`, `highpass`, `bandpass`, and the
//! phaser's `notch`).
//!
//! - Coefficients (`biquad.cc` `Set*Params`): RBJ, all in `f64`; lowpass and highpass take Q in
//!   dB through `pow10(x) = fdlibm::expf(x * ln10)` (so the resonance has `f32` precision),
//!   bandpass and notch take a linear Q; `fdlibm` `sin`/`cos` (`libm`).
//! - Kernel (`Biquad::Process`): with per-frame coefficients, a direct form I evaluated in `f64`
//!   left to right (Chromium builds with `-ffp-contract=off`) whose output and output history
//!   are rounded to `f32`, the four states passed through `FlushDenormalFloatToZero` at the end
//!   of the quantum; otherwise the macOS path (`ProcessFast`, `vDSP_deq22D` over `f64` buffers):
//!   `y = fma(-a2, y2, fma(-a1, y1, fma(b0, x, fma(b2, x2, b1 * x1))))`, the evaluation order
//!   measured from Accelerate on the reference machine (bit-identical over 1.5 million samples;
//!   a test re-checks it against Accelerate on macOS), output history kept in `f64`.
//! - Node (`BiquadFilterHandler::Process`): the dirty/sample-accurate rules, the all-constant
//!   shortcut, the first-render snap, one kernel per input channel, and the tail time
//!   (`Biquad::TailFrame`) with `AudioHandler`'s silence propagation: once the input has been
//!   silent for longer than the tail time the node outputs silence and only evaluates its params.

use crate::QUANTUM;
use crate::param::{Param, ParamInputs, Rate};

use crate::SAMPLE_RATE_F64 as SR;
const NYQUIST: f64 = 0.5 * SR;
/// `kMaxTailTime` (seconds).
pub const MAX_TAIL_TIME: f64 = 30.0;
/// `kMaxTailAmplitude` of `TailFrame`: one bit of 16-bit PCM.
pub const MAX_TAIL_AMPLITUDE: f64 = 1.0 / 32768.0;

/// The filter types superdough uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
}

impl FilterKind {
    /// `filter.type = 'lowpass' | 'highpass' | 'bandpass'`.
    #[cfg(test)]
    pub fn from_name(name: &str) -> Option<FilterKind> {
        match name {
            "lowpass" => Some(FilterKind::Lowpass),
            "highpass" => Some(FilterKind::Highpass),
            "bandpass" => Some(FilterKind::Bandpass),
            "notch" => Some(FilterKind::Notch),
            _ => None,
        }
    }
}

/// One set of normalized coefficients (`a0` divided out).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coefficients {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

/// The pass-through set `(1, 0, 0, 1, 0, 0)`.
pub const PASS: Coefficients = Coefficients {
    b0: 1.0,
    b1: 0.0,
    b2: 0.0,
    a1: 0.0,
    a2: 0.0,
};
/// The silent set `(0, 0, 0, 1, 0, 0)`.
pub const ZERO: Coefficients = Coefficients {
    b0: 0.0,
    b1: 0.0,
    b2: 0.0,
    a1: 0.0,
    a2: 0.0,
};

/// `SetNormalizedCoefficients`: every coefficient times `1 / a0`.
pub fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Coefficients {
    let a0_inverse = 1.0 / a0;
    Coefficients {
        b0: b0 * a0_inverse,
        b1: b1 * a0_inverse,
        b2: b2 * a0_inverse,
        a1: a1 * a0_inverse,
        a2: a2 * a0_inverse,
    }
}

/// `pow10(x) = fdlibm::expf(x * ln10)`: the argument is rounded to `f32`, `exp` runs in `f64`, the
/// result is rounded to `f32`; a subnormal result is flushed to zero (the audio thread runs with
/// arm64's flush-to-zero mode, which the conversion obeys), so a Q below about -758 dB gives a
/// resonance of 0.
pub fn pow10(x: f64) -> f64 {
    let r = libm::exp(f64::from((x * std::f64::consts::LN_10) as f32)) as f32;
    if r.is_subnormal() { 0.0 } else { f64::from(r) }
}

/// `ClampTo(x, lo, hi)` on doubles (NaN passes through).
fn clamp_to(x: f64, lo: f64, hi: f64) -> f64 {
    if x >= hi {
        hi
    } else if x <= lo {
        lo
    } else {
        x
    }
}

/// `SetLowpassParams(cutoff, resonance_db)`.
pub fn lowpass(cutoff: f64, resonance: f64) -> Coefficients {
    let cutoff = clamp_to(cutoff, 0.0, 1.0);
    if cutoff == 1.0 {
        PASS
    } else if cutoff > 0.0 {
        let resonance = pow10(resonance / 20.0);
        let theta = std::f64::consts::PI * cutoff;
        let alpha = libm::sin(theta) / (2.0 * resonance);
        let cosw = libm::cos(theta);
        let beta = (1.0 - cosw) / 2.0;
        normalized(
            beta,
            2.0 * beta,
            beta,
            1.0 + alpha,
            -2.0 * cosw,
            1.0 - alpha,
        )
    } else {
        ZERO
    }
}

/// `SetHighpassParams(cutoff, resonance_db)`.
pub fn highpass(cutoff: f64, resonance: f64) -> Coefficients {
    let cutoff = clamp_to(cutoff, 0.0, 1.0);
    if cutoff == 1.0 {
        ZERO
    } else if cutoff > 0.0 {
        let resonance = pow10(resonance / 20.0);
        let theta = std::f64::consts::PI * cutoff;
        let alpha = libm::sin(theta) / (2.0 * resonance);
        let cosw = libm::cos(theta);
        let beta = (1.0 + cosw) / 2.0;
        normalized(
            beta,
            -2.0 * beta,
            beta,
            1.0 + alpha,
            -2.0 * cosw,
            1.0 - alpha,
        )
    } else {
        PASS
    }
}

/// `SetBandpassParams(frequency, q)` (linear Q).
pub fn bandpass(frequency: f64, q: f64) -> Coefficients {
    let frequency = js_like_max0(frequency);
    let q = js_like_max0(q);
    if frequency > 0.0 && frequency < 1.0 {
        let w0 = std::f64::consts::PI * frequency;
        if q > 0.0 {
            let alpha = libm::sin(w0) / (2.0 * q);
            let k = libm::cos(w0);
            normalized(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * k, 1.0 - alpha)
        } else {
            PASS
        }
    } else {
        ZERO
    }
}

/// `SetNotchParams(frequency, q)` (linear Q): `PASS` at 0 and 1 (and for a NaN frequency, which
/// the clamp passes through), `ZERO` for `q <= 0` (NaN too).
pub fn notch(frequency: f64, q: f64) -> Coefficients {
    let frequency = clamp_to(frequency, 0.0, 1.0);
    let q = js_like_max0(q);
    if frequency > 0.0 && frequency < 1.0 {
        if q > 0.0 {
            let w0 = std::f64::consts::PI * frequency;
            let alpha = libm::sin(w0) / (2.0 * q);
            let k = libm::cos(w0);
            normalized(1.0, -2.0 * k, 1.0, 1.0 + alpha, -2.0 * k, 1.0 - alpha)
        } else {
            ZERO
        }
    } else {
        PASS
    }
}

/// `std::max(0.0, x)`: returns the first argument (0) unless `0 < x`, so NaN gives 0.
fn js_like_max0(x: f64) -> f64 {
    if 0.0 < x { x } else { 0.0 }
}

/// The coefficients of `kind` (`SetBiquadParams`; the gain only matters for shelving and peaking
/// filters, which superdough does not use).
pub fn coefficients(kind: FilterKind, frequency: f64, q: f64) -> Coefficients {
    match kind {
        FilterKind::Lowpass => lowpass(frequency, q),
        FilterKind::Highpass => highpass(frequency, q),
        FilterKind::Bandpass => bandpass(frequency, q),
        FilterKind::Notch => notch(frequency, q),
    }
}

/// Which `exp2` overload `NormalizeFrequency` calls on the `float` quotient `detune / 1200`: the
/// double one (`false`: the quotient widened, `exp2` in double) or the float one (`true`: libc++'s
/// `exp2f` overload, then widened). The `phaser-notch` capture settled it: the float overload
/// nulls at -inf dB, the double one misses at -135 dB
/// (`plans/0011-engine-effects/evidence/phaser-exp2.json`). Only a non-zero detune reaches it.
const DETUNE_EXP2_F32: bool = true;

/// `NormalizeFrequency(frequency, nyquist, detune)`: `frequency / nyquist`, times
/// `exp2(detune / 1200)` (float division, the platform's float `exp2f`, see [`DETUNE_EXP2_F32`])
/// only when the detune is not 0.
pub fn normalize_frequency(frequency: f32, detune: f32) -> f64 {
    let normalized = f64::from(frequency) / NYQUIST;
    if detune != 0.0 {
        if DETUNE_EXP2_F32 {
            normalized * f64::from((detune / 1200.0f32).exp2())
        } else {
            normalized * f64::from(detune / 1200.0f32).exp2()
        }
    } else {
        normalized
    }
}

/// `DenormalDisabler::FlushDenormalFloatToZero` on a double state: the value as `f32`, a
/// denormal flushed to zero (arm64 runs the audio thread with flush-to-zero).
fn flush(x: f64) -> f64 {
    let f = x as f32;
    if f.abs() < f32::MIN_POSITIVE {
        0.0
    } else {
        f64::from(f)
    }
}

/// One channel's filter (`platform/audio/biquad.cc`): up to 128 coefficient sets and the history.
#[derive(Debug, Clone)]
pub struct Kernel {
    coefficients: Box<[Coefficients; QUANTUM]>,
    sample_accurate: bool,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Default for Kernel {
    fn default() -> Self {
        Kernel::new()
    }
}

impl Kernel {
    /// A pass-through kernel with zero history.
    pub fn new() -> Kernel {
        Kernel {
            coefficients: Box::new([PASS; QUANTUM]),
            sample_accurate: false,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// `SetHasSampleAccurateValues`.
    pub fn set_sample_accurate(&mut self, on: bool) {
        self.sample_accurate = on;
    }

    #[cfg(test)]
    pub fn sample_accurate(&self) -> bool {
        self.sample_accurate
    }

    /// Stores coefficient set `index`.
    pub fn set(&mut self, index: usize, c: Coefficients) {
        self.coefficients[index] = c;
    }

    #[cfg(test)]
    pub fn get(&self, index: usize) -> Coefficients {
        self.coefficients[index]
    }

    /// `(x1, x2, y1, y2)`.
    #[cfg(test)]
    pub fn state(&self) -> (f64, f64, f64, f64) {
        (self.x1, self.x2, self.y1, self.y2)
    }

    /// `Biquad::Process` in place.
    pub fn process(&mut self, buf: &mut [f32]) {
        if self.sample_accurate {
            let (mut x1, mut x2, mut y1, mut y2) = (self.x1, self.x2, self.y1, self.y2);
            for (k, s) in buf.iter_mut().enumerate() {
                let c = &self.coefficients[k];
                let x = f64::from(*s);
                let y = (c.b0 * x + c.b1 * x1 + c.b2 * x2 - c.a1 * y1 - c.a2 * y2) as f32;
                *s = y;
                x2 = x1;
                x1 = x;
                y2 = y1;
                y1 = f64::from(y);
            }
            self.x1 = flush(x1);
            self.x2 = flush(x2);
            self.y1 = flush(y1);
            self.y2 = flush(y2);
        } else {
            self.process_fast(buf);
        }
    }

    /// `ProcessFast` (macOS): `vDSP_deq22D` with coefficient set 0 over the input as doubles,
    /// with the history as the two leading samples of each buffer; outputs cast to `f32`.
    fn process_fast(&mut self, buf: &mut [f32]) {
        let c = self.coefficients[0];
        let (na1, na2) = (-c.a1, -c.a2);
        let (mut x1, mut x2, mut y1, mut y2) = (self.x1, self.x2, self.y1, self.y2);
        for s in buf.iter_mut() {
            let x = f64::from(*s);
            let y = na2.mul_add(
                y2,
                na1.mul_add(y1, c.b0.mul_add(x, c.b2.mul_add(x2, c.b1 * x1))),
            );
            *s = y as f32;
            x2 = x1;
            x1 = x;
            y2 = y1;
            y1 = y;
        }
        self.x1 = x1;
        self.x2 = x2;
        self.y1 = y1;
        self.y2 = y2;
    }

    /// `Biquad::TailFrame(index, max_frame)`: the frame after which the impulse response of
    /// set `index` stays below [`MAX_TAIL_AMPLITUDE`].
    pub fn tail_frame(&self, index: usize, max_frame: f64) -> f64 {
        tail_frame(&self.coefficients[index], max_frame)
    }
}

/// `std::complex<double>` with libc++'s arithmetic (no fused multiply-add).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Complex {
    re: f64,
    im: f64,
}

impl Complex {
    fn mul(self, w: Complex) -> Complex {
        let (a, b, c, d) = (self.re, self.im, w.re, w.im);
        let (ac, bd, ad, bc) = (a * c, b * d, a * d, b * c);
        Complex {
            re: ac - bd,
            im: ad + bc,
        }
    }

    fn scale(self, x: f64) -> Complex {
        Complex {
            re: self.re * x,
            im: self.im * x,
        }
    }

    fn add(self, w: Complex) -> Complex {
        Complex {
            re: self.re + w.re,
            im: self.im + w.im,
        }
    }

    fn sub(self, w: Complex) -> Complex {
        Complex {
            re: self.re - w.re,
            im: self.im - w.im,
        }
    }

    /// libc++ `operator/`: scales the divisor by `logb(max(|c|, |d|))` (finite values only).
    fn div(self, w: Complex) -> Complex {
        let (a, b) = (self.re, self.im);
        let (mut c, mut d) = (w.re, w.im);
        let mut ilogbw = 0;
        let logbw = logb(c.abs().max(d.abs()));
        if logbw.is_finite() {
            ilogbw = logbw as i32;
            c = libm::scalbn(c, -ilogbw);
            d = libm::scalbn(d, -ilogbw);
        }
        let denom = c * c + d * d;
        Complex {
            re: libm::scalbn((a * c + b * d) / denom, -ilogbw),
            im: libm::scalbn((b * c - a * d) / denom, -ilogbw),
        }
    }

    /// libc++ `abs`: `hypot`.
    fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
}

/// `logb(x)`: the unbiased exponent as a double (`-inf` for 0, `inf` for infinities, NaN for NaN).
fn logb(x: f64) -> f64 {
    if x == 0.0 {
        f64::NEG_INFINITY
    } else if !x.is_finite() {
        x.abs()
    } else {
        f64::from(libm::ilogb(x))
    }
}

/// `RepeatedRootResponse`.
fn repeated_root_response(n: f64, c1: f64, c2: f64, r: f64, log_eps: f64) -> f64 {
    (n - 2.0) * libm::log(r.abs()) + libm::log((c1 * (n + 1.0) * r * r + c2).abs()) - log_eps
}

/// `RootFinder` (regula falsi with the Illinois tweak, at most 10 iterations).
#[allow(clippy::too_many_arguments)]
fn root_finder(
    mut low: f64,
    mut high: f64,
    mut f_low: f64,
    mut f_high: f64,
    log_eps: f64,
    c1: f64,
    c2: f64,
    r: f64,
) -> f64 {
    const ACCURACY: f64 = 0.5;
    const MAX_ITERATIONS: usize = 10;
    let mut side = 0;
    let mut root = 0.0;
    for _ in 0..MAX_ITERATIONS {
        root = (f_low * high - f_high * low) / (f_low - f_high);
        if (high - low).abs() < ACCURACY * (high + low).abs() {
            break;
        }
        let fr = repeated_root_response(root, c1, c2, r, log_eps);
        if fr * f_high > 0.0 {
            high = root;
            f_high = fr;
            side = -1;
        } else if f_low * fr > 0.0 {
            low = root;
            f_low = fr;
            if side == 1 {
                f_high /= 2.0;
            }
            side = 1;
        } else {
            break;
        }
    }
    root
}

/// `Biquad::TailFrame` for one coefficient set.
pub fn tail_frame(c: &Coefficients, max_frame: f64) -> f64 {
    let Coefficients { b0, b1, b2, a1, a2 } = *c;
    if !(a1.is_finite() && a2.is_finite() && b0.is_finite() && b1.is_finite() && b2.is_finite()) {
        return 0.0;
    }
    let discrim = a1 * a1 - 4.0 * a2;
    if discrim > 0.0 {
        let rplus = (-a1 + discrim.sqrt()) / 2.0;
        let rminus = (-a1 - discrim.sqrt()) / 2.0;
        let r1 = if rplus.abs() >= rminus.abs() {
            rplus
        } else {
            rminus
        };
        let r2 = a2 / r1;
        let c1 = (b0 * r1 * r1 + b1 * r1 + b2) / (r2 - r1);
        let c2 = (b0 * r2 * r2 + b1 * r2 + b2) / (r2 - r1);
        let t = 1.0 + libm::log(MAX_TAIL_AMPLITUDE / (c1.abs() + c2.abs())) / libm::log(r1.abs());
        clamp_to(t, 0.0, f64::MAX)
    } else if discrim < 0.0 {
        let x = -a1 / 2.0;
        let y = (-discrim).sqrt() / 2.0;
        let r1 = Complex { re: x, im: y };
        let r2 = Complex { re: x, im: -y };
        let r = x.hypot(y);
        if r == 1.0 {
            return max_frame;
        }
        let b2c = Complex { re: b2, im: 0.0 };
        let num = |z: Complex| z.scale(b0).mul(z).add(z.scale(b1)).add(b2c);
        let den = r2.sub(r1);
        let c1 = num(r1).div(den).abs();
        let c2 = num(r2).div(den).abs();
        let t = 1.0 + libm::log(MAX_TAIL_AMPLITUDE / (c1 + c2)) / libm::log(r);
        if c1 == 0.0 && c2 == 0.0 { 0.0 } else { t }
    } else {
        let r = -a1 / 2.0;
        if r == 0.0 {
            2.0
        } else if r.abs() >= 1.0 {
            f64::INFINITY
        } else {
            let c1 = (b0 * r * r + b1 * r + b2) / (r * r);
            let c2 = b1 * r + 2.0 * b2;
            if c1 == 0.0 && c2 == 0.0 {
                return 0.0;
            }
            let log_r = libm::log(r.abs());
            let low = clamp_to(-(1.0 + log_r) / log_r, 1.0, max_frame - 1.0);
            let high = max_frame;
            let log_eps = libm::log(MAX_TAIL_AMPLITUDE);
            let f_low = repeated_root_response(low, c1, c2, r, log_eps);
            let f_high = repeated_root_response(high, c1, c2, r, log_eps);
            if f_low <= 0.0 {
                0.0
            } else if f_high >= 0.0 {
                high
            } else {
                root_finder(low, high, f_low, f_high, log_eps, c1, c2, r)
            }
        }
    }
}

/// `HasConstantValues`: every value equals the first (exact float comparison).
pub fn has_constant_values(values: &[f32]) -> bool {
    values
        .first()
        .is_none_or(|v0| values.iter().all(|v| v == v0))
}

/// The connections into a biquad's params for one quantum (`frequency`: a filter LFO; `detune`:
/// the phaser's LFO), borrowed from the stage's inputs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BiquadInputs<'a> {
    pub frequency: &'a ParamInputs,
    pub detune: &'a ParamInputs,
    pub q: &'a ParamInputs,
    pub gain: &'a ParamInputs,
}

impl BiquadInputs<'static> {
    /// No param connected.
    pub const NONE: BiquadInputs<'static> = BiquadInputs {
        frequency: &ParamInputs::NONE,
        detune: &ParamInputs::NONE,
        q: &ParamInputs::NONE,
        gain: &ParamInputs::NONE,
    };
}

/// A `BiquadFilterNode` (`BiquadFilterHandler`) of one type, with its four params, one kernel
/// per input channel and the silence-propagation state of its `AudioHandler`.
#[derive(Debug, Clone)]
pub struct Biquad {
    kind: FilterKind,
    /// `frequency` (a-rate, default 350, range `[0, nyquist]`).
    pub frequency: Param,
    /// `Q` (a-rate, default 1, the whole float range).
    pub q: Param,
    /// `gain` (a-rate, default 0; unused by these types but evaluated as Blink does).
    pub gain: Param,
    /// `detune` (a-rate, default 0, cents).
    pub detune: Param,
    kernels: Vec<Kernel>,
    has_just_reset: bool,
    previous: [f32; 4],
    tail_time: f64,
    last_non_silent_time: f64,
}

impl Biquad {
    /// `createBiquadFilter()` then `filter.type = kind`: one mono kernel, default params.
    pub fn new(kind: FilterKind) -> Biquad {
        Biquad {
            kind,
            frequency: Param::new(350.0, 0.0, NYQUIST as f32, Rate::A),
            q: Param::new(1.0, -f32::MAX, f32::MAX, Rate::A),
            gain: Param::new(0.0, f32::MIN, 40.0 * f32::MAX.log10(), Rate::A),
            detune: Param::new(
                0.0,
                -1200.0 * f32::MAX.log2(),
                1200.0 * f32::MAX.log2(),
                Rate::A,
            ),
            kernels: vec![Kernel::new()],
            has_just_reset: true,
            previous: [f32::NAN; 4],
            tail_time: f64::INFINITY,
            last_non_silent_time: 0.0,
        }
    }

    #[cfg(test)]
    pub fn kind(&self) -> FilterKind {
        self.kind
    }

    /// The kernels (one per channel of the last input).
    #[cfg(test)]
    pub fn kernels(&self) -> &[Kernel] {
        &self.kernels
    }

    /// `TailTime()` (seconds; infinite until the first coefficient update).
    #[cfg(test)]
    pub fn tail_time(&self) -> f64 {
        self.tail_time
    }

    /// `AudioHandler::PropagatesSilence` at the quantum starting at `quantum_start` (latency 0).
    pub fn propagates_silence(&self, quantum_start: u64) -> bool {
        self.last_non_silent_time + self.tail_time < quantum_start as f64 / SR
    }

    /// `AudioHandler::ProcessIfNecessary` for this node on the quantum at `quantum_start`:
    /// filters `channels` in place (one kernel per channel; a channel count change re-creates
    /// the kernels as `CheckNumberOfChannelsForInput` does). `input_silent` is the input bus's
    /// silence hint (the upstream source is not playing); returns the output's. `inputs` are the
    /// connections into `frequency` and `detune`: a connected param makes every quantum sample
    /// accurate (per-frame coefficients unless all values are equal), the timeline plus the
    /// inputs, NaN replaced by the default, clipped to the range.
    pub fn process(
        &mut self,
        quantum_start: u64,
        channels: &mut [[f32; QUANTUM]],
        input_silent: bool,
        inputs: &BiquadInputs<'_>,
    ) -> bool {
        if channels.len() != self.kernels.len() {
            self.kernels = vec![Kernel::new(); channels.len()];
            self.has_just_reset = true;
        }
        if input_silent && self.propagates_silence(quantum_start) {
            for c in channels.iter_mut() {
                c.fill(0.0);
            }
            self.process_only_audio_params(quantum_start, inputs);
            return true;
        }
        self.update_coefficients(quantum_start, inputs);
        for (kernel, c) in self.kernels.iter_mut().zip(channels.iter_mut()) {
            kernel.process(c);
        }
        if !input_silent {
            self.last_non_silent_time = crate::quantum_end_time(quantum_start);
        }
        false
    }

    /// `ProcessOnlyAudioParams`: the four params evaluated a-rate into a scratch buffer.
    fn process_only_audio_params(&mut self, quantum_start: u64, inputs: &BiquadInputs<'_>) {
        let mut scratch = [0.0f32; QUANTUM];
        let f = &inputs.frequency;
        self.frequency
            .fill_quantum_connected(quantum_start, &mut scratch, &f.bufs, f.connected);
        let (qi, gi) = (&inputs.q, &inputs.gain);
        self.q
            .fill_quantum_connected(quantum_start, &mut scratch, &qi.bufs, qi.connected);
        self.gain
            .fill_quantum_connected(quantum_start, &mut scratch, &gi.bufs, gi.connected);
        let d = &inputs.detune;
        self.detune
            .fill_quantum_connected(quantum_start, &mut scratch, &d.bufs, d.connected);
    }

    /// The coefficient part of `BiquadFilterHandler::Process`.
    fn update_coefficients(&mut self, q: u64, inputs: &BiquadInputs<'_>) {
        let mut dirty = false;
        let mut sample_accurate = false;
        let (fi, di) = (&inputs.frequency, &inputs.detune);
        let (qi, gi) = (&inputs.q, &inputs.gain);
        if self.frequency.has_sample_accurate_values(q, fi.connected)
            || self.q.has_sample_accurate_values(q, qi.connected)
            || self.gain.has_sample_accurate_values(q, gi.connected)
            || self.detune.has_sample_accurate_values(q, di.connected)
        {
            dirty = true;
            sample_accurate = true;
        } else if self.has_just_reset {
            self.previous = [f32::NAN; 4];
            dirty = true;
            self.has_just_reset = false;
        } else {
            let finals = [
                self.frequency.final_value(q, &[]),
                self.q.final_value(q, &[]),
                self.gain.final_value(q, &[]),
                self.detune.final_value(q, &[]),
            ];
            if finals.iter().zip(self.previous.iter()).any(|(a, b)| a != b) {
                dirty = true;
                self.previous = finals;
            }
        }
        if !dirty {
            return;
        }
        // Every param is a-rate, so a sample-accurate quantum is an a-rate one.
        let is_audio_rate = [&self.frequency, &self.q, &self.gain, &self.detune]
            .iter()
            .any(|p| p.rate() == Rate::A);
        let coef_index = if sample_accurate && is_audio_rate {
            let mut f = [0.0f32; QUANTUM];
            let mut qv = [0.0f32; QUANTUM];
            let mut g = [0.0f32; QUANTUM];
            let mut d = [0.0f32; QUANTUM];
            self.frequency
                .fill_quantum_connected(q, &mut f, &fi.bufs, fi.connected);
            self.q
                .fill_quantum_connected(q, &mut qv, &qi.bufs, qi.connected);
            self.gain
                .fill_quantum_connected(q, &mut g, &gi.bufs, gi.connected);
            self.detune
                .fill_quantum_connected(q, &mut d, &di.bufs, di.connected);
            let constant = has_constant_values(&f)
                && has_constant_values(&qv)
                && has_constant_values(&g)
                && has_constant_values(&d);
            let needed = if constant { 1 } else { QUANTUM };
            for kernel in &mut self.kernels {
                kernel.set_sample_accurate(needed > 1);
                for k in 0..needed {
                    let nf = normalize_frequency(f[k], d[k]);
                    kernel.set(k, coefficients(self.kind, nf, f64::from(qv[k])));
                }
            }
            needed - 1
        } else {
            let f = self.frequency.final_value(q, &[]);
            let qv = self.q.final_value(q, &[]);
            let _gain = self.gain.final_value(q, &[]);
            let d = self.detune.final_value(q, &[]);
            for kernel in &mut self.kernels {
                kernel.set_sample_accurate(false);
                let nf = normalize_frequency(f, d);
                kernel.set(0, coefficients(self.kind, nf, f64::from(qv)));
            }
            0
        };
        if let Some(k0) = self.kernels.first() {
            let tail = k0.tail_frame(coef_index, MAX_TAIL_TIME * SR) / SR;
            self.tail_time = clamp_to(tail, 0.0, MAX_TAIL_TIME);
        }
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    /// Lowpass/highpass/bandpass straight from the RBJ cookbook as Blink writes them, computed
    /// independently (resonance via `10^(q/20)` rounded to `f32`).
    fn reference(kind: FilterKind, cutoff: f64, q: f64) -> Coefficients {
        let w = std::f64::consts::PI * cutoff;
        let (s, c) = (libm::sin(w), libm::cos(w));
        match kind {
            FilterKind::Lowpass | FilterKind::Highpass => {
                let res = f64::from(10f64.powf(q / 20.0) as f32);
                let alpha = s / (2.0 * res);
                let beta = if kind == FilterKind::Lowpass {
                    (1.0 - c) / 2.0
                } else {
                    (1.0 + c) / 2.0
                };
                let sign = if kind == FilterKind::Lowpass {
                    1.0
                } else {
                    -1.0
                };
                let a0 = 1.0 + alpha;
                Coefficients {
                    b0: beta / a0,
                    b1: sign * 2.0 * beta / a0,
                    b2: beta / a0,
                    a1: -2.0 * c / a0,
                    a2: (1.0 - alpha) / a0,
                }
            }
            FilterKind::Bandpass => {
                let alpha = s / (2.0 * q);
                let a0 = 1.0 + alpha;
                Coefficients {
                    b0: alpha / a0,
                    b1: 0.0,
                    b2: -alpha / a0,
                    a1: -2.0 * c / a0,
                    a2: (1.0 - alpha) / a0,
                }
            }
            // RBJ notch: b = (1, -2 cos w0, 1), a = (1 + alpha, -2 cos w0, 1 - alpha).
            FilterKind::Notch => {
                let alpha = s / (2.0 * q);
                let a0 = 1.0 + alpha;
                Coefficients {
                    b0: 1.0 / a0,
                    b1: -2.0 * c / a0,
                    b2: 1.0 / a0,
                    a1: -2.0 * c / a0,
                    a2: (1.0 - alpha) / a0,
                }
            }
        }
    }

    fn close(a: &Coefficients, b: &Coefficients, tol: f64) -> bool {
        [
            (a.b0, b.b0),
            (a.b1, b.b1),
            (a.b2, b.b2),
            (a.a1, b.a1),
            (a.a2, b.a2),
        ]
        .iter()
        .all(|(x, y)| (x - y).abs() <= tol * (1.0 + y.abs()))
    }

    #[test]
    fn coefficients_match_the_rbj_formulas() {
        for kind in [
            FilterKind::Lowpass,
            FilterKind::Highpass,
            FilterKind::Bandpass,
        ] {
            for cutoff in [0.001, 0.0123, 0.1, 0.5, 0.9, 0.999] {
                for q in [0.1, 0.7, 1.0, 5.0, 12.0, 30.0] {
                    let got = coefficients(kind, cutoff, q);
                    let want = reference(kind, cutoff, q);
                    // The f32 resonance differs from pow(10, q/20) by at most 1 f32 ulp.
                    assert!(
                        close(&got, &want, 1e-6),
                        "{kind:?} {cutoff} {q}: {got:?} vs {want:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn notch_coefficients() {
        for cutoff in [0.001, 0.0581, 0.1, 0.5, 0.9, 0.999] {
            for q in [0.1, 0.5, 1.6, 5.0, 40.0] {
                let got = notch(cutoff, q);
                let want = reference(FilterKind::Notch, cutoff, q);
                assert!(
                    close(&got, &want, 1e-14),
                    "{cutoff} {q}: {got:?} vs {want:?}"
                );
                assert_eq!(got.b0, got.b2, "{cutoff} {q}");
                assert_eq!(got.b1, got.a1, "{cutoff} {q}");
                assert_eq!(coefficients(FilterKind::Notch, cutoff, q), got);
            }
        }
        // The exact arithmetic of SetNotchParams: alpha = sin(w0) / (2q), k = cos(w0), times 1/a0.
        let (f, q) = (1282.0 / 22050.0, 0.5);
        let w0 = std::f64::consts::PI * f;
        let alpha = libm::sin(w0) / (2.0 * q);
        let k = libm::cos(w0);
        let inv = 1.0 / (1.0 + alpha);
        let c = notch(f, q);
        assert_eq!(c.b0.to_bits(), inv.to_bits());
        assert_eq!(c.b1.to_bits(), (-2.0 * k * inv).to_bits());
        assert_eq!(c.a2.to_bits(), ((1.0 - alpha) * inv).to_bits());
    }

    #[test]
    fn notch_edges() {
        assert_eq!(notch(0.0, 1.0), PASS);
        assert_eq!(notch(1.0, 1.0), PASS);
        assert_eq!(notch(-0.5, 1.0), PASS);
        assert_eq!(notch(1.5, 1.0), PASS);
        assert_eq!(
            notch(f64::NAN, 1.0),
            PASS,
            "the clamp passes NaN; 0 < NaN < 1 is false"
        );
        assert_eq!(notch(0.25, 0.0), ZERO);
        assert_eq!(notch(0.25, -3.0), ZERO, "q < 0 is q = 0");
        assert_eq!(notch(0.25, f64::NAN), ZERO);
        assert_eq!(notch(0.25, f64::INFINITY), notch(0.25, f64::INFINITY));
    }

    #[test]
    fn the_detune_overload() {
        // The float exp2 of the float quotient, widened (the overload the capture chose).
        let (f, d) = (1282.0f32, -1234.5f32);
        let quotient = d / 1200.0f32;
        let want = f64::from(f) / 22050.0 * f64::from(quotient.exp2());
        assert_eq!(normalize_frequency(f, d).to_bits(), want.to_bits());
        const { assert!(DETUNE_EXP2_F32) };
        // The two overloads differ on most quotients (which is what the capture could see).
        let differs = (1..2000)
            .filter(|&i| {
                let q = (i as f32 * 1.37 - 1000.0) / 1200.0f32;
                f64::from(q.exp2()) != f64::from(q).exp2()
            })
            .count();
        assert!(differs > 1000, "{differs}");
        assert_eq!(normalize_frequency(f, 0.0), f64::from(f) / 22050.0);
        assert_eq!(normalize_frequency(f, -0.0), f64::from(f) / 22050.0);
        assert_eq!(normalize_frequency(f, 1200.0), 2.0 * f64::from(f) / 22050.0);
    }

    #[test]
    fn the_resonance_has_float_precision() {
        // 10^(5/20) through expf: exactly an f32 value.
        let r = pow10(5.0 / 20.0);
        assert_eq!(r, f64::from(r as f32));
        assert!((r - 10f64.powf(0.25)).abs() < 1e-6);
        assert_ne!(r, 10f64.powf(0.25));
        // Underflow: a subnormal float result flushes to zero (flush-to-zero audio thread).
        assert!((pow10(-700.0 / 20.0) / 1e-35 - 1.0).abs() < 1e-5);
        assert!((pow10(-756.0 / 20.0) as f32).is_normal());
        assert!(pow10(-760.0 / 20.0) == 0.0);
        assert_eq!(pow10(-1000.0 / 20.0), 0.0);
        // The argument is rounded to f32 before exp.
        let x = 0.123456789;
        assert_eq!(
            pow10(x),
            f64::from(libm::exp(f64::from((x * std::f64::consts::LN_10) as f32)) as f32)
        );
    }

    #[test]
    fn special_cases() {
        assert_eq!(lowpass(1.0, 3.0), PASS);
        assert_eq!(lowpass(1.5, 3.0), PASS);
        assert_eq!(lowpass(0.0, 3.0), ZERO);
        assert_eq!(lowpass(-0.5, 3.0), ZERO);
        assert_eq!(lowpass(f64::NAN, 3.0), ZERO);
        assert_eq!(highpass(1.0, 3.0), ZERO);
        assert_eq!(highpass(0.0, 3.0), PASS);
        assert_eq!(highpass(-1.0, 3.0), PASS);
        assert_eq!(bandpass(0.5, 0.0), PASS);
        assert_eq!(bandpass(0.5, -2.0), PASS);
        assert_eq!(bandpass(0.5, f64::NAN), PASS);
        assert_eq!(bandpass(0.0, 1.0), ZERO);
        assert_eq!(bandpass(1.0, 1.0), ZERO);
        assert_eq!(bandpass(-0.3, 1.0), ZERO);
        // Q = 0 dB on a lowpass is resonance 1 (not a special case).
        let c = lowpass(0.25, 0.0);
        assert!(close(&c, &reference(FilterKind::Lowpass, 0.25, 0.0), 1e-12));
        // Negative dB Q: a valid, damped filter.
        let c = lowpass(0.25, -20.0);
        assert!(c.a2 < 1.0 && c.b0 > 0.0);
    }

    #[test]
    fn frequency_normalization() {
        assert_eq!(normalize_frequency(22050.0, 0.0), 1.0);
        assert_eq!(normalize_frequency(441.0, 0.0), 441.0 / 22050.0);
        let up = normalize_frequency(441.0, 1200.0);
        assert!((up - 2.0 * 441.0 / 22050.0).abs() < 1e-15);
        assert_eq!(normalize_frequency(0.0, 50.0), 0.0);
    }

    /// The per-frame direct form with f64 coefficients and f32-rounded output history.
    fn per_sample(c: &Coefficients, input: &[f32], state: (f64, f64, f64, f64)) -> Vec<f32> {
        let (mut x1, mut x2, mut y1, mut y2) = state;
        input
            .iter()
            .map(|&s| {
                let x = f64::from(s);
                let y = (c.b0 * x + c.b1 * x1 + c.b2 * x2 - c.a1 * y1 - c.a2 * y2) as f32;
                x2 = x1;
                x1 = x;
                y2 = y1;
                y1 = f64::from(y);
                y
            })
            .collect()
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut r = crate::js::Mulberry32::new(seed);
        (0..n).map(|_| (r.next() * 2.0 - 1.0) as f32).collect()
    }

    #[test]
    fn the_sample_accurate_branch_rounds_the_history_to_float() {
        let c = lowpass(0.05, 6.0);
        let input = noise(QUANTUM, 1);
        let mut k = Kernel::new();
        k.set_sample_accurate(true);
        for i in 0..QUANTUM {
            k.set(i, c);
        }
        let mut buf = [0.0f32; QUANTUM];
        buf.copy_from_slice(&input);
        k.process(&mut buf);
        assert_eq!(buf.to_vec(), per_sample(&c, &input, (0.0, 0.0, 0.0, 0.0)));
        // The states are f32 values afterwards.
        let (x1, x2, y1, y2) = k.state();
        for v in [x1, x2, y1, y2] {
            assert_eq!(v, f64::from(v as f32));
        }
        assert_eq!(y1, f64::from(buf[QUANTUM - 1]));
        assert_eq!(x1, f64::from(input[QUANTUM - 1]));
    }

    #[test]
    fn per_frame_coefficients_are_used_frame_by_frame() {
        let mut k = Kernel::new();
        k.set_sample_accurate(true);
        // A pass-through for even frames, silence for odd ones.
        for i in 0..QUANTUM {
            k.set(i, if i % 2 == 0 { PASS } else { ZERO });
        }
        let mut buf = [0.5f32; QUANTUM];
        k.process(&mut buf);
        for i in 0..QUANTUM {
            assert_eq!(buf[i], if i % 2 == 0 { 0.5 } else { 0.0 });
        }
    }

    #[test]
    fn the_fast_branch_keeps_double_history() {
        let c = lowpass(0.05, 6.0);
        let input = noise(2 * QUANTUM, 2);
        let mut k = Kernel::new();
        k.set(0, c);
        let mut a = [0.0f32; QUANTUM];
        let mut b = [0.0f32; QUANTUM];
        a.copy_from_slice(&input[..QUANTUM]);
        b.copy_from_slice(&input[QUANTUM..]);
        k.process(&mut a);
        let (_, _, y1, _) = k.state();
        // y1 is the unrounded double, not the f32 output.
        assert_eq!(y1 as f32, a[QUANTUM - 1]);
        assert_ne!(y1, f64::from(a[QUANTUM - 1]));
        k.process(&mut b);
        // One continuous fma recurrence over both quanta.
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f64, 0.0, 0.0, 0.0);
        let mut out = Vec::new();
        for &s in &input {
            let x = f64::from(s);
            let y = (-c.a2).mul_add(
                y2,
                (-c.a1).mul_add(y1, c.b0.mul_add(x, c.b2.mul_add(x2, c.b1 * x1))),
            );
            out.push(y as f32);
            (x2, x1, y2, y1) = (x1, x, y1, y);
        }
        assert_eq!([a.to_vec(), b.to_vec()].concat(), out);
        // Within a few ulp of the plain direct form.
        let plain = per_sample(&c, &input, (0.0, 0.0, 0.0, 0.0));
        for (x, y) in out.iter().zip(plain.iter()) {
            assert!((x - y).abs() <= 1e-6 * (1.0 + y.abs()));
        }
    }

    #[test]
    fn switching_branches_carries_the_history_and_flushes_denormals() {
        let mut k = Kernel::new();
        k.set(0, PASS);
        // Fast branch: the history holds the last input and output unrounded.
        let mut buf = [1e-40f32; QUANTUM];
        k.process(&mut buf);
        assert_eq!(k.state().0, f64::from(1e-40f32));
        // Sample-accurate branch: denormal states flush to zero at the end of the quantum.
        k.set_sample_accurate(true);
        for i in 0..QUANTUM {
            k.set(i, PASS);
        }
        let mut buf = [1e-40f32; QUANTUM];
        k.process(&mut buf);
        assert_eq!(k.state(), (0.0, 0.0, 0.0, 0.0));
        let mut buf = [0.25f32; QUANTUM];
        k.process(&mut buf);
        assert_eq!(k.state(), (0.25, 0.25, 0.25, 0.25));
        assert_eq!(flush(1e-39), 0.0);
        assert_eq!(flush(-1e-39), 0.0);
        assert_eq!(flush(1e-30), f64::from(1e-30f32));
        assert_eq!(flush(1e300), f64::INFINITY);
    }

    #[test]
    fn constant_values() {
        assert!(has_constant_values(&[]));
        assert!(has_constant_values(&[3.0]));
        assert!(has_constant_values(&[3.0; 128]));
        let mut v = [3.0f32; 128];
        v[127] = 3.0000002;
        assert!(!has_constant_values(&v));
        // NaN never equals itself.
        assert!(!has_constant_values(&[f32::NAN, f32::NAN]));
        assert!(has_constant_values(&[0.0, -0.0]));
    }

    /// The impulse response of a coefficient set (double precision).
    fn impulse(c: &Coefficients, n: usize) -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        (0..n)
            .map(|i| {
                let x = if i == 0 { 1.0 } else { 0.0 };
                let y = c.b0 * x + c.b1 * x1 + c.b2 * x2 - c.a1 * y1 - c.a2 * y2;
                (x2, x1, y2, y1) = (x1, x, y1, y);
                y
            })
            .collect()
    }

    #[test]
    fn tail_frame_bounds_the_impulse_response() {
        let max = MAX_TAIL_TIME * SR;
        for (c, label) in [
            (lowpass(0.05, 20.0), "complex poles"),
            (lowpass(0.3, -10.0), "real poles"),
            (highpass(0.1, 3.0), "highpass"),
            (bandpass(0.2, 0.3), "bandpass, real poles"),
            (bandpass(0.02, 8.0), "bandpass, complex poles"),
        ] {
            let tail = tail_frame(&c, max);
            assert!(tail.is_finite() && tail > 0.0, "{label}: {tail}");
            let h = impulse(&c, tail.ceil() as usize + 2000);
            let after = h[tail.ceil() as usize + 1..]
                .iter()
                .fold(0.0f64, |m, v| m.max(v.abs()));
            assert!(after <= MAX_TAIL_AMPLITUDE, "{label}: {after} after {tail}");
        }
        // Pass-through and silence have no tail; a pole on the unit circle is the maximum.
        assert_eq!(tail_frame(&PASS, max), 2.0);
        assert_eq!(tail_frame(&ZERO, max), 2.0);
        let ring = Coefficients {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 1.0,
        };
        assert_eq!(tail_frame(&ring, max), max);
        let nan = Coefficients {
            b0: f64::NAN,
            ..PASS
        };
        assert_eq!(tail_frame(&nan, max), 0.0);
        // Repeated poles: a double pole at r = 0.5 (a1 = -1, a2 = 0.25).
        let double = Coefficients {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: -1.0,
            a2: 0.25,
        };
        let tail = tail_frame(&double, max);
        let h = impulse(&double, 200);
        assert!(
            h[tail.ceil() as usize + 1..]
                .iter()
                .all(|v| v.abs() <= 1e-4)
        );
        assert!(tail > 10.0 && tail < 40.0, "{tail}");
        // A double pole on or outside the unit circle never decays.
        let unstable = Coefficients {
            a1: -2.0,
            a2: 1.0,
            ..PASS
        };
        assert_eq!(tail_frame(&unstable, max), f64::INFINITY);
        // c1 = c2 = 0 with a repeated root.
        let flat = Coefficients {
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            a1: -1.0,
            a2: 0.25,
        };
        assert_eq!(tail_frame(&flat, max), 0.0);
    }

    #[test]
    fn complex_division_matches_the_textbook_result() {
        let a = Complex { re: 3.0, im: -2.0 };
        let b = Complex { re: 0.5, im: 4.0 };
        let q = a.div(b);
        let back = q.mul(b);
        assert!((back.re - 3.0).abs() < 1e-14 && (back.im + 2.0).abs() < 1e-14);
        let z = Complex {
            re: 0.0,
            im: -1e-300,
        };
        let q = Complex {
            re: 1e-300,
            im: 0.0,
        }
        .div(z);
        assert!((q.re).abs() < 1e-14 && (q.im - 1.0).abs() < 1e-14);
        assert_eq!(logb(0.0), f64::NEG_INFINITY);
        assert_eq!(logb(8.0), 3.0);
        assert_eq!(logb(0.75), -1.0);
        assert_eq!(logb(f64::INFINITY), f64::INFINITY);
        assert!(logb(f64::NAN).is_nan());
    }

    /// A filter node fed a constant frequency/Q written at frame `now` (as `filter.Q.value = q;
    /// filter.frequency.value = f` do).
    fn node(kind: FilterKind, f: f32, q: f32, now: u64) -> Biquad {
        let mut b = Biquad::new(kind);
        b.q.set_value(q, now).unwrap();
        b.frequency.set_value(f, now).unwrap();
        b
    }

    #[test]
    fn a_constant_filter_is_sample_accurate_only_in_its_creation_quantum() {
        let mut b = node(FilterKind::Lowpass, 1000.0, 3.0, 0);
        let input = noise(4 * QUANTUM, 3);
        let mut out = Vec::new();
        for q in 0..4u64 {
            let mut buf = [[0.0f32; QUANTUM]; 1];
            buf[0].copy_from_slice(&input[q as usize * QUANTUM..][..QUANTUM]);
            assert!(!b.process(q * 128, &mut buf, false, &BiquadInputs::NONE));
            // Constant values: one set, never the per-frame branch.
            assert!(!b.kernels()[0].sample_accurate());
            out.extend_from_slice(&buf[0]);
        }
        let c = lowpass(normalize_frequency(1000.0, 0.0), 3.0);
        let mut k = Kernel::new();
        k.set(0, c);
        let mut want = input.clone();
        k.process(&mut want);
        assert_eq!(out, want);
        assert!(b.tail_time() > 0.0 && b.tail_time() < 1.0);
    }

    #[test]
    fn an_automated_frequency_takes_per_frame_coefficients() {
        let mut b = Biquad::new(FilterKind::Lowpass);
        b.frequency.set_value(200.0, 0).unwrap();
        b.frequency.exponential_ramp_to(4000.0, 0.01, 0).unwrap();
        let input = noise(QUANTUM, 4);
        let mut buf = [[0.0f32; QUANTUM]; 1];
        buf[0].copy_from_slice(&input);
        b.process(0, &mut buf, false, &BiquadInputs::NONE);
        assert!(b.kernels()[0].sample_accurate());
        // Frame k uses the coefficients of the frequency at frame k.
        let mut freq = Param::new(350.0, 0.0, 22050.0, Rate::A);
        freq.set_value(200.0, 0).unwrap();
        freq.exponential_ramp_to(4000.0, 0.01, 0).unwrap();
        let mut f = [0.0f32; QUANTUM];
        freq.fill_quantum(0, &mut f, &[]);
        for k in [0, 1, 64, 127] {
            assert_eq!(
                b.kernels()[0].get(k),
                lowpass(normalize_frequency(f[k], 0.0), 1.0)
            );
        }
        // After the ramp (and the pruning) the frequency is constant: back to one set.
        for q in 1..12u64 {
            let mut buf = [[0.0f32; QUANTUM]; 1];
            b.process(q * 128, &mut buf, false, &BiquadInputs::NONE);
        }
        assert!(!b.kernels()[0].sample_accurate());
        assert_eq!(
            b.kernels()[0].get(0),
            lowpass(normalize_frequency(4000.0, 0.0), 1.0)
        );
    }

    #[test]
    fn detune_input_sample_accurate() {
        // The phaser's notch: its detune always has a connection (the LFO). While the LFO is
        // silent (before it starts, after it ends) the values are constant: one coefficient set,
        // the vDSP path. While it runs, 128 sets from the per-frame detune.
        let mut b = node(FilterKind::Notch, 1282.0, 0.5, 0);
        let silent = BiquadInputs {
            frequency: &ParamInputs::NONE,
            detune: &ParamInputs {
                bufs: Vec::new(),
                connected: true,
            },
            ..BiquadInputs::NONE
        };
        for q in 0..3u64 {
            let mut buf = [[0.1f32; QUANTUM]; 1];
            b.process(q * 128, &mut buf, false, &silent);
            assert!(!b.kernels()[0].sample_accurate(), "{q}: needed == 1");
            assert_eq!(
                b.kernels()[0].get(0),
                notch(normalize_frequency(1282.0, 0.0), 0.5)
            );
        }
        let lfo: [f32; QUANTUM] = std::array::from_fn(|k| -2000.0 + 31.25 * k as f32);
        let running = BiquadInputs {
            frequency: &ParamInputs::NONE,
            detune: &ParamInputs {
                bufs: vec![lfo],
                connected: true,
            },
            ..BiquadInputs::NONE
        };
        let mut buf = [[0.1f32; QUANTUM]; 1];
        b.process(384, &mut buf, false, &running);
        assert!(b.kernels()[0].sample_accurate(), "128 sets");
        for k in [0usize, 1, 64, 127] {
            assert_eq!(
                b.kernels()[0].get(k),
                notch(normalize_frequency(1282.0, lfo[k]), 0.5),
                "{k}"
            );
        }
        // A zero LFO frame inside a running quantum takes the plain normalization.
        let mut zero_at_3 = lfo;
        zero_at_3[3] = 0.0;
        let mut buf = [[0.1f32; QUANTUM]; 1];
        b.process(
            512,
            &mut buf,
            false,
            &BiquadInputs {
                frequency: &ParamInputs::NONE,
                detune: &ParamInputs {
                    bufs: vec![zero_at_3],
                    connected: true,
                },
                ..BiquadInputs::NONE
            },
        );
        assert_eq!(
            b.kernels()[0].get(3),
            notch(normalize_frequency(1282.0, 0.0), 0.5)
        );
    }

    #[test]
    fn a_connected_frequency_takes_per_frame_coefficients_every_quantum() {
        // `frequency.value = 800` at 0, then an LFO-like connection: the timeline plus the
        // input per frame, NaN replaced by the default (350), clipped to [0, nyquist].
        let mut b = node(FilterKind::Lowpass, 800.0, 2.0, 0);
        let mut lfo = [0.0f32; QUANTUM];
        for (k, x) in lfo.iter_mut().enumerate() {
            *x = (k as f32) * 3.0 - 100.0;
        }
        lfo[5] = f32::NAN;
        lfo[6] = -5000.0;
        lfo[7] = 1e9;
        let inputs = BiquadInputs {
            frequency: &ParamInputs {
                bufs: vec![lfo],
                connected: true,
            },
            detune: &ParamInputs::NONE,
            ..BiquadInputs::NONE
        };
        // Quantum 3: the `setValueAtTime` is long past, only the connection makes it sample
        // accurate.
        for q in 0..3u64 {
            let mut buf = [[0.0f32; QUANTUM]; 1];
            b.process(q * 128, &mut buf, false, &BiquadInputs::NONE);
        }
        assert!(!b.kernels()[0].sample_accurate());
        let mut buf = [[0.0f32; QUANTUM]; 1];
        b.process(384, &mut buf, false, &inputs);
        assert!(b.kernels()[0].sample_accurate());
        let q = f64::from(2.0f32);
        for k in [0usize, 1, 64, 127] {
            let f = 800.0 + lfo[k];
            assert_eq!(
                b.kernels()[0].get(k),
                lowpass(normalize_frequency(f, 0.0), q),
                "{k}"
            );
        }
        assert_eq!(
            b.kernels()[0].get(5),
            lowpass(normalize_frequency(350.0, 0.0), q)
        );
        assert_eq!(
            b.kernels()[0].get(6),
            lowpass(normalize_frequency(0.0, 0.0), q)
        );
        assert_eq!(
            b.kernels()[0].get(7),
            lowpass(normalize_frequency(22050.0, 0.0), q)
        );
        // Connected but silent this quantum (no buffers): still the sample-accurate path, all
        // values equal, so one coefficient set.
        let silent = BiquadInputs {
            frequency: &ParamInputs {
                bufs: Vec::new(),
                connected: true,
            },
            detune: &ParamInputs::NONE,
            ..BiquadInputs::NONE
        };
        let mut buf = [[0.0f32; QUANTUM]; 1];
        b.process(512, &mut buf, false, &silent);
        assert!(!b.kernels()[0].sample_accurate());
        assert_eq!(
            b.kernels()[0].get(0),
            lowpass(normalize_frequency(800.0, 0.0), q)
        );
        // A connected detune is summed the same way.
        let detune = BiquadInputs {
            frequency: &ParamInputs::NONE,
            detune: &ParamInputs {
                bufs: vec![[1200.0; QUANTUM]],
                connected: true,
            },
            ..BiquadInputs::NONE
        };
        let mut buf = [[0.0f32; QUANTUM]; 1];
        b.process(640, &mut buf, false, &detune);
        assert_eq!(
            b.kernels()[0].get(0),
            lowpass(normalize_frequency(800.0, 1200.0), q)
        );
        // Silent input past the tail: the params are still evaluated with their inputs.
        let mut quiet = node(FilterKind::Lowpass, 800.0, 0.0, 0);
        let mut buf = [[1.0f32; QUANTUM]; 1];
        quiet.process(0, &mut buf, false, &BiquadInputs::NONE);
        for q in 1..400u64 {
            let mut buf = [[0.0f32; QUANTUM]; 1];
            quiet.process(q * 128, &mut buf, true, &inputs);
        }
        let mut buf = [[0.0f32; QUANTUM]; 1];
        assert!(
            quiet.process(400 * 128, &mut buf, true, &inputs),
            "silence propagates"
        );
        assert_eq!(
            quiet.frequency.value(),
            800.0,
            "the timeline value, not the sum"
        );
    }

    #[test]
    fn unchanged_final_values_keep_the_previous_coefficients() {
        let mut b = node(FilterKind::Bandpass, 500.0, 2.0, 0);
        let mut buf = [[0.0f32; QUANTUM]; 1];
        b.process(0, &mut buf, false, &BiquadInputs::NONE);
        b.process(128, &mut buf, false, &BiquadInputs::NONE);
        b.process(256, &mut buf, false, &BiquadInputs::NONE);
        // A stray coefficient write survives while nothing is dirty.
        b.kernels[0].set(0, PASS);
        let mut buf = [[0.5f32; QUANTUM]; 1];
        b.process(384, &mut buf, false, &BiquadInputs::NONE);
        assert_eq!(buf[0], [0.5f32; QUANTUM]);
        // A changed value (a new event, current from now on) marks them dirty again.
        b.frequency.set_value(800.0, 512).unwrap();
        b.process(512, &mut buf, false, &BiquadInputs::NONE);
        b.process(640, &mut buf, false, &BiquadInputs::NONE);
        assert_eq!(
            b.kernels[0].get(0),
            bandpass(normalize_frequency(800.0, 0.0), 2.0)
        );
    }

    #[test]
    fn silence_propagates_after_the_tail_time() {
        let mut b = node(FilterKind::Lowpass, 2000.0, 0.0, 0);
        // A first non-silent quantum computes the tail time.
        let mut buf = [[1.0f32; QUANTUM]; 1];
        assert!(!b.process(0, &mut buf, false, &BiquadInputs::NONE));
        let tail = b.tail_time();
        assert!(
            tail.is_finite() && tail > 0.0 && tail < 128.0 / SR,
            "{tail}"
        );
        // The next quantum is silent input but within the tail: still processed (ringing).
        let mut buf = [[0.0f32; QUANTUM]; 1];
        assert!(!b.process(128, &mut buf, true, &BiquadInputs::NONE));
        assert!(buf[0].iter().any(|v| *v != 0.0));
        // One more quantum later the tail has passed: silence, kernel untouched.
        let state = b.kernels()[0].state();
        let mut buf = [[0.0f32; QUANTUM]; 1];
        assert!(b.process(256, &mut buf, true, &BiquadInputs::NONE));
        assert_eq!(buf[0], [0.0f32; QUANTUM]);
        assert_eq!(b.kernels()[0].state(), state);
        // Non-silent input brings it back.
        let mut buf = [[1.0f32; QUANTUM]; 1];
        assert!(!b.process(384, &mut buf, false, &BiquadInputs::NONE));
    }

    #[test]
    fn the_tail_time_is_infinite_until_the_first_update() {
        let b = Biquad::new(FilterKind::Highpass);
        assert_eq!(b.tail_time(), f64::INFINITY);
        assert!(!b.propagates_silence(1_000_000));
        // A silent-input node that has never seen signal still processes its first quantum.
        let mut b = node(FilterKind::Highpass, 300.0, 1.0, 1280);
        let mut buf = [[0.0f32; QUANTUM]; 1];
        assert!(!b.process(1280, &mut buf, true, &BiquadInputs::NONE));
        assert!(b.tail_time().is_finite());
        assert!(b.process(1408, &mut buf, true, &BiquadInputs::NONE));
    }

    #[test]
    fn stereo_input_gets_a_kernel_per_channel() {
        let mut b = node(FilterKind::Lowpass, 3000.0, 1.0, 0);
        let input = noise(QUANTUM, 5);
        let mut buf = [[0.0f32; QUANTUM]; 2];
        buf[0].copy_from_slice(&input);
        buf[1].copy_from_slice(&input);
        b.process(0, &mut buf, false, &BiquadInputs::NONE);
        assert_eq!(b.kernels().len(), 2);
        assert_eq!(buf[0], buf[1]);
        // Back to mono: new kernels (zero history), re-snapped coefficients.
        let mut mono = [[0.0f32; QUANTUM]; 1];
        mono[0].copy_from_slice(&input);
        b.process(128, &mut mono, false, &BiquadInputs::NONE);
        assert_eq!(b.kernels().len(), 1);
        assert_eq!(mono[0], buf[0]);
    }

    #[test]
    fn filter_kind_names() {
        assert_eq!(FilterKind::from_name("lowpass"), Some(FilterKind::Lowpass));
        assert_eq!(
            FilterKind::from_name("highpass"),
            Some(FilterKind::Highpass)
        );
        assert_eq!(
            FilterKind::from_name("bandpass"),
            Some(FilterKind::Bandpass)
        );
        assert_eq!(FilterKind::from_name("notch"), Some(FilterKind::Notch));
        assert_eq!(FilterKind::from_name("peaking"), None);
        assert_eq!(
            Biquad::new(FilterKind::Bandpass).kind(),
            FilterKind::Bandpass
        );
    }

    /// The fast branch against Accelerate's own `vDSP_deq22D` (the reference machine runs it).
    #[cfg(target_os = "macos")]
    mod accelerate {
        use super::*;

        #[link(name = "Accelerate", kind = "framework")]
        unsafe extern "C" {
            fn vDSP_deq22D(
                a: *const f64,
                ia: isize,
                b: *const f64,
                c: *mut f64,
                ic: isize,
                n: usize,
            );
        }

        #[test]
        fn the_fast_branch_is_vdsp_deq22d() {
            let mut rng = crate::js::Mulberry32::new(9);
            for trial in 0..200 {
                let cutoff = 0.001 + rng.next() * 0.99;
                let q = rng.next() * 40.0 - 10.0;
                let c = match trial % 3 {
                    0 => lowpass(cutoff, q),
                    1 => highpass(cutoff, q),
                    _ => bandpass(cutoff, q.abs() + 0.01),
                };
                let n = QUANTUM;
                let mut input = vec![0.0f64; n + 2];
                let mut output = vec![0.0f64; n + 2];
                input[0] = (rng.next() - 0.5) as f32 as f64;
                input[1] = (rng.next() - 0.5) as f32 as f64;
                output[0] = rng.next() - 0.5;
                output[1] = rng.next() - 0.5;
                let mut buf = [0.0f32; QUANTUM];
                for i in 0..n {
                    let scale = 10f64.powi(-((rng.next() * 30.0) as i32));
                    buf[i] = ((rng.next() * 2.0 - 1.0) * scale) as f32;
                    input[i + 2] = f64::from(buf[i]);
                }
                let coefs = [c.b0, c.b1, c.b2, c.a1, c.a2];
                unsafe {
                    vDSP_deq22D(input.as_ptr(), 1, coefs.as_ptr(), output.as_mut_ptr(), 1, n);
                }
                let mut k = Kernel::new();
                k.set(0, c);
                (k.x2, k.x1, k.y2, k.y1) = (input[0], input[1], output[0], output[1]);
                k.process(&mut buf);
                for i in 0..n {
                    assert_eq!(buf[i], output[i + 2] as f32, "trial {trial} frame {i}");
                }
                assert_eq!(k.y1, output[n + 1]);
                assert_eq!(k.y2, output[n]);
            }
        }
    }
}
