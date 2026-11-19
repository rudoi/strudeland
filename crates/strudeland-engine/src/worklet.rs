// Port of Blink (Chromium 156.0.8078.4) modules/webaudio/audio_worklet_handler.cc, modules/webaudio/audio_worklet_processor.cc, modules/webaudio/audio_worklet_global_scope.cc, modules/webaudio/base_audio_context.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! How Chromium runs an `AudioWorkletProcessor` in an `OfflineAudioContext`, as the engine's
//! worklet processors (`shaper`, `transient`, ...) need it:
//!
//! - **Params.** `AudioWorkletNode` builds one `AudioParam` per descriptor. The descriptor
//!   dictionary has `minValue`/`maxValue`; superdough's `min`/`max` keys are ignored by the WebIDL
//!   conversion, so such a param (and one without a range) spans the whole float range
//!   ([`param`]). One quantum of a param is [`fill_block`]: the 128 values the processor reads as
//!   `parameters[name]` (a length-1 array when all are equal is invisible: superdough reads
//!   `p[0]` or `p[n] ?? p[0]`).
//! - **Time.** `currentTime` in the worklet scope is the quantum's start frame over the rate, a
//!   double, constant for the 128 frames ([`current_time`]). A processor compares it with its
//!   float32 `begin` param ([`started`], `!(currentTime <= begin)`, measured in
//!   `plans/0011-engine-effects/evidence/measure-worklet-start.json`) and `end` ([`reached`]).
//! - **Lifecycle.** `AudioWorkletHandler::Process`: a processor is called while it is an active
//!   source (its last `process()` returned `true`) or has a connected input; a thrown exception
//!   finishes it for good ([`Lifecycle`]). A finished processor's node outputs a silent zero bus;
//!   a live one never propagates silence (its tail time is infinite).
//! - **Teardown.** When a scheduled source finishes, the disabling of its outputs cascades through
//!   the gains after it, and the next worklet sees no connected input from the next quantum on
//!   ([`first_absent_after_stop`]).
//!
//! The JS outputs are `Float32Array`s zeroed before every call; a processor stores `x as f32`
//! (round to nearest even) and reads back the stored value.

use crate::QUANTUM;
use crate::param::{Param, ParamInputs, Rate};

/// The context's sample rate as the worklet scope sees it (`sampleRate`).
pub const SR: f64 = crate::SAMPLE_RATE_F64;
/// `1 / sampleRate` (worklets.mjs `INVSR`).
pub const INVSR: f64 = 1.0 / crate::SAMPLE_RATE_F64;

/// A worklet `AudioParam` whose descriptor spells its range `min`/`max` or has none: the range is
/// the whole float range (`AudioParamDescriptor` ignores the unknown keys).
pub fn param(default: f32, rate: Rate) -> Param {
    Param::new(default, -f32::MAX, f32::MAX, rate)
}

/// A worklet `AudioParam` whose descriptor spells `minValue`/`maxValue` (only the envelope
/// processor's); an omitted bound is the float range's.
pub fn ranged(default: f32, min: f32, max: f32, rate: Rate) -> Param {
    Param::new(default, min, max, rate)
}

/// `currentTime` in the worklet scope during the quantum starting at frame `quantum_start`.
pub fn current_time(quantum_start: u64) -> f64 {
    quantum_start as f64 / SR
}

/// JS `!(currentTime <= begin)` with the float32 `begin` param: true from the first quantum whose
/// start time is strictly after `f32(begin)`.
pub fn started(ct: f64, begin: f32) -> bool {
    ct > f64::from(begin)
}

/// JS `currentTime >= end` with the float32 `end` param.
pub fn reached(ct: f64, end: f32) -> bool {
    ct >= f64::from(end)
}

/// One quantum of a worklet param, as `parameters[name]` holds it: the a-rate timeline per frame
/// (or the k-rate value repeated) plus the connections, NaN replaced by the default, clipped.
pub fn fill_block(
    p: &mut Param,
    quantum_start: u64,
    input: &ParamInputs,
    out: &mut [f32; QUANTUM],
) {
    p.fill_quantum_connected(quantum_start, out, &input.bufs, input.connected);
}

