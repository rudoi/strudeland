// Port of Blink (Chromium 156.0.8078.4) platform/audio/delay.cc, platform/audio/cpu/arm/delay_neon.cc, modules/webaudio/delay_handler.cc, modules/webaudio/delay_node.cc, modules/webaudio/audio_handler.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! The orbit's feedback delay: superdough's `FeedbackDelayNode` (feedbackdelay.mjs) and
//! `Orbit.getDelay` (superdoughoutput.mjs) on Blink's `DelayNode`:
//!
//! ```text
//! sends ─► DelayNode(maxDelayTime 1 s) ─► delayGain(1) ─► the orbit's summingNode
//!          DelayNode ─► feedbackGain(min(|fb|, 0.995)) ─► DelayNode   (a cycle)
//! ```
//!
//! The cycle: Blink's `ProcessIfNecessary` marks a node processed before it pulls its inputs, so
//! the feedback gain, pulled by the delay, reads the delay's output bus as the previous quantum
//! left it. Each loop therefore adds one render quantum (`FEEDBACK_EXTRA_FRAMES`): echo k of an
//! impulse at `t0` lands at `t0 + k*D + (k-1)*128` (measured: `evidence/measure-feedback-echo`).
//!
//! The delay line ([`DelayLine`]) is Blink's `DelayHandler` with one `Delay` kernel per channel,
//! always on the a-rate path (`delayTime` is a-rate and superdough never changes that): the input
//! quantum is written into the circular buffer first, then every frame reads at `write index +
//! buffer length - delay frames` with linear interpolation, in float as the ARM NEON kernel does.
//! It starts with one channel and becomes stereo for good when a stereo send connects: the new
//! kernels keep the write index and both get the mono history (speakers up-mix); the node's output
//! bus is re-created at that moment, so the feedback reads silence for that one quantum.

use crate::QUANTUM;
use crate::SAMPLE_RATE;
use crate::coerce::{js_max, js_min};
use crate::orbit::Input;
use crate::param::{Param, ParamError, ParamInputs, Rate, idl_float};
use crate::voice::GainNode;

use crate::SAMPLE_RATE_F64 as SR;
const SR_F32: f32 = SAMPLE_RATE as f32;

/// `DelayNode`'s default `maxDelayTime` (superdough's `createFeedbackDelay(1, ..)` passes `wet`,
/// not a maximum), which is also its `delayTime` maximum and its tail time.
pub const MAX_DELAY_TIME: f64 = 1.0;
/// `128 + TimeToSampleFrame(maxDelayTime, sr, RoundUp)`.
pub const BUFFER_LENGTH: usize = QUANTUM + SAMPLE_RATE as usize;
/// The frames each loop through the feedback cycle adds to the delay time (one render quantum,
/// from the delay's output being read one quantum late; not a separate delay line).
pub const FEEDBACK_EXTRA_FRAMES: u64 = QUANTUM as u64;
/// `getDelay` clamps the feedback to `[0, 0.98]`.
pub const MAX_FEEDBACK: f64 = 0.98;
/// `FeedbackDelayNode` sets `feedbackGain.gain.value = min(|feedback|, 0.995)`.
pub const MAX_FEEDBACK_GAIN: f64 = 0.995;

/// One channel of the delay line (Blink's `Delay`): the circular buffer and its write index.
#[derive(Debug, Clone, PartialEq)]
pub struct DelayKernel {
    buffer: Vec<f32>,
    write_index: usize,
}

impl Default for DelayKernel {
    fn default() -> Self {
        DelayKernel::new()
    }
}

impl DelayKernel {
    /// A zeroed buffer of [`BUFFER_LENGTH`] frames, writing at 0.
    pub fn new() -> DelayKernel {
        DelayKernel {
            buffer: vec![0.0; BUFFER_LENGTH],
            write_index: 0,
        }
    }

    pub fn buffer(&self) -> &[f32] {
        &self.buffer
    }

    pub fn write_index(&self) -> usize {
        self.write_index
    }

    /// `CopyToCircularBuffer` at the write index.
    fn write(&mut self, input: &[f32; QUANTUM]) {
        let w = self.write_index;
        let first = (BUFFER_LENGTH - w).min(QUANTUM);
        self.buffer[w..w + first].copy_from_slice(&input[..first]);
        if first < QUANTUM {
            self.buffer[..QUANTUM - first].copy_from_slice(&input[first..]);
        }
    }

