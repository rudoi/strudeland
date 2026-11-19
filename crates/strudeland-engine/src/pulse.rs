//! Strudeland's stereo pulse worklet. Both channels share a DC-centered rectangle with
//! four-point, integrated cubic B-spline corrections at its fixed and moving edges. Their
//! support spans two frames on either side, so the oscillator delays output by two frames.
//! Frequency (including signed FM), detune and pulse width are all read at audio rate.

use crate::QUANTUM;
use crate::jsmath;
use crate::param::{Param, ParamInputs, Rate};
use crate::worklet::{self, INVSR, Lifecycle, ProcessorError};

/// Descriptor indices (the order of the [`PulseInputs`] a render takes).
pub const BEGIN: usize = 0;
pub const END: usize = 1;
pub const FREQUENCY: usize = 2;
pub const DETUNE: usize = 3;
pub const PULSEWIDTH: usize = 4;
/// The number of params.
pub const PARAMS: usize = 5;

const GAIN: f64 = 0.128;

/// The connection inputs of the five params for one quantum, in descriptor order.
pub type PulseInputs = [ParamInputs; PARAMS];

/// No param connected.
pub const NO_INPUTS: PulseInputs = [
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
];

/// The processor's state, all doubles.
#[derive(Debug, Clone, PartialEq)]
pub struct Pulse {
    phase: f64,
    duty: f64,
    samples: [f64; 4],
    older_gain: f64,
    previous_gain: f64,
    /// `Math.pow(2, detune / 100 / 12)` for the last `detune` input (by its bits): the
    /// per-sample `Math.pow` is pure, so an unchanged input reuses it.
    detune_factor: Option<(u32, f64)>,
}

impl Default for Pulse {
    fn default() -> Self {
        Pulse::new()
    }
}

impl Pulse {
    /// A fresh voice starts at phase zero with an empty two-frame delay.
    pub fn new() -> Pulse {
        Pulse {
            phase: 0.0,
            duty: 0.0,
            samples: [0.0; 4],
            older_gain: 0.0,
            previous_gain: 0.0,
            detune_factor: None,
        }
    }

    /// An edge between n and n+1 contributes to samples n-1 through n+2. Forward
    /// exact hits belong to the interval ending there, backward hits to the interval
    /// leaving there. A moving width can cross two integers in one interval.
    fn correct_edge(&mut self, start: f64, end: f64, jump: f64) {
        let mut cell = start.floor() as i32;
        let last = end.floor() as i32;
        if cell == last {
            return;
        }
        let direction = if last > cell { 1 } else { -1 };
        let scale = f64::from(direction) * jump / 24.0;
        while cell != last {
            let edge = if direction > 0 { cell + 1 } else { cell };
            let t = (f64::from(edge) - start) / (end - start);
            let left = 1.0 - t;
            let left_outer = 2.0 - t;
            let right_outer = 1.0 + t;
            let a = left * left;
            let b = left_outer * left_outer;
            let c = right_outer * right_outer;
            let d = t * t;
            let tail_left = a * a;
            let tail_right = d * d;
            self.samples[0] += scale * tail_left;
            self.samples[1] += scale * (b * b - 4.0 * tail_left);
            self.samples[2] -= scale * (c * c - 4.0 * tail_right);
            self.samples[3] -= scale * tail_right;
            cell += direction;
        }
    }

    fn sample(&mut self, step: f64, width: f64) -> f64 {
        let speed = step.abs();
        if !step.is_finite() || !width.is_finite() || speed >= 0.5 {
            self.samples = [0.0; 4];
            self.older_gain = 0.0;
            self.previous_gain = 0.0;
            return 0.0;
        }
        let duty = (1.0 - width.clamp(-0.99, 0.99)) * 0.5;
        let end = self.phase + step;
        let moving_edge = end - duty;
        let cycle = end.floor();
        self.samples[2] += 2.0 * (duty - (cycle - moving_edge.floor()));
        if self.previous_gain != 0.0 {
            self.correct_edge(self.phase, end, -2.0);
            self.correct_edge(self.phase - self.duty, moving_edge, 2.0);
        }
        let output = if self.older_gain == 0.0 {
            0.0
        } else {
            self.samples[0] * self.older_gain
        };
        self.samples = [self.samples[1], self.samples[2], self.samples[3], 0.0];
        // A tiny negative end can wrap to exactly 1 after rounding.
        self.phase = (end - cycle).min(1.0 - f64::EPSILON / 2.0);
        self.duty = duty;
        self.older_gain = self.previous_gain;
        self.previous_gain = GAIN * ((0.5 - speed) / 0.1).min(1.0);
        output
    }

