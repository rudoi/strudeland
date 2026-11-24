// Port of Blink (Chromium 156.0.8078.4) platform/audio/reverb.cc, platform/audio/reverb_convolver.cc, platform/audio/reverb_convolver_stage.cc, platform/audio/reverb_accumulation_buffer.cc, platform/audio/direct_convolver.cc, platform/audio/fft_convolver.cc, platform/audio/fft_frame.cc, modules/webaudio/convolver_handler.cc, modules/webaudio/audio_handler.cc, modules/webaudio/audio_buffer_source_handler.cc, modules/webaudio/offline_audio_context.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.
// Impulse-response generation also adapts reverbGen, Copyright 2014 Alan deLespinasse,
// Apache-2.0. Strudel integrated audio node management. Strudeland added seeded
// generation and ported buffer creation, decay, fade-in and filtering to Rust.
// See ../../../LICENSES/Apache-2.0.txt.

//! The orbit reverb's impulse response (superdough's `reverb.mjs` `createReverb`/`generate`,
//! `reverbGen.mjs` `generateReverb`, and `Orbit.getReverb`'s regeneration rules), and Blink's
//! normalization of a convolver buffer (`Reverb::CalculateNormalizationScale`).
//!
//! The IR (`generateReverb` in `reverbGen.mjs`) is two channels of `N = round(1.5 d sr)` frames
//! for the decay time `d` (`roomsize`): one seeded stream (`makeRandom('reverb', 2, sr, d,
//! fade || 0, lp || 0, dim || 0)`, the values as passed) fills channel 0 then channel 1 with
//! `(2 r - 1) * decayBase^j` (`decayBase = 0.001^(1 / round(d sr))`, double, stored as float),
//! then the first `round(fade sr)` frames are faded in (`chan[j] *= j / fadeFrames`, the float read
//! back). Unless `lp` is falsy (or loosely equal to 0), the IR is then rendered through a lowpass
//! in a nested `OfflineAudioContext(2, N, sr)` (`applyGradualLowpass` in `reverbGen.mjs`):
//! `BufferSource(IR).start()` into a `BiquadFilterNode` (lowpass, `Q.value = 0.0001` dB,
//! `frequency.setValueAtTime(min(lp, sr / 2), 0)`, `linearRampToValueAtTime(min(dim, sr / 2), d)`)
//! into the destination. The engine runs the [`Biquad`] port over the stereo IR in 128-frame
//! quanta from frame 0, as that context renders it: the buffer source starts on frame 0 with no
//! sub-frame offset, so it copies the buffer (Blink's fast path) and the filter's input is never
//! silent.
//!
//! JS values: the settings are whatever the hap carried (`undefined` replaced by the defaults
//! `2, 0.1, 15000, 1000`), so `null`, strings and arrays are coerced as JS does (`d * 1.5`,
//! `fade || 0`, `lp == 0`, `Math.min(lp, sr / 2)`, `typeof` and `String` in the hash).
//!
//! Errors (map decision 21): `createBuffer(2, N, sr)` throws for `N < 1` (or a non-finite `N`);
//! the engine also refuses `N > 2^26` (about 25 minutes, 512 MiB per channel). That is
//! [`IrError::Length`] (kind `ir`). The lowpass's `setValueAtTime` and `linearRampToValueAtTime`
//! throw a `TypeError` for a frequency that is not a finite float (`lp` `-Infinity` or a
//! non-numeric string): [`IrError::Lowpass`] (kind `hap`). Both throw inside `generate`, after
//! superdough stored the request's settings and consumed its request number, and no IR arrives.
//!
//! Regeneration ([`Reverb`]): a hap with `room > 0` on an orbit without a reverb creates it with
//! its settings; later haps regenerate it when one of their *defined* settings differs strictly
//! from the stored one (`hasChanged`), and the regeneration uses the hap's settings with the
//! defaults for the undefined ones (so a hap that changes only `roomsize` resets a stored
//! `roomfade` to 0.1). Within one pause only the newest request that produces an IR is applied
//! ("latest request wins": every request finishes before the context resumes, and a finished IR
//! is applied only when its request number is above the applied one); the engine generates only
//! that one, when the pause is settled ([`Reverb::settle`]).
//!
//! Convolution ([`Convolver`], `ConvolverNode` with `normalize = true`): each IR channel is a
//! `ReverbConvolver` (a direct stage, then FFT stages doubling to [`MAX_FFT_SIZE`], with Blink's
//! pre- and post-delays, render phases 0 and 128), so the FFT blocks and the float operations are
//! Chromium's, up to the direct stage's summation order (see [`DirectConvolver`]). The orbit's
//! [`Reverb`] node builds a new convolver for every IR that lands (the swap at the pause frame
//! drops the tail and the history) and stops running it once its input has been silent for the
//! IR's length (`AudioHandler`'s tail rule).
//!
//! Normalization ([`normalization_scale`]): `1 / sqrt(sum x^2 / (channels * length))` (floored at
//! 0.000125) times `10^(-58 * 0.05)`, in float. Deviation: Blink sums each channel's squares with
//! Accelerate's `vDSP_svesq`, whose float reduction order could not be reproduced (its error
//! against the exact sum is about 1e-6 relative for a two-second IR); the engine sums in double
//! and rounds once, which stays within that error (a macOS test checks it against Accelerate).

use crate::QUANTUM;
use crate::biquad::{Biquad, BiquadInputs, FilterKind};
use crate::controls::{js_min, js_to_number_full, js_to_string, js_truthy};
use crate::fft::FftFrame;
use crate::job::{ErrorKind, Value};
use crate::js::{HashPart, make_random, number_to_string};
use crate::orbit::Input;
use crate::param::{ParamError, idl_float};
use std::collections::HashMap;
use std::fmt;

mod tail;

/// `generate`'s defaults (reverb.mjs:32), applied to `undefined` settings only.
pub const DEFAULT_ROOMSIZE: f64 = 2.0;
pub const DEFAULT_ROOMFADE: f64 = 0.1;
pub const DEFAULT_ROOMLP: f64 = 15000.0;
pub const DEFAULT_ROOMDIM: f64 = 1000.0;
/// `createReverb` always asks for a stereo IR.
pub const IR_CHANNELS: usize = 2;
/// The engine's limit on the IR length in frames (2^26).
pub const MAX_IR_FRAMES: f64 = 67_108_864.0;
/// `filter.Q.value = 0.0001` (dB) of the gradual lowpass.
pub const LOWPASS_Q: f32 = 0.0001;
/// `kMinPower`: the RMS floor of a silent or very quiet response.
pub const MIN_POWER: f32 = 0.000125;
/// `kGainCalibration` (dB).
pub const GAIN_CALIBRATION: f32 = -58.0;
/// `kGainCalibrationSampleRate`.
pub const GAIN_CALIBRATION_SAMPLE_RATE: f32 = 44100.0;

/// JS `Math.round`: halves round up, a result of zero from a negative input is `-0`; NaN and
/// infinities pass through.
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let f = x.floor();
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    if r == 0.0 && x.is_sign_negative() {
        -0.0
    } else {
        r
    }
}

/// JS `v || 0`.
fn or_zero(v: &Value) -> Value {
    if js_truthy(v) {
        v.clone()
    } else {
        Value::Num(0.0)
    }
}

/// JS `v == 0` (loose: `null == 0` is false, a string, array or object compares by its number).
fn loosely_zero(v: &Value) -> bool {
    match v {
        Value::Null | Value::Unsupported(_) => false,
        Value::Bool(b) => !*b,
        _ => js_to_number_full(v) == 0.0,
    }
}

/// JS strict equality of two hap values. Arrays and objects are never equal: each hap carries
/// its own (the one case where the same object is compared, right after the reverb is created,
/// is skipped by the caller).
pub fn strict_equals(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}

/// `typeof v` for a hap value.
fn type_of(v: &Value) -> &'static str {
    match v {
        Value::Num(_) => "number",
        Value::Str(_) => "string",
        Value::Bool(_) => "boolean",
        Value::Null | Value::Arr(_) | Value::Obj(_) | Value::Set(_) | Value::Unsupported(_) => {
            "object"
        }
    }
}

/// The settings of one IR request: `generate(d, fade, lp, dim)` after its defaults, which is
/// also what the convolver stores (`convolver.duration = d`, ...).
#[derive(Debug, Clone, PartialEq)]
pub struct ReverbParams {
    /// `roomsize`, the decay time `d` in seconds.
    pub duration: Value,
    /// `roomfade`, the fade-in time in seconds.
    pub fade: Value,
    /// `roomlp`, the lowpass start frequency (falsy: no lowpass).
    pub lp: Value,
    /// `roomdim`, the lowpass frequency at `d`.
    pub dim: Value,
}

impl Default for ReverbParams {
    fn default() -> Self {
        ReverbParams {
            duration: Value::Num(DEFAULT_ROOMSIZE),
            fade: Value::Num(DEFAULT_ROOMFADE),
            lp: Value::Num(DEFAULT_ROOMLP),
            dim: Value::Num(DEFAULT_ROOMDIM),
        }
    }
}

impl ReverbParams {
    /// `generate(roomsize, roomfade, roomlp, roomdim)` with the hap's raw values (`None` is
    /// `undefined` and takes the default; `null` stays `null`).
    pub fn defaulted(
        roomsize: Option<&Value>,
        roomfade: Option<&Value>,
        roomlp: Option<&Value>,
        roomdim: Option<&Value>,
    ) -> ReverbParams {
        let d = ReverbParams::default();
        ReverbParams {
            duration: roomsize.cloned().unwrap_or(d.duration),
            fade: roomfade.cloned().unwrap_or(d.fade),
            lp: roomlp.cloned().unwrap_or(d.lp),
            dim: roomdim.cloned().unwrap_or(d.dim),
        }
    }

    /// `hasChanged(now, before)` for each setting: a defined hap value strictly different from
    /// the stored one.
    pub fn changed_by(
        &self,
        roomsize: Option<&Value>,
        roomfade: Option<&Value>,
        roomlp: Option<&Value>,
        roomdim: Option<&Value>,
    ) -> bool {
        [
            (roomsize, &self.duration),
            (roomfade, &self.fade),
            (roomlp, &self.lp),
            (roomdim, &self.dim),
        ]
        .iter()
        .any(|(now, before)| now.is_some_and(|v| !strict_equals(v, before)))
    }
}

/// Why an IR request produced no IR.
#[derive(Debug, Clone, PartialEq)]
pub enum IrError {
    /// `createBuffer(2, N, sr)` with `N < 1` or non-finite, or `N` above [`MAX_IR_FRAMES`].
    Length(f64),
    /// The lowpass's `setValueAtTime` or `linearRampToValueAtTime` threw.
    Lowpass(ParamError),
    /// The nested lowpass needs the engine's sample rate (the biquad port is built for it).
    SampleRate(f64),
}

impl IrError {
    /// The render error kind: `ir` for the length, `hap` for an exception superdough reports as
    /// any other.
    pub fn kind(&self) -> ErrorKind {
        match self {
            IrError::Length(_) => ErrorKind::Ir,
            IrError::Lowpass(_) | IrError::SampleRate(_) => ErrorKind::Hap,
        }
    }
}

impl fmt::Display for IrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IrError::Length(n) => write!(f, "reverb ir length {}", number_to_string(*n)),
            IrError::Lowpass(e) => write!(f, "{e}"),
            IrError::SampleRate(sr) => {
                write!(f, "reverb lowpass at sample rate {}", number_to_string(*sr))
            }
        }
    }
}

impl From<ParamError> for IrError {
    fn from(e: ParamError) -> Self {
        IrError::Lowpass(e)
    }
}

/// One checked IR request: everything `generateReverb` computes or throws before it fills the
/// buffer, so the generation itself ([`IrPlan::render`]) cannot fail.
#[derive(Debug, Clone)]
pub struct IrPlan {
    params: ReverbParams,
    sample_rate: f64,
    frames: usize,
    decay_frames: f64,
    fade_frames: f64,
    /// The nested context's filter with its events scheduled, when `lp` asks for one.
    lowpass: Option<Biquad>,
}