    /// `Delay::ProcessARate` for one quantum: the input is written first, then each frame `i`
    /// reads at delay `delay_times[i]` seconds as `ProcessARateVector` (delay_neon.cc) does, all in
    /// float: `pos = (float)w + ((float)len - max(dt, 0) * sr)`, wrapped once; `i1 = (int)pos`,
    /// `i2 = i1 + 1` (wrapped), `out = s1 + (pos - i1) * (s2 - s1)`. The scalar tail (`n % 4`
    /// frames) never runs for a whole quantum.
    pub fn process_a_rate(
        &mut self,
        input: &[f32; QUANTUM],
        delay_times: &[f32; QUANTUM],
        out: &mut [f32; QUANTUM],
    ) {
        self.write(input);
        let len = BUFFER_LENGTH;
        let len_f = len as f32;
        let wrap = |i: usize| if i >= len { i - len } else { i };
        let mut w = wrap(self.write_index);
        for (o, dt) in out.iter_mut().zip(delay_times.iter()) {
            // `vmaxq_f32(dt, 0)`: NaN cannot reach here (the param clips its values).
            let dt = if *dt > 0.0 { *dt } else { 0.0 };
            let desired = dt * SR_F32;
            let mut pos = (w as f32) + (len_f - desired);
            if pos >= len_f {
                pos -= len_f;
            }
            // `vcvtq_s32_f32` truncates; `pos` is in `[0, len)`, so the index wrap is defensive.
            let i1 = wrap(pos as usize);
            let i2 = wrap(i1 + 1);
            let frac = pos - i1 as f32;
            let s1 = self.buffer[i1];
            let s2 = self.buffer[i2];
            *o = s1 + frac * (s2 - s1);
            w = wrap(w + 1);
        }
        self.write_index = wrap(self.write_index + QUANTUM);
    }
}

/// A `DelayNode` (`DelayHandler` and its `AudioHandler` silence state): the `delayTime` param
/// (a-rate, default 0, range `[0, 1]`) and one kernel per input channel (1, then 2 for good).
#[derive(Debug, Clone)]
pub struct DelayLine {
    pub delay_time: Param,
    kernels: Vec<DelayKernel>,
    /// `last_non_silent_time_`, seconds.
    last_non_silent_time: f64,
}

impl Default for DelayLine {
    fn default() -> Self {
        DelayLine::new()
    }
}

impl DelayLine {
    /// `new DelayNode(ac)`: one channel, `delayTime` 0 without events.
    pub fn new() -> DelayLine {
        DelayLine {
            delay_time: Param::new(0.0, 0.0, MAX_DELAY_TIME as f32, Rate::A),
            kernels: vec![DelayKernel::new()],
            last_non_silent_time: 0.0,
        }
    }

    /// The channel count (the number of kernels).
    pub fn channels(&self) -> usize {
        self.kernels.len()
    }

    pub fn kernels(&self) -> &[DelayKernel] {
        &self.kernels
    }

    /// `DelayHandler::CheckNumberOfChannelsForInput` for an input of `channels` (1 or 2)
    /// channels. Only growth is modelled (the feedback path keeps a stereo delay stereo): new
    /// kernels keep the write index and get the existing history up-mixed (`AudioBus::CopyFrom`,
    /// speakers: mono goes to both channels). Returns whether the count changed.
    pub fn set_channels(&mut self, channels: usize) -> bool {
        let channels = channels.clamp(1, 2);
        if channels <= self.kernels.len() {
            return false;
        }
        let mono = self.kernels[0].clone();
        self.kernels = vec![mono; channels];
        true
    }

    /// `ProcessIfNecessary` + `DelayHandler::Process` for one quantum. `input` holds one buffer
    /// per channel of the line (the summed input bus) and `input_silent` its silence hint. With a
    /// silent input more than the tail time (1 s) after the last signal, the output is silence
    /// and only `delayTime` is evaluated (`ProcessOnlyAudioParams`); else each kernel evaluates
    /// `delayTime` (`CalculateSampleAccurateValues`, once per kernel as Blink does) and runs the
    /// a-rate kernel. Returns the output's silence hint.
    #[cfg(test)]
    pub fn process(
        &mut self,
        quantum_start: u64,
        input: &[[f32; QUANTUM]],
        input_silent: bool,
        out: &mut [[f32; QUANTUM]],
    ) -> bool {
        self.process_with(quantum_start, input, input_silent, out, &ParamInputs::NONE)
    }

    /// [`DelayLine::process`] with the connections into `delayTime` (orbit-held modulators),
    /// summed in both branches.
    pub fn process_with(
        &mut self,
        quantum_start: u64,
        input: &[[f32; QUANTUM]],
        input_silent: bool,
        out: &mut [[f32; QUANTUM]],
        time_inputs: &ParamInputs,
    ) -> bool {
        let now = quantum_start as f64 / SR;
        let mut times = [0.0f32; QUANTUM];
        let (bufs, connected) = (&time_inputs.bufs, time_inputs.connected);
        let silent = if input_silent && self.last_non_silent_time + MAX_DELAY_TIME < now {
            for o in out.iter_mut() {
                o.fill(0.0);
            }
            self.delay_time
                .fill_quantum_connected(quantum_start, &mut times, bufs, connected);
            true
        } else {
            let zeros = [0.0f32; QUANTUM];
            for (i, kernel) in self.kernels.iter_mut().enumerate() {
                self.delay_time
                    .fill_quantum_connected(quantum_start, &mut times, bufs, connected);
                let src = input.get(i).unwrap_or(&zeros);
                if let Some(o) = out.get_mut(i) {
                    kernel.process_a_rate(src, &times, o);
                }
            }
            false
        };
        if !input_silent {
            self.last_non_silent_time = crate::quantum_end_time(quantum_start);
        }
        silent
    }
}

