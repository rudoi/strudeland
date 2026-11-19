// Port of Blink (Chromium 156.0.8078.4) modules/webaudio/oscillator_handler.cc, modules/webaudio/cpu/arm/oscillator_kernel_neon.cc, modules/webaudio/audio_scheduled_source_handler.cc, modules/webaudio/constant_source_handler.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! Scheduled sources: the start/stop bookkeeping every source shares
//! (`AudioScheduledSourceHandler::UpdateSchedulingInfo`), the `OscillatorNode`
//! (`OscillatorHandler::Process` with its a-rate and k-rate paths, the arm64 NEON kernels
//! `ProcessKRateVector` and `ProcessARateVectorKernel`, and the scalar `DoInterpolation`) and the
//! `ConstantSourceNode`.
//!
//! The arithmetic mirrors the C++ types exactly: phase increments and the vector kernels are
//! `f32` (the NEON k-rate kernel even keeps its read indices in `f32`), the virtual read index is
//! an `f64` wrapped after every frame, the 2-point interpolation of the scalar paths and the
//! Lagrange paths accumulate in `f64`. No operation is fused (Chromium builds with
//! `-ffp-contract=off`; `vmlaq_f32` is a separate multiply and add). Quirks kept on purpose:
//!
//! - the sub-sample start phase uses a local `frequency` that only the k-rate path assigns, so an
//!   oscillator whose first quantum is a-rate starts at phase 0;
//! - on the a-rate path the phase increments are read from the start of the quantum's buffer
//!   while the output starts at the source's first frame in the quantum;
//! - when only `detune` is sample accurate, the Nyquist clamp applies to the detune multipliers,
//!   not to the frequency.

use std::rc::Rc;

use crate::QUANTUM;
use crate::param::{Param, Rate};
use crate::periodic::{PeriodicWave, RATE_SCALE, TABLE_SIZE};

use crate::SAMPLE_RATE_F64 as SR;
/// `kInterpolate2Point`: linear interpolation from this increment on.
pub const INTERPOLATE_2POINT: f32 = 0.3;
/// `kInterpolate3Point`: 3-point Lagrange from this increment on, 5-point below.
pub const INTERPOLATE_3POINT: f32 = 0.16;
/// `Context()->sampleRate() / 2`.
pub const NYQUIST: f32 = 22050.0;
/// The oscillator `detune` range: `1200 * log2f(FLT_MAX)` cents.
pub const DETUNE_LIMIT: f32 = 153_600.0;
const SIZE_F64: f64 = TABLE_SIZE as f64;
const INV_SIZE: f64 = 1.0 / SIZE_F64;
const MASK: u32 = TABLE_SIZE as u32 - 1;

/// A param's audio-rate connections, pulled when the param is evaluated: pushes the output of
/// each connected node for the quantum starting at the given frame, in connection order,
/// leaving out silent buses (`AudioChannel::SumFrom` skips them). Each output is added to the
/// param's values in turn, as Blink's summing bus (the values themselves) does.
pub type Input<'a> = &'a mut dyn FnMut(u64, &mut Vec<[f32; QUANTUM]>);

/// `TimeToSampleFrame(t, sr, kRoundUp)`.
fn time_to_frame(t: f64) -> u64 {
    crate::render::frame_for_buffer_start(t)
}

/// The playback state of a scheduled source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Unscheduled,
    Scheduled,
    Playing,
    Finished,
}

/// Why `start`/`stop` throw (the WebIDL `double` conversion and the range checks).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ScheduleError {
    #[error(
        "TypeError: Failed to execute '{0}' on 'AudioScheduledSourceNode': The provided double value is non-finite."
    )]
    NonFinite(&'static str),
    #[error(
        "RangeError: Failed to execute '{0}' on 'AudioScheduledSourceNode': The {0} time provided ({1}) is less than the minimum bound (0)."
    )]
    Negative(&'static str, String),
    #[error("InvalidStateError: Failed to execute '{0}' on 'AudioScheduledSourceNode': {1}")]
    State(&'static str, &'static str),
}

/// The part of the quantum a source plays: `offset` is the first frame, `frames` how many frames
/// from there sound, `start_frame_offset` the sub-sample start (non-zero only in the quantum the
/// source starts playing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Playing {
    pub offset: usize,
    pub frames: usize,
    pub start_frame_offset: f64,
}

/// `AudioScheduledSourceHandler`'s start/stop state.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    start_time: f64,
    end_time: Option<f64>,
    state: PlaybackState,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule::new()
    }
}

impl Schedule {
    pub fn new() -> Schedule {
        Schedule {
            start_time: 0.0,
            end_time: None,
            state: PlaybackState::Unscheduled,
        }
    }

    #[cfg(test)]
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    /// `start(when)` called at context frame `now`: `start_time = max(when, currentTime)`.
    pub fn start(&mut self, when: f64, now: u64) -> Result<(), ScheduleError> {
        if !when.is_finite() {
            return Err(ScheduleError::NonFinite("start"));
        }
        if self.state != PlaybackState::Unscheduled {
            return Err(ScheduleError::State(
                "start",
                "cannot call start more than once.",
            ));
        }
        if when < 0.0 {
            return Err(ScheduleError::Negative(
                "start",
                crate::js::number_to_string(when),
            ));
        }
        self.start_time = when.max(now as f64 / SR);
        self.state = PlaybackState::Scheduled;
        Ok(())
    }

    /// `stop(when)`: the last call wins (`end_time = max(0, when)`).
    pub fn stop(&mut self, when: f64) -> Result<(), ScheduleError> {
        if !when.is_finite() {
            return Err(ScheduleError::NonFinite("stop"));
        }
        if self.state == PlaybackState::Unscheduled {
            return Err(ScheduleError::State(
                "stop",
                "cannot call stop without calling start first.",
            ));
        }
        if when < 0.0 {
            return Err(ScheduleError::Negative(
                "stop",
                crate::js::number_to_string(when),
            ));
        }
        self.end_time = Some(when.max(0.0));
        Ok(())
    }

