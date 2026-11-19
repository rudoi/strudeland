//! superdough's transient shaper worklet (`TransientProcessor`, worklets.mjs:1420-1497; built
//! by superdough.mjs:657-673 first in the chain, right after the amp envelope): an attack and a
//! sustain envelope follower, a gain from their difference (`transient` boosts or cuts the
//! attack by up to 18 dB, `transsustain` the sustain by up to 36 dB), a slow average of that gain
//! as makeup, and a soft clip `y / (1 + |y|)` that always applies.
//!
//! It has no `AudioParam`s: `attack`, `sustain`, `begin` (the onset `t`) and `end`
//! (`endWithRelease`) arrive as doubles in `processorOptions`. Quirks kept: output is zero until
//! the first quantum whose start time is after `begin` and from `end` on (the node lives on while
//! its input is connected); `avgGain` is shared by the channels and carried from channel 0 into
//! channel 1 within a quantum; the envelopes are stored to a `Float32Array` once per quantum.

use crate::QUANTUM;
use crate::jsmath;
use crate::voice::zero;
use crate::worklet::{self, INVSR, Lifecycle, ProcessorError};

/// `timeToCoeff(t) = 1 - Math.exp(-INVSR / t)`.
fn time_to_coeff(t: f64) -> f64 {
    1.0 - jsmath::exp(-INVSR / t)
}

/// `lerp(a, b, n) = n * (b - a) + a`.
fn lerp(a: f64, b: f64, n: f64) -> f64 {
    n * (b - a) + a
}

/// `dbToLin(db) = Math.pow(10, db / 20)`; `Math.pow(10, ±0)` is exactly 1, so 0 dB skips it.
fn db_to_lin(db: f64) -> f64 {
    if db == 0.0 {
        1.0
    } else {
        jsmath::pow(10.0, db / 20.0)
    }
}

/// The processor's state.
#[derive(Debug, Clone, PartialEq)]
pub struct Transient {
    gain_coeff: f64,
    attack_coeff: f64,
    sustain_coeff: f64,
    attack_amt: f64,
    sustain_amt: f64,
    scaling: f64,
    mix: f64,
    begin: f64,
    end: f64,
    avg_gain: f64,
    attack_env: [f32; 2],
    sustain_env: [f32; 2],
}

impl Transient {
    /// The constructor with superdough's options: `attack = fx.transient`, `sustain =
    /// fx.transsustain` (`None` is `undefined`, which takes the default 0), `begin = t`, `end =
    /// endWithRelease`. The times, sensitivity and mix keep their defaults (0.003, 0.08, 0.1, 1).
    pub fn new(attack: Option<f64>, sustain: Option<f64>, begin: f64, end: f64) -> Transient {
        let attack_time = jsmath::clamp(0.003, 0.0005, 0.05);
        let sustain_time = jsmath::clamp(0.08, 0.01, 0.5);
        Transient {
            gain_coeff: time_to_coeff(0.2),
            attack_coeff: time_to_coeff(attack_time),
            sustain_coeff: time_to_coeff(sustain_time),
            attack_amt: jsmath::clamp(attack.unwrap_or(0.0), -1.0, 1.0),
            sustain_amt: jsmath::clamp(sustain.unwrap_or(0.0), -1.0, 1.0),
            scaling: 0.5 + 5.0 * jsmath::clamp(0.1, 0.0, 1.0),
            mix: jsmath::clamp(1.0, 0.0, 1.0),
            begin,
            end,
            avg_gain: 1.0,
            attack_env: [0.0; 2],
            sustain_env: [0.0; 2],
        }
    }

    /// One `process()` call at `currentTime` `ct`, in place on `io` (one or two channels; none
    /// when the input is not connected). Outputs zeros and returns `false` from `end` on, zeros
    /// and `true` up to `begin`.
    pub fn process(&mut self, ct: f64, io: &mut [[f32; QUANTUM]]) -> Result<bool, ProcessorError> {
        if ct >= self.end {
            zero(io);
            return Ok(false);
        }
        if ct <= self.begin {
            zero(io);
            return Ok(true);
        }
        let mut avg = self.avg_gain;
        for (ch, buf) in io.iter_mut().enumerate() {
            let mut att = f64::from(self.attack_env[ch]);
            let mut sus = f64::from(self.sustain_env[ch]);
            for v in buf.iter_mut() {
                let s = f64::from(*v);
                let x = s.abs();
                att = lerp(att, x, self.attack_coeff);
                sus = lerp(sus, x, self.sustain_coeff);
                let pk = jsmath::clamp((self.scaling * (att - sus)) / (sus + 1e-6), -1.5, 1.5);
                let att_s = if pk > 0.0 { pk } else { 0.0 };
                let sus_s = if pk < 0.0 { -pk } else { 0.0 };
                let a_g = db_to_lin((self.attack_amt * att_s) * 18.0);
                let s_g = db_to_lin((self.sustain_amt * sus_s) * 36.0);
                let g = jsmath::clamp(a_g * s_g, 0.0, 8.0);
                avg = lerp(avg, g, self.gain_coeff);
                let makeup = if avg > 1e-3 { 1.0 / avg } else { 1.0 };
                let wet = (s * g) * makeup;
                let mut y = lerp(s, wet, self.mix);
                y /= 1.0 + y.abs();
                *v = y as f32;
            }
            self.attack_env[ch] = att as f32;
            self.sustain_env[ch] = sus as f32;
        }
        self.avg_gain = avg;
        Ok(true)
    }
}