type Bus = [[f32; QUANTUM]; 2];

/// Sums `input` into the first `channels` buffers of `bus` (`AudioBus::SumFrom`, speakers): a
/// mono input goes to every channel, a stereo one channel-wise (a stereo input into a mono bus
/// cannot happen: the bus grows first).
pub(crate) fn sum_into(bus: &mut Bus, silent: &mut [bool; 2], channels: usize, input: Input<'_>) {
    for c in 0..channels.min(2) {
        let src = match input.channels {
            [mono] => mono,
            [l, r, ..] => {
                if c == 0 {
                    l
                } else {
                    r
                }
            }
            [] => return,
        };
        crate::orbit::sum_channel(&mut bus[c], &mut silent[c], src, input.silent);
    }
}

/// superdough's `FeedbackDelayNode` on an orbit: the delay line, the feedback gain and the
/// output gain (`delayGain`, 1).
///
/// Per quantum: [`FeedbackDelay::begin`] (channel count, the feedback path from the previous
/// quantum's output), [`FeedbackDelay::add_send`] for each send in connection order, then
/// [`FeedbackDelay::finish`] (the delay line and `delayGain`).
#[derive(Debug, Clone)]
pub struct FeedbackDelay {
    line: DelayLine,
    feedback: GainNode,
    delay_gain: GainNode,
    /// The delay's output bus as the feedback gain reads it: the previous quantum's output.
    prev: Bus,
    prev_silent: bool,
    /// The summed input bus of the quantum being rendered.
    input: Bus,
    input_silent: [bool; 2],
    /// The `delayGain` output of the last finished quantum.
    out: Bus,
    out_silent: bool,
}

impl FeedbackDelay {
    /// `ac.createFeedbackDelay(1, delaytime, feedback)` at context frame `now`, `feedback`
    /// already clamped by `getDelay`: `delayTime.value = delaytime` (a WebIDL float: a
    /// non-finite or overflowing value throws and nothing is created; else the intrinsic value
    /// is clamped to `[0, 1]` and a `setValueAtTime` is inserted at `now`),
    /// `feedbackGain.gain.value = min(|feedback|, 0.995)`, `delayGain.gain.value = 1`.
    /// (`start(t)`, called once by `getDelay`, schedules `delayGain`'s current value 1 again and
    /// changes nothing; 04 Resolved decision 2.)
    pub fn new(delaytime: f64, feedback: f64, now: u64) -> Result<FeedbackDelay, ParamError> {
        let mut line = DelayLine::new();
        line.delay_time
            .set_value(idl_float(delaytime, "value")?, now)?;
        let mut feedback_gain = Param::gain(1.0);
        let fb = js_min(feedback.abs(), MAX_FEEDBACK_GAIN);
        feedback_gain.set_value(idl_float(fb, "value")?, now)?;
        let mut delay_gain = Param::gain(1.0);
        delay_gain.set_value(1.0, now)?;
        Ok(FeedbackDelay {
            line,
            feedback: GainNode::new(feedback_gain),
            delay_gain: GainNode::new(delay_gain),
            prev: [[0.0; QUANTUM]; 2],
            prev_silent: true,
            input: [[0.0; QUANTUM]; 2],
            input_silent: [true; 2],
            out: [[0.0; QUANTUM]; 2],
            out_silent: true,
        })
    }

    pub fn line(&self) -> &DelayLine {
        &self.line
    }

    /// `delayNode.delayTime`.
    pub fn delay_time(&self) -> &Param {
        &self.line.delay_time
    }

    /// `delayNode.feedback` (`feedbackGain.gain`).
    pub fn feedback(&self) -> &Param {
        &self.feedback.gain
    }

    /// The channel count of the line and its output.
    pub fn channels(&self) -> usize {
        self.line.channels()
    }

    /// `getDelay`'s guard for a hap at `t` (scheduled at frame `now`), after creation:
    /// `delayTime.value !== delaytime && delayTime.setValueAtTime(delaytime, t)`, then the same
    /// for the feedback. `.value` is the param's float intrinsic value (the last `.value =` or the
    /// last rendered quantum's last value), compared with the JS double, so a value that is not
    /// exact in float, any `delaytime` above 1 s (`.value` is at most 1) and a non-number
    /// `delaytime` always schedule. A throwing `setValueAtTime` stops the guard.
    pub fn update(
        &mut self,
        delaytime: f64,
        delaytime_is_number: bool,
        feedback: f64,
        t: f64,
        now: u64,
    ) -> Result<(), ParamError> {
        let time = &mut self.line.delay_time;
        if !delaytime_is_number || f64::from(time.value()) != delaytime {
            time.set_value_at_time(idl_float(delaytime, "setValueAtTime")?, t, now)?;
        }
        let gain = &mut self.feedback.gain;
        if f64::from(gain.value()) != feedback {
            gain.set_value_at_time(idl_float(feedback, "setValueAtTime")?, t, now)?;
        }
        Ok(())
    }