/// The first quantum start from which a worklet after a source that stops at frame `end_frame`
/// (exclusive, `Schedule::end_frame`) sees no connected input: the source finishes in the quantum
/// that holds `end_frame` (also when `end_frame` is that quantum's end), and the disabling runs in
/// that quantum's post-render tasks.
pub fn first_absent_after_stop(end_frame: u64) -> u64 {
    end_frame
        .div_ceil(QUANTUM as u64)
        .saturating_mul(QUANTUM as u64)
}

/// The first quantum start whose `currentTime` is `>= end`: where a processor that compares
/// `currentTime >= end` first sees its end. `None` when no quantum reaches it (NaN, or an end
/// past any render).
pub fn first_quantum_reaching(end: f64) -> Option<u64> {
    if end.is_nan() {
        return None;
    }
    if end <= 0.0 {
        return Some(0);
    }
    let k = (end * SR / QUANTUM as f64).floor();
    if k >= 1e15 {
        return None;
    }
    // One quantum early, then up to the first start that reaches `end` on the double compare.
    let mut q = (k as u64).saturating_sub(1) * QUANTUM as u64;
    while current_time(q) < end {
        q += QUANTUM as u64;
    }
    Some(q)
}

/// A thrown exception in `process()` (a `TypeError` in superdough's JS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessorError {
    pub processor: &'static str,
    pub what: &'static str,
}

impl ProcessorError {
    /// The `hap` render error a stage reports once (Chromium outputs silence for good).
    pub fn message(&self) -> String {
        format!(
            "{}: processor error; Chromium outputs silence",
            self.processor
        )
    }
}

/// `AudioWorkletHandler::Process` steps 2, 3 and 9: whether the processor is still called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lifecycle {
    active_source: bool,
    gone: bool,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Lifecycle::new()
    }
}

impl Lifecycle {
    /// A new node: an active source (`is_active_source_` starts true), not finished.
    pub fn new() -> Lifecycle {
        Lifecycle {
            active_source: true,
            gone: false,
        }
    }

    /// Start of a quantum: finishes a processor that is no active source and has no connected
    /// input. True when `process()` must be called.
    pub fn begin_quantum(&mut self, has_connected_input: bool) -> bool {
        if !self.active_source && !has_connected_input {
            self.gone = true;
        }
        !self.gone
    }

    /// After `process()`: only the boolean `true` keeps the processor an active source; a thrown
    /// exception finishes it for good; no active source and no connected input finishes it.
    pub fn finish_quantum(&mut self, r: Result<bool, ProcessorError>, has_connected_input: bool) {
        match r {
            Ok(b) => self.active_source = b,
            Err(_) => {
                self.active_source = false;
                self.gone = true;
            }
        }
        if !self.active_source && !has_connected_input {
            self.gone = true;
        }
    }

