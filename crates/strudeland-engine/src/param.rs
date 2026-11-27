// Port of Blink (Chromium 156.0.8078.4) modules/webaudio/audio_param_handler.cc, modules/webaudio/audio_param.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! The AudioParam timeline: `AudioParamHandler` (event insertion, pruning, the per-quantum value
//! computation `ValuesForFrameRangeImpl` with its helpers, `HasSampleAccurateValues`, the k-rate
//! `FinalValue`/`Value` and the a-rate `CalculateSampleAccurateValues`) and the JS `.value` setter
//! of `AudioParam`.
//!
//! Only the event types superdough voices and `offlineDuck` create are ported: `setValueAtTime`,
//! `linearRampToValueAtTime`, `exponentialRampToValueAtTime` and `cancelAndHoldAtTime`
//! (`CancelAndHoldAtTime`, `RemoveCancelledEvents`, `HandleCancelValues`, `ProcessCancelValues`,
//! `LinearRampAtTime`, `ExponentialRampAtTime`). `setTargetAtTime`, `setValueCurveAtTime` and
//! `cancelScheduledValues` are not, so their branches of the cancel code are absent.
//!
//! Times are seconds (`f64`), values `f32`; the context's current frame is passed in as `now`
//! (scheduling, main thread) or `quantum_start` (rendering, audio thread), so the type holds no
//! clock. The ARM build of Chromium (macOS arm64, the reference platform) takes the scalar loops
//! (the SSE2 blocks are x86 only) and compiles with `-ffp-contract=off`, so nothing here fuses a
//! multiply-add. `fdlibm::pow` is `libm::pow` (map decision 18).

use std::borrow::Borrow;

use crate::QUANTUM;

use crate::SAMPLE_RATE_F64 as SR;
const RQ: f64 = QUANTUM as f64;

/// The automation rate of a param.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rate {
    /// a-rate: one value per frame.
    A,
    /// k-rate: one value per render quantum (its first frame).
    K,
}

/// The type of a timeline event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RampKind {
    /// `setValueAtTime`.
    SetValue,
    /// `linearRampToValueAtTime`.
    Linear,
    /// `exponentialRampToValueAtTime`.
    Exponential,
    /// The event `cancelAndHoldAtTime` inserts: it holds the value the cancelled ramp had at its
    /// time ([`CancelData`]).
    CancelValues,
}

/// The ramp a `CancelValues` event cancelled (Blink's `saved_event`): `Linear` or `Exponential`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavedRamp {
    pub kind: RampKind,
    pub value: f32,
    pub time: f64,
}

/// What a `CancelValues` event carries: the cancelled ramp and whether the held value has been
/// computed (`HasDefaultCancelledValue`; it is computed lazily, the first time the audio thread
/// renders the segment before the event, and then stored as the event's `value`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CancelData {
    pub saved: SavedRamp,
    pub value_cached: bool,
}

/// One timeline event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamEvent {
    pub kind: RampKind,
    pub value: f32,
    pub time: f64,
    /// Inserted since the last timeline evaluation (Blink's `new_events_`): its time is clamped to
    /// the start of the next evaluated range (`ClampNewEventsToCurrentTime`).
    pub is_new: bool,
    /// `Some` exactly for a `CancelValues` event.
    pub cancel: Option<CancelData>,
}

/// An exception the JS call would throw. Nothing is inserted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamError {
    /// WebIDL conversion of a `float` value (NaN, infinite, or rounding to an infinite `f32`).
    NonFiniteValue { method: &'static str },
    /// WebIDL conversion of a `double` time (NaN or infinite).
    NonFiniteTime { method: &'static str },
    /// `IsNonNegativeAudioParamTime` (a `RangeError`).
    NegativeTime { method: &'static str, time: f64 },
    /// `exponentialRampToValueAtTime` with a target of 0 (a `RangeError`).
    ZeroTarget,
}

impl std::fmt::Display for ParamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let on = |m: &str| format!("Failed to execute '{m}' on 'AudioParam'");
        match self {
            ParamError::NonFiniteValue { method } => write!(
                f,
                "TypeError: {}: The provided float value is non-finite.",
                on(method)
            ),
            ParamError::NonFiniteTime { method } => write!(
                f,
                "TypeError: {}: The provided double value is non-finite.",
                on(method)
            ),
            ParamError::NegativeTime { method, time } => write!(
                f,
                "RangeError: {}: Time must be a finite non-negative number: {}",
                on(method),
                crate::js::number_to_string(*time)
            ),
            ParamError::ZeroTarget => write!(
                f,
                "RangeError: {}: The float target value provided (0) should not be in the range \
                 (-1.40130e-45, 1.40130e-45).",
                on("exponentialRampToValueAtTime")
            ),
        }
    }
}

impl std::error::Error for ParamError {}

const SET_VALUE: &str = "setValueAtTime";
const LINEAR: &str = "linearRampToValueAtTime";
const EXPONENTIAL: &str = "exponentialRampToValueAtTime";
const CANCEL_AND_HOLD: &str = "cancelAndHoldAtTime";

/// WebIDL `float` conversion of a JS number passed to `method`: NaN, infinities and values that
/// round to an infinite `f32` throw a `TypeError`.
pub fn idl_float(x: f64, method: &'static str) -> Result<f32, ParamError> {
    let y = x as f32;
    if x.is_finite() && y.is_finite() {
        Ok(y)
    } else {
        Err(ParamError::NonFiniteValue { method })
    }
}

/// `ClampTo(value, min, max)`.
pub(crate) fn clamp_to(value: f32, min: f32, max: f32) -> f32 {
    if value >= max {
        max
    } else if value <= min {
        min
    } else {
        value
    }
}

/// `vector_math::Vclip` on one value (NEON `vminq(vmaxq(x, lo), hi)`).
fn vclip(x: f32, lo: f32, hi: f32) -> f32 {
    let x = if x < lo { lo } else { x };
    if x > hi { hi } else { x }
}

/// One param's connection inputs for one quantum: the buffers of the non-silent connections in
/// connection order, and whether the param has any rendering connection at all (true even when
/// every connected buffer is silent, as Blink's `HasSampleAccurateValues` sees it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParamInputs {
    pub bufs: Vec<[f32; QUANTUM]>,
    pub connected: bool,
}

impl ParamInputs {
    /// An unmodulated param: no buffers, not connected.
    pub const NONE: ParamInputs = ParamInputs {
        bufs: Vec::new(),
        connected: false,
    };
}

/// An AudioParam: its intrinsic value, nominal range, rate and event timeline. Modulation inputs
/// (connections into the param) are summed by the caller's buffers, see [`Param::fill_quantum`].
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    intrinsic: f32,
    default: f32,
    min: f32,
    max: f32,
    rate: Rate,
    events: Vec<ParamEvent>,
}

impl Param {
    /// A param whose intrinsic value is `default`.
    pub fn new(default: f32, min: f32, max: f32, rate: Rate) -> Self {
        Param {
            intrinsic: default,
            default,
            min,
            max,
            rate,
            events: Vec::new(),
        }
    }

    /// A `GainNode.gain` (default 1, range the whole float range, a-rate).
    pub fn gain(default: f32) -> Self {
        Param::new(default, -f32::MAX, f32::MAX, Rate::A)
    }

    pub fn rate(&self) -> Rate {
        self.rate
    }

    #[cfg(test)]
    pub fn default_value(&self) -> f32 {
        self.default
    }

    #[cfg(test)]
    pub fn min_value(&self) -> f32 {
        self.min
    }

    #[cfg(test)]
    pub fn max_value(&self) -> f32 {
        self.max
    }

    /// The timeline (tests and callers that inspect what was scheduled).
    #[cfg(test)]
    pub fn events(&self) -> &[ParamEvent] {
        &self.events
    }

    /// The JS `param.value` getter off the audio thread: the intrinsic value.
    pub fn value(&self) -> f32 {
        self.intrinsic
    }

    /// `AudioParamHandler::SetValue`: stores `value` clamped to the nominal range (a NaN, which
    /// WebIDL makes unreachable from JS, stores the default).
    fn set_intrinsic(&mut self, value: f32) {
        self.intrinsic = if value.is_nan() {
            self.default
        } else {
            clamp_to(value, self.min, self.max)
        };
    }

    /// A node constructor's option for this param (`new GainNode(ac, { gain: v })`): the WebIDL
    /// float `v` (a non-finite one throws for `option`), clamped to the range, as the intrinsic
    /// value **without** a timeline event; `default_value()` is unchanged (it is what a NaN
    /// connection input is replaced by).
    pub fn set_initial_value(&mut self, v: f64, option: &'static str) -> Result<(), ParamError> {
        let v = idl_float(v, option)?;
        self.set_intrinsic(v);
        Ok(())
    }

    /// The JS setter `param.value = v` (`AudioParam::setValue`): the clamped intrinsic value, then
    /// `setValueAtTime(intrinsic, currentTime)`. Every `.value =` therefore creates an event.
    pub fn set_value(&mut self, v: f32, now: u64) -> Result<(), ParamError> {
        if !v.is_finite() {
            return Err(ParamError::NonFiniteValue { method: "value" });
        }
        self.set_intrinsic(v);
        self.set_value_at_time(self.intrinsic, now as f64 / SR, now)
    }

    /// The argument checks of the three event methods, in the order JS meets them (WebIDL
    /// conversions, then `IsNonNegativeAudioParamTime`). The caller then applies
    /// `ClampedToCurrentTime`.
    fn check(v: f32, time: f64, method: &'static str) -> Result<f64, ParamError> {
        if !v.is_finite() {
            return Err(ParamError::NonFiniteValue { method });
        }
        if !time.is_finite() {
            return Err(ParamError::NonFiniteTime { method });
        }
        if time < 0.0 {
            return Err(ParamError::NegativeTime { method, time });
        }
        Ok(time)
    }

