// Port of Blink (Chromium 156.0.8078.4) platform/audio/dynamics_compressor.cc, modules/webaudio/dynamics_compressor_handler.cc, modules/webaudio/dynamics_compressor_node.cc, platform/audio/audio_utilities.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! The `DynamicsCompressorNode` superdough's `compressor` control builds per hap (`getCompressor`,
//! helpers.mjs:163-176; node pooling is off offline, so every hap has a fresh node).
//!
//! - Node (`dynamics_compressor_node.cc`): five k-rate params (threshold `-24` in `[-100, 0]`,
//!   knee `30` in `[0, 40]`, ratio `12` in `[1, 20]`, attack `0.003` and release `0.25` in `[0, 1]`),
//!   set with `.value =` in superdough's order.
//! - Handler (`dynamics_compressor_handler.cc`): the five `FinalValue()`s per quantum, two output
//!   channels whatever the input (a mono input feeds both pre-delay buffers), `LatencyTime` the
//!   pre-delay (256 frames until the first `Process`, 264 after), `TailTime` `5 * 0.325f` = 1.625 s.
//! - `AudioHandler::ProcessIfNecessary` per node: a silent input is processed (as zeros, with a
//!   non-silent output) until `last_non_silent_time + latency + tail < currentTime`; so a
//!   compressor created before frame 71936 of a chunk warms up on silence through the quantum at
//!   71808, one created later starts fresh at its first sounding quantum.
//! - Kernel (`dynamics_compressor.cc`): the static curve (`KAtSlope`'s 15-step bisection,
//!   `KneeCurve`, `Saturate`), the makeup gain, four 32-frame divisions with one envelope rate
//!   each (attack or the adaptive release polynomial), the detector and gain per frame, the
//!   264-frame look-ahead and the sine warp. All `float` as in the C++ (no fused multiply-add),
//!   with arm64's flush-to-zero applied where a result can underflow.
//!
//! Math: `DecibelsToLinear`/`LinearToDecibels` call the platform's `powf`/`log10f`, the knee and
//! the warp the platform's `exp`/`sin` on doubles (Apple's libm on the reference machine, so
//! the compressor micro cases are judged on macOS only); `fdlibm::powf` and `fdlibm::asinf` are
//! the `libm` crate's `pow`/`asin` on doubles rounded to `f32`.

use crate::QUANTUM;
use crate::chain::VoiceBus;
use crate::param::{Param, ParamError, ParamInputs, Rate, clamp_to, idl_float};

/// The connections into the five k-rate params, in slot order `threshold`, `ratio`, `knee`,
/// `attack`, `release` (the names `getNodeParam` finds), borrowed from the stage's inputs.
pub type CompressorInputs<'a> = [&'a ParamInputs; 5];

/// No param connected.
#[cfg(test)]
pub const NO_INPUTS: CompressorInputs<'static> = [
    &ParamInputs::NONE,
    &ParamInputs::NONE,
    &ParamInputs::NONE,
    &ParamInputs::NONE,
    &ParamInputs::NONE,
];

/// `kMeteringReleaseTimeConstant` (only `TailTime` reads it; the metering is not audible).
const K_METERING_RELEASE_TIME_CONSTANT: f32 = 0.325;
/// `kPreDelay` (seconds).
pub const K_PRE_DELAY: f32 = 0.006;
/// `kDefaultPreDelayFrames`.
const K_DEFAULT_PRE_DELAY_FRAMES: u32 = 256;
/// `kMaxPreDelayFrames`.
const K_MAX_PRE_DELAY_FRAMES: usize = 1024;
/// `kMaxPreDelayFramesMask`.
const K_MAX_PRE_DELAY_FRAMES_MASK: usize = K_MAX_PRE_DELAY_FRAMES - 1;
const K_RELEASE_ZONE_1: f32 = 0.09;
const K_RELEASE_ZONE_2: f32 = 0.16;
const K_RELEASE_ZONE_3: f32 = 0.42;
const K_RELEASE_ZONE_4: f32 = 0.98;
/// `kSatReleaseTime` (seconds).
const K_SAT_RELEASE_TIME: f32 = 0.0025;
/// `kNumberOfDivisionFrames`.
const K_NUMBER_OF_DIVISION_FRAMES: usize = 32;
/// `kPiOverTwoFloat` (`static_cast<float>(M_PI_2)`).
const K_PI_OVER_TWO_FLOAT: f32 = std::f32::consts::FRAC_PI_2;

// The release polynomial's bases, `constexpr float` sums of four `float` products evaluated left
// to right (the compiler evaluates these `const f32` expressions the same way).
#[allow(clippy::excessive_precision)] // Blink's literals, as written
const K_A_BASE: f32 = 0.9999999999999998f32 * K_RELEASE_ZONE_1
    + 1.843_221_968_432_392_3e-16_f32 * K_RELEASE_ZONE_2
    - 1.937_339_435_167_642_3e-16_f32 * K_RELEASE_ZONE_3
    + 8.824_516_011_816_245e-18_f32 * K_RELEASE_ZONE_4;
#[allow(clippy::excessive_precision)] // Blink's literals, as written
const K_B_BASE: f32 = -1.578_832_035_284_588_8_f32 * K_RELEASE_ZONE_1
    + 2.330_583_703_207_428_6_f32 * K_RELEASE_ZONE_2
    - 0.914_119_420_484_042_9_f32 * K_RELEASE_ZONE_3
    + 0.162_367_752_561_203_2_f32 * K_RELEASE_ZONE_4;