    /// Starts the quantum at `quantum_start` for sends of at most `send_channels` channels: the
    /// line grows to stereo when a stereo send is connected (the output bus is re-created then,
    /// so the feedback reads a silent bus this quantum), the feedback gain processes the
    /// previous quantum's output, and the input bus starts as that feedback (connected first).
    #[cfg(test)]
    pub fn begin(&mut self, quantum_start: u64, send_channels: usize) {
        self.begin_with(quantum_start, send_channels, &ParamInputs::NONE);
    }

    /// [`FeedbackDelay::begin`] with the connections into the feedback gain (orbit-held
    /// modulators).
    pub fn begin_with(
        &mut self,
        quantum_start: u64,
        send_channels: usize,
        feedback_inputs: &ParamInputs,
    ) {
        if self.line.set_channels(send_channels) {
            self.prev = [[0.0; QUANTUM]; 2];
            self.prev_silent = true;
        }
        let n = self.line.channels();
        let mut fb = self.prev;
        let fb_silent = self.feedback.process(
            quantum_start,
            &mut fb[..n],
            self.prev_silent,
            feedback_inputs,
        );
        self.input = [[0.0; QUANTUM]; 2];
        self.input_silent = [true; 2];
        let fb_input = Input {
            channels: &fb[..n],
            silent: fb_silent,
        };
        sum_into(&mut self.input, &mut self.input_silent, n, fb_input);
    }

    /// Adds one send (`post -> gainNode(level) -> delay`, already scaled by the voice),
    /// up-mixed to the line's channels.
    pub fn add_send(&mut self, send: Input<'_>) {
        let channels = self.line.channels();
        sum_into(&mut self.input, &mut self.input_silent, channels, send);
    }

    /// Ends the quantum: the delay line on the summed input, its output kept for the next
    /// quantum's feedback, then `delayGain` (1). Returns the return into the summing node.
    #[cfg(test)]
    pub fn finish(&mut self, quantum_start: u64) -> Input<'_> {
        self.finish_with(quantum_start, &ParamInputs::NONE)
    }

    /// [`FeedbackDelay::finish`] with the connections into `delayTime`.
    pub fn finish_with(&mut self, quantum_start: u64, time_inputs: &ParamInputs) -> Input<'_> {
        let n = self.line.channels();
        let input_silent = self.input_silent[..n].iter().all(|s| *s);
        let mut out: Bus = [[0.0; QUANTUM]; 2];
        let silent = self.line.process_with(
            quantum_start,
            &self.input[..n],
            input_silent,
            &mut out[..n],
            time_inputs,
        );
        self.prev = out;
        self.prev_silent = silent;
        self.out = out;
        self.out_silent = self.delay_gain.process(
            quantum_start,
            &mut self.out[..n],
            silent,
            &ParamInputs::NONE,
        );
        Input {
            channels: &self.out[..n],
            silent: self.out_silent,
        }
    }
}