    /// `setValueAtTime(v, time)` called at context frame `now`.
    pub fn set_value_at_time(&mut self, v: f32, time: f64, now: u64) -> Result<(), ParamError> {
        let time = Self::check(v, time, SET_VALUE)?.max(now as f64 / SR);
        self.insert(RampKind::SetValue, v, time, now);
        Ok(())
    }

    /// `linearRampToValueAtTime(v, time)` called at context frame `now`.
    pub fn linear_ramp_to(&mut self, v: f32, time: f64, now: u64) -> Result<(), ParamError> {
        let time = Self::check(v, time, LINEAR)?.max(now as f64 / SR);
        self.insert(RampKind::Linear, v, time, now);
        Ok(())
    }

    /// `exponentialRampToValueAtTime(v, time)` called at context frame `now`; a target of 0 throws.
    pub fn exponential_ramp_to(&mut self, v: f32, time: f64, now: u64) -> Result<(), ParamError> {
        let time = Self::check(v, time, EXPONENTIAL)?;
        if v == 0.0 {
            return Err(ParamError::ZeroTarget);
        }
        let time = time.max(now as f64 / SR);
        self.insert(RampKind::Exponential, v, time, now);
        Ok(())
    }

    /// A new event of `kind` through [`Param::insert_event`].
    fn insert(&mut self, kind: RampKind, value: f32, time: f64, now: u64) {
        self.insert_event(
            ParamEvent {
                kind,
                value,
                time,
                is_new: true,
                cancel: None,
            },
            now,
        );
    }

    /// `InsertEvent`: prunes the events already in the past at `now`, gives a linear or
    /// exponential ramp on an empty timeline a starting `SetValue(intrinsic, now)` (a
    /// `CancelValues` on an emptied timeline is inserted alone), then inserts after every event
    /// whose time is `<=` the new one (equal times keep insertion order).
    fn insert_event(&mut self, event: ParamEvent, now: u64) {
        let n = self.events.len();
        if n > 0 {
            let mut skipped = 0;
            for i in 0..n {
                if self.is_event_current(i, now) {
                    break;
                }
                skipped += 1;
            }
            if skipped > 0 {
                self.remove_old_events(skipped);
            }
        }
        if self.events.is_empty() && matches!(event.kind, RampKind::Linear | RampKind::Exponential)
        {
            self.events.push(ParamEvent {
                kind: RampKind::SetValue,
                value: self.intrinsic,
                time: now as f64 / SR,
                is_new: true,
                cancel: None,
            });
        }
        let time = event.time;
        let at = match self.events.last() {
            Some(last) if last.time > time => self.events.partition_point(|e| e.time <= time),
            _ => self.events.len(),
        };
        self.events.insert(at, event);
    }

    /// `cancelAndHoldAtTime(time)` called at context frame `now` (`CancelAndHoldAtTime`): the
    /// time is clamped to `now`; with no event after it nothing changes. Otherwise every event
    /// after it is removed (events at exactly the cancel time stay), and when the first removed
    /// event is a linear or exponential ramp a `CancelValues` event at the cancel time saves it,
    /// to hold the value that ramp has there.
    pub fn cancel_and_hold_at_time(&mut self, time: f64, now: u64) -> Result<(), ParamError> {
        if !time.is_finite() {
            return Err(ParamError::NonFiniteTime {
                method: CANCEL_AND_HOLD,
            });
        }
        if time < 0.0 {
            return Err(ParamError::NegativeTime {
                method: CANCEL_AND_HOLD,
                time,
            });
        }
        let cancel_time = time.max(now as f64 / SR);
        let n = self.events.len();
        let i = self
            .events
            .iter()
            .position(|e| e.time > cancel_time)
            .unwrap_or(n);
        if i == n {
            return Ok(());
        }
        let cancelled = self.events[i];
        let new = match cancelled.kind {
            RampKind::Linear | RampKind::Exponential => Some(ParamEvent {
                kind: RampKind::CancelValues,
                value: 0.0,
                time: cancel_time,
                is_new: true,
                cancel: Some(CancelData {
                    saved: SavedRamp {
                        kind: cancelled.kind,
                        value: cancelled.value,
                        time: cancelled.time,
                    },
                    value_cached: false,
                }),
            }),
            RampKind::SetValue | RampKind::CancelValues => None,
        };
        // `RemoveCancelledEvents`.
        self.events.truncate(i);
        if let Some(event) = new {
            self.insert_event(event, now);
        }
        Ok(())
    }

    /// `IsEventCurrent` for event `i` at `current_frame`.
    fn is_event_current(&self, i: usize, current_frame: u64) -> bool {
        let Some(next) = self.events.get(i + 1) else {
            return true;
        };
        let current = current_frame as f64;
        if next.time * SR >= current {
            return true;
        }
        let event = &self.events[i];
        let event_frame = event.time * SR;
        event.kind == RampKind::SetValue && event_frame <= current && current < event_frame + 1.0
    }

    /// `RemoveOldEvents`: drops the first `count` events, always leaving at least one.
    fn remove_old_events(&mut self, count: usize) {
        let n = self.events.len();
        if n > 1 {
            self.events.drain(..count.min(n - 1));
        }
    }

    /// `HasSampleAccurateValues` at the quantum starting at `quantum_start`. `has_inputs`: the
    /// param has rendering connections. Not monotonic (02 §7).
    pub fn has_sample_accurate_values(&self, quantum_start: u64, has_inputs: bool) -> bool {
        if has_inputs {
            return true;
        }
        let Some(first) = self.events.first() else {
            return false;
        };
        let q = quantum_start as f64;
        if first.time > (q + RQ) / SR && first.kind == RampKind::SetValue {
            return false;
        }
        if self.events.len() >= 2 {
            return true;
        }
        // Blink's `!(time < current_frame / sample_rate)`; event times are always finite.
        first.time >= q / SR
    }

    /// `FinalValue()`: the k-rate value of the quantum starting at `quantum_start` (the timeline
    /// at its first frame, or the intrinsic value before the first event), stored as the
    /// intrinsic value, plus the first frame of each connected input (`inputs`, in connection
    /// order), NaN replaced by the default and clipped.
    pub fn final_value(&mut self, quantum_start: u64, inputs: &[f32]) -> f32 {
        self.final_value_connected(quantum_start, inputs, !inputs.is_empty())
    }

    /// [`Param::final_value`] for a param whose `connected` state is known apart from its
    /// non-silent `inputs` (silent connections are skipped, but the NaN replacement and the
    /// clip apply whenever the param has a connection).
    pub fn final_value_connected(
        &mut self,
        quantum_start: u64,
        inputs: &[f32],
        connected: bool,
    ) -> f32 {
        self.final_value_of(quantum_start, inputs.iter().copied(), connected)
    }

    /// [`Param::final_value_connected`] with the inputs given as their quantum buffers (only
    /// each buffer's first frame is summed), without collecting the first frames.
    pub fn final_value_of_bufs(
        &mut self,
        quantum_start: u64,
        bufs: &[[f32; QUANTUM]],
        connected: bool,
    ) -> f32 {
        self.final_value_of(quantum_start, bufs.iter().map(|b| b[0]), connected)
    }

    fn final_value_of(
        &mut self,
        quantum_start: u64,
        inputs: impl Iterator<Item = f32>,
        connected: bool,
    ) -> f32 {
        let mut v = [0.0f32];
        self.k_rate_values(quantum_start, &mut v);
        let mut any = false;
        for x in inputs {
            v[0] += x;
            any = true;
        }
        self.finish_inputs(&mut v, connected || any);
        v[0]
    }

    /// `Value()` on the audio thread (a node's non-sample-accurate path): the k-rate timeline
    /// value without inputs, stored as the intrinsic value.
    pub fn audio_value(&mut self, quantum_start: u64) -> f32 {
        let mut v = [0.0f32];
        self.k_rate_values(quantum_start, &mut v);
        self.intrinsic
    }

    /// `CalculateSampleAccurateValues()` for the quantum at `quantum_start`: a-rate, the 128
    /// timeline values (the intrinsic value becomes the last one, before the inputs) plus each
    /// input sample by sample in connection order; k-rate, the [`Param::final_value`] repeated
    /// (only each input's first frame is summed). With inputs, NaN becomes the default and the
    /// sum is clipped to the nominal range.
    pub fn fill_quantum(
        &mut self,
        quantum_start: u64,
        out: &mut [f32; QUANTUM],
        inputs: &[&[f32; QUANTUM]],
    ) {
        self.fill_quantum_connected(quantum_start, out, inputs, !inputs.is_empty());
    }

    /// [`Param::fill_quantum`] for a param whose `connected` state is known apart from its
    /// non-silent `inputs` (see [`Param::final_value_connected`]).
    pub fn fill_quantum_connected<B: Borrow<[f32; QUANTUM]>>(
        &mut self,
        quantum_start: u64,
        out: &mut [f32; QUANTUM],
        inputs: &[B],
        connected: bool,
    ) {
        match self.rate {
            Rate::A => {
                let back = self.values_for_frame_range(
                    quantum_start,
                    quantum_start + QUANTUM as u64,
                    self.intrinsic,
                    out,
                );
                self.set_intrinsic(back);
                for input in inputs {
                    for (o, x) in out.iter_mut().zip(input.borrow().iter()) {
                        *o += *x;
                    }
                }
            }
            Rate::K => {
                self.k_rate_values(quantum_start, &mut out[..1]);
                for input in inputs {
                    out[0] += input.borrow()[0];
                }
                let first = out[0];
                out.fill(first);
            }
        }
        self.finish_inputs(out, connected || !inputs.is_empty());
    }

    /// The k-rate branch of `CalculateFinalValues`: `ValueForContextTime` (one value at the
    /// quantum's first frame) or the intrinsic value, filled and stored.
    fn k_rate_values(&mut self, quantum_start: u64, values: &mut [f32]) {
        let mut value = self.intrinsic;
        let starts_later = self
            .events
            .first()
            .is_none_or(|e| (quantum_start as f64 / SR) < e.time);
        if !starts_later {
            let mut one = [0.0f32];
            value = self.values_for_frame_range(quantum_start, quantum_start + 1, value, &mut one);
        }
        values.fill(value);
        self.set_intrinsic(value);
    }