    /// The first frame the source sounds at.
    pub fn start_frame(&self) -> u64 {
        time_to_frame(self.start_time)
    }

    /// The exclusive end frame (`None` while no stop is scheduled).
    pub fn end_frame(&self) -> Option<u64> {
        self.end_time.map(time_to_frame)
    }

    /// `UpdateSchedulingInfo` for the quantum starting at `quantum_start`: `None` when the source
    /// is silent for the whole quantum (the caller writes zeros and evaluates nothing).
    pub fn update(&mut self, quantum_start: u64) -> Option<Playing> {
        let q = QUANTUM as u64;
        let quantum_end = quantum_start + q;
        let start_frame = self.start_frame();
        let end_frame = self.end_frame();
        if let Some(end) = end_frame
            && end <= quantum_start
        {
            self.finish();
        }
        if matches!(
            self.state,
            PlaybackState::Unscheduled | PlaybackState::Finished
        ) || start_frame >= quantum_end
        {
            return None;
        }
        let start_frame_offset = if self.state == PlaybackState::Scheduled {
            self.state = PlaybackState::Playing;
            self.start_time * SR - start_frame as f64
        } else {
            0.0
        };
        let offset = start_frame.saturating_sub(quantum_start).min(q) as usize;
        let mut frames = QUANTUM - offset;
        if frames == 0 {
            return None;
        }
        if let Some(end) = end_frame
            && end >= quantum_start
            && end <= quantum_end
        {
            if end < quantum_end {
                let zero_start = (end - quantum_start) as usize;
                let to_zero = QUANTUM - zero_start;
                frames = frames.saturating_sub(to_zero);
            }
            self.finish();
        }
        if frames == 0 {
            return None;
        }
        Some(Playing {
            offset,
            frames,
            start_frame_offset,
        })
    }

    fn finish(&mut self) {
        self.state = PlaybackState::Finished;
    }

    #[cfg(test)]
    pub fn finished(&self) -> bool {
        self.state == PlaybackState::Finished
    }
}

/// `std::exp2(detune / 1200)` in float (`DetuneToFrequencyMultiplier`).
pub fn detune_to_multiplier(detune: f32) -> f32 {
    (detune / 1200.0).exp2()
}

/// `ClampFrequency`: NaN to +Nyquist, else clamped to `[-nyquist, nyquist]`.
pub fn clamp_frequency(f: f32) -> f32 {
    if f.is_nan() {
        NYQUIST
    } else {
        f.clamp(-NYQUIST, NYQUIST)
    }
}

/// `virtual_index - floor(virtual_index / size) * size` in double.
pub fn wrap(v: f64) -> f64 {
    v - (v * INV_SIZE).floor() * SIZE_F64
}

/// `WrapVirtualIndexVector` on one lane: float, truncate-then-correct floor.
fn wrap_f32(x: f32) -> f32 {
    let size = TABLE_SIZE as f32;
    let inv = INV_SIZE as f32;
    let r = x * inv;
    let mut f = r as i32;
    if r < f as f32 {
        f -= 1;
    }
    x - (f as f32) * size
}

/// `DoInterpolation`: one sample at `vri` with `incr = |phase increment|`.
pub fn do_interpolation(vri: f64, incr: f32, factor: f32, lower: &[f32], higher: &[f32]) -> f32 {
    let mut sample_lower = 0.0f64;
    let mut sample_higher = 0.0f64;
    let read_index_0 = vri as u32;
    if incr >= INTERPOLATE_2POINT {
        let read_index_2 = read_index_0.wrapping_add(1) & MASK;
        let read_index_0 = read_index_0 & MASK;
        let s1_lower = lower[read_index_0 as usize];
        let s2_lower = lower[read_index_2 as usize];
        let s1_higher = higher[read_index_0 as usize];
        let s2_higher = higher[read_index_2 as usize];
        let f = f64::from((vri as f32) - read_index_0 as f32);
        sample_higher = (1.0 - f) * f64::from(s1_higher) + f * f64::from(s2_higher);
        sample_lower = (1.0 - f) * f64::from(s1_lower) + f * f64::from(s2_lower);
    } else if incr >= INTERPOLATE_3POINT {
        let t = vri - f64::from(read_index_0);
        let a = [0.5 * t * (t - 1.0), 1.0 - t * t, 0.5 * t * (t + 1.0)];
        for (k, a) in a.iter().enumerate() {
            let i = read_index_0.wrapping_add_signed(k as i32 - 1) & MASK;
            sample_lower += a * f64::from(lower[i as usize]);
            sample_higher += a * f64::from(higher[i as usize]);
        }
    } else {
        let t = vri - f64::from(read_index_0);
        let t2 = t * t;
        let a = [
            t * (t2 - 1.0) * (t - 2.0) / 24.0,
            -t * (t - 1.0) * (t2 - 4.0) / 6.0,
            (t2 - 1.0) * (t2 - 4.0) / 4.0,
            -t * (t + 1.0) * (t2 - 4.0) / 6.0,
            t * (t2 - 1.0) * (t + 2.0) / 24.0,
        ];
        for (k, a) in a.iter().enumerate() {
            let i = read_index_0.wrapping_add_signed(k as i32 - 2) & MASK;
            sample_lower += a * f64::from(lower[i as usize]);
            sample_higher += a * f64::from(higher[i as usize]);
        }
    }
    (f64::from(1.0 - factor) * sample_higher + f64::from(factor) * sample_lower) as f32
}

/// The linear float blend of the vector kernels and `ProcessKRateScalar`.
fn blend(v: f32, r0: u32, r1: u32, factor: f32, lower: &[f32], higher: &[f32]) -> f32 {
    let f = v - r0 as f32;
    let (s1h, s2h) = (higher[r0 as usize], higher[r1 as usize]);
    let (s1l, s2l) = (lower[r0 as usize], lower[r1 as usize]);
    let sh = s1h + f * (s2h - s1h);
    let sl = s1l + f * (s2l - s1l);
    sh + factor * (sl - sh)
}