#[allow(clippy::excessive_precision)] // Blink's literals, as written
const K_C_BASE: f32 = 0.533_414_286_910_642_4_f32 * K_RELEASE_ZONE_1
    - 1.272_736_789_213_631_f32 * K_RELEASE_ZONE_2
    + 0.925_885_604_220_751_2_f32 * K_RELEASE_ZONE_3
    - 0.186_563_101_917_762_26_f32 * K_RELEASE_ZONE_4;
#[allow(clippy::excessive_precision)] // Blink's literals, as written
const K_D_BASE: f32 = 0.087_834_631_382_072_34_f32 * K_RELEASE_ZONE_1
    - 0.169_416_296_792_562_2_f32 * K_RELEASE_ZONE_2
    + 0.085_880_579_515_952_72_f32 * K_RELEASE_ZONE_3
    - 0.004_298_914_105_462_83_f32 * K_RELEASE_ZONE_4;
#[allow(clippy::excessive_precision)] // Blink's literals, as written
const K_E_BASE: f32 = -0.042_416_883_008_123_074_f32 * K_RELEASE_ZONE_1
    + 0.111_569_382_798_760_2_f32 * K_RELEASE_ZONE_2
    - 0.097_646_763_252_658_72_f32 * K_RELEASE_ZONE_3
    + 0.028_494_263_462_021_576_f32 * K_RELEASE_ZONE_4;

const SAMPLE_RATE_F32: f32 = crate::SAMPLE_RATE as f32;
use crate::SAMPLE_RATE_F64;

/// `DecibelsToLinear(db) = powf(10, 0.05f * db)`: the platform's `powf`, with the base passed
/// through `black_box` so that LLVM cannot rewrite `pow(10, x)` into an `exp10` call.
pub fn db_to_linear(db: f32) -> f32 {
    std::hint::black_box(10.0f32).powf(0.05f32 * db)
}

/// `LinearToDecibels(x) = 20 * log10f(x)`: the platform's `log10f`.
pub fn linear_to_db(x: f32) -> f32 {
    20.0f32 * x.log10()
}

/// `fdlibm::powf(x, y) = (float)pow(x, y)`.
fn fdlibm_powf(x: f32, y: f32) -> f32 {
    libm::pow(f64::from(x), f64::from(y)) as f32
}

/// `fdlibm::asinf(x) = (float)asin(x)`.
fn fdlibm_asinf(x: f32) -> f32 {
    libm::asin(f64::from(x)) as f32
}

/// `std::min(a, b)`: `b < a ? b : a` (the first argument wins on NaN).
fn cpp_min(a: f32, b: f32) -> f32 {
    if b < a { b } else { a }
}

/// `std::max(a, b)`: `a < b ? b : a` (the first argument wins on NaN).
fn cpp_max(a: f32, b: f32) -> f32 {
    if a < b { b } else { a }
}

/// `EnsureFinite(x, d)`.
fn ensure_finite(x: f32, d: f32) -> f32 {
    if x.is_finite() { x } else { d }
}

/// arm64's flush-to-zero on a float result: a subnormal becomes a zero of the same sign.
fn ftz(x: f32) -> f32 {
    if x.abs() < f32::MIN_POSITIVE {
        f32::from_bits(x.to_bits() & 0x8000_0000)
    } else {
        x
    }
}

/// `TailTime()`: `5 * kMeteringReleaseTimeConstant`, a float product (exactly 1.625) widened.
fn tail_time() -> f64 {
    f64::from(5.0f32 * K_METERING_RELEASE_TIME_CONSTANT)
}

/// superdough's `getCompressor` values (`threshold ?? -3`, `ratio ?? 10`, `knee ?? 10`, `attack
/// ?? 0.005`, `release ?? 0.05`, already defaulted).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressorParams {
    pub threshold: f64,
    pub ratio: f64,
    pub knee: f64,
    pub attack: f64,
    pub release: f64,
}

/// One `DynamicsCompressorNode`: its params, the kernel state and its handler's silence state.
#[derive(Debug, Clone)]
pub struct Compressor {
    pub threshold: Param,
    pub knee: Param,
    pub ratio: Param,
    pub attack: Param,
    pub release: Param,
    detector_average: f32,
    compressor_gain: f32,
    pre_delay: Box<[[f32; K_MAX_PRE_DELAY_FRAMES]; 2]>,
    read: usize,
    write: usize,
    last_pre_delay_frames: u32,
    db_max_attack_compression_diff: f32,
    /// Whether the static curve was computed once (Blink: the `-1` sentinels).
    initialised: bool,
    curve_ratio: f32,
    slope: f32,
    linear_threshold: f32,
    db_threshold: f32,
    db_knee: f32,
    knee_threshold: f32,
    db_knee_threshold: f32,
    db_yknee_threshold: f32,
    curve_k: f32,
    /// `AudioHandler::last_non_silent_time_` (seconds of context time).
    last_non_silent_time: f64,
    /// Consecutive input frames exactly 0 (a silent input counts), saturating.
    zero_run: u32,
    /// How many times the static curve was recomputed (tests).
    #[cfg(test)]
    curve_updates: u32,
    /// Per division: its envelope rate and the gain after each of its frames (tests).
    #[cfg(test)]
    trace: Vec<(f32, Vec<f32>)>,
}