    /// The connection tail of `CalculateFinalValues`: NaN to default, then `Vclip`.
    fn finish_inputs(&self, values: &mut [f32], has_inputs: bool) {
        if !has_inputs {
            return;
        }
        for v in values.iter_mut() {
            if v.is_nan() {
                *v = self.default;
            }
            *v = vclip(*v, self.min, self.max);
        }
    }

    /// `ValuesForFrameRange`: the timeline for frames `[start, end)` into `values`, clipped to the
    /// nominal range; returns the last value.
    fn values_for_frame_range(
        &mut self,
        start: u64,
        end: u64,
        default: f32,
        values: &mut [f32],
    ) -> f32 {
        self.values_for_frame_range_impl(start, end, default, values);
        for v in values.iter_mut() {
            *v = vclip(*v, self.min, self.max);
        }
        values.last().copied().unwrap_or(default)
    }

    /// `ValuesForFrameRangeImpl` (SetTarget and SetValueCurve not ported).
    fn values_for_frame_range_impl(
        &mut self,
        start: u64,
        end: u64,
        mut default: f32,
        values: &mut [f32],
    ) {
        let end_time = end as f64 / SR;
        if self.events.is_empty() || end_time <= self.events[0].time {
            values.fill(default);
            return;
        }
        let n = self.events.len();
        let current_time = start as f64 / SR;
        if self.events.iter().any(|e| e.is_new) {
            self.clamp_new_events_to_current_time(current_time);
        }
        if self.handle_all_events_in_the_past(current_time, &mut default, values) {
            return;
        }
        let (mut current_frame, mut write_index) =
            self.handle_first_event(values, default, start, end);
        let mut value = default;
        let mut skipped = 0;
        let len = values.len();
        for i in 0..n {
            if write_index >= len {
                break;
            }
            if !self.is_event_current(i, current_frame) {
                skipped += 1;
                continue;
            }
            let event = self.events[i];
            let next = self.events.get(i + 1).copied();
            let (value1, time1) = (event.value, event.time);
            let (value2, time2, next_kind) = self.handle_cancel_values(
                i,
                next.map_or(value1, |e| e.value),
                next.map_or(end_time + 1.0, |e| e.time),
            );
            let fill_to_end_frame = if (end as f64) > time2 * SR {
                (time2 * SR).ceil() as u64
            } else {
                end
            };
            // Blink CHECKs `fill_to_frame >= write_index`; the clamp keeps a defect from panicking.
            let fill_to_frame = (fill_to_end_frame.saturating_sub(start) as usize)
                .min(len)
                .max(write_index);
            match next_kind {
                Some(RampKind::Linear) => {
                    (current_frame, value, write_index) = process_linear_ramp(
                        fill_to_frame,
                        time1,
                        time2,
                        value1,
                        value2,
                        values,
                        current_frame,
                        value,
                        write_index,
                    );
                }
                Some(RampKind::Exponential) => {
                    (current_frame, value, write_index) = process_exponential_ramp(
                        fill_to_frame,
                        time1,
                        time2,
                        value1,
                        value2,
                        values,
                        current_frame,
                        write_index,
                    );
                }
                _ if event.kind == RampKind::CancelValues => {
                    (current_frame, value, write_index) = process_cancel_values(
                        fill_to_frame,
                        fill_to_end_frame,
                        &event,
                        values,
                        value,
                        write_index,
                    );
                }
                _ => {
                    // SetValue, or a ramp whose end is reached: hold the event's value.
                    current_frame = fill_to_end_frame;
                    value = event.value;
                    values[write_index..fill_to_frame].fill(value);
                    write_index = fill_to_frame;
                }
            }
        }
        if skipped > 0 {
            self.remove_old_events(skipped);
        }
        values[write_index..].fill(value);
    }

    /// `HandleCancelValues` for event `i` with the next event's `(value2, time2)` (or the
    /// defaults when there is none): when the next event is a `CancelValues`, the segment runs
    /// as the cancelled ramp's kind to the cancel time, ending at the ramp's value there
    /// (computed and cached on the `CancelValues` event the first time). Returns the segment's
    /// end value, end time and the kind that shapes it (the next event's kind, if any).
    fn handle_cancel_values(
        &mut self,
        i: usize,
        value2: f32,
        time2: f64,
    ) -> (f32, f64, Option<RampKind>) {
        let current = self.events[i];
        let Some(next) = self.events.get_mut(i + 1) else {
            return (value2, time2, None);
        };
        let Some(cancel) = next.cancel.filter(|_| next.kind == RampKind::CancelValues) else {
            return (value2, time2, Some(next.kind));
        };
        // Every ported kind of `current` (SetValue, the ramps, CancelValues) establishes a
        // starting value, so the cancelled ramp is examined.
        let saved = cancel.saved;
        let value2 = if cancel.value_cached {
            next.value
        } else {
            let (value1, time1) = (current.value, current.time);
            let v = match saved.kind {
                RampKind::Linear => {
                    linear_ramp_at_time(next.time, value1, time1, saved.value, saved.time)
                }
                RampKind::Exponential => {
                    exponential_ramp_at_time(next.time, value1, time1, saved.value, saved.time)
                }
                // A saved ramp is always linear or exponential (`cancel_and_hold_at_time`).
                RampKind::SetValue | RampKind::CancelValues => value1,
            };
            // `SetCancelledValue`.
            next.value = v;
            next.cancel = Some(CancelData {
                saved,
                value_cached: true,
            });
            v
        };
        (value2, next.time, Some(saved.kind))
    }

    /// `ClampNewEventsToCurrentTime`: events inserted since the last evaluation move up to
    /// `current_time`; a stable re-sort by time keeps equal times in their order.
    fn clamp_new_events_to_current_time(&mut self, current_time: f64) {
        let mut clamped = false;
        for e in self.events.iter_mut().filter(|e| e.is_new) {
            if e.time < current_time {
                e.time = current_time;
                clamped = true;
            }
        }
        if clamped {
            self.events.sort_by(|a, b| a.time.total_cmp(&b.time));
        }
        for e in &mut self.events {
            e.is_new = false;
        }
    }

    /// `HandleAllEventsInThePast`: once the last event is more than 1.5 quanta in the past, the
    /// value is the passed-in default (the intrinsic value) and the timeline shrinks to one event.
    fn handle_all_events_in_the_past(
        &mut self,
        current_time: f64,
        default: &mut f32,
        values: &mut [f32],
    ) -> bool {
        let Some(last) = self.events.last() else {
            return false;
        };
        if last.time + 1.5 * RQ / SR < current_time {
            values.fill(*default);
            self.remove_old_events(self.events.len());
            return true;
        }
        false
    }

    /// `HandleFirstEvent`: the default value up to the first event's frame `ceil(time * sr)`.
    fn handle_first_event(
        &self,
        values: &mut [f32],
        default: f32,
        start: u64,
        end: u64,
    ) -> (u64, usize) {
        let first_time = self.events[0].time;
        let mut current_frame = start;
        let mut write_index = 0;
        if first_time > start as f64 / SR {
            let first_frame = (first_time * SR).ceil();
            let fill_to_end_frame = if (end as f64) > first_frame {
                first_frame as u64
            } else {
                end
            };
            let fill_to_frame =
                (fill_to_end_frame.saturating_sub(start) as usize).min(values.len());
            values[..fill_to_frame].fill(default);
            write_index = fill_to_frame;
            current_frame += fill_to_frame as u64;
        }
        (current_frame, write_index)
    }
}

/// `LinearRampAtTime`: the value at `t` of a linear ramp from `(time1, value1)` to
/// `(time2, value2)`, `value1 + (value2 - value1) * (t - time1) / (time2 - time1)` with the
/// float difference, the rest in double and one rounding to float.
pub(crate) fn linear_ramp_at_time(t: f64, value1: f32, time1: f64, value2: f32, time2: f64) -> f32 {
    (f64::from(value1) + f64::from(value2 - value1) * (t - time1) / (time2 - time1)) as f32
}

/// `ExponentialRampAtTime`: `value1` when it is 0 or the signs differ, else
/// `value1 * fdlibm::pow(value2 / value1, (t - time1) / (time2 - time1))` with the float
/// quotient, the power and product in double and one rounding to float.
pub(crate) fn exponential_ramp_at_time(
    t: f64,
    value1: f32,
    time1: f64,
    value2: f32,
    time2: f64,
) -> f32 {
    if value1 == 0.0 || value1.is_sign_negative() != value2.is_sign_negative() {
        return value1;
    }
    (f64::from(value1) * libm::pow(f64::from(value2 / value1), (t - time1) / (time2 - time1)))
        as f32
}

/// `ProcessCancelValues`: holds the cached value (or, for an event never cached, the running
/// `value`; Blink's `SetTarget` correction does not apply) up to the next event.
fn process_cancel_values(
    fill_to_frame: usize,
    fill_to_end_frame: u64,
    event: &ParamEvent,
    values: &mut [f32],
    value: f32,
    write_index: usize,
) -> (u64, f32, usize) {
    let value = if event.cancel.is_some_and(|c| c.value_cached) {
        event.value
    } else {
        value
    };
    values[write_index..fill_to_frame].fill(value);
    (fill_to_end_frame, value, fill_to_frame)
}

/// `ProcessLinearRamp` (scalar path): `value1 + x * (value2 - value1)` with `x = (t - time1) * k`
/// rounded to `f32` and `k = 1 / (time2 - time1)` as `f32` (0 below `FLT_MIN`).
#[allow(clippy::too_many_arguments)]
fn process_linear_ramp(
    fill_to_frame: usize,
    time1: f64,
    time2: f64,
    value1: f32,
    value2: f32,
    values: &mut [f32],
    mut current_frame: u64,
    mut value: f32,
    write_index: usize,
) -> (u64, f32, usize) {
    let delta_time = time2 - time1;
    let k: f32 = if delta_time <= f64::from(f32::MIN_POSITIVE) {
        0.0
    } else {
        (1.0 / delta_time) as f32
    };
    let value_delta = value2 - value1;
    for v in &mut values[write_index..fill_to_frame] {
        let x = ((current_frame as f64 / SR - time1) * f64::from(k)) as f32;
        value = value1 + x * value_delta;
        current_frame += 1;
        *v = value;
    }
    (current_frame, value, fill_to_frame)
}

