//! superdough's envelope worklet (`EnvelopeProcessor`, worklets.mjs:989-1093) and `getEnvelope`
//! (helpers.mjs:123-125): a one-channel ADSR modulation source with no input, the `env`
//! modulator's node (built and connected by `modulate`).
//!
//! - **Params** (13, all a-rate): the descriptors spell `minValue`/`maxValue`, so the ranges apply
//!   ([`worklet::ranged`]): `attack`, `decay`, `release` `[0, max]`, `sustain` and `retrigger` `[0,
//!   1]`, the three curves `[-1, 1]`.
//! - **Gating.** `currentTime >= end` returns `false` first, then `currentTime <= begin` returns
//!   `true` with zeros, on the float32 params.
//! - **Trigger.** When the float32 `begin` differs from the stored `beginTime` (which starts at 0)
//!   and the envelope is idle or `retrigger[0] >= 0.5`: `beginTime = begin`, attack state, `endTime
//!   = end[0]`, `attackStart = val`. A hap whose `begin` is exactly 0 therefore never triggers.
//! - **Stages.** Per frame, `(time, start, target, curve)` of the current state: idle `(inf, 0,
//!   0)`, attack `(attack, attackStart, 1, attackCurve)`, decay `(attack + decay, 1, sustain,
//!   decayCurve)`, sustain `(endTime - beginTime, sustain, sustain)`, release `(susTime + release,
//!   sustain, 0, releaseCurve)`: the stage times are cumulative since `beginTime`, so the decay's
//!   phase is `elapsed / (attack + decay)` (a jump at its start). `val` moves by the warped phase,
//!   then a `while` passes every stage whose time has elapsed (several in one frame), and the
//!   output is `clamp(val * depth, min, max)` as float32. `currentTime` does not advance inside a
//!   quantum, so the output is a 128-frame staircase.
//! - **Release.** superdough sets `end` to both the gate and the release start, so the processor
//!   returns `false` before the release stage could sound: the sustain holds to the last quantum
//!   before `end`, then the output is zeros.

use crate::QUANTUM;
use crate::jsmath;
use crate::param::{Param, ParamError, ParamInputs, Rate, idl_float};
use crate::worklet::{self, Lifecycle};

/// Descriptor indices (the order of the [`EnvInputs`] a render takes).
pub const BEGIN: usize = 0;
pub const END: usize = 1;
pub const ATTACK: usize = 2;
pub const DECAY: usize = 3;
pub const SUSTAIN: usize = 4;
pub const RELEASE: usize = 5;
pub const ATTACK_CURVE: usize = 6;
pub const DECAY_CURVE: usize = 7;
pub const RELEASE_CURVE: usize = 8;
pub const DEPTH: usize = 9;
pub const MIN: usize = 10;
pub const MAX: usize = 11;
pub const RETRIGGER: usize = 12;
/// The number of params.
pub const PARAMS: usize = 13;

/// The connection inputs of the thirteen params for one quantum, in descriptor order.
pub type EnvInputs = [ParamInputs; PARAMS];

/// No param connected.
pub const NO_INPUTS: EnvInputs = [
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
    ParamInputs::NONE,
];

/// `_warp(phase, curvature, strength = 8)`: 0 and 1 unchanged; a positive curvature `1 - (1 -
/// phase)^(1 + 8c)`, else `phase^(1 - 8c)` (so 0 is `pow(phase, 1)`).
pub fn warp(phase: f64, curvature: f64) -> f64 {
    if phase == 0.0 || phase == 1.0 {
        return phase;
    }
    if curvature > 0.0 {
        let exp = 1.0 + 8.0 * curvature;
        1.0 - jsmath::pow(1.0 - phase, exp)
    } else {
        let exp = 1.0 - 8.0 * curvature;
        jsmath::pow(phase, exp)
    }
}

/// The processor's state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnvelopeProc {
    pub val: f64,
    /// 0 idle, 1 attack, 2 decay, 3 sustain, 4 release.
    pub state: u8,
    pub begin_time: f64,
    pub end_time: f64,
    pub attack_start: f64,
}

/// One frame's ADSR param values (`pv(params.x, i)`) and the curvatures.
struct Frame {
    attack: f64,
    decay: f64,
    sustain: f64,
    release: f64,
    curves: [f64; 3],
}