impl Compressor {
    /// `new DynamicsCompressorNode(ac, {})` then `.value =` on threshold, ratio, knee, attack,
    /// release in that order at frame `now`; the first non-finite value throws and stops there.
    pub fn new(p: &CompressorParams, now: u64) -> Result<Compressor, ParamError> {
        let mut c = Compressor {
            threshold: Param::new(-24.0, -100.0, 0.0, Rate::K),
            knee: Param::new(30.0, 0.0, 40.0, Rate::K),
            ratio: Param::new(12.0, 1.0, 20.0, Rate::K),
            attack: Param::new(0.003, 0.0, 1.0, Rate::K),
            release: Param::new(0.25, 0.0, 1.0, Rate::K),
            detector_average: 0.0,
            compressor_gain: 1.0,
            pre_delay: Box::new([[0.0; K_MAX_PRE_DELAY_FRAMES]; 2]),
            read: 0,
            write: K_DEFAULT_PRE_DELAY_FRAMES as usize,
            last_pre_delay_frames: K_DEFAULT_PRE_DELAY_FRAMES,
            db_max_attack_compression_diff: -1.0,
            initialised: false,
            curve_ratio: -1.0,
            slope: -1.0,
            linear_threshold: -1.0,
            db_threshold: -1.0,
            db_knee: -1.0,
            knee_threshold: -1.0,
            db_knee_threshold: -1.0,
            db_yknee_threshold: -1.0,
            curve_k: -1.0,
            last_non_silent_time: 0.0,
            zero_run: 0,
            #[cfg(test)]
            curve_updates: 0,
            #[cfg(test)]
            trace: Vec::new(),
        };
        c.threshold
            .set_value(idl_float(p.threshold, "value")?, now)?;
        c.ratio.set_value(idl_float(p.ratio, "value")?, now)?;
        c.knee.set_value(idl_float(p.knee, "value")?, now)?;
        c.attack.set_value(idl_float(p.attack, "value")?, now)?;
        c.release.set_value(idl_float(p.release, "value")?, now)?;
        Ok(c)
    }

    /// `LatencyTime()`: the pre-delay in seconds.
    fn latency_time(&self) -> f64 {
        f64::from(self.last_pre_delay_frames) / SAMPLE_RATE_F64
    }

    /// `AudioHandler::PropagatesSilence` at the quantum starting at `q`.
    pub fn propagates_silence(&self, q: u64) -> bool {
        (self.last_non_silent_time + self.latency_time()) + tail_time() < q as f64 / SAMPLE_RATE_F64
    }

    /// Whether the node can no longer output a non-zero sample: its last 264 input frames (the
    /// pre-delay) were all 0. A voice is never kept alive by the tail time alone.
    pub fn is_done(&self) -> bool {
        self.zero_run >= self.last_pre_delay_frames
    }

    /// `ProcessIfNecessary` for the quantum starting at `q`: `bus` (one or two channels) is
    /// replaced by the node's two-channel output. A silent input past the silence window gives a
    /// silent output and only evaluates the params; otherwise the kernel runs (a silent input as
    /// zeros) and the output is not silent.
    #[cfg(test)]
    pub fn process(&mut self, q: u64, bus: &mut VoiceBus) {
        self.process_with(q, bus, &NO_INPUTS);
    }

    /// [`Compressor::process`] with the connections into its k-rate params (`threshold`,
    /// `ratio`, `knee`, `attack`, `release`, in that slot order): the first frame of each input.
    pub fn process_with(&mut self, q: u64, bus: &mut VoiceBus, inputs: &CompressorInputs) {
        let input_silent = bus.silent;
        if input_silent && self.propagates_silence(q) {
            let mut scratch = [0.0f32; QUANTUM];
            for (p, i) in [
                (&mut self.threshold, inputs[0]),
                (&mut self.knee, inputs[2]),
                (&mut self.ratio, inputs[1]),
                (&mut self.attack, inputs[3]),
                (&mut self.release, inputs[4]),
            ] {
                p.fill_quantum_connected(q, &mut scratch, &i.bufs, i.connected);
            }
            self.zero_run = self.zero_run.saturating_add(QUANTUM as u32).min(1024);
            bus.ch = [[0.0; QUANTUM]; 2];
            bus.channels = 2;
            bus.silent = true;
            return;
        }
        let channels = bus.channels;
        let src: [[f32; QUANTUM]; 2] = if input_silent {
            [[0.0; QUANTUM]; 2]
        } else {
            bus.ch
        };
        for f in 0..QUANTUM {
            let zero = src[..channels].iter().all(|c| c[f] == 0.0);
            self.zero_run = if zero {
                self.zero_run.saturating_add(1).min(1024)
            } else {
                0
            };
        }
        let mut out = [[0.0f32; QUANTUM]; 2];
        self.kernel(q, &src, channels, &mut out, inputs);
        bus.ch = out;
        bus.channels = 2;
        bus.silent = false;
        if !input_silent {
            self.last_non_silent_time = crate::quantum_end_time(q);
        }
    }

