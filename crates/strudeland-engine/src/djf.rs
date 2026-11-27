//! superdough's orbit DJ filter worklet (`DJFProcessor` with its `TwoPoleFilter`s,
//! worklets.mjs:298-363), inserted by `Orbit.getDjf` (superdoughoutput.mjs:47-57) between the
//! orbit's summing node and its output on the first hap with `djf != null`.
//!
//! `value` (default 0.5, full range, a-rate) is read once per quantum at its first frame and
//! clamped to [0, 1]: above 0.51 a highpass (`in - lowpass`) at `((value - 0.5) * 22)^4` Hz,
//! below 0.49 a lowpass at `(value * 22)^4` Hz, in between (compared as doubles with the float
//! value, so `float32(0.51)` is in it) the input passes and the filter state is left untouched.
//! The filters are a state-variable two-pole (resonance 0.1) per channel, in doubles. The input
//! is the explicit-stereo summing node, always two channels; the processor always returns `true`
//! with its input connected, so the slot processes every quantum and is never silent.

use crate::QUANTUM;
use crate::jsmath;
use crate::param::{Param, ParamInputs, Rate};
use crate::worklet::{self, INVSR, SR};

/// worklets.mjs `PI = Math.PI`.
const PI: f64 = std::f64::consts::PI;

/// The `value` descriptor's default.
pub const DEFAULT_VALUE: f32 = 0.5;

/// What one quantum does with the signal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Copy the input; the state is not touched.
    None,
    /// Output the two-pole lowpass at `cutoff` Hz.
    Lopass(f64),
    /// Output the input minus the lowpass at `cutoff` Hz.
    Hipass(f64),
}

/// The mode of a quantum whose `value` block starts with `p0` (worklets.mjs:330-343).
pub fn mode(p0: f32) -> Mode {
    let value = jsmath::clamp(f64::from(p0), 0.0, 1.0);
    // NaN fails both comparisons: 'none'.
    if value > 0.51 {
        Mode::Hipass(jsmath::pow(((value - 0.5) * 2.0) * 11.0, 4.0))
    } else if value < 0.49 {
        Mode::Lopass(jsmath::pow((value * 2.0) * 11.0, 4.0))
    } else {
        Mode::None
    }
}

/// `TwoPoleFilter.update`'s per-call constants for `cutoff` (resonance 0.1): `(c, mrc)`.
pub fn two_pole_coefficients(cutoff: f64) -> (f64, f64) {
    let resonance = jsmath::clamp(0.1, 0.0, 1.0);
    let cutoff = jsmath::clamp(cutoff, 0.0, SR / 2.0 - 1.0);
    let c = jsmath::clamp(2.0 * jsmath::sin(cutoff * PI * INVSR), 0.0, 1.14);
    let r = jsmath::pow(0.5, 8.0 * resonance + 1.0);
    let mrc = 1.0 - r * c;
    (c, mrc)
}

/// The orbit's `djf-processor` node.
#[derive(Debug, Clone)]
pub struct Djf {
    pub value: Param,
    s0: [f64; 2],
    s1: [f64; 2],
}

impl Default for Djf {
    fn default() -> Self {
        Djf::new()
    }
}

impl Djf {
    /// A new node: `value` at its default, both filters at rest.
    pub fn new() -> Djf {
        Djf {
            value: worklet::param(DEFAULT_VALUE, Rate::A),
            s0: [0.0; 2],
            s1: [0.0; 2],
        }
    }

    /// One quantum in place on the summing node's two channels. Returns the output's silence
    /// hint: always `false` (the processor lives and its tail is infinite).
    #[cfg(test)]
    pub fn process(&mut self, q: u64, bus: &mut [[f32; QUANTUM]; 2]) -> bool {
        self.process_with(q, bus, &ParamInputs::NONE)
    }

    /// `Djf::process` with the connections into `value` (orbit-held modulators).
    pub fn process_with(
        &mut self,
        q: u64,
        bus: &mut [[f32; QUANTUM]; 2],
        value_inputs: &ParamInputs,
    ) -> bool {
        let mut block = [0.0f32; QUANTUM];
        worklet::fill_block(&mut self.value, q, value_inputs, &mut block);
        let (cutoff, lopass) = match mode(block[0]) {
            Mode::None => return false,
            Mode::Lopass(c) => (c, true),
            Mode::Hipass(c) => (c, false),
        };
        // `c` and `r` depend only on the cutoff: computed once (the same bits as per sample).
        let (c, mrc) = two_pole_coefficients(cutoff);
        for (i, ch) in bus.iter_mut().enumerate() {
            let (mut s0, mut s1) = (self.s0[i], self.s1[i]);
            for x in ch.iter_mut() {
                let s = f64::from(*x);
                s0 = mrc * s0 - c * s1 + c * s;
                s1 = mrc * s1 + c * s0;
                *x = if lopass { s1 } else { s - s1 } as f32;
            }
            self.s0[i] = s0;
            self.s1[i] = s1;
        }
        false
    }