/// An `OscillatorNode` with its `frequency` and `detune` params.
#[derive(Debug, Clone)]
pub struct Oscillator {
    wave: Rc<PeriodicWave>,
    /// `frequency` (a-rate, default 440, range +-Nyquist).
    pub frequency: Param,
    /// `detune` in cents (a-rate, default 0, range +-[`DETUNE_LIMIT`]).
    pub detune: Param,
    schedule: Schedule,
    virtual_read_index: f64,
    /// Per-quantum scratch, reused: the pulled `frequency` and `detune` inputs and their first
    /// frames.
    freq_bufs: Vec<[f32; QUANTUM]>,
    det_bufs: Vec<[f32; QUANTUM]>,
    firsts: Vec<f32>,
}

impl Oscillator {
    pub fn new(wave: Rc<PeriodicWave>) -> Oscillator {
        Oscillator {
            wave,
            frequency: Param::new(440.0, -NYQUIST, NYQUIST, Rate::A),
            detune: Param::new(0.0, -DETUNE_LIMIT, DETUNE_LIMIT, Rate::A),
            schedule: Schedule::new(),
            virtual_read_index: 0.0,
            freq_bufs: Vec::new(),
            det_bufs: Vec::new(),
            firsts: Vec::new(),
        }
    }

    #[cfg(test)]
    pub fn wave(&self) -> &PeriodicWave {
        &self.wave
    }

    #[cfg(test)]
    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    /// `start(when)` at context frame `now`.
    pub fn start(&mut self, when: f64, now: u64) -> Result<(), ScheduleError> {
        self.schedule.start(when, now)
    }

    /// `stop(when)`.
    pub fn stop(&mut self, when: f64) -> Result<(), ScheduleError> {
        self.schedule.stop(when)
    }

    /// The phase (virtual read index into the 4096-sample table) after the last quantum.
    #[cfg(test)]
    pub fn phase(&self) -> f64 {
        self.virtual_read_index
    }

    /// `OscillatorHandler::Process` for the quantum starting at `quantum_start` into `out`
    /// (overwritten; zero where the source is silent). `frequency_input` / `detune_input` are
    /// the params' connections (FM, vibrato), pulled only when the param is evaluated. Returns
    /// whether the output bus carries signal (`false`: the source is not playing in this
    /// quantum and its bus is zeroed, which marks it silent).
    pub fn process(
        &mut self,
        quantum_start: u64,
        out: &mut [f32; QUANTUM],
        frequency_input: Option<Input>,
        detune_input: Option<Input>,
    ) -> bool {
        out.fill(0.0);
        let Some(playing) = self.schedule.update(quantum_start) else {
            return false;
        };
        let mut increments = [0.0f32; QUANTUM];
        let sample_accurate = self.phase_increments(
            quantum_start,
            &mut increments,
            frequency_input,
            detune_input,
        );
        let mut frequency = 0.0f32;
        if !sample_accurate {
            frequency = self.k_rate_frequency(quantum_start);
        }
        let mut dest = playing.offset;
        let mut n = playing.frames;
        let mut vri = self.virtual_read_index;
        let off = playing.start_frame_offset;
        if off > 0.0 {
            dest += 1;
            n -= 1;
            vri += (1.0 - off) * f64::from(frequency) * f64::from(RATE_SCALE);
        } else if off < 0.0 {
            vri = -off * f64::from(frequency) * f64::from(RATE_SCALE);
        }
        let dst = &mut out[dest..dest + n];
        self.virtual_read_index = if sample_accurate {
            self.process_a_rate(dst, vri, &increments)
        } else {
            self.process_k_rate(quantum_start, dst, vri)
        };
        true
    }

    /// `FinalValue(frequency) * DetuneToFrequencyMultiplier(FinalValue(detune))`, clamped.
    fn k_rate_frequency(&mut self, quantum_start: u64) -> f32 {
        let mut frequency = self.frequency.final_value(quantum_start, &[]);
        let detune = self.detune.final_value(quantum_start, &[]);
        frequency *= detune_to_multiplier(detune);
        clamp_frequency(frequency)
    }

    /// `CalculateSampleAccuratePhaseIncrements`: whether the quantum takes the a-rate path, with
    /// the per-frame table increments in `increments` when it does.
    fn phase_increments(
        &mut self,
        quantum_start: u64,
        increments: &mut [f32; QUANTUM],
        frequency_input: Option<Input>,
        detune_input: Option<Input>,
    ) -> bool {
        let mut sample_accurate = false;
        let mut frequency_changes = false;
        let mut final_scale = RATE_SCALE;
        self.freq_bufs.clear();
        let freq_connected = frequency_input.is_some();
        if let Some(pull) = frequency_input {
            pull(quantum_start, &mut self.freq_bufs);
        }
        if self
            .frequency
            .has_sample_accurate_values(quantum_start, freq_connected)
            && self.frequency.rate() == Rate::A
        {
            sample_accurate = true;
            frequency_changes = true;
            self.frequency.fill_quantum_connected(
                quantum_start,
                increments,
                &self.freq_bufs,
                freq_connected,
            );
        } else {
            self.firsts.clear();
            self.firsts.extend(self.freq_bufs.iter().map(|b| b[0]));
            final_scale *=
                self.frequency
                    .final_value_connected(quantum_start, &self.firsts, freq_connected);
        }
        self.det_bufs.clear();
        let det_connected = detune_input.is_some();
        if let Some(pull) = detune_input {
            pull(quantum_start, &mut self.det_bufs);
        }
        if self
            .detune
            .has_sample_accurate_values(quantum_start, det_connected)
            && self.detune.rate() == Rate::A
        {
            sample_accurate = true;
            let mut detune_values = [0.0f32; QUANTUM];
            let k = (1.0f64 / 1200.0) as f32;
            let values: &mut [f32; QUANTUM] = if frequency_changes {
                &mut detune_values
            } else {
                increments
            };
            self.detune.fill_quantum_connected(
                quantum_start,
                values,
                &self.det_bufs,
                det_connected,
            );
            for v in values.iter_mut() {
                *v = (*v * k).exp2();
            }
            if frequency_changes {
                for (inc, d) in increments.iter_mut().zip(detune_values.iter()) {
                    *inc *= *d;
                }
            }
        } else {
            self.firsts.clear();
            self.firsts.extend(self.det_bufs.iter().map(|b| b[0]));
            final_scale *= detune_to_multiplier(self.detune.final_value_connected(
                quantum_start,
                &self.firsts,
                det_connected,
            ));
        }
        if sample_accurate {
            for inc in increments.iter_mut() {
                *inc = clamp_frequency(*inc) * final_scale;
            }
        }
        sample_accurate
    }