    /// `DynamicsCompressorHandler::Process`: the params' final values, then
    /// `DynamicsCompressor::Process` on `channels` source channels.
    fn kernel(
        &mut self,
        q: u64,
        src: &[[f32; QUANTUM]; 2],
        channels: usize,
        out: &mut [[f32; QUANTUM]; 2],
        inputs: &CompressorInputs,
    ) {
        let fin = |p: &mut Param, i: &ParamInputs| p.final_value_of_bufs(q, &i.bufs, i.connected);
        let db_threshold = fin(&mut self.threshold, inputs[0]);
        let db_knee = fin(&mut self.knee, inputs[2]);
        let ratio = fin(&mut self.ratio, inputs[1]);
        let attack_time = fin(&mut self.attack, inputs[3]);
        let release_time = fin(&mut self.release, inputs[4]);

        let sample_rate = SAMPLE_RATE_F32;
        let k = self.update_static_curve_parameters(db_threshold, db_knee, ratio);
        let linear_post_gain = fdlibm_powf(1.0 / self.saturate(1.0, k), 0.6f32);
        let attack_frames = cpp_max(0.001f32, attack_time) * sample_rate;
        let release_frames = sample_rate * release_time;
        let sat_release_frames = K_SAT_RELEASE_TIME * sample_rate;
        let a = release_frames * K_A_BASE;
        let b = release_frames * K_B_BASE;
        let c = release_frames * K_C_BASE;
        let d = release_frames * K_D_BASE;
        let e = release_frames * K_E_BASE;

        self.set_pre_delay_time(K_PRE_DELAY);

        let mut frame_index = 0;
        while frame_index < QUANTUM {
            self.detector_average = ensure_finite(self.detector_average, 1.0);
            let desired_gain = self.detector_average;
            let scaled_desired_gain = fdlibm_asinf(desired_gain) / K_PI_OVER_TWO_FLOAT;
            let is_releasing = scaled_desired_gain > self.compressor_gain;
            let mut db_compression_diff = if scaled_desired_gain == 0.0 {
                if is_releasing { -1.0 } else { 1.0 }
            } else {
                linear_to_db(self.compressor_gain / scaled_desired_gain)
            };
            let envelope_rate = if is_releasing {
                self.db_max_attack_compression_diff = -1.0;
                db_compression_diff = ensure_finite(db_compression_diff, -1.0);
                let mut x = db_compression_diff;
                x = clamp_to(x, -12.0, 0.0);
                x = 0.25f32 * (x + 12.0);
                let x2 = x * x;
                let x3 = x2 * x;
                let x4 = x2 * x2;
                let calc_release_frames = a + b * x + c * x2 + d * x3 + e * x4;
                let db_per_frame = 5.0f32 / calc_release_frames;
                db_to_linear(db_per_frame)
            } else {
                db_compression_diff = ensure_finite(db_compression_diff, 1.0);
                if self.db_max_attack_compression_diff == -1.0
                    || self.db_max_attack_compression_diff < db_compression_diff
                {
                    self.db_max_attack_compression_diff = db_compression_diff;
                }
                let db_eff_atten_diff = cpp_max(0.5f32, self.db_max_attack_compression_diff);
                let x = 0.25f32 / db_eff_atten_diff;
                1.0f32 - fdlibm_powf(x, 1.0f32 / attack_frames)
            };

            let mut read = self.read;
            let mut write = self.write;
            let mut detector_average = self.detector_average;
            let mut compressor_gain = self.compressor_gain;
            let loop_frames = K_NUMBER_OF_DIVISION_FRAMES.min(QUANTUM - frame_index);
            #[cfg(test)]
            self.trace.push((envelope_rate, Vec::new()));
            for _ in 0..loop_frames {
                let mut compressor_input = 0.0f32;
                for (j, delay) in self.pre_delay.iter_mut().enumerate() {
                    let undelayed_source = src[if channels > j { j } else { 0 }][frame_index];
                    delay[write] = undelayed_source;
                    let abs_undelayed_source = if undelayed_source > 0.0 {
                        undelayed_source
                    } else {
                        -undelayed_source
                    };
                    if compressor_input < abs_undelayed_source {
                        compressor_input = abs_undelayed_source;
                    }
                }
                let scaled_input = compressor_input;
                let abs_input = if scaled_input > 0.0 {
                    scaled_input
                } else {
                    -scaled_input
                };
                let shaped_input = self.saturate(abs_input, k);
                let attenuation = if abs_input <= 0.0001f32 {
                    1.0
                } else {
                    shaped_input / abs_input
                };
                let is_release = attenuation > detector_average;
                // The release rate is pure, so it is computed only where it is used.
                let rate = if is_release {
                    let db_attenuation = cpp_max(2.0f32, -linear_to_db(attenuation));
                    let db_per_frame = db_attenuation / sat_release_frames;
                    db_to_linear(db_per_frame) - 1.0
                } else {
                    1.0
                };
                detector_average =
                    ftz(detector_average + ftz(ftz(attenuation - detector_average) * rate));
                detector_average = cpp_min(1.0, detector_average);
                detector_average = ensure_finite(detector_average, 1.0);
                if envelope_rate < 1.0 {
                    compressor_gain = ftz(compressor_gain
                        + ftz(ftz(scaled_desired_gain - compressor_gain) * envelope_rate));
                } else {
                    compressor_gain = ftz(compressor_gain * envelope_rate);
                    compressor_gain = cpp_min(1.0, compressor_gain);
                }
                #[cfg(test)]
                if let Some(t) = self.trace.last_mut() {
                    t.1.push(compressor_gain);
                }
                let post_warp_compressor_gain =
                    ftz(f64::from(K_PI_OVER_TWO_FLOAT * compressor_gain).sin() as f32);
                let total_gain = ftz(linear_post_gain * post_warp_compressor_gain);
                for (o, delay) in out.iter_mut().zip(self.pre_delay.iter()) {
                    o[frame_index] = ftz(delay[read] * total_gain);
                }
                frame_index += 1;
                read = (read + 1) & K_MAX_PRE_DELAY_FRAMES_MASK;
                write = (write + 1) & K_MAX_PRE_DELAY_FRAMES_MASK;
            }
            self.read = read;
            self.write = write;
            self.detector_average = ftz(detector_average);
            self.compressor_gain = ftz(compressor_gain);
        }
    }