impl IrPlan {
    /// `generateReverb`'s checks, in its order: the buffer length, then the lowpass's events.
    pub fn new(params: &ReverbParams, sample_rate: f64) -> Result<IrPlan, IrError> {
        let d = js_to_number_full(&params.duration);
        let total_time = d * 1.5;
        let decay_frames = js_round(d * sample_rate);
        let n = js_round(total_time * sample_rate);
        let fade_frames = js_round(js_to_number_full(&or_zero(&params.fade)) * sample_rate);
        if !(1.0..=MAX_IR_FRAMES).contains(&n) {
            return Err(IrError::Length(n));
        }
        let lp = or_zero(&params.lp);
        let lowpass = if loosely_zero(&lp) {
            None
        } else {
            if sample_rate != f64::from(crate::SAMPLE_RATE) {
                return Err(IrError::SampleRate(sample_rate));
            }
            let start = js_min(js_to_number_full(&lp), sample_rate / 2.0);
            let end = js_min(js_to_number_full(&or_zero(&params.dim)), sample_rate / 2.0);
            let mut filter = Biquad::new(FilterKind::Lowpass);
            filter.q.set_value(LOWPASS_Q, 0)?;
            filter
                .frequency
                .set_value_at_time(idl_float(start, "setValueAtTime")?, 0.0, 0)?;
            filter
                .frequency
                .linear_ramp_to(idl_float(end, "linearRampToValueAtTime")?, d, 0)?;
            Some(filter)
        };
        Ok(IrPlan {
            params: params.clone(),
            sample_rate,
            frames: n as usize,
            decay_frames,
            fade_frames,
            lowpass,
        })
    }

    /// The IR length `N` in frames.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Whether the IR goes through the nested lowpass.
    pub fn has_lowpass(&self) -> bool {
        self.lowpass.is_some()
    }

    /// The request's settings.
    pub fn params(&self) -> &ReverbParams {
        &self.params
    }

    /// The run-wide cache key of this request's IR for the audio seed `seed`: the sample rate
    /// and the typed texts `makeRandom('reverb', ...)` hashes, which fix the IR bit for bit.
    pub fn key(&self, seed: i32) -> IrKey {
        IrKey {
            sample_rate_bits: self.sample_rate.to_bits(),
            seed,
            texts: self.hash_texts(),
        }
    }

    /// `(typeof v, String(v))` of `d`, `fade || 0`, `lp || 0`, `dim || 0`: what the noise's
    /// seed hashes (reverbGen.mjs:40-48).
    fn hash_texts(&self) -> [(&'static str, String); 4] {
        let p = &self.params;
        [
            &p.duration,
            &or_zero(&p.fade),
            &or_zero(&p.lp),
            &or_zero(&p.dim),
        ]
        .map(|v| (type_of(v), js_to_string(v)))
    }

    /// Generates the IR for the audio seed `seed`.
    pub fn render(self, seed: i32) -> [Vec<f32>; IR_CHANNELS] {
        let mut ir = self.noise(seed);
        if let Some(filter) = self.lowpass {
            gradual_lowpass(&mut ir, filter);
        }
        ir
    }

    /// The decaying noise with its fade-in (reverbGen.mjs:42-58).
    fn noise(&self, seed: i32) -> [Vec<f32>; IR_CHANNELS] {
        let texts = self.hash_texts();
        let mut parts = vec![
            HashPart::Str("reverb"),
            HashPart::Number(IR_CHANNELS as f64),
            HashPart::Number(self.sample_rate),
        ];
        parts.extend(texts.iter().map(|(t, s)| HashPart::Typed(t, s)));
        let mut random = make_random(seed, &parts);
        let decay_base = libm::pow(1.0 / 1000.0, 1.0 / self.decay_frames);
        let n = self.frames;
        // `for (j = 0; j < fadeInSampleFrames; j++)`: writes past the end of the buffer are
        // dropped, so at most `N` frames change (an infinite fade never ends in Chromium).
        let fade_n = if self.fade_frames > 0.0 {
            (self.fade_frames.min(n as f64)).ceil() as usize
        } else {
            0
        };
        std::array::from_fn(|_| {
            let mut chan = vec![0.0f32; n];
            for (j, x) in chan.iter_mut().enumerate() {
                *x = ((random.next() * 2.0 - 1.0) * libm::pow(decay_base, j as f64)) as f32;
            }
            for (j, x) in chan.iter_mut().enumerate().take(fade_n) {
                *x = (f64::from(*x) * (j as f64 / self.fade_frames)) as f32;
            }
            chan
        })
    }
}

/// Renders `ir` through `filter` as the nested `OfflineAudioContext` does: 128-frame quanta from
/// frame 0, the buffer source copying the IR (zeros past its end in the last quantum), the
/// destination keeping the first `N` frames.
fn gradual_lowpass(ir: &mut [Vec<f32>; IR_CHANNELS], mut filter: Biquad) {
    let n = ir[0].len();
    let mut bus = [[0.0f32; QUANTUM]; IR_CHANNELS];
    let mut q = 0usize;
    while q < n {
        let len = QUANTUM.min(n - q);
        for (b, c) in bus.iter_mut().zip(ir.iter()) {
            b.fill(0.0);
            b[..len].copy_from_slice(&c[q..q + len]);
        }
        filter.process(q as u64, &mut bus, false, &BiquadInputs::NONE);
        for (b, c) in bus.iter().zip(ir.iter_mut()) {
            c[q..q + len].copy_from_slice(&b[..len]);
        }
        q += QUANTUM;
    }
}

/// `generateReverb` with the nested lowpass: the IR of `params` for the audio seed `seed`.
#[cfg(test)]
pub fn generate_ir(
    params: &ReverbParams,
    seed: i32,
    sample_rate: f64,
) -> Result<[Vec<f32>; IR_CHANNELS], IrError> {
    Ok(IrPlan::new(params, sample_rate)?.render(seed))
}

/// `vector_math::Vsvesq` (the sum of squares; see the module notes on the summation order).
pub fn sum_of_squares(x: &[f32]) -> f32 {
    x.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>() as f32
}

/// `CalculateNormalizationScale` of a response at the engine's sample rate.
pub fn normalization_scale(response: &[Vec<f32>]) -> f32 {
    let channels = response.len() as u32;
    let length = response.first().map_or(0, Vec::len) as u32;
    let mut power = 0.0f32;
    for c in response {
        power += sum_of_squares(c);
    }
    power = (power / channels.wrapping_mul(length) as f32).sqrt();
    if !power.is_finite() || power < MIN_POWER {
        power = MIN_POWER;
    }
    let mut scale = 1.0 / power;
    scale *= libm::powf(10.0, GAIN_CALIBRATION * 0.05);
    scale *= GAIN_CALIBRATION_SAMPLE_RATE / crate::SAMPLE_RATE as f32;
    if channels == 4 {
        scale *= 0.5;
    }
    scale
}

/// `kMaxFftSize` of `ConvolverHandler`: the largest FFT of a convolver stage.
pub const MAX_FFT_SIZE: usize = 8192;
/// `ReverbConvolver::kMinFFTSize`: the size whose half is the direct stage's kernel.
pub const MIN_FFT_SIZE: usize = 128;
/// The direct stage's kernel length (`DirectConvolver` over `kMinFFTSize / 2` taps).
pub const DIRECT_TAPS: usize = MIN_FFT_SIZE / 2;

/// `vector_math::Zvmul` (Accelerate's `vDSP_zvmul`, not conjugated) in place on `(re1, im1)`:
/// `re = fma(-im1, im2, re1 * re2)`, `im = fma(im1, re2, re1 * im2)`, the order measured on the
/// reference machine (bit-identical; a macOS test re-checks it against Accelerate).
pub fn zvmul(re1: &mut [f32], im1: &mut [f32], re2: &[f32], im2: &[f32]) {
    for (((ar, ai), br), bi) in re1
        .iter_mut()
        .zip(im1.iter_mut())
        .zip(re2.iter())
        .zip(im2.iter())
    {
        let (a_re, a_im) = (*ar, *ai);
        *ar = (-a_im).mul_add(*bi, a_re * *br);
        *ai = a_im.mul_add(*br, a_re * *bi);
    }
}

/// `DirectConvolver` over [`DIRECT_TAPS`] taps for 128-frame blocks: the input history in
/// `buffer` (the last 63 frames, then the new block), `out[n] = sum_k x[n - k] * kernel[k]`.
/// Deviation: Blink calls Accelerate's `vDSP_conv`, whose float summation order depends on the
/// buffer's alignment and was not reproduced; the engine accumulates in ascending lag order with
/// fused multiply-adds (rounding-level differences, about 1e-7 relative).
#[derive(Debug, Clone)]
struct DirectConvolver {
    kernel: [f32; DIRECT_TAPS],
    buffer: [f32; DIRECT_TAPS - 1 + QUANTUM],
}

impl DirectConvolver {
    fn new(kernel: [f32; DIRECT_TAPS]) -> DirectConvolver {
        DirectConvolver {
            kernel,
            buffer: [0.0; DIRECT_TAPS - 1 + QUANTUM],
        }
    }

    fn process(&mut self, source: &[f32; QUANTUM], dest: &mut [f32; QUANTUM]) {
        const HISTORY: usize = DIRECT_TAPS - 1;
        self.buffer[HISTORY..].copy_from_slice(source);
        for (n, d) in dest.iter_mut().enumerate() {
            let mut acc = 0.0f32;
            for (k, h) in self.kernel.iter().enumerate() {
                acc = self.buffer[n + HISTORY - k].mul_add(*h, acc);
            }
            *d = acc;
        }
        self.buffer.copy_within(QUANTUM.., 0);
    }
}

/// `FFTConvolver`: overlap-add of `size / 2`-frame blocks (the second half of `input` stays
/// zero), one FFT, multiply and inverse FFT each time a block fills; its output lags its input
/// by `size / 2` frames.
#[derive(Debug, Clone)]
struct FftConvolver {
    read_write_index: usize,
    input: Vec<f32>,
    output: Vec<f32>,
    last_overlap: Vec<f32>,
}

impl FftConvolver {
    fn new(size: usize) -> FftConvolver {
        FftConvolver {
            read_write_index: 0,
            input: vec![0.0; size],
            output: vec![0.0; size],
            last_overlap: vec![0.0; size / 2],
        }
    }