impl Frame {
    /// The `states` table's `(time, start, target, curve)` for `state`; the curve is unused where
    /// `start == target`.
    fn stage(&self, state: u8, attack_start: f64, sus_time: f64) -> (f64, f64, f64, f64) {
        match state {
            1 => (self.attack, attack_start, 1.0, self.curves[0]),
            2 => (self.attack + self.decay, 1.0, self.sustain, self.curves[1]),
            3 => (sus_time, self.sustain, self.sustain, 0.0),
            4 => (sus_time + self.release, self.sustain, 0.0, self.curves[2]),
            _ => (f64::INFINITY, 0.0, 0.0, 0.0),
        }
    }
}

impl EnvelopeProc {
    /// `_advance(start, target, time, curvature)` at `currentTime` `ct`.
    fn advance(&mut self, ct: f64, start: f64, target: f64, time: f64, curvature: f64) {
        if time == 0.0 || start == target {
            self.val = target;
        } else {
            let phase = jsmath::min(1.0, (ct - self.begin_time) / time);
            self.val = start + (target - start) * warp(phase, curvature);
        }
    }

    /// One `process()` call at `currentTime` `ct` with the thirteen param blocks, writing `out`
    /// (zeroed by the caller).
    pub fn process(
        &mut self,
        ct: f64,
        p: &[[f32; QUANTUM]; PARAMS],
        out: &mut [f32; QUANTUM],
    ) -> bool {
        let begin = f64::from(p[BEGIN][0]);
        let end = f64::from(p[END][0]);
        if ct >= end {
            return false;
        }
        if ct <= begin {
            return true;
        }
        let retrigger = p[RETRIGGER][0] >= 0.5;
        if begin != self.begin_time && (self.state == 0 || retrigger) {
            self.begin_time = begin;
            self.state = 1;
            self.end_time = f64::from(p[END][0]);
            self.attack_start = self.val;
        }
        let sus_time = self.end_time - self.begin_time;
        for (i, o) in out.iter_mut().enumerate() {
            let v = |k: usize| f64::from(p[k][i]);
            let frame = Frame {
                attack: v(ATTACK),
                decay: v(DECAY),
                sustain: v(SUSTAIN),
                release: v(RELEASE),
                curves: [v(ATTACK_CURVE), v(DECAY_CURVE), v(RELEASE_CURVE)],
            };
            let (depth, min, max) = (v(DEPTH), v(MIN), v(MAX));
            let (mut time, start, target, curve) =
                frame.stage(self.state, self.attack_start, sus_time);
            self.advance(ct, start, target, time, curve);
            while ct - self.begin_time >= time {
                self.state = (self.state + 1) % 5;
                time = frame.stage(self.state, self.attack_start, sus_time).0;
            }
            *o = jsmath::clamp(self.val * depth, min, max) as f32;
        }
        true
    }
}

/// An `envelope-processor` `AudioWorkletNode`: the processor, its thirteen params and its
/// lifecycle (no input, so it finishes at its first `false`).
#[derive(Debug, Clone)]
pub struct Envelope {
    pub proc: EnvelopeProc,
    /// In descriptor order (see [`BEGIN`] .. [`RETRIGGER`]).
    pub params: [Param; PARAMS],
    life: Lifecycle,
}

impl Default for Envelope {
    fn default() -> Self {
        Envelope::new()
    }
}

impl Envelope {
    /// `new AudioWorkletNode(ac, 'envelope-processor')`: every param at its descriptor default,
    /// with the descriptor ranges.
    pub fn new() -> Envelope {
        let full = |d: f32| worklet::ranged(d, -f32::MAX, f32::MAX, Rate::A);
        Envelope {
            proc: EnvelopeProc::default(),
            params: [
                full(0.0),
                full(0.0),
                worklet::ranged(0.005, 0.0, f32::MAX, Rate::A),
                worklet::ranged(0.14, 0.0, f32::MAX, Rate::A),
                worklet::ranged(0.0, 0.0, 1.0, Rate::A),
                worklet::ranged(0.1, 0.0, f32::MAX, Rate::A),
                worklet::ranged(0.0, -1.0, 1.0, Rate::A),
                worklet::ranged(0.0, -1.0, 1.0, Rate::A),
                worklet::ranged(0.0, -1.0, 1.0, Rate::A),
                full(1.0),
                full(-1e9),
                full(1e9),
                worklet::ranged(1.0, 0.0, 1.0, Rate::A),
            ],
            life: Lifecycle::new(),
        }
    }