/// `ProcessExponentialRamp`: holds `value1` when the values have opposite signs or one is 0 (the
/// product in `f32`) or the times do not increase; else `V * m^k` in a double accumulator, and the
/// value snaps to `value2` once the current frame passes `time2 * sr - 0.5`.
#[allow(clippy::too_many_arguments)]
fn process_exponential_ramp(
    fill_to_frame: usize,
    time1: f64,
    time2: f64,
    value1: f32,
    value2: f32,
    values: &mut [f32],
    mut current_frame: u64,
    write_index: usize,
) -> (u64, f32, usize) {
    let mut value;
    if value1 * value2 <= 0.0 || time1 >= time2 {
        value = value1;
        values[write_index..fill_to_frame].fill(value);
    } else {
        let delta_time = time2 - time1;
        let num_sample_frames = delta_time * SR;
        let multiplier = libm::pow(f64::from(value2 / value1), 1.0 / num_sample_frames);
        value = (f64::from(value1)
            * libm::pow(
                f64::from(value2) / f64::from(value1),
                (current_frame as f64 / SR - time1) / delta_time,
            )) as f32;
        let mut accumulator = f64::from(value);
        for v in &mut values[write_index..fill_to_frame] {
            value = accumulator as f32;
            accumulator *= multiplier;
            current_frame += 1;
            *v = value;
        }
        if current_frame as f64 > time2 * SR - 0.5 {
            value = value2;
        }
    }
    (current_frame, value, fill_to_frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::QUANTUM_U64 as Q;

    /// The a-rate values of frames `[0, n)`, quantum by quantum from frame 0.
    fn render(p: &mut Param, n: u64) -> Vec<f32> {
        render_from(p, 0, n)
    }

    fn render_from(p: &mut Param, from: u64, n: u64) -> Vec<f32> {
        let mut out = Vec::new();
        let mut q = from;
        while q < from + n {
            let mut buf = [0.0f32; QUANTUM];
            p.fill_quantum(q, &mut buf, &[]);
            out.extend_from_slice(&buf);
            q += Q;
        }
        out.truncate(n as usize);
        out
    }

    fn gain() -> Param {
        Param::gain(1.0)
    }

    /// Blink's scalar linear ramp value at `frame`.
    fn linear_at(frame: u64, t1: f64, v1: f32, t2: f64, v2: f32) -> f32 {
        let k = (1.0 / (t2 - t1)) as f32;
        let x = ((frame as f64 / SR - t1) * f64::from(k)) as f32;
        v1 + x * (v2 - v1)
    }

    #[test]
    fn construction_and_the_value_setter() {
        let mut p = Param::new(0.5, -1.0, 1.0, Rate::A);
        assert_eq!(p.value(), 0.5);
        assert_eq!(p.default_value(), 0.5);
        assert_eq!(
            (p.min_value(), p.max_value(), p.rate()),
            (-1.0, 1.0, Rate::A)
        );
        assert!(p.events().is_empty());
        // `.value = 3` clamps the intrinsic value and inserts setValueAtTime(1, now).
        p.set_value(3.0, 256).unwrap();
        assert_eq!(p.value(), 1.0);
        assert_eq!(
            p.events(),
            &[ParamEvent {
                kind: RampKind::SetValue,
                value: 1.0,
                time: 256.0 / SR,
                is_new: true,
                cancel: None,
            }]
        );
        p.set_value(-7.0, 256).unwrap();
        assert_eq!(p.value(), -1.0);
        assert_eq!(p.events().len(), 2);
        assert_eq!(
            p.set_value(f32::NAN, 0),
            Err(ParamError::NonFiniteValue { method: "value" })
        );
        assert_eq!(p.value(), -1.0, "a failed set changes nothing");
    }

    #[test]
    fn events_apply_from_the_ceiling_of_their_frame() {
        // 10.5 frames: frames 0..=10 keep the intrinsic value, frame 11 has the new one.
        let mut p = gain();
        p.set_value_at_time(0.25, 10.5 / SR, 0).unwrap();
        let v = render(&mut p, 256);
        assert!(v[..11].iter().all(|x| *x == 1.0));
        assert!(v[11..].iter().all(|x| *x == 0.25));
        // Exactly on frame 10: frame 10 has it.
        let mut p = gain();
        p.set_value_at_time(0.25, 10.0 / SR, 0).unwrap();
        let v = render(&mut p, 256);
        assert_eq!((v[9], v[10]), (1.0, 0.25));
        // In a later quantum.
        let mut p = gain();
        p.set_value_at_time(0.25, 300.2 / SR, 0).unwrap();
        let v = render(&mut p, 512);
        assert_eq!((v[300], v[301]), (1.0, 0.25));
    }

    #[test]
    fn linear_ramps_between_fractional_frames() {
        let (t1, t2) = (10.25 / SR, 200.75 / SR);
        let mut p = gain();
        p.set_value_at_time(0.0, t1, 0).unwrap();
        p.linear_ramp_to(1.0, t2, 0).unwrap();
        let v = render(&mut p, 384);
        assert!(v[..11].iter().all(|x| *x == 1.0), "before the first event");
        for f in 11..=200 {
            assert_eq!(v[f as usize], linear_at(f, t1, 0.0, t2, 1.0), "frame {f}");
        }
        assert!(v[201..].iter().all(|x| *x == 1.0), "held after the end");
        assert!(v[11] > 0.0 && v[11] < 0.01);
        assert!(v[200] < 1.0 && v[200] > 0.99);
        // Downwards, across quanta, with the intrinsic value following the last frame.
        let mut p = gain();
        p.set_value_at_time(1.0, 0.0, 0).unwrap();
        p.linear_ramp_to(-1.0, 1000.0 / SR, 0).unwrap();
        let v = render(&mut p, 128);
        assert_eq!(p.value(), v[127]);
        assert_eq!(v[127], linear_at(127, 0.0, 1.0, 1000.0 / SR, -1.0));
    }

    #[test]
    fn a_ramp_with_no_time_jumps() {
        // set(0, t) then linear(1, t): k = 0, the ramp is empty and its end value holds.
        let t = 100.0 / SR;
        let mut p = gain();
        p.set_value_at_time(0.0, t, 0).unwrap();
        p.linear_ramp_to(1.0, t, 0).unwrap();
        let v = render(&mut p, 256);
        assert_eq!((v[99], v[100], v[200]), (1.0, 1.0, 1.0));
        let mut p = gain();
        p.set_value_at_time(0.5, 0.0, 0).unwrap();
        p.set_value_at_time(0.0, t, 0).unwrap();
        p.linear_ramp_to(0.75, t, 0).unwrap();
        let v = render(&mut p, 256);
        assert_eq!((v[99], v[100]), (0.5, 0.75));
    }

    #[test]
    fn exponential_ramps() {
        let (t1, t2) = (0.0, 1000.5 / SR);
        let mut p = gain();
        p.set_value_at_time(0.01, t1, 0).unwrap();
        p.exponential_ramp_to(1.0, t2, 0).unwrap();
        let v = render(&mut p, 1280);
        // V * m^k in a double accumulator, restarted each quantum from the closed form.
        let m = libm::pow(f64::from(1.0f32 / 0.01f32), 1.0 / (t2 * SR));
        for q in (0..1024).step_by(128) {
            let mut acc = f64::from(
                (f64::from(0.01f32) * libm::pow(1.0 / f64::from(0.01f32), (q as f64 / SR) / t2))
                    as f32,
            );
            for (f, x) in v.iter().enumerate().take((q + 128).min(1001)).skip(q) {
                assert_eq!(*x, acc as f32, "frame {f}");
                acc *= m;
            }
        }
        assert!(v[1001..].iter().all(|x| *x == 1.0));
        assert!(
            (v[500] - 0.1).abs() < 1e-3,
            "geometric mean half way: {}",
            v[500]
        );
        assert_eq!(v[0], 0.01);
        // Negative to negative works as well.
        let mut p = gain();
        p.set_value_at_time(-1.0, 0.0, 0).unwrap();
        p.exponential_ramp_to(-0.001, 100.0 / SR, 0).unwrap();
        let v = render(&mut p, 128);
        assert!(v[50] < -0.02 && v[50] > -0.04, "{}", v[50]);
        assert_eq!(v[100], -0.001);
    }

    #[test]
    fn exponential_ramps_through_zero_or_a_sign_change_hold() {
        for (v1, v2) in [(0.0f32, 1.0f32), (-1.0, 1.0), (1.0, -0.5), (1e-30, 1e-30)] {
            let mut p = gain();
            p.set_value_at_time(v1, 0.0, 0).unwrap();
            p.exponential_ramp_to(v2, 100.0 / SR, 0).unwrap();
            let v = render(&mut p, 256);
            // `1e-30 * 1e-30` underflows to 0 in f32: held as well.
            assert!(v[..100].iter().all(|x| *x == v1), "{v1} -> {v2}");
            assert!(
                v[100..].iter().all(|x| *x == v2),
                "{v1} -> {v2} after the end"
            );
        }
        // time1 >= time2 holds too (a setValue after the ramp's time).
        let mut p = gain();
        p.set_value_at_time(0.5, 0.0, 0).unwrap();
        p.exponential_ramp_to(0.25, 0.0, 0).unwrap();
        let v = render(&mut p, 128);
        assert!(v.iter().all(|x| *x == 0.25));
    }

    #[test]
    fn exponential_ramp_to_zero_throws() {
        let mut p = gain();
        assert_eq!(
            p.exponential_ramp_to(0.0, 1.0, 0),
            Err(ParamError::ZeroTarget)
        );
        assert_eq!(
            p.exponential_ramp_to(-0.0, 1.0, 0),
            Err(ParamError::ZeroTarget)
        );
        assert!(p.events().is_empty());
        assert!(
            ParamError::ZeroTarget
                .to_string()
                .starts_with("RangeError:")
        );
        // The time is checked first.
        assert_eq!(
            p.exponential_ramp_to(0.0, -1.0, 0),
            Err(ParamError::NegativeTime {
                method: EXPONENTIAL,
                time: -1.0
            })
        );
    }

    #[test]
    fn argument_errors() {
        let mut p = gain();
        assert_eq!(
            p.set_value_at_time(1.0, -0.5, 0),
            Err(ParamError::NegativeTime {
                method: SET_VALUE,
                time: -0.5
            })
        );
        assert_eq!(
            p.linear_ramp_to(1.0, f64::NAN, 0),
            Err(ParamError::NonFiniteTime { method: LINEAR })
        );
        assert_eq!(
            p.linear_ramp_to(f32::INFINITY, 1.0, 0),
            Err(ParamError::NonFiniteValue { method: LINEAR })
        );
        assert!(p.events().is_empty());
        assert_eq!(
            idl_float(1e39, SET_VALUE),
            Err(ParamError::NonFiniteValue { method: SET_VALUE })
        );
        assert!(idl_float(f64::NAN, SET_VALUE).is_err());
        assert_eq!(idl_float(1e-50, SET_VALUE), Ok(0.0));
        assert_eq!(idl_float(0.1, SET_VALUE), Ok(0.1f32));
        assert_eq!(
            ParamError::NegativeTime {
                method: SET_VALUE,
                time: -0.5
            }
            .to_string(),
            "RangeError: Failed to execute 'setValueAtTime' on 'AudioParam': Time must be a \
             finite non-negative number: -0.5"
        );
        assert!(
            ParamError::NonFiniteTime { method: LINEAR }
                .to_string()
                .contains("double")
        );
    }

    #[test]
    fn same_time_events_keep_insertion_order() {
        let t = 50.0 / SR;
        let mut p = gain();
        p.set_value_at_time(0.5, t, 0).unwrap();
        p.linear_ramp_to(0.2, t, 0).unwrap();
        p.set_value_at_time(0.7, t, 0).unwrap();
        let kinds: Vec<RampKind> = p.events().iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![RampKind::SetValue, RampKind::Linear, RampKind::SetValue]
        );
        let v = render(&mut p, 128);
        assert_eq!((v[49], v[50]), (1.0, 0.7), "the last inserted wins");
        // An earlier event goes before, a later one after (binary search).
        p.set_value_at_time(0.1, 10.0 / SR, 0).unwrap();
        p.set_value_at_time(0.9, 60.0 / SR, 0).unwrap();
        p.set_value_at_time(0.3, t, 0).unwrap();
        let times: Vec<f64> = p.events().iter().map(|e| e.time).collect();
        assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
        assert_eq!(p.events().iter().rfind(|e| e.time == t).unwrap().value, 0.3);
    }

    #[test]
    fn overlapping_ramps_follow_the_sorted_timeline() {
        // A ramp inserted before an existing ramp's end splits it: 0 -> 1 at 200 and -> 0.5 at 100.
        let mut p = gain();
        p.set_value_at_time(0.0, 0.0, 0).unwrap();
        p.linear_ramp_to(1.0, 200.0 / SR, 0).unwrap();
        p.linear_ramp_to(0.5, 100.0 / SR, 0).unwrap();
        let v = render(&mut p, 256);
        assert_eq!(v[100], 0.5);
        assert_eq!(v[150], linear_at(150, 100.0 / SR, 0.5, 200.0 / SR, 1.0));
        assert_eq!(v[200], 1.0);
    }

    #[test]
    fn events_before_now_are_clamped_and_ramps_get_a_start() {
        let mut p = gain();
        p.set_value_at_time(0.5, 0.0, 441).unwrap();
        assert_eq!(p.events()[0].time, 441.0 / SR);
        // A ramp on an empty timeline starts from setValueAtTime(intrinsic, now).
        let mut p = Param::gain(0.25);
        p.linear_ramp_to(1.0, 0.0, 128).unwrap();
        assert_eq!(p.events().len(), 2);
        assert_eq!(p.events()[0].kind, RampKind::SetValue);
        assert_eq!(
            (p.events()[0].value, p.events()[0].time),
            (0.25, 128.0 / SR)
        );
        assert_eq!(
            p.events()[1].time,
            128.0 / SR,
            "the ramp end is clamped to now"
        );
        let mut p = Param::gain(0.25);
        p.exponential_ramp_to(1.0, 1.0, 0).unwrap();
        assert_eq!(p.events()[0].value, 0.25);
    }

    #[test]
    fn new_events_are_clamped_to_the_first_evaluated_quantum() {
        // Scheduled at frame 0, first evaluated at frame 512: the ramp now starts at 512.
        let mut p = gain();
        p.set_value_at_time(0.0, 0.0, 0).unwrap();
        p.linear_ramp_to(1.0, 1024.0 / SR, 0).unwrap();
        let v = render_from(&mut p, 512, 128);
        assert_eq!(v[0], 0.0);
        assert_eq!(v[1], linear_at(513, 512.0 / SR, 0.0, 1024.0 / SR, 1.0));
        assert_eq!(p.events()[0].time, 512.0 / SR);
        assert!(p.events().iter().all(|e| !e.is_new));
        // Evaluated from frame 0, the same events give the unclamped ramp.
        let mut p = gain();
        p.set_value_at_time(0.0, 0.0, 0).unwrap();
        p.linear_ramp_to(1.0, 1024.0 / SR, 0).unwrap();
        let v = render(&mut p, 640);
        assert_eq!(v[513], linear_at(513, 0.0, 0.0, 1024.0 / SR, 1.0));
    }

    #[test]
    fn k_rate_values_are_the_first_frame_of_the_quantum() {
        let mut p = Param::new(1.0, -f32::MAX, f32::MAX, Rate::K);
        p.set_value_at_time(0.0, 0.0, 0).unwrap();
        p.linear_ramp_to(1.0, 1280.0 / SR, 0).unwrap();
        let mut buf = [0.0f32; QUANTUM];
        p.fill_quantum(0, &mut buf, &[]);
        assert!(buf.iter().all(|x| *x == 0.0));
        p.fill_quantum(128, &mut buf, &[]);
        let first = linear_at(128, 0.0, 0.0, 1280.0 / SR, 1.0);
        assert!(buf.iter().all(|x| *x == first));
        assert_eq!(p.value(), first, "k-rate stores the first frame");
        assert_eq!(
            p.final_value(256, &[]),
            linear_at(256, 0.0, 0.0, 1280.0 / SR, 1.0)
        );
        // a-rate stores the last frame.
        let mut a = gain();
        a.set_value_at_time(0.0, 0.0, 0).unwrap();
        a.linear_ramp_to(1.0, 1280.0 / SR, 0).unwrap();
        a.fill_quantum(0, &mut buf, &[]);
        a.fill_quantum(128, &mut buf, &[]);
        assert_eq!(a.value(), buf[127]);
        assert_eq!(buf[127], linear_at(255, 0.0, 0.0, 1280.0 / SR, 1.0));
    }

    #[test]
    fn k_rate_before_the_first_event_is_the_intrinsic_value() {
        let mut p = Param::gain(0.75);
        p.set_value_at_time(0.1, 1.0, 0).unwrap();
        assert_eq!(p.final_value(0, &[]), 0.75);
        assert_eq!(p.audio_value(128), 0.75);
        assert_eq!(p.audio_value(44160), 0.1);
        assert_eq!(p.value(), 0.1);
    }

    #[test]
    fn summed_inputs() {
        // a-rate: timeline + every input sample by sample, NaN -> default, clipped; the intrinsic
        // value is the timeline's last frame, before the inputs.
        let mut p = Param::new(0.5, -1.0, 1.0, Rate::A);
        let mut a = [0.25f32; QUANTUM];
        a[3] = f32::NAN;
        a[4] = 10.0;
        let b = [0.125f32; QUANTUM];
        let mut out = [0.0f32; QUANTUM];
        p.fill_quantum(0, &mut out, &[&a, &b]);
        assert_eq!(out[0], 0.875);
        assert_eq!(out[3], 0.5, "NaN becomes the default");
        assert_eq!(out[4], 1.0, "clipped to max");
        assert_eq!(p.value(), 0.5);
        // k-rate: only the first frame of each input.
        let mut k = Param::new(0.0, -10.0, 10.0, Rate::K);
        let mut c = [0.0f32; QUANTUM];
        c[0] = 2.0;
        c[1] = 100.0;
        k.fill_quantum(0, &mut out, &[&c]);
        assert!(out.iter().all(|x| *x == 2.0));
        assert_eq!(k.value(), 0.0);
        assert_eq!(k.final_value(0, &[3.0, 20.0]), 10.0, "summed then clipped");
        assert_eq!(k.final_value(0, &[f32::NAN]), 0.0);
        // Without inputs nothing is clipped beyond the timeline's own clip.
        let mut p = Param::new(0.0, -1.0, 1.0, Rate::A);
        p.set_value_at_time(5.0, 0.0, 0).unwrap();
        p.fill_quantum(0, &mut out, &[]);
        assert!(
            out.iter().all(|x| *x == 1.0),
            "timeline values are clipped (Vclip)"
        );
    }

    #[test]
    fn sample_accurate_predicate() {
        let mut p = gain();
        assert!(!p.has_sample_accurate_values(0, false), "no events");
        assert!(p.has_sample_accurate_values(0, true), "inputs");
        // One SetValue after the quantum's end: false; inside it: true; in the past: false.
        p.set_value_at_time(0.5, 300.0 / SR, 0).unwrap();
        assert!(!p.has_sample_accurate_values(0, false));
        assert!(p.has_sample_accurate_values(256, false));
        assert!(
            !p.has_sample_accurate_values(128, false),
            "300 is after (128 + 128)"
        );
        assert!(!p.has_sample_accurate_values(384, false), "past");
        // Two events: true even once both are past (until pruned).
        p.set_value_at_time(0.25, 310.0 / SR, 0).unwrap();
        assert!(p.has_sample_accurate_values(10_000, false));
        // A ramp first in the future is still sample accurate (only SetValue is exempt).
        let mut r = gain();
        r.linear_ramp_to(0.5, 1.0, 0).unwrap();
        assert!(r.has_sample_accurate_values(0, false));
        // Not monotonic: after rendering past the events they are pruned to one past event.
        let mut s = gain();
        s.set_value_at_time(0.0, 0.0, 0).unwrap();
        s.linear_ramp_to(1.0, 100.0 / SR, 0).unwrap();
        assert!(s.has_sample_accurate_values(0, false));
        render(&mut s, 1024);
        assert_eq!(s.events().len(), 1);
        assert!(!s.has_sample_accurate_values(1024, false));
    }

    #[test]
    fn long_past_timelines_return_the_intrinsic_value_and_shrink() {
        let mut p = gain();
        p.set_value_at_time(0.0, 0.0, 0).unwrap();
        p.linear_ramp_to(0.5, 10.0 / SR, 0).unwrap();
        p.set_value_at_time(0.25, 20.0 / SR, 0).unwrap();
        render(&mut p, 128);
        assert_eq!(p.value(), 0.25);
        // More than 1.5 quanta after the last event: the intrinsic value, one event left.
        let v = render_from(&mut p, 256, 128);
        assert!(v.iter().all(|x| *x == 0.25));
        assert_eq!(p.events().len(), 1);
        // The intrinsic value is what comes out, even if it was overwritten.
        p.set_intrinsic(0.125);
        let v = render_from(&mut p, 512, 128);
        assert!(v.iter().all(|x| *x == 0.125));
    }

    #[test]
    fn insertion_prunes_events_already_in_the_past() {
        let mut p = gain();
        p.set_value_at_time(0.0, 0.0, 0).unwrap();
        p.set_value_at_time(0.5, 100.0 / SR, 0).unwrap();
        p.set_value_at_time(0.75, 200.0 / SR, 0).unwrap();
        // At frame 1000 the first two are superseded; the last one stays.
        p.set_value_at_time(1.0, 2000.0 / SR, 1000).unwrap();
        let values: Vec<f32> = p.events().iter().map(|e| e.value).collect();
        assert_eq!(values, vec![0.75, 1.0]);
        // A SetValue whose frame is within one frame before `now` is kept.
        let mut p = gain();
        p.set_value_at_time(0.0, 0.0, 0).unwrap();
        p.set_value_at_time(0.5, 999.5 / SR, 0).unwrap();
        p.set_value_at_time(0.6, 999.7 / SR, 0).unwrap();
        p.set_value_at_time(1.0, 2000.0 / SR, 1000).unwrap();
        assert_eq!(p.events()[0].value, 0.5);
        assert_eq!(p.events().len(), 3);
    }

    #[test]
    fn remove_old_events_keeps_one() {
        let mut p = gain();
        p.set_value_at_time(0.0, 0.0, 0).unwrap();
        p.remove_old_events(5);
        assert_eq!(p.events().len(), 1);
        p.set_value_at_time(0.5, 1.0, 0).unwrap();
        p.remove_old_events(5);
        assert_eq!(p.events().len(), 1);
        assert_eq!(p.events()[0].value, 0.5);
    }

    #[test]
    fn clamp_and_vclip() {
        assert_eq!(clamp_to(2.0, -1.0, 1.0), 1.0);
        assert_eq!(clamp_to(-2.0, -1.0, 1.0), -1.0);
        assert_eq!(clamp_to(0.5, -1.0, 1.0), 0.5);
        assert_eq!(vclip(2.0, -1.0, 1.0), 1.0);
        assert_eq!(vclip(-2.0, -1.0, 1.0), -1.0);
        assert_eq!(vclip(f32::INFINITY, -f32::MAX, f32::MAX), f32::MAX);
    }

    #[test]
    fn huge_times_do_not_overflow() {
        let mut p = gain();
        p.set_value_at_time(0.0, 0.0, 0).unwrap();
        p.linear_ramp_to(1.0, 1e300, 0).unwrap();
        p.exponential_ramp_to(0.5, f64::MAX, 0).unwrap();
        let v = render(&mut p, 256);
        assert!(v.iter().all(|x| x.is_finite() && *x >= 0.0 && *x < 1e-6));
        let mut p = gain();
        p.set_value_at_time(0.5, 1e9, 0).unwrap();
        assert!(render(&mut p, 128).iter().all(|x| *x == 1.0));
    }

    #[test]
    fn connections_are_added_one_after_another() {
        // (1 + 2^24) rounds to 2^24 in f32, so the order of the additions shows.
        let a = [16_777_216.0f32; QUANTUM];
        let b = [-16_777_216.0f32; QUANTUM];
        let mut p = Param::new(1.0, -f32::MAX, f32::MAX, Rate::A);
        let mut out = [0.0f32; QUANTUM];
        p.fill_quantum(0, &mut out, &[&a, &b]);
        assert!(out.iter().all(|x| *x == 0.0));
        assert_eq!(p.final_value(128, &[a[0], b[0]]), 0.0);
        // Pre-summed connections would give 1.
        assert_eq!(p.final_value(256, &[a[0] + b[0]]), 1.0);
    }

    #[test]
    fn a_connected_param_without_signal_still_replaces_nan_and_clips() {
        let mut p = Param::new(0.0, -1.0, 1.0, Rate::A);
        // A NaN intrinsic value is impossible; the clip is observable only through inputs, so
        // check that a silent connection leaves the timeline value and a live one clips.
        let mut out = [0.0f32; QUANTUM];
        p.fill_quantum_connected::<[f32; QUANTUM]>(0, &mut out, &[], true);
        assert!(out.iter().all(|x| *x == 0.0));
        p.fill_quantum_connected(128, &mut out, &[&[5.0; QUANTUM]], true);
        assert!(out.iter().all(|x| *x == 1.0));
        p.fill_quantum_connected(256, &mut out, &[&[f32::NAN; QUANTUM]], true);
        assert!(out.iter().all(|x| *x == 0.0));
        assert_eq!(p.final_value_connected(384, &[], true), 0.0);
        assert_eq!(p.final_value_connected(512, &[-3.0], true), -1.0);
    }

    #[test]
    fn param_inputs_none_and_a_connected_buffer() {
        let none = ParamInputs::NONE;
        assert!(none.bufs.is_empty());
        assert!(!none.connected);
        // NONE leaves the timeline alone (no NaN replacement, no clip, no inputs summed).
        let mut p = Param::new(0.25, -1.0, 1.0, Rate::A);
        let mut out = [0.0f32; QUANTUM];
        p.fill_quantum_connected(0, &mut out, &none.bufs, none.connected);
        assert!(out.iter().all(|x| x.to_bits() == 0.25f32.to_bits()));
        // A connected buffer is summed frame by frame, NaN becomes the default, then the clip.
        let mut buf = [0.5f32; QUANTUM];
        buf[3] = f32::NAN;
        buf[4] = 9.0;
        let one = ParamInputs {
            bufs: vec![buf],
            connected: true,
        };
        p.fill_quantum_connected(128, &mut out, &one.bufs, one.connected);
        assert_eq!(out[0], 0.75);
        assert_eq!(out[3], 0.25, "NaN sum replaced by the default");
        assert_eq!(out[4], 1.0, "clipped to the range");
        // Connected with every buffer silent: no buffers, but still connected.
        let silent = ParamInputs {
            bufs: Vec::new(),
            connected: true,
        };
        assert!(p.has_sample_accurate_values(256, silent.connected));
        assert_ne!(silent, ParamInputs::NONE);
    }

    #[test]
    fn set_initial_value_makes_no_event() {
        // `new GainNode(ac, { gain: 0.5 })`: intrinsic 0.5, no event, default still 1.
        let mut g = Param::gain(1.0);
        g.set_initial_value(0.5, "gain").unwrap();
        assert_eq!(g.value(), 0.5);
        assert!(g.events().is_empty());
        assert_eq!(g.default_value(), 1.0, "default_value() stays");
        assert!(!g.has_sample_accurate_values(0, false));
        let mut out = [0.0f32; QUANTUM];
        g.fill_quantum(0, &mut out, &[]);
        assert!(out.iter().all(|x| *x == 0.5));
        // A NaN connection input is replaced by the default (1), not the constructor value.
        g.fill_quantum_connected(128, &mut out, &[&[f32::NAN; QUANTUM]], true);
        assert!(out.iter().all(|x| *x == 1.0));
        // Clamped to the range; the f32 rounding of the double.
        let mut p = Param::new(0.0, -1.0, 1.0, Rate::A);
        p.set_initial_value(3.0, "pan").unwrap();
        assert_eq!(p.value(), 1.0);
        p.set_initial_value(0.1, "pan").unwrap();
        assert_eq!(p.value(), 0.1f32);
        // A non-finite or float-overflowing value throws for the option; nothing changes.
        for bad in [f64::NAN, f64::INFINITY, -1e39] {
            assert_eq!(
                g.set_initial_value(bad, "gain"),
                Err(ParamError::NonFiniteValue { method: "gain" }),
                "{bad}"
            );
            assert_eq!(g.value(), 0.5);
        }
        // `-0` is kept.
        g.set_initial_value(-0.0, "gain").unwrap();
        assert_eq!(g.value().to_bits(), (-0.0f32).to_bits());
    }

    fn cancel_event(time: f64, kind: RampKind, value: f32, saved_time: f64) -> ParamEvent {
        ParamEvent {
            kind: RampKind::CancelValues,
            value: 0.0,
            time,
            is_new: true,
            cancel: Some(CancelData {
                saved: SavedRamp {
                    kind,
                    value,
                    time: saved_time,
                },
                value_cached: false,
            }),
        }
    }

    /// The exponential segment from `(t1, v1)` to `(t2, v2)` as `ProcessExponentialRamp` renders
    /// frames `[from, to)` (each quantum restarts from the closed form, then the accumulator).
    fn exp_segment(t1: f64, v1: f32, t2: f64, v2: f32, from: u64, to: u64) -> Vec<f32> {
        let dt = t2 - t1;
        let m = libm::pow(f64::from(v2 / v1), 1.0 / (dt * SR));
        let mut out = Vec::new();
        let mut acc = 0.0f64;
        for f in from..to {
            if f == from || f % Q == 0 {
                acc = f64::from(
                    (f64::from(v1)
                        * libm::pow(f64::from(v2) / f64::from(v1), (f as f64 / SR - t1) / dt))
                        as f32,
                );
            }
            out.push(acc as f32);
            acc *= m;
        }
        out
    }

    #[test]
    fn cancel_no_later_event_is_noop() {
        let mut p = gain();
        p.cancel_and_hold_at_time(0.5, 0).unwrap();
        assert!(p.events().is_empty(), "empty timeline");
        p.set_value_at_time(0.5, 0.0, 0).unwrap();
        p.exponential_ramp_to(0.25, 100.0 / SR, 0).unwrap();
        let before = p.events().to_vec();
        // At the last event's time and after it: nothing lies strictly after the cancel time.
        p.cancel_and_hold_at_time(100.0 / SR, 0).unwrap();
        p.cancel_and_hold_at_time(1.0, 0).unwrap();
        assert_eq!(p.events(), &before[..]);
    }

    #[test]
    fn cancel_inside_exponential_ramp() {
        let (t2, tc) = (1000.0 / SR, 400.5 / SR);
        let mut p = gain();
        p.set_value_at_time(1.0, 0.0, 0).unwrap();
        p.exponential_ramp_to(0.1, t2, 0).unwrap();
        p.cancel_and_hold_at_time(tc, 0).unwrap();
        assert_eq!(p.events().len(), 2);
        assert_eq!(
            p.events()[1],
            cancel_event(tc, RampKind::Exponential, 0.1, t2)
        );
        // The held value: the float quotient, the double power and product, one rounding.
        let held = (1.0f64 * libm::pow(f64::from(0.1f32 / 1.0f32), tc / t2)) as f32;
        assert_eq!(
            held.to_bits(),
            exponential_ramp_at_time(tc, 1.0, 0.0, 0.1, t2).to_bits()
        );
        let v = render(&mut p, 640);
        // Frames 0..=400 follow the recomputed ramp to (tc, held), not the original one.
        let want = exp_segment(0.0, 1.0, tc, held, 0, 401);
        for f in 0..401 {
            assert_eq!(v[f].to_bits(), want[f].to_bits(), "frame {f}");
        }
        let original = exp_segment(0.0, 1.0, t2, 0.1, 0, 401);
        assert_ne!(
            v[300], original[300],
            "the segment's multiplier is recomputed"
        );
        assert!(
            v[401..].iter().all(|x| x.to_bits() == held.to_bits()),
            "held after the cancel"
        );
        // Rendering past the cancel pruned the SetValue; the CancelValues keeps its value.
        let c = p.events().last().unwrap();
        assert_eq!(c.kind, RampKind::CancelValues);
        assert_eq!(c.value.to_bits(), held.to_bits());
        assert!(c.cancel.unwrap().value_cached);
    }

    #[test]
    fn cancel_inside_linear_ramp() {
        let (t2, tc) = (1000.0 / SR, 300.25 / SR);
        let mut p = gain();
        p.set_value_at_time(0.0, 0.0, 0).unwrap();
        p.linear_ramp_to(1.0, t2, 0).unwrap();
        p.cancel_and_hold_at_time(tc, 0).unwrap();
        assert_eq!(p.events()[1], cancel_event(tc, RampKind::Linear, 1.0, t2));
        let held = (0.0f64 + f64::from(1.0f32 - 0.0f32) * (tc - 0.0) / (t2 - 0.0)) as f32;
        assert_eq!(
            held.to_bits(),
            linear_ramp_at_time(tc, 0.0, 0.0, 1.0, t2).to_bits()
        );
        let v = render(&mut p, 512);
        for f in 0..301u64 {
            assert_eq!(v[f as usize], linear_at(f, 0.0, 0.0, tc, held), "frame {f}");
        }
        assert!(v[301..].iter().all(|x| *x == held));
    }

    #[test]
    fn ramp_at_time_helpers() {
        // Opposite signs or a zero start hold the start value.
        assert_eq!(exponential_ramp_at_time(0.5, 0.0, 0.0, 1.0, 1.0), 0.0);
        assert_eq!(exponential_ramp_at_time(0.5, -1.0, 0.0, 1.0, 1.0), -1.0);
        assert_eq!(exponential_ramp_at_time(0.5, 1.0, 0.0, -0.0, 1.0), 1.0);
        assert_eq!(exponential_ramp_at_time(0.5, -1.0, 0.0, -0.25, 1.0), -0.5);
        assert_eq!(exponential_ramp_at_time(0.0, 0.3, 0.0, 0.7, 1.0), 0.3);
        assert_eq!(exponential_ramp_at_time(1.0, 0.3, 0.0, 0.7, 1.0), 0.7);
        // The float quotient: 0.7f / 0.3f differs from the double quotient of the same floats.
        let q32 = f64::from(0.7f32 / 0.3f32);
        let q64 = f64::from(0.7f32) / f64::from(0.3f32);
        assert_ne!(q32, q64);
        let t = 0.37;
        assert_eq!(
            exponential_ramp_at_time(t, 0.3, 0.0, 0.7, 1.0).to_bits(),
            ((f64::from(0.3f32) * libm::pow(q32, t)) as f32).to_bits()
        );
        // Linear: the float difference, then double arithmetic.
        assert_eq!(linear_ramp_at_time(0.25, 1.0, 0.0, 0.0, 1.0), 0.75);
        assert_eq!(
            linear_ramp_at_time(0.1, 0.3, 0.0, 0.7, 1.0).to_bits(),
            ((f64::from(0.3f32) + f64::from(0.7f32 - 0.3f32) * 0.1 / 1.0) as f32).to_bits()
        );
    }

    #[test]
    fn cancel_before_a_set_value_removes_it() {
        let mut p = gain();
        p.set_value_at_time(0.5, 100.0 / SR, 0).unwrap();
        p.set_value_at_time(0.25, 200.0 / SR, 0).unwrap();
        p.linear_ramp_to(0.0, 300.0 / SR, 0).unwrap();
        p.cancel_and_hold_at_time(150.0 / SR, 0).unwrap();
        let kinds: Vec<RampKind> = p.events().iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![RampKind::SetValue],
            "no CancelValues for a SetValue"
        );
        let v = render(&mut p, 512);
        assert!(
            v[100..].iter().all(|x| *x == 0.5),
            "the later events are gone"
        );
        // A cancel whose first later event is a CancelValues inserts nothing either.
        let mut p = gain();
        p.set_value_at_time(1.0, 0.0, 0).unwrap();
        p.linear_ramp_to(0.0, 1000.0 / SR, 0).unwrap();
        p.cancel_and_hold_at_time(500.0 / SR, 0).unwrap();
        p.cancel_and_hold_at_time(100.0 / SR, 0).unwrap();
        let kinds: Vec<RampKind> = p.events().iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![RampKind::SetValue]);
    }

    #[test]
    fn cancel_with_equal_time_event_keeps_it() {
        let (t1, t2) = (100.0 / SR, 300.0 / SR);
        let mut p = gain();
        p.set_value_at_time(1.0, 0.0, 0).unwrap();
        p.linear_ramp_to(0.5, t1, 0).unwrap();
        p.linear_ramp_to(0.2, t2, 0).unwrap();
        p.cancel_and_hold_at_time(t1, 0).unwrap();
        let kinds: Vec<RampKind> = p.events().iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![RampKind::SetValue, RampKind::Linear, RampKind::CancelValues],
            "the event at the cancel time stays, the CancelValues goes after it"
        );
        assert_eq!(p.events()[2], cancel_event(t1, RampKind::Linear, 0.2, t2));
        let v = render(&mut p, 512);
        assert_eq!(v[99], linear_at(99, 0.0, 1.0, t1, 0.5));
        assert!(
            v[100..].iter().all(|x| *x == 0.5),
            "held at the ramp's end value"
        );
    }

    #[test]
    fn cancel_at_time_zero_onset_zero_duck() {
        // A duck with onset 0 inside a recovery: cancel at t, the fall ends at t (time1 >= time2:
        // the cached value holds for no frame) and the gain is ducked from frame ceil(t * sr).
        let t = 500.0 / SR;
        let mut p = gain();
        p.set_value_at_time(1.0, 0.0, 0).unwrap();
        p.exponential_ramp_to(0.1, 200.0 / SR, 0).unwrap();
        p.exponential_ramp_to(1.0, 1000.0 / SR, 0).unwrap();
        p.cancel_and_hold_at_time(t, 0).unwrap();
        p.exponential_ramp_to(0.3, t, 0).unwrap();
        p.exponential_ramp_to(1.0, t + 0.01, 0).unwrap();
        let v = render(&mut p, 640);
        let held = exponential_ramp_at_time(t, 0.1, 200.0 / SR, 1.0, 1000.0 / SR);
        let rise = exp_segment(200.0 / SR, 0.1, t, held, 200, 500);
        assert_eq!(&v[200..500], &rise[..], "the recovery up to the cancel");
        assert_eq!(v[500], 0.3, "ducked at frame 500");
        let fall = exp_segment(t, 0.3, t + 0.01, 1.0, 500, 640);
        assert_eq!(&v[500..640], &fall[..]);
        // The same at a fractional frame: the jump lands on the ceiling.
        let t = 500.5 / SR;
        let mut p = gain();
        p.set_value_at_time(1.0, 0.0, 0).unwrap();
        p.exponential_ramp_to(0.1, 1000.0 / SR, 0).unwrap();
        p.cancel_and_hold_at_time(t, 0).unwrap();
        p.exponential_ramp_to(0.3, t, 0).unwrap();
        let v = render(&mut p, 640);
        assert_ne!(v[500], 0.3);
        assert!(v[501..].iter().all(|x| *x == 0.3));
    }

    #[test]
    fn cancel_emptied_timeline_has_no_synthetic_set_value() {
        // Every event after the cancel time and the first a ramp (unreachable from the public
        // calls, which start a ramp with a SetValue at `now`): the CancelValues is inserted alone.
        let mut p = gain();
        p.events = vec![ParamEvent {
            kind: RampKind::Exponential,
            value: 0.5,
            time: 200.0 / SR,
            is_new: false,
            cancel: None,
        }];
        p.cancel_and_hold_at_time(100.0 / SR, 0).unwrap();
        assert_eq!(
            p.events(),
            &[cancel_event(
                100.0 / SR,
                RampKind::Exponential,
                0.5,
                200.0 / SR
            )]
        );
        // A SetValue first: the timeline is emptied and nothing is inserted.
        let mut p = gain();
        p.set_value_at_time(0.8, 100.0 / SR, 0).unwrap();
        p.linear_ramp_to(0.2, 300.0 / SR, 0).unwrap();
        p.cancel_and_hold_at_time(50.0 / SR, 0).unwrap();
        assert!(p.events().is_empty());
    }

    #[test]
    fn cancel_clamps_to_now() {
        let mut p = gain();
        p.set_value_at_time(1.0, 0.0, 0).unwrap();
        p.exponential_ramp_to(0.1, 1000.0 / SR, 0).unwrap();
        p.cancel_and_hold_at_time(0.0, 256).unwrap();
        assert_eq!(
            p.events()[1],
            cancel_event(256.0 / SR, RampKind::Exponential, 0.1, 1000.0 / SR)
        );
        // `now` past every event: nothing lies after it.
        let mut p = gain();
        p.set_value_at_time(1.0, 0.0, 0).unwrap();
        p.exponential_ramp_to(0.1, 100.0 / SR, 0).unwrap();
        let before = p.events().to_vec();
        p.cancel_and_hold_at_time(0.0, 128).unwrap();
        assert_eq!(p.events(), &before[..]);
    }

    #[test]
    fn cancel_errors() {
        let mut p = gain();
        p.set_value_at_time(1.0, 0.0, 0).unwrap();
        p.exponential_ramp_to(0.1, 1.0, 0).unwrap();
        let before = p.events().to_vec();
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                p.cancel_and_hold_at_time(bad, 0),
                Err(ParamError::NonFiniteTime {
                    method: CANCEL_AND_HOLD
                }),
                "{bad}"
            );
        }
        assert_eq!(
            p.cancel_and_hold_at_time(-0.25, 0),
            Err(ParamError::NegativeTime {
                method: CANCEL_AND_HOLD,
                time: -0.25
            })
        );
        assert_eq!(p.events(), &before[..], "a throw changes nothing");
        // -0 is not negative.
        assert!(p.cancel_and_hold_at_time(-0.0, 0).is_ok());
        assert_eq!(
            ParamError::NegativeTime {
                method: CANCEL_AND_HOLD,
                time: -0.25
            }
            .to_string(),
            "RangeError: Failed to execute 'cancelAndHoldAtTime' on 'AudioParam': Time must be a \
             finite non-negative number: -0.25"
        );
    }

    #[test]
    fn lazy_cancelled_value_cached_on_first_evaluation() {
        let (t2, tc) = (1000.0 / SR, 600.0 / SR);
        let mut p = gain();
        p.set_value_at_time(1.0, 0.0, 0).unwrap();
        p.exponential_ramp_to(0.1, t2, 0).unwrap();
        p.cancel_and_hold_at_time(tc, 0).unwrap();
        let c = p.events()[1];
        assert_eq!((c.value, c.cancel.unwrap().value_cached), (0.0, false));
        // The first quantum ends long before the cancel frame, but its segment ends there: the
        // value is computed and cached on the first evaluation.
        render(&mut p, 128);
        let c = p.events()[1];
        let held = exponential_ramp_at_time(tc, 1.0, 0.0, 0.1, t2);
        assert_eq!(c.value.to_bits(), held.to_bits());
        assert!(c.cancel.unwrap().value_cached);
        // An uncached CancelValues reached without its predecessor holds the running value (the
        // intrinsic one here), not its own 0.
        let mut p = Param::gain(0.75);
        let mut e = cancel_event(0.0, RampKind::Linear, 0.2, 1.0);
        e.is_new = false;
        p.events = vec![e];
        let v = render(&mut p, 128);
        assert!(v.iter().all(|x| *x == 0.75), "{:?}", &v[..4]);
        // A ramp after an uncached CancelValues starts from its value 0 (Blink reads
        // `event->Value()`): an exponential ramp from 0 holds 0.
        let mut p = Param::gain(0.75);
        p.events = vec![e];
        p.exponential_ramp_to(0.5, 200.0 / SR, 0).unwrap();
        let v = render(&mut p, 128);
        assert!(v.iter().all(|x| *x == 0.0), "{:?}", &v[..4]);
    }

    #[test]
    fn two_cancels_in_one_pause() {
        // Duck A at 0.375 s, then duck B called later at 0.25 s in the same pause.
        let now = 0;
        let mut p = gain();
        p.cancel_and_hold_at_time(0.375, now).unwrap();
        p.exponential_ramp_to(0.1, 0.375, now).unwrap();
        p.exponential_ramp_to(1.0, 0.575, now).unwrap();
        p.cancel_and_hold_at_time(0.25, now).unwrap();
        assert_eq!(p.events().len(), 2);
        assert_eq!(
            p.events()[1],
            cancel_event(0.25, RampKind::Exponential, 0.1, 0.375),
            "the saved ramp is A's fall"
        );
        p.exponential_ramp_to(0.5, 0.3, now).unwrap();
        p.exponential_ramp_to(1.0, 0.4, now).unwrap();
        let kinds: Vec<RampKind> = p.events().iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                RampKind::SetValue,
                RampKind::CancelValues,
                RampKind::Exponential,
                RampKind::Exponential
            ]
        );
        // A second cancel inside B's fall saves B's ramp and replaces everything after it.
        p.cancel_and_hold_at_time(0.28, now).unwrap();
        assert_eq!(
            p.events()[2],
            cancel_event(0.28, RampKind::Exponential, 0.5, 0.3)
        );
        assert_eq!(p.events().len(), 3);
        // Rendering: the second CancelValues's value comes from the segment that starts at the
        // first one's cached value.
        let frames = (0.3 * SR) as u64;
        let v = render(&mut p, frames);
        let held1 = exponential_ramp_at_time(0.25, 1.0, 0.0, 0.1, 0.375);
        let held2 = exponential_ramp_at_time(0.28, held1, 0.25, 0.5, 0.3);
        assert_eq!(
            v[11025].to_bits(),
            held1.to_bits(),
            "B's segment starts at A's held value"
        );
        let at = (0.28 * SR).ceil() as usize;
        let want = exp_segment(0.25, held1, 0.28, held2, 11025, at as u64);
        assert_eq!(&v[11025..at], &want[..]);
        assert!(v[at..].iter().all(|x| x.to_bits() == held2.to_bits()));
        let last = p.events().last().unwrap();
        assert_eq!(
            (last.kind, last.value.to_bits()),
            (RampKind::CancelValues, held2.to_bits())
        );
    }

    #[test]
    fn ramp_after_cancel_starts_from_held() {
        let (tc, t3) = (256.0 / SR, 768.0 / SR);
        let mut p = gain();
        p.set_value_at_time(1.0, 0.0, 0).unwrap();
        p.exponential_ramp_to(0.1, 1000.0 / SR, 0).unwrap();
        p.cancel_and_hold_at_time(tc, 0).unwrap();
        p.exponential_ramp_to(0.2, t3, 0).unwrap();
        let v = render(&mut p, 1024);
        let held = exponential_ramp_at_time(tc, 1.0, 0.0, 0.1, 1000.0 / SR);
        assert_eq!(
            v[256].to_bits(),
            held.to_bits(),
            "the next ramp starts at the held value"
        );
        let want = exp_segment(tc, held, t3, 0.2, 256, 768);
        assert_eq!(&v[256..768], &want[..]);
        assert!(v[768..].iter().all(|x| *x == 0.2));
        // A linear ramp after a cancel inside a linear ramp.
        let mut p = gain();
        p.set_value_at_time(0.0, 0.0, 0).unwrap();
        p.linear_ramp_to(1.0, 1000.0 / SR, 0).unwrap();
        p.cancel_and_hold_at_time(tc, 0).unwrap();
        p.linear_ramp_to(0.0, t3, 0).unwrap();
        let v = render(&mut p, 1024);
        let held = linear_ramp_at_time(tc, 0.0, 0.0, 1.0, 1000.0 / SR);
        for f in 256..768u64 {
            assert_eq!(v[f as usize], linear_at(f, tc, held, t3, 0.0), "frame {f}");
        }
    }

    #[test]
    fn insert_gives_only_ramps_a_starting_set_value() {
        let mut p = Param::gain(0.25);
        p.set_value_at_time(0.5, 1.0, 128).unwrap();
        assert_eq!(
            p.events().len(),
            1,
            "a SetValue on an empty timeline is alone"
        );
        for ramp in [RampKind::Linear, RampKind::Exponential] {
            let mut p = Param::gain(0.25);
            p.insert(ramp, 0.5, 1.0, 128);
            assert_eq!(p.events()[0].kind, RampKind::SetValue);
            assert_eq!(
                (p.events()[0].value, p.events()[0].time),
                (0.25, 128.0 / SR)
            );
        }
    }
}