/// `Orbit.getDelay(delaytime, feedback, t)` on the orbit's delay `slot` for a hap at `t`
/// scheduled at frame `now`: clamps `feedback` to `[0, 0.98]`, creates the delay on first use,
/// then runs the guard ([`FeedbackDelay::update`]). An error is the exception superdough throws
/// (a creation that throws leaves the slot empty).
pub fn get_delay(
    slot: &mut Option<FeedbackDelay>,
    delaytime: f64,
    delaytime_is_number: bool,
    feedback: f64,
    t: f64,
    now: u64,
) -> Result<(), ParamError> {
    let feedback = js_min(js_max(feedback, 0.0), MAX_FEEDBACK);
    let delay = match slot {
        Some(d) => d,
        None => slot.insert(FeedbackDelay::new(delaytime, feedback, now)?),
    };
    delay.update(delaytime, delaytime_is_number, feedback, t, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::param::RampKind;

    use crate::QUANTUM_U64 as Q;

    fn mono(x: &[f32]) -> [f32; QUANTUM] {
        let mut b = [0.0f32; QUANTUM];
        b[..x.len()].copy_from_slice(x);
        b
    }

    /// A delay line fixed at `seconds` (`.value =` at frame 0).
    fn line(seconds: f32) -> DelayLine {
        let mut l = DelayLine::new();
        l.delay_time.set_value(seconds, 0).unwrap();
        l
    }

    /// Runs a mono line over `input` (zero-padded to whole quanta) and returns the output.
    fn run_line(l: &mut DelayLine, input: &[f32], quanta: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for q in 0..quanta {
            let mut chunk = [0.0f32; QUANTUM];
            for (i, c) in chunk.iter_mut().enumerate() {
                *c = input.get(q * QUANTUM + i).copied().unwrap_or(0.0);
            }
            let mut o = [[0.0f32; QUANTUM]];
            let silent = chunk.iter().all(|x| *x == 0.0);
            l.process(q as u64 * Q, &[chunk], silent, &mut o);
            out.extend_from_slice(&o[0]);
        }
        out
    }

    fn impulse(at: usize) -> Vec<f32> {
        let mut x = vec![0.0f32; at + 1];
        x[at] = 1.0;
        x
    }

    #[test]
    fn constants_follow_blink() {
        assert_eq!(BUFFER_LENGTH, 44228);
        assert_eq!(FEEDBACK_EXTRA_FRAMES, 128);
        let l = DelayLine::new();
        let p = &l.delay_time;
        assert_eq!(
            (p.default_value(), p.min_value(), p.max_value()),
            (0.0, 0.0, 1.0)
        );
        assert_eq!(p.rate(), Rate::A);
        assert_eq!(l.channels(), 1);
        assert_eq!(l.kernels()[0].buffer().len(), BUFFER_LENGTH);
    }

    #[test]
    fn whole_frame_delays_shift_exactly() {
        for (seconds, frames) in [(0.25f32, 11025usize), (0.05, 2205), (0.01, 441)] {
            assert_eq!(
                seconds * SR_F32,
                frames as f32,
                "{seconds} is a whole frame count"
            );
            let mut l = line(seconds);
            let out = run_line(&mut l, &impulse(37), frames / QUANTUM + 2);
            assert_eq!(out.iter().position(|x| *x != 0.0), Some(37 + frames));
            assert_eq!(out[37 + frames], 1.0);
            assert_eq!(out.iter().filter(|x| **x != 0.0).count(), 1);
        }
    }

    #[test]
    fn fractional_delays_interpolate_linearly_in_float() {
        // 0.0625 s = 2756.25 frames: the impulse at frame 10 lands 3/4 on frame 2766 and 1/4
        // on frame 2767 (`s1 + f * (s2 - s1)` with f = 0.75 reading backwards).
        let mut l = line(0.0625);
        let out = run_line(&mut l, &impulse(10), 2800 / QUANTUM + 1);
        assert_eq!(out[2766], 0.75);
        assert_eq!(out[2767], 0.25);
        assert_eq!(out.iter().filter(|x| **x != 0.0).count(), 2);
    }

    #[test]
    fn delays_below_a_quantum_read_what_was_just_written() {
        // Write before read: delay 0 is a pass-through and 0.001 s (44.1 frames) reads inside
        // the quantum being processed.
        let x: Vec<f32> = (0..QUANTUM).map(|i| i as f32 + 1.0).collect();
        let mut zero = line(0.0);
        assert_eq!(run_line(&mut zero, &x, 1), x);
        let mut short = line(0.001);
        let out = run_line(&mut short, &impulse(5), 1);
        let d = 0.001f32 * SR_F32;
        assert!(d > 44.0 && d < 45.0);
        let at = out.iter().position(|v| *v != 0.0).unwrap();
        assert_eq!(at, 5 + 44, "the echo lands in the same quantum");
    }

    #[test]
    fn the_kernel_is_a_rate_with_and_without_events() {
        // No event: the intrinsic value 0 is filled every frame (a pass-through).
        let mut l = DelayLine::new();
        assert_eq!(run_line(&mut l, &impulse(3), 1)[3], 1.0);
        // A step inside a quantum applies from its frame on: frames before 64 read with delay
        // 0, frames from 64 on with 10 frames.
        let mut l = DelayLine::new();
        let ten = 10.0 / SR;
        l.delay_time
            .set_value_at_time(ten as f32, 64.0 / SR, 0)
            .unwrap();
        let x: Vec<f32> = (0..QUANTUM).map(|i| i as f32).collect();
        let out = run_line(&mut l, &x, 1);
        assert_eq!(out[63], 63.0);
        let d = (ten as f32) * SR_F32;
        let pos = (64.0f32 + (BUFFER_LENGTH as f32 - d)) - BUFFER_LENGTH as f32;
        let f = pos - pos.trunc();
        let expected = x[pos as usize] + f * (x[pos as usize + 1] - x[pos as usize]);
        assert_eq!(out[64], expected);
        assert!((out[100] - 90.0).abs() < 1e-3);
    }

    #[test]
    fn the_param_clips_delays_above_the_maximum() {
        // `.value = 1.5` stores 1; a timeline value of 1.5 is clipped to 1 s as well.
        let mut l = DelayLine::new();
        l.delay_time.set_value(1.5, 0).unwrap();
        assert_eq!(l.delay_time.value(), 1.0);
        l.delay_time.set_value_at_time(1.5, 0.0, 0).unwrap();
        let out = run_line(&mut l, &impulse(0), 44100 / QUANTUM + 2);
        assert_eq!(out.iter().position(|x| *x != 0.0), Some(44100));
        assert_eq!(out[44100], 1.0);
    }

    #[test]
    fn the_circular_buffer_wraps() {
        // 0.25 s for 3 s of quanta: the write index wraps around the 44228-frame buffer and
        // every echo still lands 11025 frames later.
        let mut l = line(0.25);
        let mut x = vec![0.0f32; 3 * 44100];
        for at in [100usize, 44000, 50000, 88300] {
            x[at] = 1.0;
        }
        let out = run_line(&mut l, &x, x.len() / QUANTUM + 100);
        let echoes: Vec<usize> = (0..out.len()).filter(|i| out[*i] != 0.0).collect();
        assert_eq!(echoes, vec![11125, 55025, 61025, 99325]);
    }

    #[test]
    fn silent_input_past_the_tail_only_evaluates_the_param() {
        let mut l = line(0.01);
        let zeros = [[0.0f32; QUANTUM]];
        let mut o = [[1.0f32; QUANTUM]];
        // Within the first second (last signal at time 0) the line processes.
        assert!(!l.process(0, &zeros, true, &mut o));
        let w = l.kernels()[0].write_index();
        // Past 1 s of silence it outputs silence without advancing the kernels.
        let late = 44160;
        assert!(l.process(late, &zeros, true, &mut o));
        assert!(o[0].iter().all(|x| *x == 0.0));
        assert_eq!(l.kernels()[0].write_index(), w);
        // Signal brings it back; silence then stays processed for the tail time.
        let x = [mono(&[1.0])];
        assert!(!l.process(late + Q, &x, false, &mut o));
        assert!(!l.process(late + 2 * Q, &zeros, true, &mut o));
        assert!(!l.process(late + Q + 44100 - 4, &zeros, true, &mut o));
        assert!(l.process(late + 3 * Q + 44100, &zeros, true, &mut o));
    }

    #[test]
    fn going_stereo_copies_the_history_and_keeps_the_write_index() {
        let mut l = line(0.25);
        run_line(&mut l, &impulse(200), 3);
        let before = l.kernels()[0].clone();
        assert!(!l.set_channels(1), "same count: nothing changes");
        assert!(l.set_channels(2));
        assert_eq!(l.channels(), 2);
        for k in l.kernels() {
            assert_eq!(k, &before);
        }
        // Monotonic: a mono input later keeps two channels.
        assert!(!l.set_channels(1));
        assert!(!l.set_channels(7), "at most two channels");
        assert_eq!(l.channels(), 2);
        // Both channels then delay their own input with the shared delay time.
        let mut o = [[0.0f32; QUANTUM]; 2];
        let input = [mono(&[0.0, 2.0]), mono(&[0.0, 0.0, 3.0])];
        let mut q = 3 * Q;
        l.process(q, &input, false, &mut o);
        let mut l_out = Vec::new();
        let mut r_out = Vec::new();
        while q < 3 * Q + 11025 + Q {
            if q > 3 * Q {
                l.process(q, &[[0.0; QUANTUM]; 2], false, &mut o);
            }
            l_out.extend_from_slice(&o[0]);
            r_out.extend_from_slice(&o[1]);
            q += Q;
        }
        // The old impulse (frame 200) comes out on both channels at 11225.
        let at = |v: &[f32], f: usize| v[f - 3 * QUANTUM];
        assert_eq!(at(&l_out, 11225), 1.0);
        assert_eq!(at(&r_out, 11225), 1.0);
        assert_eq!(at(&l_out, 3 * QUANTUM + 1 + 11025), 2.0);
        assert_eq!(at(&r_out, 3 * QUANTUM + 1 + 11025), 0.0);
        assert_eq!(at(&r_out, 3 * QUANTUM + 2 + 11025), 3.0);
    }

    /// Runs a feedback delay with one mono send of `x` (level 1) and returns its return.
    fn run_feedback(d: &mut FeedbackDelay, x: &[f32], quanta: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for q in 0..quanta {
            let mut chunk = [[0.0f32; QUANTUM]];
            for (i, c) in chunk[0].iter_mut().enumerate() {
                *c = x.get(q * QUANTUM + i).copied().unwrap_or(0.0);
            }
            let silent = chunk[0].iter().all(|v| *v == 0.0);
            d.begin(q as u64 * Q, 1);
            d.add_send(Input {
                channels: &chunk,
                silent,
            });
            out.extend_from_slice(&d.finish(q as u64 * Q).channels[0]);
        }
        out
    }

    fn onsets(x: &[f32]) -> Vec<usize> {
        let mut v = Vec::new();
        let mut zeros = usize::MAX;
        for (i, s) in x.iter().enumerate() {
            if *s != 0.0 {
                if zeros > 64 {
                    v.push(i);
                }
                zeros = 0;
            } else {
                zeros = zeros.saturating_add(1);
            }
        }
        v
    }

    #[test]
    fn each_loop_through_the_feedback_adds_one_quantum() {
        // Echo k of an impulse at t0 lands at t0 + k*D + (k-1)*128 with amplitude g^(k-1).
        let mut d = FeedbackDelay::new(0.05, 0.5, 0).unwrap();
        let t0 = 1000;
        let out = run_feedback(&mut d, &impulse(t0), 12000 / QUANTUM);
        let dly = 2205;
        let expected: Vec<usize> = (1..=4)
            .map(|k| t0 + k * dly + (k - 1) * FEEDBACK_EXTRA_FRAMES as usize)
            .collect();
        assert_eq!(onsets(&out)[..4], expected[..]);
        for (k, at) in expected.iter().enumerate() {
            assert_eq!(out[*at], 0.5f32.powi(k as i32));
        }
    }

    #[test]
    fn creation_sets_values_with_events_and_clamps() {
        let d = FeedbackDelay::new(0.3, 0.98, 256).unwrap();
        assert_eq!(d.delay_time().value(), 0.3f32);
        let e = d.delay_time().events();
        assert_eq!(
            (e.len(), e[0].kind, e[0].time),
            (1, RampKind::SetValue, 256.0 / SR)
        );
        assert_eq!(d.feedback().value(), 0.98f32);
        assert_eq!(d.feedback().events().len(), 1);
        assert_eq!(d.channels(), 1);
        // Above the maximum: the intrinsic value is 1 (the event holds the clamped value).
        let d = FeedbackDelay::new(1e9, 0.5, 0).unwrap();
        assert_eq!(d.delay_time().value(), 1.0);
        // The feedback gain is |fb| capped at 0.995 (getDelay's clamp already caps it lower).
        assert_eq!(
            FeedbackDelay::new(0.1, -0.5, 0).unwrap().feedback().value(),
            0.5
        );
        assert_eq!(
            FeedbackDelay::new(0.1, 3.0, 0).unwrap().feedback().value(),
            0.995
        );
        // A delay time that is not a finite float throws.
        for bad in [f64::INFINITY, f64::NAN, 1e39] {
            let e = FeedbackDelay::new(bad, 0.5, 0).unwrap_err();
            assert!(matches!(e, ParamError::NonFiniteValue { .. }), "{bad}");
        }
    }

    #[test]
    fn get_delay_creates_once_and_clamps_the_feedback() {
        let mut slot = None;
        get_delay(&mut slot, 0.25, true, 5.0, 0.0, 0).unwrap();
        let d = slot.as_ref().unwrap();
        assert_eq!(
            d.feedback().value(),
            0.98f32,
            "clamped to 0.98 before creation"
        );
        // 0.98 is not exact in float: the guard fires right after creation.
        assert_eq!(d.feedback().events().len(), 2);
        assert_eq!(d.delay_time().events().len(), 1, "0.25 is exact: skipped");
        // A second call reuses the node.
        get_delay(&mut slot, 0.5, true, 0.5, 0.1, 0).unwrap();
        let d = slot.as_ref().unwrap();
        assert_eq!(d.delay_time().events().last().unwrap().value, 0.5);
        assert_eq!(d.delay_time().value(), 0.25, "setValueAtTime leaves .value");
        // A throwing creation leaves the slot empty; a throwing guard keeps the node.
        let mut empty = None;
        assert!(get_delay(&mut empty, f64::INFINITY, true, 0.5, 0.0, 0).is_err());
        assert!(empty.is_none());
        assert!(get_delay(&mut slot, 1e39, true, 0.5, 0.2, 0).is_err());
        assert!(slot.is_some());
    }

    #[test]
    fn the_guard_compares_the_float_value_with_the_double() {
        let mut d = FeedbackDelay::new(0.375, 0.5, 0).unwrap();
        let n = |d: &FeedbackDelay| (d.delay_time().events().len(), d.feedback().events().len());
        // Equal in float: skipped.
        d.update(0.375, true, 0.5, 0.1, 0).unwrap();
        assert_eq!(n(&d), (1, 1));
        // Not exact in float: always scheduled, even when it is the value already held.
        let mut e = FeedbackDelay::new(0.3, 0.6, 0).unwrap();
        e.update(0.3, true, 0.6, 0.1, 0).unwrap();
        assert_eq!(n(&e), (2, 2));
        // A string delaytime never equals `.value`.
        d.update(0.375, false, 0.5, 0.1, 0).unwrap();
        assert_eq!(n(&d), (2, 1));
        // Above 1 s `.value` reads 1: always scheduled; exactly 1 is then skipped.
        let mut big = FeedbackDelay::new(1.5, 0.5, 0).unwrap();
        big.update(1.5, true, 0.5, 0.0, 0).unwrap();
        assert_eq!(n(&big), (2, 1));
        big.update(1.0, true, 0.5, 0.0, 0).unwrap();
        assert_eq!(n(&big), (2, 1));
    }

    #[test]
    fn the_guard_reads_the_value_rendering_left() {
        // Orbit at 0.375; within one pause 0.3 is scheduled and 0.375 is skipped (the intrinsic
        // value has not moved), so the delay stays at 0.3 after both.
        let mut d = FeedbackDelay::new(0.375, 0.5, 0).unwrap();
        d.update(0.3, true, 0.5, 0.01, 0).unwrap();
        d.update(0.375, true, 0.5, 0.02, 0).unwrap();
        assert_eq!(d.delay_time().events().len(), 2);
        // After rendering past 0.01 s the intrinsic value is 0.3: now 0.375 is scheduled.
        for q in 0..8 {
            d.begin(q * Q, 1);
            d.finish(q * Q);
        }
        assert_eq!(d.delay_time().value(), 0.3f32);
        d.update(0.375, true, 0.5, 0.05, 1024).unwrap();
        assert_eq!(d.delay_time().events().last().unwrap().value, 0.375);
    }

    #[test]
    fn a_stereo_send_makes_the_delay_stereo_and_drops_one_quantum_of_feedback() {
        let mut d = FeedbackDelay::new(0.01, 0.5, 0).unwrap();
        let tone = [[0.25f32; QUANTUM]];
        // Fill the mono line for 8 quanta: from frame 441 on the output is non-zero.
        for q in 0..8 {
            d.begin(q * Q, 1);
            d.add_send(Input {
                channels: &tone,
                silent: false,
            });
            d.finish(q * Q);
        }
        let w = d.line().kernels()[0].write_index();
        let history = d.line().kernels()[0].clone();
        // A stereo send (silent here) joins: two channels, same write index and history.
        let st = [[0.0f32; QUANTUM]; 2];
        d.begin(8 * Q, 2);
        assert_eq!(d.channels(), 2);
        for k in d.line().kernels() {
            assert_eq!(k.write_index(), w);
            assert_eq!(k.buffer(), history.buffer());
        }
        d.add_send(Input {
            channels: &st,
            silent: true,
        });
        let ret = d.finish(8 * Q);
        assert_eq!(ret.channels.len(), 2);
        assert!(!ret.silent);
        // This quantum's input was silent (the feedback read the re-created output bus), so
        // 441 frames later the line plays zeros where the feedback would have been.
        let mut out = Vec::new();
        for q in 9..14 {
            d.begin(q * Q, 1);
            let r = d.finish(q * Q);
            out.push((r.channels[0], r.channels[1]));
        }
        let frame = |f: u64| {
            let (l, r) = out[(f / Q - 9) as usize];
            (l[(f % Q) as usize], r[(f % Q) as usize])
        };
        assert_eq!(frame(8 * Q + 441), (0.0, 0.0));
        let (l, r) = frame(9 * Q + 441);
        assert_eq!(
            l, r,
            "the up-mixed history and the feedback match on both sides"
        );
        assert_ne!(l, 0.0);
    }

    #[test]
    fn mono_sends_up_mix_into_a_stereo_delay() {
        let mut d = FeedbackDelay::new(0.0, 0.5, 0).unwrap();
        // The sends arrive scaled by their gains (0.5 and 0.25).
        let st = [[0.25f32; QUANTUM], [0.125; QUANTUM]];
        let m = [[0.25f32; QUANTUM]];
        d.begin(0, 2);
        d.add_send(Input {
            channels: &st,
            silent: false,
        });
        d.add_send(Input {
            channels: &m,
            silent: false,
        });
        // Delay 0: the return is this quantum's input (no feedback yet).
        let r = d.finish(0);
        assert_eq!(
            (r.channels[0][7], r.channels[1][7]),
            (0.25 + 0.25, 0.125 + 0.25)
        );
        // A silent send adds nothing; the feedback (previous output * 0.5) comes first.
        d.begin(Q, 2);
        d.add_send(Input {
            channels: &m,
            silent: true,
        });
        let r = d.finish(Q);
        assert_eq!((r.channels[0][0], r.channels[1][0]), (0.25, 0.1875));
    }

    #[test]
    fn an_idle_delay_is_silent() {
        let mut d = FeedbackDelay::new(0.1, 0.5, 0).unwrap();
        d.begin(0, 1);
        let r = d.finish(0);
        assert!(r.channels[0].iter().all(|x| *x == 0.0));
        // Created more than a second into the render with nothing sent: silent output.
        let mut late = FeedbackDelay::new(0.1, 0.5, 88320).unwrap();
        late.begin(88320, 1);
        assert!(late.finish(88320).silent);
    }
}