/// The transient's `AudioWorkletNode`: the processor and its lifecycle. Whether its input has a
/// connection each quantum comes from the chain's teardown walk.
#[derive(Debug, Clone)]
pub struct TransientNode {
    pub proc: Transient,
    life: Lifecycle,
}

impl TransientNode {
    pub fn new(proc: Transient) -> TransientNode {
        TransientNode {
            proc,
            life: Lifecycle::new(),
        }
    }

    /// The first quantum start whose `currentTime` reaches the processor's `end` (it returns
    /// `false` from there on); `None` when none does.
    pub fn end_quantum(&self) -> Option<u64> {
        worklet::first_quantum_reaching(self.proc.end)
    }

    /// One quantum in place on the voice bus's channels; `connected` is whether the input has a
    /// rendering connection this quantum. Returns the output's silence hint (false while the
    /// processor lives, true with zeros once it finished: after `end` with its input gone).
    pub fn process(&mut self, q: u64, io: &mut [[f32; QUANTUM]], connected: bool) -> bool {
        if !self.life.begin_quantum(connected) {
            zero(io);
            return true;
        }
        let ct = worklet::current_time(q);
        let r = if connected {
            self.proc.process(ct, io)
        } else {
            zero(io);
            self.proc.process(ct, &mut [])
        };
        self.life.finish_quantum(r, connected);
        false
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    const Q: usize = QUANTUM;

    fn tri(phase: f32) -> [f32; Q] {
        let mut b = [0.0f32; Q];
        for (i, x) in b.iter_mut().enumerate() {
            let p = (i as f32 * 0.031 + phase).fract();
            *x = 0.3 * (4.0 * (p - 0.5).abs() - 1.0);
        }
        b
    }

    /// The JS loop for one channel, in f64, from the given state; returns (outputs, att, sus,
    /// avg) with the envelopes not rounded.
    fn reference(
        t: &Transient,
        input: &[f32; Q],
        att0: f64,
        sus0: f64,
        avg0: f64,
    ) -> ([f32; Q], f64, f64, f64) {
        let (mut att, mut sus, mut avg) = (att0, sus0, avg0);
        let mut out = [0.0f32; Q];
        for n in 0..Q {
            let s = f64::from(input[n]);
            let x = s.abs();
            att = t.attack_coeff * (x - att) + att;
            sus = t.sustain_coeff * (x - sus) + sus;
            let pk = jsmath::clamp((t.scaling * (att - sus)) / (sus + 1e-6), -1.5, 1.5);
            let att_s = if pk > 0.0 { pk } else { 0.0 };
            let sus_s = if pk < 0.0 { -pk } else { 0.0 };
            let a_g = jsmath::pow(10.0, ((t.attack_amt * att_s) * 18.0) / 20.0);
            let s_g = jsmath::pow(10.0, ((t.sustain_amt * sus_s) * 36.0) / 20.0);
            let g = jsmath::clamp(a_g * s_g, 0.0, 8.0);
            avg = t.gain_coeff * (g - avg) + avg;
            let makeup = if avg > 1e-3 { 1.0 / avg } else { 1.0 };
            let wet = (s * g) * makeup;
            let y = t.mix * (wet - s) + s;
            out[n] = (y / (1.0 + y.abs())) as f32;
        }
        (out, att, sus, avg)
    }

    #[test]
    fn constructor_coefficients() {
        let t = Transient::new(Some(0.8), None, 0.0, 1.0);
        let c = |time: f64| 1.0 - jsmath::exp(-(1.0 / 44100.0) / time);
        assert_eq!(t.gain_coeff.to_bits(), c(0.2).to_bits());
        assert_eq!(t.attack_coeff.to_bits(), c(0.003).to_bits());
        assert_eq!(t.sustain_coeff.to_bits(), c(0.08).to_bits());
        assert_eq!((t.attack_amt, t.sustain_amt), (0.8, 0.0));
        assert_eq!((t.scaling, t.mix, t.avg_gain), (1.0, 1.0, 1.0));
        // Amounts clamp to [-1, 1]; NaN stays NaN (JS clamp).
        let t = Transient::new(Some(5.0), Some(-3.0), 0.0, 1.0);
        assert_eq!((t.attack_amt, t.sustain_amt), (1.0, -1.0));
        let t = Transient::new(Some(f64::NAN), None, 0.0, 1.0);
        assert!(t.attack_amt.is_nan());
        let t = Transient::new(None, None, 0.0, 1.0);
        assert_eq!((t.attack_amt, t.sustain_amt), (0.0, 0.0));
    }

    #[test]
    fn gating_at_begin_and_end() {
        let mut t = Transient::new(Some(0.5), None, 0.25, 0.5);
        let mut io = [tri(0.0)];
        assert_eq!(
            t.process(0.25, &mut io),
            Ok(true),
            "ct == begin: zeros, true"
        );
        assert!(io[0].iter().all(|x| x.to_bits() == 0));
        assert_eq!(t.process(0.1, &mut io), Ok(true));
        let mut io = [tri(0.0)];
        assert_eq!(
            t.process(0.5, &mut io),
            Ok(false),
            "ct == end: zeros, false"
        );
        assert!(io[0].iter().all(|x| x.to_bits() == 0));
        assert_eq!(t.avg_gain, 1.0, "no state change while gated");
        let mut io = [tri(0.0)];
        assert_eq!(t.process(0.3, &mut io), Ok(true));
        assert!(io[0].iter().any(|x| *x != 0.0));
        // A double begin just below a quantum start: that quantum runs.
        let begin = 256.0 / 44100.0 - 1e-15;
        let mut t = Transient::new(Some(0.5), None, begin, 1.0);
        let mut io = [tri(0.0)];
        t.process(worklet::current_time(256), &mut io).unwrap();
        assert!(io[0].iter().any(|x| *x != 0.0));
    }

    #[test]
    fn transient_zero_still_soft_clips() {
        let mut t = Transient::new(Some(0.0), None, -1.0, 1.0);
        let mut io = [[0.5f32; Q]];
        t.process(0.0, &mut io).unwrap();
        // g is 1, makeup about 1: y = lerp(0.5, 0.5 * 1 * makeup, 1) then 0.5 / 1.5.
        let (want, ..) = reference(
            &Transient::new(Some(0.0), None, -1.0, 1.0),
            &[0.5; Q],
            0.0,
            0.0,
            1.0,
        );
        assert_eq!(io[0][0].to_bits(), want[0].to_bits());
        assert_eq!(io[0][0], (0.5f64 / 1.5) as f32);
    }

    #[test]
    fn matches_the_js_loop_and_stores_f32_envelopes() {
        let mut t = Transient::new(Some(0.8), None, -1.0, 10.0);
        let r0 = t.clone();
        let a = tri(0.0);
        let mut io = [a];
        t.process(0.0, &mut io).unwrap();
        let (want, att, sus, avg) = reference(&r0, &a, 0.0, 0.0, 1.0);
        assert_eq!(io[0].map(f32::to_bits), want.map(f32::to_bits));
        assert_eq!(t.attack_env[0].to_bits(), (att as f32).to_bits());
        assert_eq!(t.sustain_env[0].to_bits(), (sus as f32).to_bits());
        assert_ne!(
            f64::from(t.attack_env[0]),
            att,
            "the stored envelope is rounded"
        );
        assert_eq!(
            t.avg_gain.to_bits(),
            avg.to_bits(),
            "avgGain stays a double"
        );
        // The next block starts from the f32 envelopes.
        let b = tri(0.4);
        let mut io = [b];
        let r1 = t.clone();
        t.process(0.01, &mut io).unwrap();
        let (want, ..) = reference(
            &r1,
            &b,
            f64::from(r1.attack_env[0]),
            f64::from(r1.sustain_env[0]),
            r1.avg_gain,
        );
        assert_eq!(io[0].map(f32::to_bits), want.map(f32::to_bits));
    }

    #[test]
    fn avg_gain_carries_from_channel_zero_into_channel_one() {
        let mut t = Transient::new(Some(1.0), Some(-0.5), -1.0, 10.0);
        let r = t.clone();
        let (l, rr) = (tri(0.0), tri(0.3));
        let mut io = [l, rr];
        t.process(0.0, &mut io).unwrap();
        let (want_l, _, _, avg_l) = reference(&r, &l, 0.0, 0.0, 1.0);
        let (want_r, _, _, avg_r) = reference(&r, &rr, 0.0, 0.0, avg_l);
        assert_eq!(io[0].map(f32::to_bits), want_l.map(f32::to_bits));
        assert_eq!(io[1].map(f32::to_bits), want_r.map(f32::to_bits));
        assert_eq!(t.avg_gain.to_bits(), avg_r.to_bits());
        // A per-channel avgGain (each from 1) would differ.
        let (per_channel, ..) = reference(&r, &rr, 0.0, 0.0, 1.0);
        assert_ne!(io[1].map(f32::to_bits), per_channel.map(f32::to_bits));
    }

    #[test]
    fn mix_one_is_a_lerp_not_wet() {
        // With mix = 1, `lerp(s, wet, 1) = (wet - s) + s` can differ from `wet` in the last bit
        // of the double (the f32 store and the soft clip hide it in almost every sample).
        let (s, wet) = (0.7f64, 0.1f64);
        assert_ne!(lerp(s, wet, 1.0).to_bits(), wet.to_bits());
        assert_eq!(lerp(s, wet, 1.0).to_bits(), ((wet - s) + s).to_bits());
        // The processor equals the reference loop (which lerps) on full-boost steps.
        for i in 1..40 {
            let amp = i as f32 * 0.023;
            let t = Transient::new(Some(1.0), None, -1.0, 10.0);
            let input = [amp; Q];
            let (want, ..) = reference(&t, &input, 0.0, 0.0, 1.0);
            let mut t2 = t.clone();
            let mut io = [input];
            t2.process(0.0, &mut io).unwrap();
            assert_eq!(io[0].map(f32::to_bits), want.map(f32::to_bits), "amp {amp}");
        }
    }

    #[test]
    fn the_sustain_branch() {
        // transient("0:-0.8"): a decaying input makes the sustain envelope lead, pk < 0, and the
        // sustain gain cuts.
        let mut t = Transient::new(Some(0.0), Some(-0.8), -1.0, 10.0);
        t.sustain_env = [0.5; 2];
        t.attack_env = [0.0; 2];
        let input = [0.01f32; Q];
        let r = t.clone();
        let mut io = [input];
        t.process(0.0, &mut io).unwrap();
        let (want, ..) = reference(&r, &input, 0.0, 0.5, 1.0);
        assert_eq!(io[0].map(f32::to_bits), want.map(f32::to_bits));
        // pk < 0 at the first sample: sus_s > 0 and the sustain gain cuts below unity.
        let att = r.attack_coeff * (0.01 - 0.0) + 0.0;
        let sus = r.sustain_coeff * (0.01 - 0.5) + 0.5;
        let pk = jsmath::clamp((r.scaling * (att - sus)) / (sus + 1e-6), -1.5, 1.5);
        assert!(pk < -0.9, "{pk}");
        let s_g = db_to_lin((r.sustain_amt * -pk) * 36.0);
        assert!(s_g < 0.05, "{s_g}");
        assert!(f64::from(io[0][0]) < 0.01 / 1.01);
    }

    #[test]
    fn node_teardown() {
        // Begin 0, end at 0.01 s (frame 441): the quantum at 0 is gated (ct == begin), 128..384
        // run, from 512 (ct >= end) zeros. The input goes at 640: finished from the quantum after.
        let mut node = TransientNode::new(Transient::new(Some(0.5), None, 0.0, 0.01));
        assert_eq!(node.end_quantum(), Some(512));
        let mut run = |q: u64| {
            let mut io = [tri(0.0)];
            let silent = node.process(q, &mut io, q < 640);
            (silent, io[0].iter().any(|x| *x != 0.0))
        };
        assert_eq!(run(0), (false, false));
        assert_eq!(run(128), (false, true));
        assert_eq!(run(384), (false, true));
        assert_eq!(
            run(512),
            (false, false),
            "after end: zeros, alive (input connected)"
        );
        assert_eq!(
            run(640),
            (true, false),
            "input gone and returned false: finished"
        );
        assert_eq!(run(768), (true, false));
        // Input gone before end: the processor runs on with no channels (zeros), returns true,
        // and finishes at its first false.
        let mut node = TransientNode::new(Transient::new(Some(0.5), None, 0.0, 0.01));
        let mut io = [tri(0.0)];
        assert!(!node.process(256, &mut io, false));
        assert!(io[0].iter().all(|x| x.to_bits() == 0));
        let mut io = [tri(0.0)];
        assert!(
            !node.process(512, &mut io, false),
            "returns false in this quantum"
        );
        assert!(node.process(640, &mut io, false), "gone");
        // A NaN end never reaches a quantum.
        let node = TransientNode::new(Transient::new(Some(0.5), None, 0.0, f64::NAN));
        assert_eq!(node.end_quantum(), None);
    }
}