    /// `SetPreDelayTime`: `(unsigned)(t * sr)` frames (264 for 6 ms), capped at 1023; a change
    /// (the first call: from 256) zeroes both buffers and resets the indices.
    fn set_pre_delay_time(&mut self, pre_delay_time: f32) {
        let mut pre_delay_frames = (pre_delay_time * SAMPLE_RATE_F32) as u32;
        if pre_delay_frames > K_MAX_PRE_DELAY_FRAMES as u32 - 1 {
            pre_delay_frames = K_MAX_PRE_DELAY_FRAMES as u32 - 1;
        }
        if self.last_pre_delay_frames != pre_delay_frames {
            self.last_pre_delay_frames = pre_delay_frames;
            for b in self.pre_delay.iter_mut() {
                b.fill(0.0);
            }
            self.read = 0;
            self.write = pre_delay_frames as usize;
        }
    }

    /// `KneeCurve(x, k)`: linear up to the threshold, then the exponential knee.
    fn knee_curve(&self, x: f32, k: f32) -> f32 {
        if x < self.linear_threshold {
            return x;
        }
        let ex = f64::from(-k * (x - self.linear_threshold)).exp() as f32;
        self.linear_threshold + (1.0 - ex) / k
    }

    /// `Saturate(x, k)`: the knee curve, then the constant ratio past the knee.
    fn saturate(&self, x: f32, k: f32) -> f32 {
        if x < self.knee_threshold {
            return self.knee_curve(x, k);
        }
        let db_x = linear_to_db(x);
        let db_y = self.db_yknee_threshold + self.slope * (db_x - self.db_knee_threshold);
        db_to_linear(db_y)
    }

    /// `KAtSlope(desired_slope)`: the knee's `k` whose slope at the knee's end is
    /// `desired_slope` (15 geometric bisection steps between 0.1 and 10000).
    fn k_at_slope(&self, desired_slope: f32) -> f32 {
        let db_x = self.db_threshold + self.db_knee;
        let x = db_to_linear(db_x);
        let mut x2 = 1.0f32;
        let mut db_x2 = 0.0f32;
        // `!(x < linear_threshold_)`: true for a NaN too.
        let above = x.partial_cmp(&self.linear_threshold) != Some(std::cmp::Ordering::Less);
        if above {
            // `x * 1.001` is a double product.
            x2 = (f64::from(x) * 1.001) as f32;
            db_x2 = linear_to_db(x2);
        }
        let mut min_k = 0.1f64 as f32;
        let mut max_k = 10000.0f32;
        let mut k = 5.0f32;
        let mut slope = 1.0f32;
        for _ in 0..15 {
            if above {
                let db_y = linear_to_db(self.knee_curve(x, k));
                let db_y2 = linear_to_db(self.knee_curve(x2, k));
                slope = (db_y2 - db_y) / (db_x2 - db_x);
            }
            if slope < desired_slope {
                max_k = k;
            } else {
                min_k = k;
            }
            k = (min_k * max_k).sqrt();
        }
        k
    }