    /// One `process()` call at `currentTime` `ct` with the five param blocks (descriptor order),
    /// writing both channels of `out` (zeroed by the caller). Zeros and `Ok(true)` up to `begin`
    /// (checked first), zeros and `Ok(false)` from `end` on.
    pub fn process(
        &mut self,
        ct: f64,
        p: &[[f32; QUANTUM]; PARAMS],
        out: &mut [[f32; QUANTUM]; 2],
    ) -> Result<bool, ProcessorError> {
        if !worklet::started(ct, p[BEGIN][0]) {
            return Ok(true);
        }
        if worklet::reached(ct, p[END][0]) {
            return Ok(false);
        }
        for i in 0..QUANTUM {
            let detune_bits = p[DETUNE][i].to_bits();
            let detune_factor = match self.detune_factor {
                Some((bits, f)) if bits == detune_bits => f,
                _ => {
                    let detune = f64::from(p[DETUNE][i]);
                    let f = if detune.is_finite() {
                        jsmath::pow(2.0, (detune / 100.0) / 12.0)
                    } else {
                        f64::NAN
                    };
                    self.detune_factor = Some((detune_bits, f));
                    f
                }
            };
            let freq = f64::from(p[FREQUENCY][i]) * detune_factor;
            let v = self.sample(freq * INVSR, f64::from(p[PULSEWIDTH][i])) as f32;
            out[0][i] = v;
            out[1][i] = v;
        }
        Ok(true)
    }
}

/// The pulse's `AudioWorkletNode`: the processor, its five params and its lifecycle (no input,
/// so it finishes at its first `false`).
#[derive(Debug, Clone)]
pub struct PulseNode {
    pub proc: Pulse,
    /// In descriptor order: `begin`, `end`, `frequency`, `detune`, `pulsewidth`, all a-rate,
    /// defaults 0, 0, 440, 0, 1, full ranges.
    pub params: [Param; PARAMS],
    life: Lifecycle,
}

impl Default for PulseNode {
    fn default() -> Self {
        PulseNode::new()
    }
}

impl PulseNode {
    /// `new AudioWorkletNode(ac, 'pulse-oscillator', { outputChannelCount: [2] })`.
    pub fn new() -> PulseNode {
        PulseNode {
            proc: Pulse::new(),
            params: [
                worklet::param(0.0, Rate::A),
                worklet::param(0.0, Rate::A),
                worklet::param(440.0, Rate::A),
                worklet::param(0.0, Rate::A),
                worklet::param(1.0, Rate::A),
            ],
            life: Lifecycle::new(),
        }
    }

    /// Whether the processor has finished (its node outputs a silent zero bus).
    pub fn gone(&self) -> bool {
        self.life.gone()
    }