    /// One quantum starting at frame `q` into `out` (overwritten), its params pulling `inputs`.
    /// Returns the output's silence hint: false while the processor lives, true once finished.
    pub fn render(&mut self, q: u64, inputs: &EnvInputs, out: &mut [f32; QUANTUM]) -> bool {
        *out = [0.0; QUANTUM];
        if !self.life.begin_quantum(false) {
            return true;
        }
        let mut blocks = [[0.0f32; QUANTUM]; PARAMS];
        for ((p, input), block) in self.params.iter_mut().zip(inputs).zip(blocks.iter_mut()) {
            worklet::fill_block(p, q, input, block);
        }
        let r = self.proc.process(worklet::current_time(q), &blocks, out);
        self.life.finish_quantum(Ok(r), false);
        false
    }

    /// The first quantum start at which the processor is gone (`u64::MAX` when none is).
    pub fn end_quantum(&self) -> u64 {
        worklet::first_quantum_reaching(f64::from(self.params[END].value())).unwrap_or(u64::MAX)
    }
}

/// `getEnvelope`'s properties in the key order `connectEnvelope` passes them; `None` is
/// `undefined` (the key is skipped and the processor default stays).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnvProps {
    pub begin: Option<f64>,
    pub end: Option<f64>,
    pub depth: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub attack_curve: Option<f64>,
    pub decay_curve: Option<f64>,
    pub release_curve: Option<f64>,
    pub attack: Option<f64>,
    pub decay: Option<f64>,
    pub sustain: Option<f64>,
    pub release: Option<f64>,
    pub retrigger: Option<f64>,
}