    fn process(
        &mut self,
        frame: &mut FftFrame,
        kernel: &FftKernel,
        source: &[f32],
        dest: &mut [f32],
    ) {
        let half = self.last_overlap.len();
        let mut done = 0;
        while done < source.len() {
            let i = self.read_write_index;
            let n = (source.len() - done).min(half - i);
            self.input[i..i + n].copy_from_slice(&source[done..done + n]);
            dest[done..done + n].copy_from_slice(&self.output[i..i + n]);
            done += n;
            self.read_write_index += n;
            if self.read_write_index == half {
                frame.do_fft(&self.input);
                // `FFTFrame::Multiply`: the packed DC and Nyquist bins are real products.
                let (real0, imag0) = (frame.real[0], frame.imag[0]);
                zvmul(&mut frame.real, &mut frame.imag, &kernel.real, &kernel.imag);
                frame.real[0] = real0 * kernel.real[0];
                frame.imag[0] = imag0 * kernel.imag[0];
                frame.do_inverse_fft(&mut self.output);
                for (o, l) in self.output[..half].iter_mut().zip(self.last_overlap.iter()) {
                    *o += *l;
                }
                self.last_overlap.copy_from_slice(&self.output[half..]);
                self.read_write_index = 0;
            }
        }
    }
}

/// A stage's kernel spectrum (`DoPaddedFFT` of its IR slice, then `ScaleFFT(scale)`).
#[derive(Debug, Clone)]
struct FftKernel {
    real: Vec<f32>,
    imag: Vec<f32>,
}

impl FftKernel {
    fn from_frame(frame: &FftFrame, scale: f32) -> Self {
        let mut kernel = Self {
            real: frame.real.clone(),
            imag: frame.imag.clone(),
        };
        if scale != 1.0 {
            for sample in kernel.real.iter_mut().chain(&mut kernel.imag) {
                *sample *= scale;
            }
        }
        kernel
    }
}

#[derive(Debug, Clone)]
enum StageKind {
    Direct(Box<DirectConvolver>),
    Fft {
        size: usize,
        kernel: FftKernel,
        convolver: FftConvolver,
    },
}

/// `ReverbConvolverStage`: one slice of the IR, its pre-delay (which staggers the FFTs of the
/// stages) and the post-delay into the shared accumulation buffer.
#[derive(Debug, Clone)]
struct Stage {
    kind: StageKind,
    pre_delay: Vec<f32>,
    pre_delay_length: usize,
    post_delay_length: usize,
    pre_read_write_index: usize,
    frames_processed: usize,
    accumulation_read_index: usize,
}

impl Stage {
    /// `ReverbConvolverStage::Initialize` for `ir[offset..offset + length]` (latency 0).
    fn new(
        ir: &[f32],
        offset: usize,
        length: usize,
        fft_size: usize,
        render_phase: usize,
        scale: f32,
        frames: &mut FftFrames,
    ) -> Stage {
        let direct = offset == 0;
        let half = fft_size / 2;
        let kind = if direct {
            let mut kernel = [0.0f32; DIRECT_TAPS];
            kernel[..length].copy_from_slice(&ir[..length]);
            if scale != 1.0 {
                for k in &mut kernel[..length] {
                    *k *= scale;
                }
            }
            StageKind::Direct(Box::new(DirectConvolver::new(kernel)))
        } else {
            let frame = frames.get(fft_size);
            let mut padded = vec![0.0f32; fft_size];
            padded[..length].copy_from_slice(&ir[offset..offset + length]);
            frame.do_fft(&padded);
            StageKind::Fft {
                size: fft_size,
                kernel: FftKernel::from_frame(frame, scale),
                convolver: FftConvolver::new(fft_size),
            }
        };
        let mut total_delay = offset;
        if !direct {
            // The FFT's own latency of half its size is taken off the stage's delay.
            total_delay -= half;
        }
        let max_pre_delay = half.min(total_delay);
        let mut pre_delay_length = if total_delay > 0 {
            render_phase % max_pre_delay
        } else {
            0
        };
        pre_delay_length = (pre_delay_length / QUANTUM) * QUANTUM;
        if pre_delay_length > total_delay {
            pre_delay_length = 0;
        }
        Stage {
            kind,
            pre_delay: vec![0.0; pre_delay_length],
            pre_delay_length,
            post_delay_length: total_delay - pre_delay_length,
            pre_read_write_index: 0,
            frames_processed: 0,
            accumulation_read_index: 0,
        }
    }

    /// `ReverbConvolverStage::Process` of one quantum into `accumulation`.
    fn process(
        &mut self,
        source: &[f32; QUANTUM],
        accumulation: &mut [f32],
        frames: &mut FftFrames,
    ) {
        let len = accumulation.len();
        let delayed: [f32; QUANTUM] = if self.pre_delay_length > 0 {
            let i = self.pre_read_write_index;
            self.pre_delay[i..i + QUANTUM].try_into().unwrap()
        } else {
            *source
        };
        if self.frames_processed < self.pre_delay_length {
            self.accumulation_read_index = (self.accumulation_read_index + QUANTUM) % len;
        } else {
            let mut temp = [0.0f32; QUANTUM];
            match &mut self.kind {
                StageKind::Direct(d) => d.process(&delayed, &mut temp),
                StageKind::Fft {
                    size,
                    kernel,
                    convolver,
                } => convolver.process(frames.get(*size), kernel, &delayed, &mut temp),
            }
            accumulate(
                accumulation,
                &temp,
                &mut self.accumulation_read_index,
                self.post_delay_length,
            );
        }
        if self.pre_delay_length > 0 {
            let i = self.pre_read_write_index;
            self.pre_delay[i..i + QUANTUM].copy_from_slice(source);
            self.pre_read_write_index += QUANTUM;
            if self.pre_read_write_index >= self.pre_delay_length {
                self.pre_read_write_index = 0;
            }
        }
        self.frames_processed += QUANTUM;
    }
}

/// `ReverbAccumulationBuffer::Accumulate`: adds `source` at `read_index + delay` (wrapping) and
/// advances the caller's read index.
fn accumulate(buffer: &mut [f32], source: &[f32], read_index: &mut usize, delay: usize) {
    let len = buffer.len();
    let write = (*read_index + delay) % len;
    *read_index = (*read_index + source.len()) % len;
    let n1 = source.len().min(len - write);
    for (d, s) in buffer[write..write + n1].iter_mut().zip(source.iter()) {
        *d += *s;
    }
    for (d, s) in buffer.iter_mut().zip(source[n1..].iter()) {
        *d += *s;
    }
}

/// One `FftFrame` per FFT size, shared by the stages of a [`Convolver`] (a frame's buffers are
/// scratch between transforms, so sharing them changes nothing; Blink gives each stage its own).
#[derive(Debug, Default)]
struct FftFrames {
    frames: Vec<FftFrame>,
}

impl Clone for FftFrames {
    /// The frames hold no state between transforms: a clone plans its own.
    fn clone(&self) -> Self {
        FftFrames::default()
    }
}

impl FftFrames {
    fn get(&mut self, size: usize) -> &mut FftFrame {
        let i = match self.frames.iter().position(|f| f.size() == size) {
            Some(i) => i,
            None => {
                self.frames
                    .push(FftFrame::new(size).expect("convolver FFT sizes are powers of two"));
                self.frames.len() - 1
            }
        };
        &mut self.frames[i]
    }
}

/// `ReverbConvolver`: one IR channel split into stages (a direct one over the first
/// [`DIRECT_TAPS`] taps, then FFT stages of 64 taps at FFT 128, doubling up to 4096 taps at FFT
/// [`MAX_FFT_SIZE`], then 4096-tap stages to the end of the IR), summed into an accumulation
/// buffer of `N + 128` frames. Zero latency: the output is the linear convolution.
#[derive(Debug, Clone)]
struct ChannelConvolver {
    stages: Vec<Stage>,
    accumulation: Vec<f32>,
    read_index: usize,
}

impl ChannelConvolver {
    fn new(ir: &[f32], render_phase: usize, scale: f32, frames: &mut FftFrames) -> Self {
        let total = ir.len();
        let mut stages = Vec::new();
        let mut offset = 0;
        let mut fft_size = MIN_FFT_SIZE;
        let mut i = 0;
        while offset < total {
            let length = (fft_size / 2).min(total - offset);
            let phase = render_phase + i * QUANTUM;
            let direct = offset == 0;
            stages.push(Stage::new(
                ir, offset, length, fft_size, phase, scale, frames,
            ));
            offset += length;
            i += 1;
            if !direct {
                fft_size *= 2;
            }
            fft_size = fft_size.min(MAX_FFT_SIZE);
        }
        ChannelConvolver {
            stages,
            accumulation: vec![0.0; total + QUANTUM],
            read_index: 0,
        }
    }

    fn process(&mut self, source: &[f32; QUANTUM], dest: &mut [f32; QUANTUM], f: &mut FftFrames) {
        for stage in &mut self.stages {
            stage.process(source, &mut self.accumulation, f);
        }
        // `ReadAndClear`.
        let len = self.accumulation.len();
        let n1 = QUANTUM.min(len - self.read_index);
        let r = self.read_index;
        dest[..n1].copy_from_slice(&self.accumulation[r..r + n1]);
        self.accumulation[r..r + n1].fill(0.0);
        let n2 = QUANTUM - n1;
        dest[n1..].copy_from_slice(&self.accumulation[..n2]);
        self.accumulation[..n2].fill(0.0);
        self.read_index = (r + QUANTUM) % len;
    }
}

/// Blink's `Reverb` for a stereo response: one [`ChannelConvolver`] per IR channel (render
/// phases 0 and 128), both scaled by the normalization `scale` (the direct kernel in time, the
/// FFT kernels in frequency, as Blink does). A mono input feeds both; a stereo input feeds
/// `L -> IR 0`, `R -> IR 1`. Built new for every IR (`ConvolverNode.buffer =`), so a swap starts
/// from an empty history and drops the old tail.
#[derive(Debug, Clone)]
pub struct Convolver {
    channels: [ChannelConvolver; IR_CHANNELS],
    frames: FftFrames,
    length: usize,
    tail: Option<Box<[tail::Tail; IR_CHANNELS]>>,
}

impl Convolver {
    /// `Reverb::TryCreate(ir, 128, kMaxFftSize, normalize)` with the scale already computed
    /// ([`normalization_scale`] of `ir`, or 1 without normalization). Both channels must have
    /// the same length of at least one frame.
    pub fn new(ir: &[Vec<f32>; IR_CHANNELS], scale: f32) -> Convolver {
        Self::build(ir, scale, false)
    }

    /// Live playback groups long tails so their largest FFTs can share work.
    pub(crate) fn new_live(ir: &[Vec<f32>; IR_CHANNELS], scale: f32) -> Convolver {
        Self::build(ir, scale, true)
    }

    fn build(ir: &[Vec<f32>; IR_CHANNELS], scale: f32, live: bool) -> Convolver {
        assert!(
            !ir[0].is_empty() && ir[0].len() == ir[1].len(),
            "a convolver needs two IR channels of one length"
        );
        let length = ir[0].len();
        let early = if live {
            length.min(tail::START)
        } else {
            length
        };
        let mut frames = FftFrames::default();
        let channels = [
            ChannelConvolver::new(&ir[0][..early], 0, scale, &mut frames),
            ChannelConvolver::new(&ir[1][..early], QUANTUM, scale, &mut frames),
        ];
        Convolver {
            channels,
            frames,
            length,
            tail: (early < length).then(|| {
                Box::new([
                    tail::Tail::new(&ir[0][early..], scale),
                    tail::Tail::new(&ir[1][early..], scale),
                ])
            }),
        }
    }

    /// Instantiate a cached live convolver with fresh state and its FFT plans ready.
    /// The cache only holds convolvers which have never processed audio.
    pub(crate) fn fresh_live(&self) -> Self {
        Self {
            channels: self.channels.clone(),
            frames: FftFrames {
                frames: self.frames.frames.clone(),
            },
            length: self.length,
            tail: self
                .tail
                .as_ref()
                .map(|t| Box::new([t[0].fresh_history(), t[1].fresh_history()])),
        }
    }

    /// The IR length in frames (`TailTime` is this over the sample rate).
    pub fn length(&self) -> usize {
        self.length
    }

    /// `LatencyFrames`: always 0 (the direct stage covers the start of the IR).
    pub fn latency_frames(&self) -> usize {
        0
    }

    /// The stage layout: `(offset, length, fft_size or 0 for the direct stage, pre-delay,
    /// post-delay)` per stage of channel `c`.
    pub fn stages(&self, c: usize) -> Vec<(usize, usize, usize, usize, usize)> {
        let mut offset = 0;
        self.channels[c]
            .stages
            .iter()
            .map(|s| {
                let (size, length) = match &s.kind {
                    StageKind::Direct(_) => (0, DIRECT_TAPS.min(self.length)),
                    StageKind::Fft { size, .. } => (*size, (*size / 2).min(self.length - offset)),
                };
                let row = (
                    offset,
                    length,
                    size,
                    s.pre_delay_length,
                    s.post_delay_length,
                );
                offset += length;
                row
            })
            .collect()
    }