    /// `UpdateStaticCurveParameters`: recomputed only when threshold, knee or ratio changed
    /// (always on the first call); returns the knee's `k`.
    fn update_static_curve_parameters(
        &mut self,
        db_threshold: f32,
        db_knee: f32,
        ratio: f32,
    ) -> f32 {
        if !self.initialised
            || db_threshold != self.db_threshold
            || db_knee != self.db_knee
            || ratio != self.curve_ratio
        {
            self.initialised = true;
            #[cfg(test)]
            {
                self.curve_updates += 1;
            }
            self.db_threshold = db_threshold;
            self.linear_threshold = db_to_linear(db_threshold);
            self.db_knee = db_knee;
            self.curve_ratio = ratio;
            self.slope = 1.0 / self.curve_ratio;
            let k = self.k_at_slope(1.0 / self.curve_ratio);
            self.db_knee_threshold = db_threshold + db_knee;
            self.knee_threshold = db_to_linear(self.db_knee_threshold);
            self.db_yknee_threshold = linear_to_db(self.knee_curve(self.knee_threshold, k));
            self.curve_k = k;
        }
        self.curve_k
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::QUANTUM_U64 as Q;

    fn defaults() -> CompressorParams {
        CompressorParams {
            threshold: -3.0,
            ratio: 10.0,
            knee: 10.0,
            attack: 0.005,
            release: 0.05,
        }
    }

    fn with(threshold: f64, ratio: f64, knee: f64) -> Compressor {
        Compressor::new(
            &CompressorParams {
                threshold,
                ratio,
                knee,
                ..defaults()
            },
            0,
        )
        .unwrap()
    }

    fn mono(samples: [f32; QUANTUM]) -> VoiceBus {
        let mut b = VoiceBus::mono();
        b.ch[0] = samples;
        b.silent = false;
        b
    }

    #[test]
    fn connected_k_rate_params_read_the_first_frame_of_each_input() {
        // A threshold of -3 with an input whose first frame is -17 (and whose later frames are
        // ignored) runs the kernel at -20, as a compressor built at -20 does.
        let loud: [f32; QUANTUM] = std::array::from_fn(|i| ((i as f32) * 0.3).sin() * 0.9);
        let mut base = with(-20.0, 10.0, 10.0);
        let mut connected = with(-3.0, 10.0, 10.0);
        let mut input = [5.0f32; QUANTUM];
        input[0] = -17.0;
        let threshold = ParamInputs {
            bufs: vec![input],
            connected: true,
        };
        let mut inputs = NO_INPUTS;
        inputs[0] = &threshold;
        for k in 0..20 {
            let (mut a, mut b) = (mono(loud), mono(loud));
            base.process(k * Q, &mut a);
            connected.process_with(k * Q, &mut b, &inputs);
            let bits = |x: &VoiceBus| {
                x.ch.iter()
                    .flatten()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>()
            };
            assert_eq!(bits(&a), bits(&b), "quantum {k}");
        }
    }

    #[test]
    fn bases_bit_patterns() {
        // Computed from the C++ text with IEEE float32 left-to-right arithmetic; recompute them
        // with a C program (not by editing this test) if they ever differ.
        assert_eq!(K_A_BASE.to_bits(), 0x3db8_51ec, "kABase");
        assert_eq!(K_B_BASE.to_bits(), 0x3bc4_3d40, "kBBase");
        assert_eq!(K_C_BASE.to_bits(), 0x3d4e_7a34, "kCBase");
        assert_eq!(K_D_BASE.to_bits(), 0x3c4f_58aa, "kDBase");
        assert_eq!(K_E_BASE.to_bits(), 0x3a78_1280, "kEBase");
        assert_eq!(tail_time(), 1.625, "5 * 0.325f rounds to exactly 1.625");
        assert_eq!((K_PRE_DELAY * SAMPLE_RATE_F32) as u32, 264);
    }

    #[test]
    fn helpers_follow_the_cpp_argument_order() {
        assert_eq!(cpp_min(1.0, f32::NAN), 1.0);
        assert!(cpp_min(f32::NAN, 1.0).is_nan());
        assert_eq!(cpp_max(2.0, f32::NAN), 2.0);
        assert!(cpp_max(f32::NAN, 2.0).is_nan());
        assert_eq!(cpp_min(1.0, 0.5), 0.5);
        assert_eq!(cpp_max(2.0, 3.0), 3.0);
        assert_eq!(ensure_finite(f32::INFINITY, 1.0), 1.0);
        assert_eq!(ensure_finite(f32::NAN, -1.0), -1.0);
        assert_eq!(ensure_finite(0.25, 1.0), 0.25);
    }

    #[test]
    fn static_curve_k_at_slope() {
        for (t, r, kn) in [(-3.0, 10.0, 10.0), (-20.0, 4.0, 6.0), (-30.0, 20.0, 40.0)] {
            let mut c = with(t, r, kn);
            let k = c.update_static_curve_parameters(t as f32, kn as f32, r as f32);
            // The numeric slope at the knee's end in dB is 1 / ratio within the bisection's
            // resolution: k moves by a factor (10000 / 0.1)^(1 / 2^15) per step at the end.
            let x = db_to_linear((t + kn) as f32);
            let x2 = (f64::from(x) * 1.001) as f32;
            let slope_at = |k: f32| {
                (linear_to_db(c.knee_curve(x2, k)) - linear_to_db(c.knee_curve(x, k)))
                    / (linear_to_db(x2) - linear_to_db(x))
            };
            let step = (10000.0f64 / 0.1).powf(1.0 / 32768.0) as f32;
            let (lo, hi) = (slope_at(k * step), slope_at(k / step));
            let s = 1.0 / r as f32;
            assert!(
                lo <= s + 1e-3 && s - 1e-3 <= hi,
                "{t}/{r}/{kn}: {lo} {s} {hi}"
            );
            assert!(c.saturate(1.0, k) > 0.0, "{t}/{r}/{kn}");
        }
    }

    #[test]
    fn static_curve_recompute_only_on_change() {
        let mut c = with(-3.0, 10.0, 10.0);
        let k1 = c.update_static_curve_parameters(-3.0, 10.0, 10.0);
        assert_eq!(c.curve_updates, 1, "the first call always computes");
        let k2 = c.update_static_curve_parameters(-3.0, 10.0, 10.0);
        assert_eq!(c.curve_updates, 1, "equal params do not recompute");
        assert_eq!(k1.to_bits(), k2.to_bits());
        c.update_static_curve_parameters(-3.0, 10.0, 12.0);
        assert_eq!(c.curve_updates, 2, "a ratio change recomputes");
        c.update_static_curve_parameters(-4.0, 10.0, 12.0);
        c.update_static_curve_parameters(-4.0, 9.0, 12.0);
        assert_eq!(c.curve_updates, 4);
        // A threshold of -1 with knee -1 (the sentinels) still computes on the first call.
        let mut c = with(-1.0, 1.0, 0.0);
        c.db_knee = -1.0;
        c.update_static_curve_parameters(-1.0, -1.0, -1.0);
        assert_eq!(c.curve_updates, 1);
    }

    #[test]
    fn division_rates() {
        // A constant input of 0.5: one envelope rate per 32-frame division (four per quantum),
        // every frame of a division following `g += (s - g) * rate` with that one rate.
        let mut c = Compressor::new(&defaults(), 0).unwrap();
        for i in 0..3 {
            let mut bus = mono([0.5; QUANTUM]);
            c.process(i * Q, &mut bus);
            assert_eq!(bus.channels, 2);
            assert!(!bus.silent);
        }
        assert_eq!(c.trace.len(), 12, "four divisions per quantum");
        assert!(c.trace.iter().all(|(_, g)| g.len() == 32));
        // The first division attacks from gain 1 towards the fresh detector's desired gain 0.
        let attack_frames = 0.005f32 * SAMPLE_RATE_F32;
        let rate = 1.0f32 - fdlibm_powf(0.25 / 1.0, 1.0 / attack_frames);
        assert_eq!(c.trace[0].0.to_bits(), rate.to_bits());
        let mut gain = 1.0f32;
        for (f, got) in c.trace[0].1.iter().enumerate() {
            gain += (0.0 - gain) * rate;
            assert_eq!(got.to_bits(), gain.to_bits(), "frame {f}");
        }
        let rates: Vec<u32> = c.trace.iter().map(|(r, _)| r.to_bits()).collect();
        assert!(
            rates.windows(2).any(|w| w[0] != w[1]),
            "the rate changes between divisions"
        );
    }

    #[test]
    fn pre_delay_264() {
        let mut c = Compressor::new(&defaults(), 0).unwrap();
        assert_eq!(c.last_pre_delay_frames, 256);
        assert_eq!(c.latency_time(), 256.0 / 44100.0);
        let mut first = [0.0f32; QUANTUM];
        first[0] = 0.25;
        let mut out = Vec::new();
        let mut b = mono(first);
        c.process(0, &mut b);
        out.extend_from_slice(&b.ch[0]);
        assert_eq!(c.last_pre_delay_frames, 264);
        assert_eq!(c.latency_time(), 264.0 / 44100.0);
        for i in 1..4 {
            let mut b = mono([0.0; QUANTUM]);
            c.process(i * Q, &mut b);
            assert_eq!(
                b.ch[0], b.ch[1],
                "a mono input feeds both pre-delay buffers"
            );
            out.extend_from_slice(&b.ch[0]);
        }
        let nonzero: Vec<usize> = (0..out.len()).filter(|&i| out[i] != 0.0).collect();
        assert_eq!(nonzero, vec![264], "the impulse comes out 264 frames late");
        assert!(out[264] > 0.0 && out[264] < 0.25 * 4.0);
    }

    #[test]
    fn silence_window_boundary() {
        let mut c = Compressor::new(&defaults(), 0).unwrap();
        // Before the first `process`: latency 256 frames.
        assert!(!c.propagates_silence(71808));
        assert!(c.propagates_silence(71936));
        let mut b = VoiceBus::mono();
        c.process(0, &mut b);
        assert!(!b.silent, "a silent input inside the window is processed");
        // After it: latency 264 frames, the same boundary.
        assert_eq!(
            c.last_non_silent_time, 0.0,
            "a silent input does not move it"
        );
        assert!(!c.propagates_silence(71808));
        assert!(c.propagates_silence(71936));
        // A non-silent quantum moves the window.
        let mut b = mono([0.1; QUANTUM]);
        c.process(1280, &mut b);
        assert_eq!(c.last_non_silent_time, 1408.0 / 44100.0);
        assert!(!c.propagates_silence(71936));
        // A silent input past the window: silent output, the kernel untouched.
        let mut c = Compressor::new(&defaults(), 0).unwrap();
        let mut b = VoiceBus::mono();
        b.ch[0] = [9.0; QUANTUM];
        c.process(71936, &mut b);
        assert!(b.silent);
        assert_eq!(b.channels, 2);
        assert!(b.ch.iter().flatten().all(|x| *x == 0.0));
        assert_eq!(c.last_pre_delay_frames, 256, "the kernel did not run");
        assert_eq!(c.detector_average, 0.0);
        let mut b = VoiceBus::mono();
        c.process(71808, &mut b);
        assert!(!b.silent, "71808 is still inside");
        assert_eq!(c.last_pre_delay_frames, 264);
    }

    /// The output of `frames` quanta of zeros from frame `from`, then 4 quanta of a 0.3 tone.
    fn after_silence(from: u64, quanta: u64) -> Vec<f32> {
        let mut c = Compressor::new(&defaults(), from).unwrap();
        for i in 0..quanta {
            let mut b = VoiceBus::mono();
            c.process(from + i * Q, &mut b);
        }
        let mut out = Vec::new();
        for i in 0..4 {
            let tone: [f32; QUANTUM] = std::array::from_fn(|f| if f % 2 == 0 { 0.3 } else { -0.3 });
            let mut b = mono(tone);
            c.process(from + (quanta + i) * Q, &mut b);
            out.extend_from_slice(&b.ch[0]);
        }
        out
    }

    #[test]
    fn warmup_vs_fresh() {
        let fresh = after_silence(0, 0);
        let warm = after_silence(0, 60);
        assert_ne!(fresh, warm, "silence warmed the detector");
        // Created past the window: the silent quanta are skipped, the onset state is fresh.
        let late = after_silence(80_000, 60);
        assert_eq!(late, fresh);
    }

    #[test]
    fn silent_input_is_zero_samples() {
        let mut a = Compressor::new(&defaults(), 0).unwrap();
        let mut b = Compressor::new(&defaults(), 0).unwrap();
        for i in 0..4 {
            let mut garbage = VoiceBus::mono();
            garbage.ch[0] = [0.7; QUANTUM];
            garbage.silent = true;
            a.process(i * Q, &mut garbage);
            let mut zeros = mono([0.0; QUANTUM]);
            b.process(i * Q, &mut zeros);
            assert_eq!(garbage.ch, zeros.ch);
        }
        assert_eq!(a.detector_average.to_bits(), b.detector_average.to_bits());
        assert_eq!(a.last_non_silent_time, 0.0);
        assert_eq!(b.last_non_silent_time, 512.0 / 44100.0);
    }

    #[test]
    fn release_zero_never_nan() {
        for release in [0.0, 1e-9] {
            let mut c = Compressor::new(
                &CompressorParams {
                    release,
                    ..defaults()
                },
                0,
            )
            .unwrap();
            for i in 0..40 {
                let level = if (i / 5) % 2 == 0 { 0.9 } else { 0.001 };
                let mut b = mono([level; QUANTUM]);
                c.process(i * Q, &mut b);
                assert!(b.ch.iter().flatten().all(|x| x.is_finite()), "quantum {i}");
            }
            assert!(c.compressor_gain.is_finite() && c.compressor_gain <= 1.0);
        }
    }

    #[test]
    fn ftz_flushes_with_the_sign() {
        assert_eq!(ftz(1e-39).to_bits(), 0);
        assert_eq!(ftz(-1e-39).to_bits(), (-0.0f32).to_bits());
        assert_eq!(ftz(f32::MIN_POSITIVE), f32::MIN_POSITIVE);
        assert!(ftz(f32::NAN).is_nan());
        // An underflowing gain update stores 0.
        let mut c = Compressor::new(&defaults(), 0).unwrap();
        c.compressor_gain = 1e-38;
        c.detector_average = 0.0;
        c.db_max_attack_compression_diff = 100.0;
        let mut b = mono([0.0; QUANTUM]);
        c.process(0, &mut b);
        assert_eq!(
            c.compressor_gain.to_bits(),
            0,
            "the subnormal gain is flushed to +0"
        );
        assert!(!c.detector_average.is_subnormal());
    }

    #[test]
    fn is_done_after_264_zero_frames() {
        let mut c = Compressor::new(&defaults(), 0).unwrap();
        let mut b = mono([0.5; QUANTUM]);
        c.process(0, &mut b);
        assert!(!c.is_done());
        let mut tail = [0.0f32; QUANTUM];
        tail[0] = 0.5;
        let mut b = mono(tail);
        c.process(Q, &mut b);
        assert_eq!(c.zero_run, 127);
        let mut b = mono([0.0; QUANTUM]);
        c.process(2 * Q, &mut b);
        assert_eq!(c.zero_run, 255);
        assert!(
            !c.is_done(),
            "255 zero frames: the pre-delay still holds a sample"
        );
        let mut b = VoiceBus::mono();
        c.process(3 * Q, &mut b);
        assert!(c.is_done(), "a silent input counts as zeros");
        let mut b = VoiceBus::mono();
        c.process(4 * Q, &mut b);
        assert!(b.ch.iter().flatten().all(|x| *x == 0.0));
        for i in 5..20 {
            let mut b = VoiceBus::mono();
            c.process(i * Q, &mut b);
        }
        assert_eq!(c.zero_run, 1024, "saturates");
        // A negative zero is a zero.
        let mut b = mono([-0.0; QUANTUM]);
        c.process(20 * Q, &mut b);
        assert!(c.is_done());
    }

    #[test]
    fn params_clamp_and_throw() {
        let c = Compressor::new(
            &CompressorParams {
                threshold: -200.0,
                ratio: 30.0,
                knee: 45.0,
                attack: -1.0,
                release: 3.0,
            },
            256,
        )
        .unwrap();
        assert_eq!(c.threshold.value(), -100.0);
        assert_eq!(c.ratio.value(), 20.0);
        assert_eq!(c.knee.value(), 40.0);
        assert_eq!(c.attack.value(), 0.0);
        assert_eq!(c.release.value(), 1.0);
        for p in [&c.threshold, &c.ratio, &c.knee, &c.attack, &c.release] {
            let ev = p.events();
            assert_eq!(ev.len(), 1);
            assert_eq!(ev[0].time, 256.0 / 44100.0);
            assert_eq!(ev[0].value, p.value(), "the event holds the clamped value");
        }
        // The first non-finite value throws, in the order threshold, ratio, knee, attack, release.
        for k in 0..5 {
            let mut v = [-3.0, 10.0, 10.0, 0.005, 0.05];
            v[k] = f64::NAN;
            if k == 2 {
                v[3] = f64::INFINITY;
            }
            let p = CompressorParams {
                threshold: v[0],
                ratio: v[1],
                knee: v[2],
                attack: v[3],
                release: v[4],
            };
            assert_eq!(
                Compressor::new(&p, 0).unwrap_err(),
                ParamError::NonFiniteValue { method: "value" },
                "{k}"
            );
        }
        assert!(
            Compressor::new(
                &CompressorParams {
                    attack: 1e39,
                    ..defaults()
                },
                0
            )
            .is_err(),
            "a double that rounds to an infinite float throws"
        );
    }

    #[test]
    fn stereo_input_takes_the_louder_channel() {
        let mut c = Compressor::new(&defaults(), 0).unwrap();
        let mut m = Compressor::new(&defaults(), 0).unwrap();
        let mut out_s = Vec::new();
        let mut out_m = Vec::new();
        for i in 0..4 {
            let mut s = VoiceBus::mono();
            s.channels = 2;
            s.silent = false;
            s.ch[0] = [0.2; QUANTUM];
            s.ch[1] = [-0.8; QUANTUM];
            c.process(i * Q, &mut s);
            let mut b = mono([0.8; QUANTUM]);
            m.process(i * Q, &mut b);
            out_s.extend_from_slice(&s.ch[1]);
            out_m.extend_from_slice(&b.ch[0]);
        }
        // The detector sees max |x| = 0.8 in both: the right channel is the negated mono one.
        let neg: Vec<f32> = out_m.iter().map(|x| -x).collect();
        assert_eq!(out_s, neg);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn db_helpers_are_the_platform_libm() {
        unsafe extern "C" {
            fn powf(x: f32, y: f32) -> f32;
            fn log10f(x: f32) -> f32;
        }
        for i in 0..1000 {
            let db = -120.0 + 0.137 * i as f32;
            // SAFETY: pure C functions of floats.
            let want = unsafe { powf(10.0, 0.05 * db) };
            assert_eq!(db_to_linear(db).to_bits(), want.to_bits(), "{db}");
            let x = 1e-6f32 * 1.0173f32.powi(i);
            // SAFETY: as above.
            let want = unsafe { 20.0 * log10f(x) };
            assert_eq!(linear_to_db(x).to_bits(), want.to_bits(), "{x}");
        }
    }
}