    /// The filter state `(s0, s1)` of channel `ch`.
    #[cfg(test)]
    pub fn state(&self, ch: usize) -> (f64, f64) {
        (self.s0[ch], self.s1[ch])
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    const Q: usize = QUANTUM;

    fn bits(x: &[f32]) -> Vec<u32> {
        x.iter().map(|v| v.to_bits()).collect()
    }

    fn signal(c: usize, k: usize) -> [f32; Q] {
        let mut s = [0.0f32; Q];
        for i in 0..Q {
            let n = (k * Q + i) as f32;
            s[i] = ((n * 0.071 + c as f32).sin() * 0.4) + 0.2 * ((n * 0.0031).sin());
        }
        s
    }

    /// The JS `update` per sample, transcribed literally (sin and pow every sample).
    fn reference(x: &[f32; Q], cutoff: f64, s0: &mut f64, s1: &mut f64, lopass: bool) -> [f32; Q] {
        let clamp = jsmath::clamp;
        let mut out = [0.0f32; Q];
        for n in 0..Q {
            let resonance = clamp(0.1, 0.0, 1.0);
            let cutoff = clamp(cutoff, 0.0, 44100.0 / 2.0 - 1.0);
            let c = clamp(
                2.0 * jsmath::sin(cutoff * std::f64::consts::PI * (1.0 / 44100.0)),
                0.0,
                1.14,
            );
            let r = jsmath::pow(0.5, 8.0 * resonance + 1.0);
            let mrc = 1.0 - r * c;
            let s = f64::from(x[n]);
            *s0 = mrc * *s0 - c * *s1 + c * s;
            *s1 = mrc * *s1 + c * *s0;
            out[n] = if lopass { *s1 } else { s - *s1 } as f32;
        }
        out
    }

    #[test]
    fn dead_zone_and_float_comparison() {
        assert_eq!(mode(0.5), Mode::None);
        assert_eq!(mode(0.49), Mode::None, "float32(0.49) > 0.49 as a double");
        assert!(f64::from(0.49f32) > 0.49);
        // float32(0.51) is just below the double 0.51: still none.
        assert!(f64::from(0.51f32) < 0.51);
        assert_eq!(mode(0.51), Mode::None);
        assert_eq!(mode(0.505), Mode::None);
        assert!(matches!(mode(0.5101), Mode::Hipass(_)));
        assert!(matches!(mode(0.4899), Mode::Lopass(_)));
        // NaN selects none; the clamp maps out-of-range values to the ends.
        assert_eq!(mode(f32::NAN), Mode::None);
        assert_eq!(mode(-3.0), Mode::Lopass(0.0));
        assert_eq!(mode(7.0), mode(1.0));
        assert_eq!(mode(f32::INFINITY), mode(1.0));
    }

    #[test]
    fn cutoffs_for_point_two_and_point_eight() {
        // djf(0.2): (0.4 * 11)^4 = 4.4^4 ~ 374.8 Hz; djf(0.8): 6.6^4 ~ 1897.5 Hz.
        let v = f64::from(0.2f32);
        let Mode::Lopass(lo) = mode(0.2) else {
            panic!("0.2 is a lowpass")
        };
        assert_eq!(lo.to_bits(), jsmath::pow((v * 2.0) * 11.0, 4.0).to_bits());
        assert!((lo - 374.8).abs() < 0.05, "{lo}");
        let v = f64::from(0.8f32);
        let Mode::Hipass(hi) = mode(0.8) else {
            panic!("0.8 is a highpass")
        };
        assert_eq!(
            hi.to_bits(),
            jsmath::pow(((v - 0.5) * 2.0) * 11.0, 4.0).to_bits()
        );
        assert!((hi - 1897.5).abs() < 0.1, "{hi}");
    }

    #[test]
    fn coefficients_clamp() {
        // The cutoff clamps to 22049 Hz; c to 1.14.
        let (c, _) = two_pole_coefficients(1e9);
        let (c2, _) = two_pole_coefficients(22049.0);
        assert_eq!(c.to_bits(), c2.to_bits());
        assert!(c <= 1.14);
        let (c, mrc) = two_pole_coefficients(0.0);
        assert_eq!((c, mrc), (0.0, 1.0));
        let (c, _) = two_pole_coefficients(-5.0);
        assert_eq!(c, 0.0);
        // r = 0.5^1.8.
        let (c, mrc) = two_pole_coefficients(1000.0);
        let r = jsmath::pow(0.5, 1.8);
        assert_eq!(mrc.to_bits(), (1.0 - r * c).to_bits());
    }

    #[test]
    fn djf_filter_state_is_f64() {
        for (value, lopass) in [(0.2f32, true), (0.8, false), (0.0, true), (1.0, false)] {
            let mut d = Djf::new();
            d.value.set_value(value, 0).unwrap();
            let cutoff = match mode(value) {
                Mode::Lopass(c) | Mode::Hipass(c) => c,
                Mode::None => unreachable!(),
            };
            let (mut s0, mut s1) = ([0.0f64; 2], [0.0f64; 2]);
            for k in 0..5 {
                let mut bus = [signal(0, k), signal(1, k)];
                let input = bus;
                assert!(!d.process((k * Q) as u64, &mut bus));
                for ch in 0..2 {
                    let want = reference(&input[ch], cutoff, &mut s0[ch], &mut s1[ch], lopass);
                    assert_eq!(
                        bits(&bus[ch]),
                        bits(&want),
                        "value {value} quantum {k} ch {ch}"
                    );
                    // The state is the unrounded double.
                    assert_eq!(d.state(ch).0.to_bits(), s0[ch].to_bits());
                    assert_eq!(d.state(ch).1.to_bits(), s1[ch].to_bits());
                }
            }
            if value == 0.2 {
                assert!(
                    d.state(0).1 as f32 as f64 != d.state(0).1,
                    "the state keeps double precision"
                );
            }
        }
    }

    #[test]
    fn djf_none_leaves_state() {
        let mut d = Djf::new();
        d.value.set_value(0.2, 0).unwrap();
        let mut bus = [signal(0, 0), signal(1, 0)];
        d.process(0, &mut bus);
        let st = (d.state(0), d.state(1));
        d.value.set_value(0.5, 128).unwrap();
        let mut bus = [signal(0, 1), signal(1, 1)];
        let input = bus;
        assert!(!d.process(128, &mut bus));
        assert_eq!(bits(&bus[0]), bits(&input[0]));
        assert_eq!(bits(&bus[1]), bits(&input[1]));
        assert_eq!((d.state(0), d.state(1)), st, "stale in none");
        // Back to 0.2: resumes from the stale state.
        d.value.set_value(0.2, 256).unwrap();
        let mut bus = [signal(0, 2), signal(1, 2)];
        let input = bus;
        d.process(256, &mut bus);
        let Mode::Lopass(c) = mode(0.2) else {
            unreachable!()
        };
        let (mut s0, mut s1) = st.0;
        assert_eq!(
            bits(&bus[0]),
            bits(&reference(&input[0], c, &mut s0, &mut s1, true))
        );
    }

    #[test]
    fn state_is_kept_across_switches() {
        // Lowpass then highpass: the highpass starts from the lowpass's state.
        let mut d = Djf::new();
        d.value.set_value(0.2, 0).unwrap();
        let mut bus = [signal(0, 0), signal(1, 0)];
        d.process(0, &mut bus);
        let (mut s0, mut s1) = d.state(0);
        d.value.set_value(0.8, 128).unwrap();
        let mut bus = [signal(0, 1), signal(1, 1)];
        let input = bus;
        d.process(128, &mut bus);
        let Mode::Hipass(c) = mode(0.8) else {
            unreachable!()
        };
        assert_eq!(
            bits(&bus[0]),
            bits(&reference(&input[0], c, &mut s0, &mut s1, false))
        );
    }

    #[test]
    fn djf_never_silent() {
        // Default 0.5 (none) on zeros, and a lowpass on zeros: never a silent hint.
        let mut d = Djf::new();
        let mut bus = [[0.0f32; Q]; 2];
        assert!(!d.process(0, &mut bus));
        d.value.set_value(0.1, 128).unwrap();
        assert!(!d.process(128, &mut bus));
        assert!(bus.iter().flatten().all(|x| x.to_bits() == 0));
    }

    #[test]
    fn djf_value_change_lands_on_quantum_start() {
        // setValueAtTime(0.8, t) with ceil(t * sr) = 33075: the first quantum with q >= 33075 is
        // 33152; the quantum 32896..33024 (and 33024, which holds the event) still read 0.2.
        let mut d = Djf::new();
        d.value.set_value(0.2, 0).unwrap();
        d.value.set_value_at_time(0.8, 0.75, 0).unwrap();
        let read = |d: &mut Djf, q: u64| {
            let mut b = [0.0f32; Q];
            worklet::fill_block(&mut d.value, q, &ParamInputs::NONE, &mut b);
            b[0]
        };
        assert_eq!(read(&mut d, 32896), 0.2);
        assert_eq!(read(&mut d, 33024), 0.2);
        assert_eq!(read(&mut d, 33152), 0.8);
    }
}