/// `getEnvelope(ac, props)` scheduled at frame `now`: `.value =` for every defined prop in
/// [`EnvProps`]'s order (each clamped to its param's range, a non-finite value throwing there).
pub fn get_envelope(props: &EnvProps, now: u64) -> Result<Envelope, ParamError> {
    let mut env = Envelope::new();
    let sets = [
        (BEGIN, props.begin),
        (END, props.end),
        (DEPTH, props.depth),
        (MIN, props.min),
        (MAX, props.max),
        (ATTACK_CURVE, props.attack_curve),
        (DECAY_CURVE, props.decay_curve),
        (RELEASE_CURVE, props.release_curve),
        (ATTACK, props.attack),
        (DECAY, props.decay),
        (SUSTAIN, props.sustain),
        (RELEASE, props.release),
        (RETRIGGER, props.retrigger),
    ];
    for (k, v) in sets {
        if let Some(v) = v {
            env.params[k].set_value(idl_float(v, "value")?, now)?;
        }
    }
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::QUANTUM_U64 as Q;

    fn bits(x: f64) -> u64 {
        x.to_bits()
    }

    /// The descriptor defaults with `begin`, `end` and the ADSR set.
    fn blocks(begin: f32, end: f32, adsr: [f32; 4]) -> [[f32; QUANTUM]; PARAMS] {
        let mut v = [0.0f32; PARAMS];
        v[BEGIN] = begin;
        v[END] = end;
        v[ATTACK] = adsr[0];
        v[DECAY] = adsr[1];
        v[SUSTAIN] = adsr[2];
        v[RELEASE] = adsr[3];
        v[DEPTH] = 1.0;
        v[MIN] = -1e9;
        v[MAX] = 1e9;
        v[RETRIGGER] = 1.0;
        v.map(|x| [x; QUANTUM])
    }

    fn at(e: &mut EnvelopeProc, ct: f64, p: &[[f32; QUANTUM]; PARAMS]) -> (bool, [f32; QUANTUM]) {
        let mut out = [0.0f32; QUANTUM];
        let r = e.process(ct, p, &mut out);
        (r, out)
    }

    #[test]
    fn warp_endpoints_and_curvatures() {
        for c in [-1.0, -0.3, 0.0, 0.4, 1.0] {
            assert_eq!(warp(0.0, c), 0.0, "{c}");
            assert_eq!(warp(1.0, c), 1.0, "{c}");
        }
        // Zero curvature is pow(phase, 1) = phase.
        assert_eq!(bits(warp(0.3, 0.0)), bits(0.3));
        assert_eq!(bits(warp(0.3, 0.5)), bits(1.0 - jsmath::pow(0.7, 5.0)));
        assert_eq!(bits(warp(0.3, -0.5)), bits(jsmath::pow(0.3, 5.0)));
        assert_eq!(bits(warp(0.25, 1.0)), bits(1.0 - jsmath::pow(0.75, 9.0)));
        assert_eq!(bits(warp(0.25, -1.0)), bits(jsmath::pow(0.25, 9.0)));
    }

    #[test]
    fn the_stage_table_at_hand_picked_times() {
        // begin 0.1 s, end 0.5 s, attack 0.05, decay 0.1, sustain 0.5.
        let p = blocks(0.1, 0.5, [0.05, 0.1, 0.5, 0.1]);
        let b = f64::from(0.1f32);
        let mut e = EnvelopeProc::default();
        // Before begin: zeros, alive; nothing triggered.
        let (r, out) = at(&mut e, 0.1, &p);
        assert!(r && out.iter().all(|x| x.to_bits() == 0));
        assert_eq!(e.state, 0);
        // 0.12 s: attack, phase (0.12 - b) / attack, linear.
        let ct = 0.12;
        let (r, out) = at(&mut e, ct, &p);
        assert!(r);
        assert_eq!(e.state, 1);
        assert_eq!(e.begin_time, b);
        assert_eq!(e.end_time, f64::from(0.5f32));
        let attack = f64::from(0.05f32);
        let want = 0.0 + (1.0 - 0.0) * ((ct - b) / attack);
        assert!(
            out.iter().all(|x| x.to_bits() == (want as f32).to_bits()),
            "a staircase"
        );
        // 0.16 s: past the attack: the frame ends in decay (the while moved on); 0.17 s: decay
        // with the cumulative time: phase = elapsed / (attack + decay), the jump.
        let (_, _) = at(&mut e, 0.16, &p);
        assert_eq!(e.state, 2);
        let ct = 0.17;
        let (_, out) = at(&mut e, ct, &p);
        let (decay, sustain) = (f64::from(0.1f32), f64::from(0.5f32));
        let phase = (ct - b) / (attack + decay);
        let want = 1.0 + (sustain - 1.0) * phase;
        assert_eq!(out[0].to_bits(), (want as f32).to_bits());
        let restarted = 1.0 + (sustain - 1.0) * ((ct - b - attack) / decay);
        assert_ne!(
            out[0].to_bits(),
            (restarted as f32).to_bits(),
            "not the restarted phase"
        );
        // 0.3 s: sustain held.
        let (_, out) = at(&mut e, 0.3, &p);
        assert_eq!(e.state, 3);
        assert_eq!(out[0], 0.5);
        let (_, out) = at(&mut e, 0.49, &p);
        assert_eq!(
            out[127], 0.5,
            "the sustain holds to the last quantum before end"
        );
        // At end: false, nothing written.
        let (r, out) = at(&mut e, 0.5, &p);
        assert!(!r && out.iter().all(|x| x.to_bits() == 0));
    }

    #[test]
    fn begin_zero_never_triggers() {
        let p = blocks(0.0, 1.0, [0.01, 0.01, 0.5, 0.1]);
        let mut e = EnvelopeProc::default();
        for k in 1..20u64 {
            let (r, out) = at(&mut e, worklet::current_time(k * Q), &p);
            assert!(r);
            assert!(out.iter().all(|x| x.to_bits() == 0), "quantum {k}");
        }
        assert_eq!(e.state, 0);
        // clamp(0 * depth, min, max) with min > 0 gives min.
        let mut p = p;
        p[MIN] = [0.25; QUANTUM];
        let (_, out) = at(&mut e, 0.1, &p);
        assert_eq!(out[0], 0.25);
    }

    #[test]
    fn retrigger_on_a_second_begin() {
        let mut p = blocks(0.1, 1.0, [0.05, 0.05, 0.5, 0.1]);
        let mut e = EnvelopeProc::default();
        at(&mut e, 0.3, &p);
        assert_eq!(e.state, 3);
        // A second begin with retrigger 1: restarts the attack from the current value.
        p[BEGIN] = [0.4; QUANTUM];
        at(&mut e, 0.41, &p);
        assert_eq!(e.begin_time, f64::from(0.4f32));
        assert_eq!(e.attack_start, 0.5);
        assert_eq!(e.state, 1);
        // Retrigger below 0.5 while not idle: ignored.
        let mut e = EnvelopeProc::default();
        let mut p = blocks(0.1, 1.0, [0.05, 0.05, 0.5, 0.1]);
        p[RETRIGGER] = [0.49; QUANTUM];
        at(&mut e, 0.3, &p);
        p[BEGIN] = [0.4; QUANTUM];
        at(&mut e, 0.41, &p);
        assert_eq!(e.begin_time, f64::from(0.1f32));
        assert_eq!(e.state, 3);
        // Exactly 0.5 retriggers.
        p[RETRIGGER] = [0.5; QUANTUM];
        at(&mut e, 0.42, &p);
        assert_eq!(e.begin_time, f64::from(0.4f32));
    }

    #[test]
    fn shortcuts_and_two_stages_in_one_frame() {
        // attack 0: `time == 0` sets the target at once, and the while passes attack and (decay
        // 0) decay in the same frame: sustain.
        let p = blocks(0.1, 1.0, [0.0, 0.0, 0.25, 0.1]);
        let mut e = EnvelopeProc::default();
        let (_, out) = at(&mut e, 0.2, &p);
        assert_eq!(e.state, 3, "attack and decay passed in one frame");
        assert_eq!(
            out[0], 1.0,
            "frame 0 advanced the attack (time 0: the target)"
        );
        assert_eq!(out[1], 0.25, "frame 1 is in sustain");
        // start == target: no division (attackStart 1 into the attack's target 1).
        let mut e = EnvelopeProc {
            val: 1.0,
            ..EnvelopeProc::default()
        };
        let p = blocks(0.1, 1.0, [0.5, 0.1, 0.25, 0.1]);
        let (_, out) = at(&mut e, 0.12, &p);
        assert_eq!(e.attack_start, 1.0);
        assert_eq!(out[0], 1.0);
        // After the release state comes idle (infinite time): reachable when end_time is early.
        let mut e = EnvelopeProc::default();
        let mut p = blocks(0.1, 1.0, [0.01, 0.01, 0.5, 0.01]);
        at(&mut e, 0.12, &p);
        e.end_time = 0.15; // susTime 0.05 (the release stage is reached only this way)
        p[END] = [1.0; QUANTUM];
        let (_, out) = at(&mut e, 0.5, &p);
        assert_eq!(e.state, 0);
        assert_eq!(out[127], 0.0);
    }

    #[test]
    fn ranges_clip_and_defaults() {
        let env = get_envelope(
            &EnvProps {
                attack: Some(-1.0),
                sustain: Some(2.0),
                retrigger: Some(-3.0),
                attack_curve: Some(5.0),
                ..EnvProps::default()
            },
            128,
        )
        .unwrap();
        assert_eq!(env.params[ATTACK].value(), 0.0);
        assert_eq!(env.params[SUSTAIN].value(), 1.0);
        assert_eq!(env.params[RETRIGGER].value(), 0.0);
        assert_eq!(env.params[ATTACK_CURVE].value(), 1.0);
        // Absent props keep the defaults and schedule nothing.
        assert_eq!(env.params[DECAY].value(), 0.14);
        assert_eq!(env.params[RELEASE].value(), 0.1);
        assert!(env.params[DECAY].events().is_empty());
        assert_eq!(env.params[ATTACK].events().len(), 1);
        assert_eq!(env.params[MIN].value(), -1e9);
        // A non-finite value throws.
        assert_eq!(
            get_envelope(
                &EnvProps {
                    depth: Some(f64::NAN),
                    ..EnvProps::default()
                },
                0
            )
            .unwrap_err(),
            ParamError::NonFiniteValue { method: "value" }
        );
    }

    #[test]
    fn render_gates_at_end_and_reports_its_end_quantum() {
        let mut env = get_envelope(
            &EnvProps {
                begin: Some(0.001),
                end: Some(0.01),
                attack: Some(0.001),
                decay: Some(0.001),
                sustain: Some(0.5),
                ..EnvProps::default()
            },
            0,
        )
        .unwrap();
        assert_eq!(env.end_quantum(), 512);
        let mut out = [0.0f32; QUANTUM];
        assert!(!env.render(0, &NO_INPUTS, &mut out));
        assert!(out.iter().all(|x| x.to_bits() == 0));
        assert!(!env.render(Q, &NO_INPUTS, &mut out));
        assert!(out.iter().any(|x| *x != 0.0));
        for k in 2..4u64 {
            assert!(!env.render(k * Q, &NO_INPUTS, &mut out));
            assert_eq!(out[0], 0.5, "quantum {k}: sustain");
        }
        assert!(
            !env.render(4 * Q, &NO_INPUTS, &mut out),
            "returns false: still live"
        );
        assert!(out.iter().all(|x| x.to_bits() == 0));
        assert!(env.render(5 * Q, &NO_INPUTS, &mut out), "gone");
        assert_eq!(
            get_envelope(&EnvProps::default(), 0).unwrap().end_quantum(),
            0
        );
    }
}