    /// `Reverb::Process` of one quantum: `input` holds one (mono) or two channels.
    pub fn process(&mut self, input: &[[f32; QUANTUM]], out: &mut [[f32; QUANTUM]; 2]) {
        let (l, r) = match input {
            [mono] => (mono, mono),
            [l, r, ..] => (l, r),
            [] => panic!("a convolver input has one or two channels"),
        };
        let [c0, c1] = &mut self.channels;
        let (o0, o1) = out.split_at_mut(1);
        c0.process(l, &mut o0[0], &mut self.frames);
        c1.process(r, &mut o1[0], &mut self.frames);
        if let Some(tail) = &mut self.tail {
            tail[0].process(l, &mut o0[0]);
            tail[1].process(r, &mut o1[0]);
        }
    }
}

/// The key of a generated IR in the run's [`IrCache`]: the sample rate's bits, the audio seed and
/// `(typeof, String)` of `d`, `fade || 0`, `lp || 0`, `dim || 0` (exactly what
/// `makeRandom('reverb', ...)` hashes, which also fixes the length, the fade and the lowpass), so
/// equal keys give bit-identical IRs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IrKey {
    pub sample_rate_bits: u64,
    pub seed: i32,
    pub texts: [(&'static str, String); 4],
}

/// A generated IR's convolver, never processed: a reverb that hits it takes a clone (a fresh
/// convolver with an empty history).
#[derive(Debug, Clone)]
struct CachedIr {
    convolver: Convolver,
    #[cfg(test)]
    ir: [Vec<f32>; IR_CHANNELS],
}

/// The run's generated IRs: every orbit and local reverb with equal settings and
/// seed shares one generation.
#[derive(Debug, Clone, Default)]
pub struct IrCache {
    map: HashMap<IrKey, CachedIr>,
    /// How many IRs were generated (tests count the misses).
    generated: usize,
}

impl IrCache {
    pub fn new() -> IrCache {
        IrCache::default()
    }

    /// How many IRs the cache generated (one per miss).
    pub fn generated(&self) -> usize {
        self.generated
    }

    /// The cached IRs.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// A clone of the cached convolver for `key` (a fresh history), if the cache holds it.
    pub(crate) fn get(&self, key: &IrKey) -> Option<Convolver> {
        self.map.get(key).map(|c| c.convolver.fresh_live())
    }

    /// Caches a convolver built elsewhere (the live engine's IR worker) under `key`.
    pub(crate) fn insert(&mut self, key: IrKey, convolver: Convolver) {
        self.map.insert(
            key,
            CachedIr {
                convolver,
                #[cfg(test)]
                ir: [Vec::new(), Vec::new()],
            },
        );
    }

    /// The convolver of `plan`'s IR for the audio seed `seed`: a clone of the cached one on a
    /// hit; on a miss the IR is generated, its normalized convolver cached (a clone) and returned.
    fn convolver(&mut self, plan: IrPlan, seed: i32) -> CachedIr {
        let key = plan.key(seed);
        if let Some(c) = self.map.get(&key) {
            return c.clone();
        }
        let ir = plan.render(seed);
        let entry = CachedIr {
            convolver: Convolver::new(&ir, normalization_scale(&ir)),
            #[cfg(test)]
            ir,
        };
        self.generated += 1;
        self.map.insert(key, entry.clone());
        entry
    }
}

/// An orbit's reverb node (`createReverb`'s convolver with its `generate`): the stored settings,
/// the request counters, the applied IR and the `ConvolverNode` that plays it.
#[derive(Debug, Clone)]
pub struct Reverb {
    sample_rate: f64,
    /// `convolver.duration/fade/lp/dim`: the settings of the last request.
    stored: ReverbParams,
    /// `latest`: the number of the last request.
    latest: u64,
    /// `applied`: the request number of the IR in place.
    applied: u64,
    /// The newest request of this pause that will produce an IR.
    pending: Option<(u64, IrPlan)>,
    /// The applied IR (not normalized); only tests read it.
    #[cfg(test)]
    ir: Option<[Vec<f32>; IR_CHANNELS]>,
    /// `ConvolverHandler::reverb_`: built from the applied IR.
    convolver: Option<Convolver>,
    /// The handler's `last_non_silent_time_` (seconds); it outlives IR swaps.
    last_non_silent_time: f64,
    /// The node's output of the last processed quantum and its silence hint.
    out: [[f32; QUANTUM]; 2],
    out_silent: bool,
}

impl Reverb {
    /// `createReverb(roomsize, roomfade, roomlp, roomdim)`: the node and its first request. A
    /// failing request escapes, so no node exists (`this.reverbNode` is never assigned).
    pub fn create(
        roomsize: Option<&Value>,
        roomfade: Option<&Value>,
        roomlp: Option<&Value>,
        roomdim: Option<&Value>,
        sample_rate: f64,
    ) -> Result<Reverb, IrError> {
        let mut r = Reverb {
            sample_rate,
            stored: ReverbParams::default(),
            latest: 0,
            applied: 0,
            pending: None,
            #[cfg(test)]
            ir: None,
            convolver: None,
            last_non_silent_time: 0.0,
            out: [[0.0; QUANTUM]; 2],
            out_silent: true,
        };
        r.generate(ReverbParams::defaulted(roomsize, roomfade, roomlp, roomdim))?;
        Ok(r)
    }

    /// `getReverb` on an existing node: a request when a defined setting differs from the
    /// stored one. Returns whether a request was made; its error, if it fails.
    pub fn update(
        &mut self,
        roomsize: Option<&Value>,
        roomfade: Option<&Value>,
        roomlp: Option<&Value>,
        roomdim: Option<&Value>,
    ) -> Result<bool, IrError> {
        if !self.stored.changed_by(roomsize, roomfade, roomlp, roomdim) {
            return Ok(false);
        }
        self.generate(ReverbParams::defaulted(roomsize, roomfade, roomlp, roomdim))?;
        Ok(true)
    }

    /// `convolver.generate(...)`: a new request number, the settings stored, then
    /// `generateReverb`'s checks; a request that passes them replaces the pending one.
    fn generate(&mut self, params: ReverbParams) -> Result<(), IrError> {
        self.latest += 1;
        let request = self.latest;
        self.stored = params;
        let plan = IrPlan::new(&self.stored, self.sample_rate)?;
        self.pending = Some((request, plan));
        Ok(())
    }

    /// The end of a pause (`settleIR`): the pending request's IR is generated (audio seed
    /// `seed`) and applied when its number is above the applied one. Returns whether the IR
    /// changed.
    #[cfg(test)]
    pub fn settle(&mut self, seed: i32) -> bool {
        self.settle_cached(seed, &mut IrCache::new())
    }

    /// [`Reverb::settle`] through the run's IR cache: a pending request whose IR the cache holds
    /// takes a clone of its convolver (bit-identical to generating it again), a miss generates it
    /// and caches it. A pending request whose number is not above the applied one is dropped.
    pub fn settle_cached(&mut self, seed: i32, cache: &mut IrCache) -> bool {
        match self.pending.take() {
            Some((request, plan)) if request > self.applied => {
                self.applied = request;
                // `convolver.buffer = buffer`: a new `Reverb` (normalized) replaces the old one,
                // dropping its tail and input history.
                let c = cache.convolver(plan, seed);
                self.convolver = Some(c.convolver);
                #[cfg(test)]
                {
                    self.ir = Some(c.ir);
                }
                true
            }
            _ => false,
        }
    }

    /// Takes the pending request (number and plan) for an IR built elsewhere (the live
    /// engine's worker); [`Self::apply`] lands its result.
    pub fn take_pending(&mut self) -> Option<(u64, IrPlan)> {
        self.pending.take()
    }

    /// Lands the convolver built for `request`, as [`Self::settle_cached`] does, when the
    /// request's number is above the applied one (so the newest request wins whatever order
    /// results arrive in). Returns whether it was applied.
    pub fn apply(&mut self, request: u64, convolver: impl FnOnce() -> Convolver) -> bool {
        if request <= self.applied {
            return false;
        }
        self.applied = request;
        self.convolver = Some(convolver());
        #[cfg(test)]
        {
            self.ir = None;
        }
        true
    }

    /// The stored settings (`convolver.duration`, `fade`, `lp`, `dim`).
    pub fn settings(&self) -> &ReverbParams {
        &self.stored
    }

    /// `latest`: how many requests were made.
    pub fn latest(&self) -> u64 {
        self.latest
    }

    /// `applied`: the request number of the IR in place (0 before the first).
    pub fn applied(&self) -> u64 {
        self.applied
    }

    /// The request waiting for the end of the pause, if any.
    pub fn pending(&self) -> Option<(u64, &IrPlan)> {
        self.pending.as_ref().map(|(r, p)| (*r, p))
    }

    /// The applied IR (before normalization).
    #[cfg(test)]
    pub fn ir(&self) -> Option<&[Vec<f32>; IR_CHANNELS]> {
        self.ir.as_ref()
    }

    /// The convolver playing the applied IR.
    pub fn convolver(&self) -> Option<&Convolver> {
        self.convolver.as_ref()
    }

    /// `TailTime`: the IR length in seconds (0 without an IR).
    pub fn tail_time(&self) -> f64 {
        self.convolver
            .as_ref()
            .map_or(0.0, |c| c.length() as f64 / self.sample_rate)
    }

    /// `AudioHandler::ProcessIfNecessary` of the `ConvolverNode` for the quantum at
    /// `quantum_start` on its summed `input` (one or two channels; the channel count follows
    /// the connected sends): once the input has been silent for longer than the tail time
    /// (latency 0) the node outputs silence without running the convolver, so its state waits;
    /// otherwise `ConvolverHandler::Process` (zeros before the first IR). Returns the output
    /// (always two channels once an IR is set) and its silence hint.
    pub fn process(&mut self, quantum_start: u64, input: Input<'_>) -> Input<'_> {
        let now = quantum_start as f64 / self.sample_rate;
        if input.silent && self.last_non_silent_time + self.tail_time() < now {
            self.out = [[0.0; QUANTUM]; 2];
            self.out_silent = true;
        } else {
            match &mut self.convolver {
                Some(c) if !input.channels.is_empty() => c.process(input.channels, &mut self.out),
                _ => self.out = [[0.0; QUANTUM]; 2],
            }
            self.out_silent = false;
            if !input.silent {
                self.last_non_silent_time =
                    (quantum_start + QUANTUM as u64) as f64 / self.sample_rate;
            }
        }
        Input {
            channels: &self.out,
            silent: self.out_silent,
        }
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;
    use crate::test_vectors::{decode, load};
    use serde_json::json;

    use crate::SAMPLE_RATE_F64 as SR;

    fn v(x: serde_json::Value) -> Value {
        Value::from_json(x).unwrap()
    }

    fn params(
        d: serde_json::Value,
        fade: serde_json::Value,
        lp: serde_json::Value,
    ) -> ReverbParams {
        ReverbParams {
            duration: v(d),
            fade: v(fade),
            lp: v(lp),
            dim: Value::Num(1000.0),
        }
    }

    fn length_of(d: serde_json::Value) -> Result<usize, IrError> {
        IrPlan::new(&params(d, json!(0), json!(0)), SR).map(|p| p.frames())
    }

    #[test]
    fn js_round_is_math_round() {
        let cases = [
            (0.5, 1.0),
            (1.5, 2.0),
            (2.5, 3.0),
            (-0.5, -0.0),
            (-0.2, -0.0),
            (-1.5, -1.0),
            (-2.5, -2.0),
            (-2.6, -3.0),
            (0.49999999999999994, 0.0),
            (4503599627370495.5, 4503599627370496.0),
            (1e300, 1e300),
        ];
        for (x, want) in cases {
            let got = js_round(x);
            assert_eq!(got, want, "{x}");
            assert_eq!(
                got.is_sign_negative(),
                want.is_sign_negative(),
                "sign of {x}"
            );
        }
        assert!(js_round(f64::NAN).is_nan());
        assert_eq!(js_round(f64::INFINITY), f64::INFINITY);
        assert_eq!(js_round(f64::NEG_INFINITY), f64::NEG_INFINITY);
    }

    #[test]
    fn js_coercions() {
        assert_eq!(js_to_number_full(&v(json!([]))), 0.0);
        assert_eq!(js_to_number_full(&v(json!([5]))), 5.0);
        assert!(js_to_number_full(&v(json!([1, 2]))).is_nan());
        assert!(js_to_number_full(&v(json!({}))).is_nan());
        assert_eq!(js_to_number_full(&v(json!(" 2 "))), 2.0);
        assert_eq!(js_to_number_full(&Value::Null), 0.0);
        assert_eq!(or_zero(&Value::Null), Value::Num(0.0));
        assert_eq!(or_zero(&v(json!(""))), Value::Num(0.0));
        assert_eq!(or_zero(&v(json!({ "$num": "NaN" }))), Value::Num(0.0));
        assert_eq!(
            or_zero(&v(json!("0"))),
            v(json!("0")),
            "a non-empty string is truthy"
        );
        for (x, zero) in [
            (json!(0), true),
            (json!({ "$num": "-0" }), true),
            (json!("0"), true),
            (json!(" "), true),
            (json!([]), true),
            (json!([0]), true),
            (json!(false), true),
            (json!(null), false),
            (json!(true), false),
            (json!("x"), false),
            (json!({}), false),
            (json!(15000), false),
        ] {
            assert_eq!(loosely_zero(&v(x.clone())), zero, "{x} == 0");
        }
        assert!(strict_equals(&Value::Num(0.0), &Value::Num(-0.0)));
        assert!(!strict_equals(&Value::Num(f64::NAN), &Value::Num(f64::NAN)));
        assert!(!strict_equals(&Value::Num(1.0), &v(json!("1"))));
        assert!(strict_equals(&Value::Null, &Value::Null));
        assert!(strict_equals(&v(json!("a")), &v(json!("a"))));
        assert!(
            !strict_equals(&v(json!([1])), &v(json!([1]))),
            "two objects"
        );
        assert_eq!(
            [json!(1), json!("1"), json!(true), json!(null), json!([1])].map(|x| type_of(&v(x))),
            ["number", "string", "boolean", "object", "object"]
        );
    }

    #[test]
    fn defaults_apply_to_undefined_only() {
        assert_eq!(
            ReverbParams::defaulted(None, None, None, None),
            ReverbParams::default()
        );
        let p = ReverbParams::defaulted(
            Some(&Value::Num(4.0)),
            Some(&Value::Null),
            None,
            Some(&v(json!("200"))),
        );
        assert_eq!(p.duration, Value::Num(4.0));
        assert_eq!(p.fade, Value::Null);
        assert_eq!(p.lp, Value::Num(DEFAULT_ROOMLP));
        assert_eq!(p.dim, v(json!("200")));
    }

    #[test]
    fn ir_length_rounds_as_js() {
        // N = round(d * 1.5 * sr); halves round up.
        assert_eq!(length_of(json!(2)).unwrap(), 132300);
        assert_eq!(length_of(json!(0.05)).unwrap(), 3308, "3307.5 rounds up");
        assert_eq!(length_of(json!(0.5)).unwrap(), 33075);
        assert_eq!(length_of(json!(1.0 / 3.0)).unwrap(), 22050);
        assert_eq!(length_of(json!(1.0 / SR)).unwrap(), 2, "1.5 frames");
        assert_eq!(length_of(json!(0.4 / SR)).unwrap(), 1, "0.6 frames");
        // 1/132300 * 1.5 * 44100 is 0.49999999999999994 in double: 0 frames, an error.
        assert_eq!(length_of(json!(1.0 / 132300.0)), Err(IrError::Length(0.0)));
        assert_eq!(
            length_of(json!("0.01")).unwrap(),
            662,
            "strings are numbers"
        );
        assert_eq!(
            length_of(json!([0.01])).unwrap(),
            662,
            "so is a one-element array"
        );
        assert_eq!(length_of(json!(true)).unwrap(), 66150);
        // Odd roomsizes: the decay frames round on their own.
        let p = IrPlan::new(&params(json!(0.0101), json!(0.003), json!(0)), SR).unwrap();
        assert_eq!(
            (p.frames(), p.decay_frames, p.fade_frames),
            (668, 445.0, 132.0)
        );
        let p = IrPlan::new(&params(json!(0.4 / SR), json!(0), json!(0)), SR).unwrap();
        assert_eq!(p.decay_frames, 0.0, "round(0.4)");
    }

    #[test]
    fn short_and_non_finite_lengths_are_ir_errors() {
        let big = MAX_IR_FRAMES / 1.5 / SR;
        assert_eq!(
            length_of(json!(big)).unwrap(),
            1 << 26,
            "the limit itself is fine"
        );
        for (d, n) in [
            (json!(0), "0"),
            (json!(-1), "-66150"),
            (json!(-1e-9), "0"),
            (json!(0.3 / SR), "0"),
            (json!(null), "0"),
            (json!(false), "0"),
            (json!([]), "0"),
            (json!("abc"), "NaN"),
            (json!({}), "NaN"),
            (json!({ "$num": "NaN" }), "NaN"),
            (json!({ "$num": "Infinity" }), "Infinity"),
            (json!({ "$num": "-Infinity" }), "-Infinity"),
            (json!(1e9), "66150000000000"),
            (json!(big * 1.00001), "67109535"),
        ] {
            let e = length_of(d.clone()).unwrap_err();
            assert_eq!(e.to_string(), format!("reverb ir length {n}"), "{d}");
            assert_eq!(e.kind(), ErrorKind::Ir);
            assert!(generate_ir(&params(d, json!(0), json!(0)), 0, SR).is_err());
        }
    }

    /// `generateReverb` without the lowpass against `reverb-ir.json`: every stored sample within
    /// 1 ulp (Bun's JavaScriptCore `Math.pow` may differ from V8's and `libm`'s in the last bit),
    /// and the channel hashes.
    #[test]
    fn ir_matches_the_js_vectors() {
        let file = load("reverb-ir.json");
        let cases = file["cases"].as_array().unwrap();
        assert!(cases.len() >= 17);
        let within = |a: u32, b: u32| {
            let (x, y) = (f32::from_bits(a), f32::from_bits(b));
            x == y || (x.is_sign_negative() == y.is_sign_negative() && a.abs_diff(b) <= 1)
        };
        let bits = |j: &serde_json::Value| -> Vec<u32> {
            j.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap() as u32)
                .collect()
        };
        let mut hashes_equal = 0;
        for c in cases {
            let p = ReverbParams {
                duration: decode(&c["d"]),
                fade: decode(&c["fade"]),
                lp: decode(&c["lp"]),
                dim: decode(&c["dim"]),
            };
            let seed = c["seed"].as_i64().unwrap() as i32;
            let plan = IrPlan::new(&p, SR).unwrap();
            assert!(!plan.has_lowpass(), "{c}");
            let ir = plan.render(seed);
            let n = c["length"].as_u64().unwrap() as usize;
            for (ch, want) in ir.iter().zip(c["channels"].as_array().unwrap()) {
                assert_eq!(ch.len(), n);
                let got: Vec<u32> = ch.iter().map(|x| x.to_bits()).collect();
                let check = |got: &[u32], want: Vec<u32>, what: &str| {
                    assert_eq!(got.len(), want.len());
                    for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
                        assert!(within(*g, *w), "seed {seed} d {:?} {what}[{i}]", p.duration);
                    }
                };
                if want.get("samples").is_some() {
                    check(&got, bits(&want["samples"]), "samples");
                } else {
                    check(&got[..1024], bits(&want["head"]), "head");
                    check(&got[n - 1024..], bits(&want["tail"]), "tail");
                }
                let mut h = 2166136261u32;
                for b in ch.iter().flat_map(|x| x.to_le_bytes()) {
                    h ^= u32::from(b);
                    h = h.wrapping_mul(16777619);
                }
                if u64::from(h) == want["fnv1a"].as_u64().unwrap() {
                    hashes_equal += 1;
                }
            }
        }
        assert_eq!(
            hashes_equal,
            2 * cases.len(),
            "every channel is bit-identical"
        );
    }

    /// The noise, decay and fade written out naively from reverbGen.mjs.
    fn naive_ir(d: f64, fade: f64, seed: i32) -> [Vec<f32>; 2] {
        let n = js_round(d * 1.5 * SR) as usize;
        let decay_base = libm::pow(0.001, 1.0 / js_round(d * SR));
        let fade_frames = js_round(fade * SR);
        let mut r = make_random(
            seed,
            &[
                HashPart::Str("reverb"),
                HashPart::Number(2.0),
                HashPart::Number(SR),
                HashPart::Number(d),
                HashPart::Number(fade),
                HashPart::Number(0.0),
                HashPart::Number(1000.0),
            ],
        );
        std::array::from_fn(|_| {
            let mut c: Vec<f32> = (0..n)
                .map(|j| ((r.next() * 2.0 - 1.0) * libm::pow(decay_base, j as f64)) as f32)
                .collect();
            for j in 0..(fade_frames as usize).min(n) {
                c[j] = (f64::from(c[j]) * (j as f64 / fade_frames)) as f32;
            }
            c
        })
    }

    #[test]
    fn one_stream_fills_channel_zero_then_one_and_the_fade_reads_the_float() {
        let p = params(json!(0.02), json!(0.005), json!(0));
        let ir = generate_ir(&p, 3, SR).unwrap();
        assert_eq!(ir, naive_ir(0.02, 0.005, 3));
        assert_ne!(ir[0], ir[1]);
        // The fade: frame 0 is silent, frame j of the first 220 is scaled by j / 220 from the
        // float sample.
        assert_eq!(ir[0][0], 0.0);
        let unfaded = naive_ir(0.02, 0.0, 3);
        assert_ne!(unfaded[0], ir[0], "the fade time is part of the seed");
        // A fade longer than the IR fades all of it (writes past the end are dropped).
        let long = generate_ir(&params(json!(0.01), json!(1), json!(0)), 0, SR).unwrap();
        assert_eq!(long, naive_ir(0.01, 1.0, 0));
        // A negative or NaN fade fades nothing; an infinite fade zeroes the IR (j / Infinity),
        // where Chromium would never finish the loop.
        let n = generate_ir(&params(json!(0.01), json!(-1), json!(0)), 0, SR).unwrap();
        assert_ne!(n[0][0], 0.0);
        let inf = generate_ir(
            &params(json!(0.01), json!({ "$num": "Infinity" }), json!(0)),
            0,
            SR,
        )
        .unwrap();
        assert!(inf.iter().flatten().all(|x| *x == 0.0));
        // decayFrames 0: decayBase 0, so only frame 0 (decayBase^0 = 1) sounds.
        let one = generate_ir(&params(json!(0.4 / SR), json!(0), json!(0)), 0, SR).unwrap();
        assert_eq!(one[0].len(), 1);
        let two = generate_ir(&params(json!(1.4 / SR), json!(0), json!(0)), 0, SR).unwrap();
        assert_eq!(two[0].len(), 2, "2.1 frames");
        assert_ne!(two[0][1], 0.0, "decayFrames 1: 0.001^1");
    }

    #[test]
    fn seeds_and_settings_change_the_noise() {
        let p = params(json!(0.01), json!(0), json!(0));
        let a = generate_ir(&p, 0, SR).unwrap();
        assert_eq!(a, generate_ir(&p, 0, SR).unwrap(), "deterministic");
        assert_ne!(a, generate_ir(&p, 1, SR).unwrap());
        let mut q = p.clone();
        q.dim = Value::Num(999.0);
        assert_ne!(a, generate_ir(&q, 0, SR).unwrap(), "dim is hashed");
        // `lp || 0`: a null and a 0 lowpass hash alike; `'0'` is loosely 0 (no lowpass) but
        // hashed as the string.
        let mut z = p.clone();
        z.lp = Value::Null;
        assert_eq!(a, generate_ir(&z, 0, SR).unwrap());
        z.lp = v(json!("0"));
        let s = IrPlan::new(&z, SR).unwrap();
        assert!(!s.has_lowpass());
        assert_ne!(a, s.render(0));
    }

    fn band_energy(x: &[f32]) -> (f64, f64) {
        // Total power and the power of the first difference (a highpass proxy).
        let total = x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
        let diff = x
            .windows(2)
            .map(|w| (f64::from(w[1]) - f64::from(w[0])).powi(2))
            .sum::<f64>();
        (total, diff)
    }

    #[test]
    fn the_gradual_lowpass_darkens_the_tail() {
        let p = params(json!(0.25), json!(0), json!(15000));
        let plan = IrPlan::new(&p, SR).unwrap();
        assert!(plan.has_lowpass());
        let n = plan.frames();
        let raw = plan.noise(5);
        let filtered = plan.clone().render(5);
        assert_eq!(filtered[0].len(), n);
        let d = js_round(0.25 * SR) as usize;
        for c in 0..2 {
            // Near the start the cutoff is 15 kHz: most of the white noise passes.
            let (rt, _) = band_energy(&raw[c][..512]);
            let (ft, _) = band_energy(&filtered[c][..512]);
            assert!(ft / rt > 0.5 && ft / rt < 1.0, "early ratio {}", ft / rt);
            // After the ramp ends at d the cutoff is 1 kHz: a twentieth of the power, and the
            // high band (first difference) far less.
            let (rt, rd) = band_energy(&raw[c][d + 256..]);
            let (ft, fd) = band_energy(&filtered[c][d + 256..]);
            assert!(ft / rt < 0.12, "late ratio {}", ft / rt);
            assert!(fd / rd < 0.01, "late high band ratio {}", fd / rd);
        }
        // The filter is the voice's biquad run in quanta from frame 0 on both channels.
        let mut f = Biquad::new(FilterKind::Lowpass);
        f.q.set_value(0.0001, 0).unwrap();
        f.frequency.set_value_at_time(15000.0, 0.0, 0).unwrap();
        f.frequency.linear_ramp_to(1000.0, 0.25, 0).unwrap();
        let mut by_hand = raw.clone();
        let mut q = 0;
        while q < n {
            let len = 128.min(n - q);
            let mut bus = [[0.0f32; 128]; 2];
            for c in 0..2 {
                bus[c][..len].copy_from_slice(&raw[c][q..q + len]);
            }
            f.process(q as u64, &mut bus, false, &BiquadInputs::NONE);
            for c in 0..2 {
                by_hand[c][q..q + len].copy_from_slice(&bus[c][..len]);
            }
            q += 128;
        }
        assert_eq!(filtered, by_hand);
    }

    #[test]
    fn lowpass_settings_follow_js() {
        let plan = |lp: serde_json::Value, dim: serde_json::Value| {
            let mut p = params(json!(0.01), json!(0), lp);
            p.dim = v(dim);
            IrPlan::new(&p, SR)
        };
        // Falsy or loosely-zero lp: no lowpass.
        for lp in [
            json!(0),
            json!(null),
            json!(""),
            json!("0"),
            json!([]),
            json!(false),
        ] {
            assert!(
                !plan(lp.clone(), json!(1000)).unwrap().has_lowpass(),
                "{lp}"
            );
        }
        let freq = |p: &IrPlan| {
            let e = p.lowpass.as_ref().unwrap().frequency.events();
            (e[0].value, e[1].value, e[1].time)
        };
        // Clamped to Nyquist by Math.min; a falsy dim ramps to 0 Hz; strings are numbers.
        assert_eq!(
            freq(&plan(json!(1e9), json!(1e9)).unwrap()),
            (22050.0, 22050.0, 0.01)
        );
        let inf = json!({ "$num": "Infinity" });
        assert_eq!(
            freq(&plan(inf.clone(), json!(null)).unwrap()),
            (22050.0, 0.0, 0.01)
        );
        assert_eq!(
            freq(&plan(json!("800"), json!(-5)).unwrap()),
            (800.0, -5.0, 0.01)
        );
        assert_eq!(
            freq(&plan(json!(true), json!(0)).unwrap()),
            (1.0, 0.0, 0.01)
        );
        let q = plan(json!(500), json!(100)).unwrap();
        assert_eq!(q.lowpass.as_ref().unwrap().q.value(), LOWPASS_Q);
        // A frequency that is not a finite float throws from the event method.
        for (lp, dim, method) in [
            (
                json!({ "$num": "-Infinity" }),
                json!(1000),
                "setValueAtTime",
            ),
            (json!("abc"), json!(1000), "setValueAtTime"),
            (json!({}), json!(1000), "setValueAtTime"),
            (
                json!(1000),
                json!({ "$num": "-Infinity" }),
                "linearRampToValueAtTime",
            ),
            (json!(1000), json!("x"), "linearRampToValueAtTime"),
        ] {
            let e = plan(lp.clone(), dim.clone()).unwrap_err();
            assert_eq!(e.kind(), ErrorKind::Hap);
            let msg = e.to_string();
            assert!(
                msg.starts_with("TypeError") && msg.contains(method),
                "{lp} {dim}: {msg}"
            );
        }
        // The length is checked first (createBuffer comes before the lowpass).
        let mut p = params(json!(0), json!(0), json!("abc"));
        p.dim = Value::Num(1000.0);
        assert!(matches!(IrPlan::new(&p, SR), Err(IrError::Length(_))));
        // The biquad port is built for the engine's rate.
        let e = IrPlan::new(&params(json!(0.01), json!(0), json!(500)), 48000.0).unwrap_err();
        assert_eq!(e, IrError::SampleRate(48000.0));
        assert_eq!(e.kind(), ErrorKind::Hap);
        assert!(IrPlan::new(&params(json!(0.01), json!(0), json!(0)), 48000.0).is_ok());
    }

    #[test]
    fn normalization_constants() {
        let calibration = libm::powf(10.0, -58.0f32 * 0.05);
        assert!(
            (f64::from(calibration) - 10f64.powf(-2.9)).abs() < 1e-9,
            "10^(-58/20) = {calibration}"
        );
        // A full-scale square IR has power 1.
        let ones = vec![vec![1.0f32; 1000], vec![-1.0f32; 1000]];
        assert_eq!(normalization_scale(&ones), calibration);
        // Power 0.5: scale 2 * calibration.
        let half = vec![vec![0.5f32; 64]; 2];
        assert_eq!(normalization_scale(&half), 2.0 * calibration);
        // Silence, very quiet, NaN and infinite responses use the 0.000125 floor.
        let floor = (1.0 / MIN_POWER) * calibration;
        for x in [0.0f32, 1e-6, f32::NAN, f32::INFINITY] {
            assert_eq!(
                normalization_scale(&[vec![x; 300], vec![0.0; 300]]),
                floor,
                "{x}"
            );
        }
        assert_eq!(normalization_scale(&[]), floor, "0 / 0");
        // Just above the floor it is used as is.
        let r = 0.0002f32;
        let s = normalization_scale(&[vec![r; 4096], vec![r; 4096]]);
        assert!((s / (calibration / r) - 1.0).abs() < 1e-6);
        // Four channels (true stereo) halve it.
        assert_eq!(
            normalization_scale(&vec![vec![1.0f32; 10]; 4]),
            calibration * 0.5
        );
        // Float arithmetic in Blink's order: the sum, / (channels * length), sqrt, 1 / power.
        let ir = [vec![0.3f32, -0.7, 0.1], vec![0.25f32, 0.5, -0.125]];
        let sum = sum_of_squares(&ir[0]) + sum_of_squares(&ir[1]);
        let power = (sum / 6.0f32).sqrt();
        assert_eq!(normalization_scale(&ir), (1.0 / power) * calibration);
    }

    #[test]
    fn sum_of_squares_rounds_the_exact_sum_once() {
        assert_eq!(sum_of_squares(&[]), 0.0);
        assert_eq!(sum_of_squares(&[3.0, 4.0]), 25.0);
        // 2^24 + 1 is not a float: summing in float would lose the ones.
        let x = [4096.0f32, 1.0, 1.0];
        assert_eq!(sum_of_squares(&x), 16777218.0);
    }

    #[cfg(target_os = "macos")]
    mod accelerate {
        use super::*;

        #[link(name = "Accelerate", kind = "framework")]
        unsafe extern "C" {
            fn vDSP_svesq(a: *const f32, ia: isize, c: *mut f32, n: usize);
        }

        #[link(name = "Accelerate", kind = "framework")]
        unsafe extern "C" {
            fn vDSP_zvmul(
                a: *const SplitComplex,
                ia: isize,
                b: *const SplitComplex,
                ib: isize,
                c: *const SplitComplex,
                ic: isize,
                n: usize,
                conjugate: i32,
            );
            fn vDSP_conv(
                a: *const f32,
                ia: isize,
                f: *const f32,
                i_f: isize,
                c: *mut f32,
                ic: isize,
                n: usize,
                p: usize,
            );
        }

        #[repr(C)]
        struct SplitComplex {
            real: *mut f32,
            imag: *mut f32,
        }

        /// `Zvmul` is `vDSP_zvmul`: bit-identical for every length a convolver uses (and
        /// odd ones, which exercise the scalar tail).
        #[test]
        fn zvmul_is_vdsp_zvmul() {
            let mut rng = crate::js::Mulberry32::new(17);
            let mut r =
                || ((rng.next() * 2.0 - 1.0) * 2f64.powi((rng.next() * 20.0) as i32 - 10)) as f32;
            for n in [1usize, 3, 4, 7, 32, 64, 65, 128, 4096] {
                let a: Vec<[f32; 4]> = (0..n).map(|_| [r(), r(), r(), r()]).collect();
                let (mut re1, mut im1): (Vec<f32>, Vec<f32>) =
                    a.iter().map(|v| (v[0], v[1])).unzip();
                let (mut re2, mut im2): (Vec<f32>, Vec<f32>) =
                    a.iter().map(|v| (v[2], v[3])).unzip();
                let (mut cr, mut ci) = (vec![0.0f32; n], vec![0.0f32; n]);
                let sa = SplitComplex {
                    real: re1.as_mut_ptr(),
                    imag: im1.as_mut_ptr(),
                };
                let sb = SplitComplex {
                    real: re2.as_mut_ptr(),
                    imag: im2.as_mut_ptr(),
                };
                let sc = SplitComplex {
                    real: cr.as_mut_ptr(),
                    imag: ci.as_mut_ptr(),
                };
                unsafe { vDSP_zvmul(&sa, 1, &sb, 1, &sc, 1, n, 1) };
                zvmul(&mut re1, &mut im1, &re2, &im2);
                assert_eq!((re1, im1), (cr, ci), "n {n}");
            }
        }

        /// The direct stage against `vDSP_conv` (Blink's `Conv` on macOS): not the same
        /// summation order, the same sums to float rounding.
        #[test]
        fn direct_stage_is_close_to_vdsp_conv() {
            let mut rng = crate::js::Mulberry32::new(23);
            for _ in 0..50 {
                let kernel: [f32; DIRECT_TAPS] =
                    std::array::from_fn(|_| (rng.next() * 2.0 - 1.0) as f32);
                let mut d = DirectConvolver::new(kernel);
                let mut history = vec![0.0f32; DIRECT_TAPS - 1];
                for _ in 0..3 {
                    let block: [f32; QUANTUM] =
                        std::array::from_fn(|_| (rng.next() * 2.0 - 1.0) as f32);
                    let mut buf = history.clone();
                    buf.extend_from_slice(&block);
                    let mut want = [0.0f32; QUANTUM];
                    unsafe {
                        vDSP_conv(
                            buf.as_ptr(),
                            1,
                            kernel.as_ptr().add(DIRECT_TAPS - 1),
                            -1,
                            want.as_mut_ptr(),
                            1,
                            QUANTUM,
                            DIRECT_TAPS,
                        )
                    };
                    let mut got = [0.0f32; QUANTUM];
                    d.process(&block, &mut got);
                    for (g, w) in got.iter().zip(want.iter()) {
                        assert!((g - w).abs() <= 2e-6 * 8.0, "{g} vs {w}");
                    }
                    history = buf[QUANTUM..].to_vec();
                }
            }
        }

        /// `Vsvesq` on the reference machine is `vDSP_svesq`; the double sum stays within its
        /// float rounding (about 1e-6 relative for a two-second IR).
        #[test]
        fn sum_of_squares_is_close_to_vdsp_svesq() {
            for (k, n) in [1usize, 3, 4, 5, 127, 128, 4096, 33075, 132300]
                .iter()
                .enumerate()
            {
                let p = params(json!(*n as f64 / 1.5 / SR), json!(0), json!(0));
                let ir = generate_ir(&p, k as i32, SR).unwrap();
                for c in &ir {
                    let mut want = 0.0f32;
                    unsafe { vDSP_svesq(c.as_ptr(), 1, &mut want, c.len()) };
                    let got = sum_of_squares(c);
                    let rel = (f64::from(got) / f64::from(want) - 1.0).abs();
                    assert!(rel < 1e-5, "n {n}: {got} vs {want}");
                }
            }
        }
    }

    // ---- Convolver

    /// A deterministic noise IR of `n` frames per channel (decaying, so the tail matters less
    /// than the head, as in a reverb).
    fn noise_ir(n: usize, seed: u32) -> [Vec<f32>; IR_CHANNELS] {
        let mut rng = crate::js::Mulberry32::new(seed);
        std::array::from_fn(|_| {
            (0..n)
                .map(|j| ((rng.next() * 2.0 - 1.0) * (-(j as f64) / (n as f64)).exp()) as f32)
                .collect()
        })
    }

    /// Sparse bursts of noise: `quanta` quanta, every third one non-zero.
    fn bursts(quanta: usize, seed: u32) -> Vec<f32> {
        let mut rng = crate::js::Mulberry32::new(seed);
        let mut x = vec![0.0f32; quanta * QUANTUM];
        for (q, chunk) in x.chunks_mut(QUANTUM).enumerate() {
            if q % 3 == 0 {
                for (i, s) in chunk.iter_mut().enumerate() {
                    if i % 5 != 0 {
                        *s = (rng.next() * 2.0 - 1.0) as f32;
                    }
                }
            }
        }
        x
    }

    /// The exact linear convolution (double), first `len` frames.
    fn naive(x: &[f32], h: &[f32], len: usize) -> Vec<f64> {
        let mut y = vec![0.0f64; len];
        for (n, &xn) in x.iter().enumerate().filter(|(_, v)| **v != 0.0) {
            for (k, &hk) in h.iter().enumerate() {
                if n + k >= len {
                    break;
                }
                y[n + k] += f64::from(xn) * f64::from(hk);
            }
        }
        y
    }

    /// Runs `input` (one or two channels) through `c` quantum by quantum.
    fn run(c: &mut Convolver, input: &[Vec<f32>]) -> [Vec<f32>; 2] {
        let frames = input[0].len();
        let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
        for q in (0..frames).step_by(QUANTUM) {
            let bus: Vec<[f32; QUANTUM]> = input
                .iter()
                .map(|ch| ch[q..q + QUANTUM].try_into().unwrap())
                .collect();
            let mut o = [[0.0f32; QUANTUM]; 2];
            c.process(&bus, &mut o);
            out[0][q..q + QUANTUM].copy_from_slice(&o[0]);
            out[1][q..q + QUANTUM].copy_from_slice(&o[1]);
        }
        out
    }

    fn max_error(got: &[f32], want: &[f64]) -> f64 {
        let peak = want.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        let err = got
            .iter()
            .zip(want.iter())
            .fold(0.0f64, |m, (g, w)| m.max((f64::from(*g) - w).abs()));
        err / peak
    }

    #[test]
    fn convolver_equals_a_naive_convolution() {
        // Lengths around every stage boundary (64, 128, ..., 8192) and into the 4096-tap
        // stages with their pre-delays; the input runs past the IR end.
        for (k, n) in [
            1usize, 2, 63, 64, 65, 127, 128, 129, 255, 1000, 8191, 8192, 8193, 30000,
        ]
        .iter()
        .enumerate()
        {
            let ir = noise_ir(*n, k as u32);
            let quanta = n.div_ceil(QUANTUM) * 2 + 6;
            let l = bursts(quanta, 100 + k as u32);
            let r = bursts(quanta, 200 + k as u32);
            let mut c = Convolver::new(&ir, 1.0);
            let out = run(&mut c, &[l.clone(), r.clone()]);
            let len = l.len();
            for (got, x, h) in [(&out[0], &l, &ir[0]), (&out[1], &r, &ir[1])] {
                let e = max_error(got, &naive(x, h, len));
                assert!(e < 1e-5, "n {n}: relative error {e}");
            }
        }
    }

    #[test]
    fn live_tail_matches_linear_convolution_across_partition_boundaries() {
        for length in [
            tail::START - 1,
            tail::START,
            tail::START + 1,
            tail::START * 3 + 37,
        ] {
            let ir = noise_ir(length, 19);
            let frames = (2 * length + tail::START).next_multiple_of(QUANTUM);
            let mut left = vec![0.0; frames];
            let mut right = vec![0.0; frames];
            for (i, frame) in [0, 17, MAX_FFT_SIZE / 2 - 1, MAX_FFT_SIZE / 2, length]
                .into_iter()
                .enumerate()
            {
                left[frame] = 0.5 - i as f32 * 0.0625;
                right[frame + 5] = 0.125 + i as f32 * 0.0625;
            }
            let mut live = Convolver::new_live(&ir, 0.25);
            let output = run(&mut live, &[left.clone(), right.clone()]);
            for (output, input, ir) in [(&output[0], &left, &ir[0]), (&output[1], &right, &ir[1])] {
                let mut expected = naive(input, ir, frames);
                expected.iter_mut().for_each(|sample| *sample *= 0.25);
                let error = max_error(output, &expected);
                assert!(error < 1e-5, "IR length {length}: relative error {error}");
            }
        }
    }

    #[test]
    fn cached_live_tail_starts_with_empty_independent_history() {
        let ir = noise_ir(tail::START * 2, 7);
        let template = Convolver::new_live(&ir, 1.0);
        let mut a = template.fresh_live();
        let mut b = template.fresh_live();
        let input = bursts(3 * tail::START / QUANTUM, 29);
        let silence = vec![0.0; input.len()];
        let output = run(&mut a, std::slice::from_ref(&input));
        let silent = run(&mut b, &[silence]);
        assert!(silent.iter().flatten().all(|sample| *sample == 0.0));
        assert!(output[0][tail::START..].iter().any(|sample| *sample != 0.0));
        assert_eq!(output, run(&mut template.fresh_live(), &[input]));
    }

    #[test]
    fn convolver_has_no_latency_and_scales_its_kernels() {
        let n = 5000;
        let ir = noise_ir(n, 3);
        let frames = 96 * QUANTUM;
        for at in [0usize, 77, 128, 4321] {
            let mut x = vec![0.0f32; frames];
            x[at] = 1.0;
            let mut c = Convolver::new(&ir, 0.5);
            assert_eq!(c.latency_frames(), 0);
            let out = run(&mut c, std::slice::from_ref(&x));
            assert!(
                out[0][..at].iter().all(|v| *v == 0.0),
                "nothing before {at}"
            );
            for (j, (y0, y1)) in out[0][at..at + n]
                .iter()
                .zip(&out[1][at..at + n])
                .enumerate()
            {
                assert!((y0 - ir[0][j] * 0.5).abs() < 1e-6, "{at}+{j}: {y0}");
                assert!((y1 - ir[1][j] * 0.5).abs() < 1e-6, "{at}+{j}: {y1}");
            }
            assert!(
                out[0][at + n..].iter().all(|v| v.abs() < 1e-6),
                "the response ends with the IR"
            );
        }
    }

    #[test]
    fn a_mono_input_feeds_both_channels() {
        let ir = noise_ir(3000, 5);
        let x = bursts(40, 9);
        let mono = run(&mut Convolver::new(&ir, 0.25), std::slice::from_ref(&x));
        let stereo = run(&mut Convolver::new(&ir, 0.25), &[x.clone(), x.clone()]);
        assert_eq!(mono, stereo);
        assert_ne!(mono[0], mono[1], "each channel has its own IR");
    }

    #[test]
    fn the_stage_layout_is_blinks() {
        let c = Convolver::new(&noise_ir(20000, 1), 1.0);
        assert_eq!(c.length(), 20000);
        // (offset, length, fft size (0: direct), pre-delay, post-delay)
        let head = [
            (0, 64, 0, 0, 0),
            (64, 64, 128, 0, 0),
            (128, 128, 256, 0, 0),
            (256, 256, 512, 0, 0),
            (512, 512, 1024, 0, 0),
            (1024, 1024, 2048, 0, 0),
            (2048, 2048, 4096, 0, 0),
            (4096, 4096, 8192, 0, 0),
        ];
        let ch0 = c.stages(0);
        assert_eq!(&ch0[..8], &head);
        // The 4096-tap stages: delay `offset - 4096`, split by the render phase `i * 128`
        // (channel 1: `128 + i * 128`) modulo 4096.
        assert_eq!(
            &ch0[8..],
            &[
                (8192, 4096, 8192, 1024, 3072),
                (12288, 4096, 8192, 1152, 7040),
                (16384, 3616, 8192, 1280, 11008)
            ]
        );
        let ch1 = c.stages(1);
        assert_eq!(&ch1[..8], &head);
        assert_eq!(ch1[8], (8192, 4096, 8192, 1152, 2944));
        // A short IR: the direct stage only, or one more FFT stage.
        assert_eq!(
            Convolver::new(&noise_ir(10, 1), 1.0).stages(0),
            [(0, 10, 0, 0, 0)]
        );
        assert_eq!(
            Convolver::new(&noise_ir(100, 1), 1.0).stages(1),
            [(0, 64, 0, 0, 0), (64, 36, 128, 0, 0)]
        );
    }

    #[test]
    #[should_panic(expected = "two IR channels of one length")]
    fn an_empty_ir_is_refused() {
        Convolver::new(&[vec![], vec![]], 1.0);
    }

    #[test]
    fn zvmul_rounds_the_first_product() {
        let (mut re, mut im) = (vec![3.0f32, 1.0 + f32::EPSILON], vec![2.0f32, 1.0]);
        zvmul(&mut re, &mut im, &[5.0, 1.0 - f32::EPSILON], &[7.0, 1.0]);
        assert_eq!(
            (re[0], im[0]),
            (3.0 * 5.0 - 2.0 * 7.0, 3.0 * 7.0 + 2.0 * 5.0)
        );
        // (1+e)(1-e) - 1 is -e^2 exactly; rounding the first product loses it, as Accelerate.
        assert_eq!(re[1], 0.0);
    }

    // ---- Reverb (the orbit's node)

    fn num(x: f64) -> Value {
        Value::Num(x)
    }

    #[test]
    fn create_requests_once_with_the_defaults() {
        let r = Reverb::create(None, None, None, None, SR).unwrap();
        assert_eq!(r.settings(), &ReverbParams::default());
        assert_eq!((r.latest(), r.applied()), (1, 0));
        let (req, plan) = r.pending().unwrap();
        assert_eq!((req, plan.frames(), plan.has_lowpass()), (1, 132300, true));
        assert!(r.ir().is_none(), "silent until the pause is settled");
        // A failing first request creates nothing.
        let e = Reverb::create(Some(&num(0.0)), None, None, None, SR).unwrap_err();
        assert_eq!(e, IrError::Length(0.0));
    }

    #[test]
    fn regeneration_follows_has_changed() {
        let small = num(0.01);
        let mut r = Reverb::create(Some(&small), Some(&num(0.0)), None, None, SR).unwrap();
        assert_eq!(r.latest(), 1);
        // Undefined settings never trigger; equal ones neither.
        assert!(!r.update(None, None, None, None).unwrap());
        assert!(
            !r.update(Some(&small), Some(&num(-0.0)), None, None)
                .unwrap()
        );
        assert!(
            !r.update(None, None, Some(&num(15000.0)), Some(&num(1000.0)))
                .unwrap()
        );
        assert_eq!(r.latest(), 1);
        // A defined, different value regenerates with the hap's settings, defaults for the
        // undefined ones: the stored fade 0 goes back to 0.1.
        assert!(r.update(Some(&num(0.02)), None, None, None).unwrap());
        assert_eq!(r.latest(), 2);
        assert_eq!(
            r.settings(),
            &ReverbParams {
                duration: num(0.02),
                ..ReverbParams::default()
            }
        );
        // Sticky: a hap without roomsize compares nothing against the stored 0.02.
        assert!(!r.update(None, Some(&num(0.1)), None, None).unwrap());
        // Strict equality: a string never equals a stored number, null equals null, NaN and
        // arrays never equal.
        assert!(r.update(Some(&v(json!("0.02"))), None, None, None).unwrap());
        assert!(r.update(None, Some(&Value::Null), None, None).unwrap());
        assert!(!r.update(None, Some(&Value::Null), None, None).unwrap());
        assert!(r.update(None, Some(&num(f64::NAN)), None, None).unwrap());
        assert!(r.update(None, Some(&num(f64::NAN)), None, None).unwrap());
        assert!(r.update(None, Some(&v(json!([0]))), None, None).unwrap());
        assert!(r.update(None, Some(&v(json!([0]))), None, None).unwrap());
        assert_eq!(r.latest(), 8);
    }

    #[test]
    fn the_newest_request_of_a_pause_wins() {
        let mut r = Reverb::create(Some(&num(0.01)), None, Some(&num(0.0)), None, SR).unwrap();
        assert!(r.settle(0));
        assert_eq!(r.applied(), 1);
        let first = r.ir().unwrap().clone();
        assert_eq!(first[0].len(), 662);
        assert!(!r.settle(0), "nothing pending");
        // Two requests in one pause: only the second is generated and applied.
        r.update(Some(&num(0.02)), None, None, None).unwrap();
        r.update(Some(&num(0.03)), None, Some(&num(0.0)), None)
            .unwrap();
        assert_eq!(r.pending().unwrap().0, 3);
        assert!(r.settle(7));
        assert_eq!(r.applied(), 3);
        let want = generate_ir(r.settings(), 7, SR).unwrap();
        assert_eq!(r.ir().unwrap(), &want);
        assert_eq!(want[0].len(), 1985);
    }

    #[test]
    fn a_failing_request_keeps_the_ir_and_consumes_its_number() {
        let mut r = Reverb::create(Some(&num(0.01)), None, Some(&num(0.0)), None, SR).unwrap();
        r.settle(0);
        let kept = r.ir().unwrap().clone();
        // roomsize 0: the settings are stored and the number used, no IR arrives.
        let e = r.update(Some(&num(0.0)), None, None, None).unwrap_err();
        assert_eq!(e.to_string(), "reverb ir length 0");
        assert_eq!((r.latest(), r.applied()), (2, 1));
        assert_eq!(r.settings().duration, num(0.0));
        assert!(r.pending().is_none());
        assert!(!r.settle(0));
        assert_eq!(r.ir().unwrap(), &kept, "the previous IR is kept");
        // The stored 0 is what later haps compare with: roomsize 0 again is no change (no
        // error), 0.01 again is a change.
        assert!(!r.update(Some(&num(0.0)), None, None, None).unwrap());
        assert!(
            r.update(Some(&num(0.01)), None, Some(&num(0.0)), None)
                .unwrap()
        );
        // A request that fails after an earlier one in the same pause: the earlier one still
        // lands (its number is above the applied one).
        r.update(Some(&num(0.02)), None, Some(&num(0.0)), None)
            .unwrap();
        assert!(r.update(Some(&num(-1.0)), None, None, None).is_err());
        assert_eq!((r.latest(), r.pending().unwrap().0), (5, 4));
        assert!(r.settle(0));
        assert_eq!(r.applied(), 4);
        assert_eq!(r.ir().unwrap()[0].len(), 1323);
        // A lowpass error is the same kind of failure for the counters, with a hap error.
        let e = r
            .update(Some(&num(0.05)), None, Some(&v(json!("x"))), None)
            .unwrap_err();
        assert_eq!(e.kind(), ErrorKind::Hap);
        assert_eq!(r.latest(), 6);
        assert!(r.pending().is_none());
    }

    // ---- The convolver node

    fn impulse(at: usize) -> [[f32; QUANTUM]; 2] {
        let mut b = [[0.0f32; QUANTUM]; 2];
        b[0][at] = 1.0;
        b
    }

    fn node_out(
        r: &mut Reverb,
        q: u64,
        bus: &[[f32; QUANTUM]],
        silent: bool,
    ) -> ([[f32; QUANTUM]; 2], bool) {
        let out = r.process(
            q * QUANTUM as u64,
            Input {
                channels: bus,
                silent,
            },
        );
        (out.channels.try_into().unwrap(), out.silent)
    }

    #[test]
    fn the_node_plays_the_normalized_ir() {
        let mut r = Reverb::create(Some(&num(0.01)), None, Some(&num(0.0)), None, SR).unwrap();
        assert!(r.convolver().is_none());
        assert_eq!(r.tail_time(), 0.0);
        // Before an IR the node outputs zeros (not silent: `Process` runs).
        let (o, silent) = node_out(&mut r, 0, &impulse(0)[..1], false);
        assert_eq!((o, silent), ([[0.0; QUANTUM]; 2], false));
        assert!(r.settle(0));
        let ir = r.ir().unwrap().clone();
        assert_eq!(r.tail_time(), 662.0 / SR);
        let scale = normalization_scale(&ir);
        let mut c = Convolver::new(&ir, scale);
        let mut out = Vec::new();
        let mut want = Vec::new();
        for q in 0..8u64 {
            let bus = if q == 1 {
                impulse(5)
            } else {
                [[0.0; QUANTUM]; 2]
            };
            let (o, _) = node_out(&mut r, q, &bus[..1], q != 1);
            out.extend_from_slice(&o[1]);
            let mut w = [[0.0; QUANTUM]; 2];
            c.process(&bus[..1], &mut w);
            want.extend_from_slice(&w[1]);
        }
        assert_eq!(
            out, want,
            "the node is the convolver with the normalization scale"
        );
        assert!(out[128 + 5..128 + 5 + 662].iter().any(|x| *x != 0.0));
    }

    #[test]
    fn the_node_stops_after_its_tail_and_keeps_its_state() {
        let mut r = Reverb::create(Some(&num(0.01)), None, Some(&num(0.0)), None, SR).unwrap();
        r.settle(0);
        // Silent from the start: frame 0 still processes (`last_non_silent_time_` starts at 0,
        // and 0 + tail is not below 0), the next quantum past the tail does not.
        let silence = [[0.0f32; QUANTUM]; 1];
        assert!(!node_out(&mut r, 0, &silence, true).1);
        assert!(
            !node_out(&mut r, 5, &silence, true).1,
            "5 * 128 / sr < 662 / sr"
        );
        assert!(
            node_out(&mut r, 6, &silence, true).1,
            "768 frames is past the tail"
        );
        // An impulse in quantum 10: processed, and the tail is played while the input is
        // silent, until `(11 * 128 + 662) / sr`.
        assert!(!node_out(&mut r, 10, &impulse(0)[..1], false).1);
        for q in 11..=16 {
            let (_, silent) = node_out(&mut r, q, &silence, true);
            assert!(!silent, "quantum {q} is within the tail");
        }
        assert!(node_out(&mut r, 17, &silence, true).1);
    }

    #[test]
    fn a_swap_cuts_the_tail_and_the_input_history() {
        let mut r = Reverb::create(Some(&num(0.1)), None, Some(&num(0.0)), None, SR).unwrap();
        r.settle(0);
        let silence = [[0.0f32; QUANTUM]; 2];
        // A burst, then two quanta of its tail.
        let burst = [[0.5f32; QUANTUM]; 2];
        node_out(&mut r, 0, &burst, false);
        let (tail, _) = node_out(&mut r, 1, &silence, true);
        assert!(tail[0].iter().any(|x| *x != 0.0));
        // A new IR lands at the pause: the old tail and the burst in the history are gone.
        assert!(
            r.update(Some(&num(0.05)), None, Some(&num(0.0)), None)
                .unwrap()
        );
        assert!(r.settle(0));
        let (after, silent) = node_out(&mut r, 2, &silence, true);
        assert!(!silent, "still within the tail time of the last input");
        assert_eq!(after, [[0.0; QUANTUM]; 2], "nothing of the old tail");
        // New input convolves with the new IR from an empty history (a fresh convolver that
        // also ran the silent quantum, so its FFT blocks line up).
        let ir = r.ir().unwrap().clone();
        let mut fresh = Convolver::new(&ir, normalization_scale(&ir));
        let mut w = [[0.0; QUANTUM]; 2];
        fresh.process(&silence, &mut w);
        let (o, _) = node_out(&mut r, 3, &burst, false);
        fresh.process(&burst, &mut w);
        assert_eq!(o, w);
        // A pause without a request leaves the convolver alone.
        assert!(!r.settle(0));
        let (o, _) = node_out(&mut r, 4, &silence, true);
        fresh.process(&silence, &mut w);
        assert_eq!(o, w);
    }

    // ---- The IR cache

    fn reverb(size: Value, lp: Option<Value>) -> Reverb {
        Reverb::create(Some(&size), None, lp.as_ref(), None, SR).unwrap()
    }

    /// Four quanta of a stereo test signal through `r` (bit patterns).
    fn played(r: &mut Reverb) -> Vec<u32> {
        let mut bits = Vec::new();
        for q in 0..4u64 {
            let mut bus = [[0.0f32; QUANTUM]; 2];
            for (c, ch) in bus.iter_mut().enumerate() {
                for (i, x) in ch.iter_mut().enumerate() {
                    *x = ((i as f32 * 0.31 + c as f32 + q as f32).sin()) * 0.5;
                }
            }
            let (o, _) = node_out(r, q, &bus, false);
            bits.extend(o.iter().flatten().map(|x| x.to_bits()));
        }
        bits
    }

    #[test]
    fn ir_cache_hits_are_bit_identical_to_misses() {
        let mut cache = IrCache::new();
        let mut a = reverb(num(0.05), None);
        let mut b = reverb(num(0.05), None);
        assert!(a.settle_cached(3, &mut cache));
        assert_eq!(cache.generated(), 1);
        assert!(b.settle_cached(3, &mut cache));
        assert_eq!(cache.generated(), 1, "two equal reverbs generate one IR");
        // A fresh generation (no cache) and both cached ones play the same bits; the cached
        // convolver itself is never processed (a hit after `a` played is still fresh).
        let mut c = reverb(num(0.05), None);
        assert!(c.settle(3));
        let (pa, pb, pc) = (played(&mut a), played(&mut b), played(&mut c));
        assert_eq!(pa, pc);
        assert_eq!(pb, pc);
        let mut d = reverb(num(0.05), None);
        d.settle_cached(3, &mut cache);
        assert_eq!(played(&mut d), pc, "a hit after another reverb played");
        assert_eq!(d.ir(), c.ir());
    }

    #[test]
    fn ir_cache_misses_on_any_hashed_difference() {
        let mut cache = IrCache::new();
        let mut settle = |seed: i32, size: Value, lp: Option<Value>| {
            let mut r = reverb(size, lp);
            r.settle_cached(seed, &mut cache);
        };
        settle(1, num(0.05), None);
        settle(2, num(0.05), None); // another seed
        settle(1, Value::Str("0.05".into()), None); // a string roomsize hashes as a string
        settle(1, num(0.05), Some(num(800.0))); // another roomlp
        settle(1, num(0.05), Some(num(15000.0))); // the default roomlp: the first one again
        assert_eq!((cache.generated(), cache.len()), (4, 4));
        let p = IrPlan::new(
            &ReverbParams::defaulted(Some(&num(0.05)), None, None, None),
            SR,
        )
        .unwrap();
        let k = p.key(1);
        assert_eq!(k.seed, 1);
        assert_eq!(k.sample_rate_bits, SR.to_bits());
        assert_eq!(
            k.texts,
            [
                ("number", "0.05".to_string()),
                ("number", "0.1".to_string()),
                ("number", "15000".to_string()),
                ("number", "1000".to_string()),
            ]
        );
        // `fade || 0`: a null fade hashes as the number 0, as `makeRandom` sees it.
        let p = IrPlan::new(
            &ReverbParams::defaulted(Some(&num(0.05)), Some(&Value::Null), None, None),
            SR,
        )
        .unwrap();
        assert_eq!(p.key(1).texts[1], ("number", "0".to_string()));
    }

    #[test]
    fn the_orbit_and_a_local_reverb_share_an_ir() {
        let mut cache = IrCache::new();
        let mut orbit = crate::orbit::Orbit::new(
            &crate::job::OrbitSpec {
                orbit: 1.0,
                channels: vec![0, 1],
            },
            2,
        );
        orbit
            .get_reverb(Some(&num(0.05)), None, None, None)
            .unwrap();
        assert!(orbit.settle_reverb(0, &mut cache));
        let mut local = reverb(num(0.05), None);
        local.settle_cached(0, &mut cache);
        assert_eq!(cache.generated(), 1);
        assert_eq!(orbit.reverb().unwrap().ir(), local.ir());
    }

    #[test]
    fn settle_cached_drops_a_stale_pending_plan() {
        let mut cache = IrCache::new();
        let mut r = reverb(num(0.05), None);
        assert!(r.settle_cached(0, &mut cache));
        assert!(!r.settle_cached(0, &mut cache), "nothing pending");
        // A request that is not above the applied one is dropped (never generated).
        r.update(Some(&num(0.1)), None, None, None).unwrap();
        r.applied = r.latest();
        assert!(!r.settle_cached(0, &mut cache));
        assert!(r.pending().is_none());
        assert_eq!(cache.generated(), 1);
        assert_eq!(r.ir().unwrap()[0].len(), 3308, "the old IR stays");
    }
}