    /// `ProcessKRate`: one frequency for the quantum.
    fn process_k_rate(&mut self, quantum_start: u64, dst: &mut [f32], mut vri: f64) -> f64 {
        let frequency = self.k_rate_frequency(quantum_start);
        let wave = self.wave.clone();
        let (lower, higher, factor) = wave.wave_data(frequency);
        let incr = frequency * RATE_SCALE;
        let n = dst.len();
        if incr >= INTERPOLATE_2POINT {
            let (k, v) = k_rate_vector(dst, vri, frequency, &wave);
            if k < n {
                k_rate_scalar(&mut dst[k..], v, frequency, &wave);
            }
            vri += f64::from(n as f32 * incr);
            wrap(vri)
        } else {
            for d in dst.iter_mut() {
                *d = do_interpolation(vri, incr.abs(), factor, lower, higher);
                vri += f64::from(incr);
                vri = wrap(vri);
            }
            vri
        }
    }

    /// `ProcessARate`: groups of four (the NEON kernel when all four increments are large, else
    /// `DoInterpolation` per frame), then the scalar tail. `increments` is read from its start.
    fn process_a_rate(&self, dst: &mut [f32], mut vri: f64, increments: &[f32; QUANTUM]) -> f64 {
        let n = dst.len();
        let inv_rate_scale = 1.0f32 / RATE_SCALE;
        let mut k = 0;
        for _ in 0..n / 4 {
            let mut big = true;
            let mut frequency = [0.0f32; 4];
            for m in 0..4 {
                let inc = increments[k + m];
                big = big && inc.abs() >= INTERPOLATE_2POINT;
                frequency[m] = inv_rate_scale * inc;
            }
            let (lower, higher, factor) = self.wave.wave_data4(frequency);
            if big {
                vri = a_rate_vector_kernel(
                    &mut dst[k..k + 4],
                    vri,
                    &increments[k..k + 4],
                    &lower,
                    &higher,
                    &factor,
                );
            } else {
                for m in 0..4 {
                    let inc = increments[k + m];
                    dst[k + m] = do_interpolation(vri, inc.abs(), factor[m], lower[m], higher[m]);
                    vri += f64::from(inc);
                    vri = wrap(vri);
                }
            }
            k += 4;
        }
        for m in k..n {
            let inc = increments[m];
            let (lower, higher, factor) = self.wave.wave_data(inv_rate_scale * inc);
            dst[m] = do_interpolation(vri, inc.abs(), factor, lower, higher);
            vri += f64::from(inc);
            vri = wrap(vri);
        }
        vri
    }
}

/// `ProcessKRateVector` (NEON): float lanes for `4 * floor(n / 4)` frames; returns the frames
/// done and the double index advanced by `k * incr`.
fn k_rate_vector(dst: &mut [f32], vri: f64, frequency: f32, wave: &PeriodicWave) -> (usize, f64) {
    let incr = frequency * RATE_SCALE;
    let (lower, higher, factor) = wave.wave_data(frequency);
    let v_incr = 4.0 * incr;
    let base = vri as f32;
    let mut v = [
        wrap_f32(base + 0.0 * incr),
        wrap_f32(base + 1.0 * incr),
        wrap_f32(base + 2.0 * incr),
        wrap_f32(base + 3.0 * incr),
    ];
    let n = dst.len();
    let mut k = 0;
    for _ in 0..n / 4 {
        for m in 0..4 {
            let r0 = (v[m] as u32) & MASK;
            let r1 = (r0 + 1) & MASK;
            dst[k + m] = blend(v[m], r0, r1, factor, lower, higher);
        }
        for x in v.iter_mut() {
            *x = wrap_f32(*x + v_incr);
        }
        k += 4;
    }
    let vri = wrap(vri + f64::from(k as f32 * incr));
    (k, vri)
}

/// `ProcessKRateScalar`: the linear float path for the frames the vector kernel left.
fn k_rate_scalar(dst: &mut [f32], mut vri: f64, frequency: f32, wave: &PeriodicWave) -> f64 {
    let (lower, higher, factor) = wave.wave_data(frequency);
    let incr = frequency * RATE_SCALE;
    for d in dst.iter_mut() {
        let r0 = (vri as u32) & MASK;
        let r1 = (r0 + 1) & MASK;
        *d = blend(vri as f32, r0, r1, factor, lower, higher);
        vri += f64::from(incr);
        vri = wrap(vri);
    }
    vri
}

/// `ProcessARateVectorKernel` (NEON): four frames with double start indices converted to float.
fn a_rate_vector_kernel(
    dst: &mut [f32],
    vri: f64,
    increments: &[f32],
    lower: &[&[f32]; 4],
    higher: &[&[f32]; 4],
    factor: &[f32; 4],
) -> f64 {
    let mut sum = [0.0f64; 4];
    sum[0] = f64::from(increments[0]);
    for m in 1..4 {
        sum[m] = sum[m - 1] + f64::from(increments[m]);
    }
    let starts = [
        wrap(vri),
        wrap(vri + sum[0]),
        wrap(vri + sum[1]),
        wrap(vri + sum[2]),
    ];
    for m in 0..4 {
        let v = wrap_f32(starts[m] as f32);
        let r0 = (v as u32) & MASK;
        let r1 = (r0 + 1) & MASK;
        dst[m] = blend(v, r0, r1, factor[m], lower[m], higher[m]);
    }
    wrap(vri + sum[3])
}