    /// One quantum starting at frame `q` into `out` (overwritten). The params are rendered
    /// (pulling their connections, `inputs`) only while the processor lives. Returns the output's
    /// silence hint: false while the processor lives, true (zeros) once it finished.
    pub fn render(&mut self, q: u64, inputs: &PulseInputs, out: &mut [[f32; QUANTUM]; 2]) -> bool {
        *out = [[0.0; QUANTUM]; 2];
        if !self.life.begin_quantum(false) {
            return true;
        }
        let mut blocks = [[0.0f32; QUANTUM]; PARAMS];
        for ((p, input), block) in self.params.iter_mut().zip(inputs).zip(blocks.iter_mut()) {
            worklet::fill_block(p, q, input, block);
        }
        let r = self.proc.process(worklet::current_time(q), &blocks, out);
        self.life.finish_quantum(r, false);
        false
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    fn blocks(begin: f32, end: f32, freq: f32, pw: f32) -> [[f32; QUANTUM]; PARAMS] {
        [
            [begin; QUANTUM],
            [end; QUANTUM],
            [freq; QUANTUM],
            [0.0; QUANTUM],
            [pw; QUANTUM],
        ]
    }

    fn run(
        s: &mut Pulse,
        ct: f64,
        p: &[[f32; QUANTUM]; PARAMS],
    ) -> (Result<bool, ProcessorError>, [[f32; QUANTUM]; 2]) {
        let mut out = [[0.0f32; QUANTUM]; 2];
        let r = s.process(ct, p, &mut out);
        (r, out)
    }

    #[test]
    fn exact_edge_hits_in_both_directions() {
        for step in [0.25_f64, -0.25] {
            let mut pulse = Pulse::new();
            for _ in 0..8 {
                pulse.sample(step, 0.0);
            }
            // The period is four frames, so the two edges overlap on the plateaus.
            for level in [5.0 / 6.0, 0.0, -5.0 / 6.0, 0.0] {
                let expected = level * step.signum() * GAIN;
                assert!((pulse.sample(step, 0.0) - expected).abs() < 1e-14);
            }
        }
    }

    #[test]
    fn centered_rectangle_has_the_requested_negative_interval_and_no_dc() {
        for (width, negative_frames) in [(-0.5, 48), (0.0, 32), (0.5, 16), (0.875, 4)] {
            let mut pulse = Pulse::new();
            for _ in 0..64 {
                pulse.sample(1.0 / 64.0, width);
            }
            let cycle: [f64; 64] = std::array::from_fn(|_| pulse.sample(1.0 / 64.0, width));
            let duty = f64::from(negative_frames) / 64.0;
            // Only the exact edge and its immediate neighbors differ from the plateaus.
            let low = 2.0 * (duty - 1.0) * GAIN;
            let high = 2.0 * duty * GAIN;
            assert_eq!(
                cycle.iter().filter(|&&v| v == low).count(),
                negative_frames as usize - 3
            );
            assert_eq!(
                cycle.iter().filter(|&&v| v == high).count(),
                61 - negative_frames as usize
            );
            assert!(cycle.iter().sum::<f64>().abs() < 1e-12);
        }
        for (outside, limit) in [(1e30, 0.99), (-1e30, -0.99)] {
            let (mut a, mut b) = (Pulse::new(), Pulse::new());
            for _ in 0..256 {
                assert_eq!(a.sample(-0.007, outside), b.sample(-0.007, limit));
            }
        }
    }

    // Independently integrate the interval indicator against the cubic B-spline density.
    // The phase here is unwrapped throughout the trajectory; width and phase both move.
    fn weighted_occupancy(a: (f64, f64), b: (f64, f64), offset: f64) -> f64 {
        const BINS: usize = 8192;
        let mut area = 0.0;
        for i in 0..BINS {
            let t = (i as f64 + 0.5) / BINS as f64;
            let phase = a.0 + (b.0 - a.0) * t;
            let duty = a.1 + (b.1 - a.1) * t;
            if phase.rem_euclid(1.0) < duty {
                let r = (offset + t).abs();
                area += if r < 1.0 {
                    2.0 / 3.0 - r * r + 0.5 * r * r * r
                } else {
                    (2.0 - r).powi(3) / 6.0
                };
            }
        }
        area / BINS as f64
    }

    #[test]
    fn fractional_edges_under_signed_fm_and_moving_width_match_quadrature() {
        let mut pulse = Pulse::new();
        let mut phase = 0.0;
        // Starting at 0.875, moving forward while duty falls from 0.95 to 0.05 crosses
        // two integers of phase-minus-duty and the wrap itself, all in one interval.
        let steps = [-0.125, 0.375, 0.0, -0.375, -0.31, 0.21, 0.08, 0.0];
        let widths = [-0.9, 0.9, -0.8, 0.95, -0.65, 0.75, -0.99, 0.99];
        let mut path = Vec::new();
        let mut samples = Vec::new();
        for i in 0..128 {
            let step = steps[i % steps.len()];
            let width = widths[i % widths.len()];
            phase += step;
            path.push((phase, (1.0 - width) * 0.5));
            samples.push(pulse.sample(step, width));
        }
        for (i, points) in path.windows(5).enumerate() {
            let area: f64 = points
                .windows(2)
                .enumerate()
                .map(|(j, pair)| weighted_occupancy(pair[0], pair[1], j as f64 - 2.0))
                .sum();
            let expected = 2.0 * (points[2].1 - area);
            let actual = samples[i + 4] / GAIN;
            // Each crossing can shift a midpoint estimate by at most half a bin.
            assert!(
                (actual - expected).abs() < 0.0008,
                "frame {i}: {actual}, expected {expected}"
            );
        }
    }

    #[test]
    fn stationary_phase_still_responds_to_pwm() {
        let mut pulse = Pulse::new();
        pulse.sample(0.25, 0.75); // Phase 1/4, duty 1/8.
        for _ in 0..4 {
            pulse.sample(0.0, 0.75);
        }
        // Duty sweeps to 3/8, crossing the stationary phase halfway through the interval.
        for level in [
            0.25 - 1.0 / 192.0,
            0.25 - 77.0 / 192.0,
            -1.25 + 77.0 / 192.0,
            -1.25 + 1.0 / 192.0,
            -1.25,
        ] {
            assert!((pulse.sample(0.0, 0.25) - level * GAIN).abs() < 1e-14);
        }
    }

    #[test]
    fn folded_energy_falls_relative_to_a_sharp_pulse() {
        const N: usize = 256;
        const BIN: usize = 35;
        let step = BIN as f64 / N as f64;
        let mut pulse = Pulse::new();
        for _ in 0..N {
            pulse.sample(step, 0.5);
        }
        let corrected: Vec<_> = (0..N).map(|_| pulse.sample(step, 0.5) / GAIN).collect();
        let sharp: Vec<f64> = (0..N)
            .map(|i| if (i * BIN) % N < N / 4 { -1.5 } else { 0.5 })
            .collect();
        // A coherent tone can only have in-band harmonics at bins 35, 70 and 105.
        let folded_power = |cycle: &[f64]| {
            let mut power = 0.0;
            for k in (1..=N / 2).filter(|k| !k.is_multiple_of(BIN)) {
                let (mut re, mut im) = (0.0, 0.0);
                for (i, &v) in cycle.iter().enumerate() {
                    let theta = std::f64::consts::TAU * (k * i) as f64 / N as f64;
                    re += v * theta.cos();
                    im += v * theta.sin();
                }
                power += re * re + im * im;
            }
            power
        };
        assert!(folded_power(&corrected) < 0.1 * folded_power(&sharp));
    }

    #[test]
    fn nyquist_and_invalid_controls_mute_without_poisoning_the_voice() {
        let mut pulse = Pulse::new();
        pulse.sample(0.45, 0.5);
        assert!((pulse.previous_gain - 0.5 * GAIN).abs() < 1e-15);
        for (step, width) in [
            (0.5, 0.5),
            (-0.5, 0.5),
            (1e300, 0.5),
            (f64::NAN, 0.5),
            (0.0, f64::INFINITY),
        ] {
            let last_phase = pulse.phase;
            assert_eq!(pulse.sample(step, width), 0.0);
            assert_eq!(pulse.phase, last_phase);
            assert_eq!(pulse.sample(0.0, 0.5), 0.0); // Re-prime after muting.
            assert_eq!(pulse.sample(0.0, 0.5), 0.0);
            assert_eq!(pulse.sample(0.0, 0.5), GAIN * 0.5);
        }
        let mut p = blocks(0.0, 1.0, 220.0, 0.5);
        p[DETUNE][2..5].copy_from_slice(&[f32::NEG_INFINITY, f32::MAX, f32::NAN]);
        p[FREQUENCY][5] = f32::INFINITY;
        let (_, out) = run(&mut pulse, 0.01, &p);
        assert_eq!(out[0][2..8], [0.0; 6]);
        assert!(out[0][8..].iter().any(|&v| v != 0.0));
        assert!(out[0].iter().all(|v| v.is_finite() && v.abs() < 1.0));
    }

    #[test]
    fn a_rate_controls_and_delay_are_continuous_across_quanta() {
        let (mut node, mut continuous) = (Pulse::new(), Pulse::new());
        let mut p = blocks(0.0, 1.0, 440.0, 0.5);
        for q in 0..3 {
            for i in 0..QUANTUM {
                p[FREQUENCY][i] = if i < 64 { -220.0 } else { 440.0 };
                p[PULSEWIDTH][i] = (i as f32 - 64.0) / 80.0;
                p[DETUNE][i] = if (i + q).is_multiple_of(3) {
                    1200.0
                } else {
                    0.0
                };
            }
            let (_, out) = run(
                &mut node,
                worklet::current_time((q + 1) as u64 * QUANTUM as u64),
                &p,
            );
            for i in 0..QUANTUM {
                let octave = if p[DETUNE][i] == 1200.0 { 2.0 } else { 1.0 };
                let expected = continuous.sample(
                    f64::from(p[FREQUENCY][i]) * octave * INVSR,
                    f64::from(p[PULSEWIDTH][i]),
                ) as f32;
                assert_eq!(
                    out[0][i].to_bits(),
                    expected.to_bits(),
                    "quantum {q} frame {i}"
                );
                assert_eq!(
                    out[1][i].to_bits(),
                    expected.to_bits(),
                    "quantum {q} frame {i}"
                );
            }
        }
    }

    #[test]
    fn left_equals_right() {
        let mut s = Pulse::new();
        let (_, out) = run(&mut s, 0.01, &blocks(0.0, 10.0, 220.0, 0.3));
        for i in 0..QUANTUM {
            assert_eq!(out[0][i].to_bits(), out[1][i].to_bits(), "frame {i}");
        }
        assert!(out[0].iter().any(|x| *x != 0.0));
    }

    #[test]
    fn begin_and_end_gates() {
        let b = 0.1f32;
        let p = blocks(b, 0.2, 220.0, 0.5);
        let mut s = Pulse::new();
        let (r, out) = run(&mut s, f64::from(b), &p);
        assert_eq!(r, Ok(true));
        assert!(out.iter().flatten().all(|x| *x == 0.0));
        assert_eq!(s, Pulse::new(), "no state advances before the start");
        let (r, out) = run(&mut s, f64::from(0.2f32), &p);
        assert_eq!(r, Ok(false));
        assert!(out.iter().flatten().all(|x| *x == 0.0));
        // The begin test comes first: begin = end = 0 past 0 is gone at once.
        let mut s = Pulse::new();
        assert_eq!(
            run(&mut s, 0.01, &blocks(0.0, 0.0, 220.0, 0.5)).0,
            Ok(false)
        );
        let mut s = Pulse::new();
        // At ct = 0 with begin 0 it waits.
        assert_eq!(run(&mut s, 0.0, &blocks(0.0, 0.0, 220.0, 0.5)).0, Ok(true));
    }

    #[test]
    fn node_params_and_lifecycle() {
        let mut n = PulseNode::new();
        let d: Vec<f32> = n.params.iter().map(|p| p.default_value()).collect();
        assert_eq!(d, vec![0.0, 0.0, 440.0, 0.0, 1.0]);
        assert!(n.params.iter().all(
            |p| p.rate() == Rate::A && (p.min_value(), p.max_value()) == (-f32::MAX, f32::MAX)
        ));
        n.params[END].set_value(0.01, 0).unwrap();
        let mut out = [[0.0f32; QUANTUM]; 2];
        // Quantum 0: ct 0 <= begin 0, zeros, alive.
        assert!(!n.render(0, &NO_INPUTS, &mut out));
        assert!(!n.render(128, &NO_INPUTS, &mut out));
        assert!(out[0].iter().any(|x| *x != 0.0));
        // The first quantum reaching end returns false: zeros, not silent, then gone.
        let q_end = worklet::first_quantum_reaching(f64::from(0.01f32)).unwrap();
        assert!(!n.render(q_end, &NO_INPUTS, &mut out));
        assert!(out.iter().flatten().all(|x| *x == 0.0));
        assert!(n.gone());
        assert!(n.render(q_end + 128, &NO_INPUTS, &mut out));
    }
}