    /// Whether the processor has finished (its node outputs a silent zero bus).
    pub fn gone(&self) -> bool {
        self.gone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::QUANTUM_U64 as Q;

    #[test]
    fn rate_and_time_constants() {
        assert_eq!(SR, 44100.0);
        assert_eq!(INVSR.to_bits(), (1.0f64 / 44100.0).to_bits());
        assert_eq!(current_time(0), 0.0);
        assert_eq!(current_time(128).to_bits(), (128.0f64 / 44100.0).to_bits());
        assert_eq!(current_time(44100), 1.0);
    }

    #[test]
    fn param_constructors() {
        let p = param(1.0, Rate::A);
        assert_eq!(p.default_value(), 1.0);
        assert_eq!(p.value(), 1.0);
        assert_eq!((p.min_value(), p.max_value()), (-f32::MAX, f32::MAX));
        assert_eq!(p.rate(), Rate::A);
        assert!(p.events().is_empty());
        assert_eq!(param(0.0, Rate::K).rate(), Rate::K);
        let r = ranged(0.5, 0.0, 1.0, Rate::A);
        assert_eq!((r.min_value(), r.max_value(), r.value()), (0.0, 1.0, 0.5));
        // `.value =` clamps to a ranged param's range, never to a full-range one's.
        let mut r = ranged(0.5, 0.0, 1.0, Rate::A);
        r.set_value(3.0, 0).unwrap();
        assert_eq!(r.value(), 1.0);
        let mut p = param(0.0, Rate::A);
        p.set_value(3.0e38, 0).unwrap();
        assert_eq!(p.value(), 3.0e38);
    }

    #[test]
    fn started_and_reached_on_exact_boundaries() {
        // A begin exactly on a quantum start whose float32 rounds down: started in that quantum.
        let b = 256.0 / 44100.0;
        let bf = b as f32;
        assert!(f64::from(bf) < b);
        assert!(started(current_time(256), bf));
        assert!(!started(current_time(128), bf));
        // Compared on the double: the unrounded begin would not start there.
        assert!(current_time(256) <= b);
        // A begin whose float32 rounds down below a boundary: started in that very quantum.
        let t = 53376.0 / 44100.0; // measure-worklet-start, offset 0 with f32(t) > t
        assert!(f64::from(t as f32) > t);
        assert!(!started(current_time(53376), t as f32));
        let t = 70912.0 / 44100.0; // offset 0 with f32(t) < t
        assert!(f64::from(t as f32) < t);
        assert!(started(current_time(70912), t as f32));
        assert!(!started(current_time(70912 - 128), t as f32));
        // The comparison is strict: ct == begin is not started.
        assert!(!started(0.0, 0.0));
        assert!(started(current_time(128), 0.0));
        // reached is `>=`.
        assert!(reached(0.0, 0.0));
        let e = 0.5f32;
        assert!(reached(0.5, e));
        assert!(!reached(f64::from(e) - 1e-12, e));
        // The end compared as float32: an end just above a quantum start rounds below it.
        let end = 22016.0 / 44100.0 + 1e-12;
        assert!(current_time(22016) < end);
        assert!(reached(current_time(22016), end as f32));
        assert!(!reached(current_time(22016), 0.4993));
    }

    #[test]
    fn lifecycle_rules() {
        // `false` with a connected input does not finish the processor.
        let mut l = Lifecycle::new();
        assert!(l.begin_quantum(true));
        l.finish_quantum(Ok(false), true);
        assert!(!l.gone());
        assert!(
            l.begin_quantum(true),
            "still called while the input is connected"
        );
        // Then the input goes: finished at the next begin_quantum.
        l.finish_quantum(Ok(false), true);
        assert!(!l.begin_quantum(false));
        assert!(l.gone());
        assert!(!l.begin_quantum(true), "never called again");
        // `true` keeps an active source alive without input.
        let mut l = Lifecycle::new();
        assert!(l.begin_quantum(false));
        l.finish_quantum(Ok(true), false);
        assert!(!l.gone());
        assert!(l.begin_quantum(false));
        // A source finishes at its first `false`.
        l.finish_quantum(Ok(false), false);
        assert!(l.gone());
        assert!(!l.begin_quantum(false));
        // An error finishes it for good, even with a connected input.
        let mut l = Lifecycle::new();
        assert!(l.begin_quantum(true));
        l.finish_quantum(
            Err(ProcessorError {
                processor: "distort",
                what: "x",
            }),
            true,
        );
        assert!(l.gone());
        assert!(!l.begin_quantum(true));
        // A fresh node with no input is called (it starts as an active source).
        let mut l = Lifecycle::default();
        assert!(l.begin_quantum(false));
    }

    #[test]
    fn processor_error_message() {
        let e = ProcessorError {
            processor: "distort",
            what: "algorithm is not a function",
        };
        assert_eq!(
            e.message(),
            "distort: processor error; Chromium outputs silence"
        );
    }

    #[test]
    fn fill_block_one_event_agrees_with_the_128_value_path() {
        // `.value = 0.3` at the pause frame: the a-rate block equals the k-rate value repeated
        // (Blink's FinalValue path) in the event's quantum and after it.
        let mut a = param(0.0, Rate::A);
        a.set_value(0.3, 256).unwrap();
        let mut k = param(0.0, Rate::K);
        k.set_value(0.3, 256).unwrap();
        for q in [256, 384, 4096] {
            let (mut va, mut vk) = ([0.0f32; QUANTUM], [0.0f32; QUANTUM]);
            fill_block(&mut a, q, &ParamInputs::NONE, &mut va);
            fill_block(&mut k, q, &ParamInputs::NONE, &mut vk);
            assert!(va.iter().all(|x| x.to_bits() == 0.3f32.to_bits()), "q {q}");
            assert_eq!(va, vk, "q {q}");
        }
    }

    #[test]
    fn fill_block_replaces_a_nan_input_with_the_default() {
        let mut p = param(1.0, Rate::A);
        p.set_value(0.25, 0).unwrap();
        let mut buf = [0.5f32; QUANTUM];
        buf[7] = f32::NAN;
        let input = ParamInputs {
            bufs: vec![buf],
            connected: true,
        };
        let mut out = [0.0f32; QUANTUM];
        fill_block(&mut p, 0, &input, &mut out);
        assert_eq!(out[0], 0.75);
        assert_eq!(
            out[7], 1.0,
            "NaN replaced by the default, not the intrinsic value"
        );
        // Connected but silent (no buffers): the timeline value.
        let silent = ParamInputs {
            bufs: Vec::new(),
            connected: true,
        };
        fill_block(&mut p, 128, &silent, &mut out);
        assert!(out.iter().all(|x| *x == 0.25));
    }

    #[test]
    fn f32_output_stores() {
        // Round to nearest: 1 + 2^-24 + 2^-30 rounds up to 1 + 2^-23.
        let up = 1.0 + 2f64.powi(-24) + 2f64.powi(-30);
        assert_eq!((up as f32).to_bits(), (1.0f32 + f32::EPSILON).to_bits());
        // A tie goes to even: 1 + 2^-24 lies halfway between 1 and 1 + 2^-23 and stores 1.
        let tie = 1.0 + 2f64.powi(-24);
        assert_eq!((tie as f32).to_bits(), 1.0f32.to_bits());
        // And 1 + 3 * 2^-24 (halfway between 1 + 2^-23 and 1 + 2^-22) goes up to the even one.
        let tie2 = 1.0 + 3.0 * 2f64.powi(-24);
        assert_eq!(
            (tie2 as f32).to_bits(),
            (1.0f32 + 2.0 * f32::EPSILON).to_bits()
        );
    }

    #[test]
    fn first_absent_after_stop_rounds_up_to_a_quantum() {
        for (end, want) in [
            (0, 0),
            (1, Q),
            (127, Q),
            (128, Q),
            (129, 2 * Q),
            (256, 2 * Q),
        ] {
            assert_eq!(first_absent_after_stop(end), want, "end {end}");
        }
        // The measured teardown (measure-worklet-teardown): stop 13671 -> 13696.
        assert_eq!(first_absent_after_stop(13671), 13696);
        // Saturates instead of overflowing past the last quantum.
        assert_eq!(first_absent_after_stop(u64::MAX), u64::MAX);
    }

    #[test]
    fn first_quantum_reaching_compares_the_quantum_start_time() {
        assert_eq!(first_quantum_reaching(0.0), Some(0));
        assert_eq!(first_quantum_reaching(-1.0), Some(0));
        assert_eq!(first_quantum_reaching(f64::NEG_INFINITY), Some(0));
        assert_eq!(first_quantum_reaching(f64::NAN), None);
        assert_eq!(first_quantum_reaching(f64::INFINITY), None);
        assert_eq!(first_quantum_reaching(1e300), None);
        // Exactly a quantum start: that quantum (`>=`).
        assert_eq!(first_quantum_reaching(current_time(256)), Some(256));
        // Just after it: the next one.
        let t = current_time(256);
        assert_eq!(
            first_quantum_reaching(f64::from_bits(t.to_bits() + 1)),
            Some(384)
        );
        assert_eq!(
            first_quantum_reaching(f64::from_bits(t.to_bits() - 1)),
            Some(256)
        );
        // 0.11 s (frame 4851): the quantum at 4864.
        assert_eq!(first_quantum_reaching(0.11), Some(4864));
        assert!(current_time(4736) < 0.11 && current_time(4864) >= 0.11);
        // An f32 end widened (pulse): 0.5f32 is exact, frame 22050 -> 22144.
        assert_eq!(first_quantum_reaching(f64::from(0.5f32)), Some(22144));
    }
}