/// A `ConstantSourceNode` whose `offset` has no automation (superdough's `one`).
#[derive(Debug, Clone, PartialEq)]
pub struct ConstantSource {
    offset: f32,
    schedule: Schedule,
}

impl ConstantSource {
    pub fn new(offset: f32) -> ConstantSource {
        ConstantSource {
            offset,
            schedule: Schedule::new(),
        }
    }

    pub fn start(&mut self, when: f64, now: u64) -> Result<(), ScheduleError> {
        self.schedule.start(when, now)
    }

    pub fn stop(&mut self, when: f64) -> Result<(), ScheduleError> {
        self.schedule.stop(when)
    }

    /// The `offset` (constant: superdough never automates it).
    pub fn offset(&self) -> f32 {
        self.offset
    }

    #[cfg(test)]
    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    /// `ConstantSourceHandler::Process`: `offset` from the first playing frame (no sub-sample
    /// start), zero elsewhere. Returns whether the output bus carries signal (the source plays in
    /// this quantum).
    pub fn process(&mut self, quantum_start: u64, out: &mut [f32; QUANTUM]) -> bool {
        out.fill(0.0);
        let Some(p) = self.schedule.update(quantum_start) else {
            return false;
        };
        if self.offset != 0.0 {
            out[p.offset..p.offset + p.frames].fill(self.offset);
        }
        true
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;
    use crate::periodic::{WaveKind, WaveTables};

    fn sine() -> Rc<PeriodicWave> {
        WaveTables::new().builtin(WaveKind::Sine).unwrap()
    }

    fn wave(kind: WaveKind) -> Rc<PeriodicWave> {
        WaveTables::new().builtin(kind).unwrap()
    }

    /// An oscillator whose `.value = f` was written at `now` and started at `t`.
    fn osc(kind: WaveKind, f: f32, now: u64, t: f64) -> Oscillator {
        let mut o = Oscillator::new(wave(kind));
        o.frequency.set_value(f, now).unwrap();
        o.start(t, now).unwrap();
        o
    }

    fn run(o: &mut Oscillator, from: u64, quanta: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for i in 0..quanta {
            let mut buf = [0.0; QUANTUM];
            o.process(from + (i * QUANTUM) as u64, &mut buf, None, None);
            out.extend_from_slice(&buf);
        }
        out
    }

    #[test]
    fn constants() {
        assert_eq!(DETUNE_LIMIT, 1200.0f32 * f32::MAX.log2());
        assert_eq!(NYQUIST, (44100.0f32) / 2.0);
        assert_eq!(crate::periodic::at(&[1.0; 4096], 4097), 1.0);
    }

    #[test]
    fn schedule_rules() {
        let mut s = Schedule::new();
        assert_eq!(s.state(), PlaybackState::Unscheduled);
        assert!(s.update(0).is_none());
        assert!(matches!(s.stop(1.0), Err(ScheduleError::State(..))));
        assert!(matches!(
            s.start(f64::NAN, 0),
            Err(ScheduleError::NonFinite(_))
        ));
        assert!(matches!(s.start(-1.0, 0), Err(ScheduleError::Negative(..))));
        // start(when) before currentTime starts now.
        s.start(0.001, 256).unwrap();
        assert_eq!(s.start_frame(), 256);
        assert!(matches!(s.start(1.0, 0), Err(ScheduleError::State(..))));
        assert!(s.update(128).is_none());
        let p = s.update(256).unwrap();
        assert_eq!((p.offset, p.frames, p.start_frame_offset), (0, 128, 0.0));
        assert_eq!(s.state(), PlaybackState::Playing);
        // Later stops win; negative and non-finite stops throw.
        s.stop(10.0).unwrap();
        s.stop(600.0 / SR).unwrap();
        assert!(matches!(
            s.stop(f64::INFINITY),
            Err(ScheduleError::NonFinite(_))
        ));
        assert!(matches!(s.stop(-0.5), Err(ScheduleError::Negative(..))));
        assert_eq!(s.end_frame(), Some(600));
        let p = s.update(512).unwrap();
        assert_eq!((p.offset, p.frames), (0, 88));
        assert!(s.finished());
        assert!(s.update(640).is_none());
    }

    #[test]
    fn schedule_start_offsets() {
        // A start between frames: the next frame, offset in (-1, 0).
        let mut s = Schedule::new();
        s.start(1000.25 / SR, 0).unwrap();
        assert_eq!(s.start_frame(), 1001);
        assert!(s.update(768).is_none());
        let p = s.update(896).unwrap();
        assert_eq!(p.offset, 1001 - 896);
        assert_eq!(p.frames, 128 - (1001 - 896));
        assert!((p.start_frame_offset + 0.75).abs() < 1e-9);
        // The offset is only reported in the starting quantum.
        assert_eq!(s.update(1024).unwrap().start_frame_offset, 0.0);
        // A start 1/4096 frame past a frame rounds to that frame (1/1024 rounding): offset > 0.
        let mut s = Schedule::new();
        let t = (300.0 + 1.0 / 4096.0) / SR;
        s.start(t, 0).unwrap();
        assert_eq!(s.start_frame(), 300);
        let p = s.update(256).unwrap();
        assert!(p.start_frame_offset > 0.0 && p.start_frame_offset < 1e-3);
        // Stop before start: silent and finished.
        let mut s = Schedule::new();
        s.start(200.0 / SR, 0).unwrap();
        s.stop(100.0 / SR).unwrap();
        assert!(s.update(128).is_none());
        assert!(s.finished());
        // Start and stop in one quantum.
        let mut s = Schedule::new();
        s.start(130.0 / SR, 0).unwrap();
        s.stop(140.0 / SR).unwrap();
        let p = s.update(128).unwrap();
        assert_eq!((p.offset, p.frames), (2, 10));
        // A stop exactly on the quantum end finishes without trimming.
        let mut s = Schedule::new();
        s.start(0.0, 0).unwrap();
        s.stop(256.0 / SR).unwrap();
        assert_eq!(s.update(128).unwrap().frames, 128);
        assert!(s.finished());
    }

    #[test]
    fn helpers() {
        assert_eq!(clamp_frequency(f32::NAN), NYQUIST);
        assert_eq!(clamp_frequency(1e9), NYQUIST);
        assert_eq!(clamp_frequency(-1e9), -NYQUIST);
        assert_eq!(clamp_frequency(440.0), 440.0);
        assert_eq!(detune_to_multiplier(1200.0), 2.0);
        assert_eq!(detune_to_multiplier(-2400.0), 0.25);
        assert_eq!(detune_to_multiplier(0.0), 1.0);
        assert_eq!(wrap(4096.0), 0.0);
        assert_eq!(wrap(-1.0), 4095.0);
        assert_eq!(wrap(8192.5), 0.5);
        assert_eq!(wrap_f32(4096.0), 0.0);
        assert_eq!(wrap_f32(-1.0), 4095.0);
        assert_eq!(wrap_f32(5000.0), 904.0);
        assert_eq!(wrap_f32(4095.5), 4095.5);
    }

    /// The 5-point Lagrange formula of `DoInterpolation`, written out.
    fn five_point(vri: f64, f: f32, lo: &[f32], hi: &[f32]) -> f32 {
        let i0 = vri as u32;
        let t = vri - f64::from(i0);
        let t2 = t * t;
        let a = [
            t * (t2 - 1.0) * (t - 2.0) / 24.0,
            -t * (t - 1.0) * (t2 - 4.0) / 6.0,
            (t2 - 1.0) * (t2 - 4.0) / 4.0,
            -t * (t + 1.0) * (t2 - 4.0) / 6.0,
            t * (t2 - 1.0) * (t + 2.0) / 24.0,
        ];
        let idx = |k: usize| ((i0 as i64 + k as i64 - 2).rem_euclid(4096)) as usize;
        let h: f64 = (0..5).map(|k| a[k] * f64::from(hi[idx(k)])).sum();
        let l: f64 = (0..5).map(|k| a[k] * f64::from(lo[idx(k)])).sum();
        (f64::from(1.0 - f) * h + f64::from(f) * l) as f32
    }

    #[test]
    fn interpolation_bands_at_their_boundaries() {
        let w = wave(WaveKind::Sawtooth);
        let (lo, hi, f) = w.wave_data(1000.0);
        let vri = 100.37;
        // Linear (>= 0.3): the float factor and double blend.
        let lin = do_interpolation(vri, INTERPOLATE_2POINT, f, lo, hi);
        let fr = f64::from((vri as f32) - 100.0);
        let h = (1.0 - fr) * f64::from(hi[100]) + fr * f64::from(hi[101]);
        let l = (1.0 - fr) * f64::from(lo[100]) + fr * f64::from(lo[101]);
        assert_eq!(lin, (f64::from(1.0 - f) * h + f64::from(f) * l) as f32);
        // Just below 0.3 and at 0.16: 3-point Lagrange.
        let t = vri - 100.0;
        let a = [0.5 * t * (t - 1.0), 1.0 - t * t, 0.5 * t * (t + 1.0)];
        let h3: f64 = (0..3).map(|k| a[k] * f64::from(hi[99 + k])).sum();
        let l3: f64 = (0..3).map(|k| a[k] * f64::from(lo[99 + k])).sum();
        let three = (f64::from(1.0 - f) * h3 + f64::from(f) * l3) as f32;
        assert_eq!(do_interpolation(vri, 0.2999, f, lo, hi), three);
        assert_eq!(do_interpolation(vri, INTERPOLATE_3POINT, f, lo, hi), three);
        // Just below 0.16: 5-point.
        assert_eq!(
            do_interpolation(vri, 0.1599, f, lo, hi),
            five_point(vri, f, lo, hi)
        );
        // In a steep region the three bands differ.
        let steep = 2048.4;
        let (lo, hi, f) = w.wave_data(20.0);
        let b2 = do_interpolation(steep, 0.5, f, lo, hi);
        let b3 = do_interpolation(steep, 0.2, f, lo, hi);
        let b5 = do_interpolation(steep, 0.1, f, lo, hi);
        assert!(b2 != b3 && b3 != b5, "{b2} {b3} {b5}");
        // On a table point every band returns the table value (Lagrange passes through points).
        for incr in [1.0, 0.2, 0.01] {
            let v = do_interpolation(7.0, incr, 0.0, hi, hi);
            assert!((v - hi[7]).abs() < 1e-6, "{incr}");
        }
        // Indices wrap at the table ends.
        for vri in [4095.5, 0.5] {
            assert_eq!(
                do_interpolation(vri, 0.01, f, lo, hi),
                five_point(vri, f, lo, hi)
            );
        }
    }

    #[test]
    fn k_rate_sine_follows_the_table() {
        // 440 Hz started on frame 0 in the pause quantum: a-rate for quantum 0 (the .value event
        // is at its start), k-rate after.
        let mut o = osc(WaveKind::Sine, 440.0, 0, 0.0);
        let out = run(&mut o, 0, 40);
        for (i, x) in out.iter().enumerate() {
            let want = (2.0 * std::f64::consts::PI * 440.0 * i as f64 / SR).sin();
            assert!((f64::from(*x) - want).abs() < 2e-4, "{i}: {x} vs {want}");
        }
        // The phase after n frames is n * incr (wrapped), recomputed per quantum.
        // Quantum 0 is a-rate: the NEON kernel advances by the double sum of four increments.
        let incr = 440.0f32 * RATE_SCALE;
        let mut v = 0.0f64;
        for _ in 0..32 {
            let i = f64::from(incr);
            v = wrap(v + (((i + i) + i) + i));
        }
        for _ in 1..40 {
            v = wrap(v + f64::from(128.0f32 * incr));
        }
        assert_eq!(o.phase(), v);
    }

    #[test]
    fn first_quantum_path_choice() {
        let w = sine();
        // `.value` at frame 0, start in quantum 0: a-rate (event at the quantum start).
        let mut a = Oscillator::new(w.clone());
        a.frequency.set_value(440.0, 0).unwrap();
        assert!(a.frequency.has_sample_accurate_values(0, false));
        // Same oscillator in a later quantum: k-rate.
        assert!(!a.frequency.has_sample_accurate_values(128, false));
        // Detune with inputs is always sample accurate.
        assert!(a.detune.has_sample_accurate_values(1280, true));
    }

    #[test]
    fn sub_sample_start_k_rate_negative_offset() {
        // `.value` at frame 0 (pause), start at 1000.25 frames: k-rate first quantum; the first
        // frame (1001) has phase -off * incr = 0.75 * incr.
        let t = 1000.25 / SR;
        let mut o = osc(WaveKind::Sine, 1000.0, 0, t);
        let out = run(&mut o, 0, 10);
        assert!(out[..1001].iter().all(|x| *x == 0.0));
        let incr = 1000.0f32 * RATE_SCALE;
        let v0 = 0.75 * f64::from(1000.0f32) * f64::from(RATE_SCALE);
        let w = sine();
        let (lo, hi, f) = w.wave_data(1000.0);
        let r0 = (v0 as f32) as u32;
        assert_eq!(out[1001], blend(v0 as f32, r0, r0 + 1, f, lo, hi));
        // Matches the analytic sine at the true onset within table error.
        for i in 1001..1280 {
            let tt = (i as f64 - 1000.25) / SR;
            let want = (2.0 * std::f64::consts::PI * 1000.0 * tt).sin();
            assert!((f64::from(out[i]) - want).abs() < 5e-4, "{i}");
        }
        let _ = incr;
    }

    #[test]
    fn sub_sample_start_positive_offset_skips_a_frame() {
        let t = (300.0 + 1.0 / 4096.0) / SR;
        let mut o = osc(WaveKind::Sine, 1000.0, 0, t);
        let out = run(&mut o, 0, 4);
        assert_eq!(out[300], 0.0);
        let off = t * SR - 300.0;
        let v = (1.0 - off) * f64::from(1000.0f32) * f64::from(RATE_SCALE);
        let w = sine();
        let (lo, hi, f) = w.wave_data(1000.0);
        let r0 = (v as f32) as u32;
        assert_eq!(out[301], blend(v as f32, r0, r0 + 1, f, lo, hi));
    }

    #[test]
    fn sub_sample_start_zero_offset() {
        let mut o = osc(WaveKind::Sine, 1000.0, 0, 640.0 / SR);
        let out = run(&mut o, 0, 7);
        assert!(out[..640].iter().all(|x| *x == 0.0));
        assert_eq!(out[640], 0.0); // sine at phase 0
        assert!(out[641] > 0.0);
    }

    #[test]
    fn a_rate_first_quantum_starts_at_phase_zero() {
        // `.value` at frame 0 and start at 28.665 frames, both in quantum 0: the a-rate path
        // leaves the local frequency 0, so the start phase is 0 despite the offset.
        let t = 28.665 / SR;
        let mut o = osc(WaveKind::Sine, 440.0, 0, t);
        let out = run(&mut o, 0, 2);
        assert!(out[..29].iter().all(|x| *x == 0.0));
        assert!(out[29].abs() < 1e-6, "{}", out[29]);
        // The same start one quantum later (k-rate) has the sub-sample phase 0.335 * incr.
        let mut k = osc(WaveKind::Sine, 440.0, 0, (128.0 + 28.665) / SR);
        let out = run(&mut k, 0, 2);
        let phase = 0.335 * 440.0 / SR;
        let want = (2.0 * std::f64::consts::PI * phase).sin();
        assert!(
            (f64::from(out[157]) - want).abs() < 1e-4,
            "{} vs {want}",
            out[157]
        );
    }

    #[test]
    fn a_rate_increments_are_read_from_the_quantum_start() {
        // A detune ramp makes the quantum a-rate; the start at offset 64 reads increments
        // 0.. for its frames 64.. (Blink passes the whole buffer).
        let w = sine();
        let mut o = Oscillator::new(w.clone());
        o.frequency.set_value(1000.0, 0).unwrap();
        o.detune.set_value_at_time(0.0, 0.0, 0).unwrap();
        o.detune.linear_ramp_to(1200.0, 128.0 / SR, 0).unwrap();
        o.start(64.0 / SR, 0).unwrap();
        let mut out = [0.0; QUANTUM];
        o.process(0, &mut out, None, None);
        // Increment for frame 0 of the quantum (detune 0) is used for output frame 64.
        let inc0 = 1000.0f32 * RATE_SCALE;
        let w2 = sine();
        let (lo, hi, f) = w2.wave_data4([1000.0; 4]);
        assert_eq!(out[64], blend(0.0, 0, 1, f[0], lo[0], hi[0]));
        let v1 = wrap_f32(inc0);
        let r0 = v1 as u32;
        assert_eq!(out[65], blend(v1, r0, r0 + 1, f[1], lo[1], hi[1]));
    }

    #[test]
    fn detune_input_is_summed_and_changes_pitch() {
        let w = sine();
        let mut o = Oscillator::new(w.clone());
        o.frequency.set_value(440.0, 0).unwrap();
        o.start(0.0, 0).unwrap();
        // +1200 cents from an input: one octave up.
        let mut pull = |_q: u64, bufs: &mut Vec<[f32; QUANTUM]>| bufs.push([1200.0; QUANTUM]);
        let mut out = Vec::new();
        for q in 0..20u64 {
            let mut buf = [0.0; QUANTUM];
            o.process(q * 128, &mut buf, None, Some(&mut pull));
            out.extend_from_slice(&buf);
        }
        for (i, x) in out.iter().enumerate() {
            let want = (2.0 * std::f64::consts::PI * 880.0 * i as f64 / SR).sin();
            assert!((f64::from(*x) - want).abs() < 5e-4, "{i}");
        }
        assert_eq!(o.detune.value(), 0.0);
    }

    #[test]
    fn frequency_input_is_summed() {
        let w = sine();
        let mut o = Oscillator::new(w.clone());
        o.frequency.set_value(400.0, 0).unwrap();
        o.start(0.0, 0).unwrap();
        let mut calls = 0;
        let mut pull = |_q: u64, bufs: &mut Vec<[f32; QUANTUM]>| {
            calls += 1;
            bufs.push([30.0; QUANTUM]);
            bufs.push([10.0; QUANTUM]);
        };
        let mut out = Vec::new();
        for q in 0..20u64 {
            let mut buf = [0.0; QUANTUM];
            o.process(q * 128, &mut buf, Some(&mut pull), None);
            out.extend_from_slice(&buf);
        }
        assert_eq!(calls, 20);
        for (i, x) in out.iter().enumerate() {
            let want = (2.0 * std::f64::consts::PI * 440.0 * i as f64 / SR).sin();
            assert!((f64::from(*x) - want).abs() < 5e-4, "{i}");
        }
    }

    #[test]
    fn inputs_are_not_pulled_while_silent() {
        let w = sine();
        let mut o = Oscillator::new(w);
        o.frequency.set_value(400.0, 0).unwrap();
        o.start(1000.0 / SR, 0).unwrap();
        o.stop(1100.0 / SR).unwrap();
        let mut calls = 0;
        let mut pull = |_q: u64, bufs: &mut Vec<[f32; QUANTUM]>| {
            calls += 1;
            bufs.push([0.0; QUANTUM]);
        };
        for q in 0..20u64 {
            let mut buf = [0.0; QUANTUM];
            o.process(q * 128, &mut buf, None, Some(&mut pull));
        }
        // Only quanta 896 and 1024 play.
        assert_eq!(calls, 2);
    }

    #[test]
    fn nyquist_clamp_and_negative_frequency() {
        // A frequency above Nyquist is clamped by the param range; detune can push the k-rate
        // frequency over, and ClampFrequency holds it at Nyquist (incr 2048: every other sample).
        let mut o = osc(WaveKind::Sine, 20000.0, 0, 0.0);
        o.detune.set_value(2400.0, 0).unwrap();
        let out = run(&mut o, 0, 4);
        assert!(out.iter().all(|x| x.is_finite()));
        // Range 35 (no partials) at Nyquist: silence.
        assert!(
            out[200..].iter().all(|x| x.abs() < 1e-6),
            "{:?}",
            &out[200..210]
        );
        // Negative frequencies play the wave backwards (same table, decreasing phase).
        let mut pos = osc(WaveKind::Sawtooth, 100.0, 0, 0.0);
        let mut neg = osc(WaveKind::Sawtooth, -100.0, 0, 0.0);
        let p = run(&mut pos, 0, 8);
        let n = run(&mut neg, 0, 8);
        for i in 1..1000 {
            assert!((p[i] + n[i]).abs() < 2e-3, "{i}: {} {}", p[i], n[i]);
        }
    }

    #[test]
    fn zero_frequency_is_silent_for_odd_waves() {
        let mut o = osc(WaveKind::Sine, 0.0, 0, 0.0);
        let out = run(&mut o, 0, 3);
        assert!(out.iter().all(|x| x.abs() < 1e-6));
        assert_eq!(o.phase(), 0.0);
    }

    #[test]
    fn low_frequencies_use_lagrange_paths() {
        // 2.5 Hz -> incr 0.232 (3-point), 1 Hz -> 0.093 (5-point): both smooth and accurate.
        for f in [2.5f32, 1.0] {
            let mut o = osc(WaveKind::Sine, f, 0, 0.0);
            let out = run(&mut o, 0, 30);
            for (i, x) in out.iter().enumerate() {
                let want = (2.0 * std::f64::consts::PI * f64::from(f) * i as f64 / SR).sin();
                assert!((f64::from(*x) - want).abs() < 1e-5, "{f} {i}");
            }
        }
    }

    #[test]
    fn stops_at_the_end_frame() {
        let mut o = osc(WaveKind::Square, 440.0, 0, 0.0);
        o.stop(300.0 / SR).unwrap();
        let out = run(&mut o, 0, 4);
        assert!(out[1..300].iter().any(|x| *x != 0.0));
        assert!(out[300..].iter().all(|x| *x == 0.0));
        assert!(o.schedule().finished());
    }

    #[test]
    fn a_rate_paths_match_k_rate_for_a_constant_frequency() {
        // A frequency with a far-future ramp end keeps the quantum a-rate with constant values:
        // the a-rate kernels must agree with k-rate within float rounding.
        let w = wave(WaveKind::Triangle);
        let mut a = Oscillator::new(w.clone());
        a.frequency.set_value(523.25, 0).unwrap();
        a.frequency.set_value_at_time(523.25, 10.0, 0).unwrap();
        a.start(0.0, 0).unwrap();
        let mut k = Oscillator::new(w);
        k.frequency.set_value(523.25, 0).unwrap();
        k.start(0.0, 0).unwrap();
        let oa = run(&mut a, 0, 20);
        let ok = run(&mut k, 0, 20);
        for i in 0..oa.len() {
            assert!((oa[i] - ok[i]).abs() < 1e-3, "{i}: {} {}", oa[i], ok[i]);
        }
    }

    #[test]
    fn constant_source() {
        let mut c = ConstantSource::new(1.0);
        let mut out = [9.0; QUANTUM];
        c.process(0, &mut out);
        assert!(out.iter().all(|x| *x == 0.0));
        c.start(100.25 / SR, 0).unwrap();
        c.stop(200.0 / SR).unwrap();
        c.process(0, &mut out);
        assert!(out[..101].iter().all(|x| *x == 0.0));
        assert!(out[101..].iter().all(|x| *x == 1.0));
        c.process(128, &mut out);
        assert!(out[..72].iter().all(|x| *x == 1.0));
        assert!(out[72..].iter().all(|x| *x == 0.0));
        assert!(c.schedule().finished());
        let mut z = ConstantSource::new(0.0);
        z.start(0.0, 0).unwrap();
        z.process(0, &mut out);
        assert!(out.iter().all(|x| *x == 0.0));
    }
}
