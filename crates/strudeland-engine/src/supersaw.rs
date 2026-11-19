//! superdough's supersaw worklet (`SuperSawOscillatorProcessor`, worklets.mjs:467-580; built by
//! synth.mjs:153-219): `voices` polyBLEP sawtooths per frame, each detuned by the detuner
//! `n * (freqspread / (voices - 1)) - freqspread / 2` semitones, panned by swapped `sqrt` gains,
//! summed into a stereo output.
//!
//! Quirks kept: the params' `min`/`max` are not descriptor members, so every range is the whole
//! float range; `voices` is k-rate and may be fractional (`ceil(voices)` passes); the output is a
//! `Float32Array` the voices are added into with `+=`, so the sum is rounded to float32 after
//! every voice; the phases are drawn from `mulberry32(voiceSeed)` in voice order in the first
//! active quantum (the first whose start time is after the float32 `begin`), and advance only
//! while active; the processor outputs zeros from `end` on and finishes at `end + 0.5` s.
//!
//! One engine rule that is not superdough's: the voice loop is capped at [`MAX_VOICES`] passes.
//! Chromium would loop `ceil(voices)` times per frame (and, for a huge value, effectively hang);
//! the engine finishes the processor and reports a `hap` error instead.

use crate::QUANTUM;
use crate::js::{Mulberry32, random_from_seed};
use crate::jsmath;
use crate::param::{Param, ParamInputs, Rate};
use crate::worklet::{self, INVSR, Lifecycle, ProcessorError};

/// Descriptor indices (the order of the [`SupersawInputs`] a render takes).
pub const BEGIN: usize = 0;
pub const END: usize = 1;
pub const FREQUENCY: usize = 2;
pub const PANSPREAD: usize = 3;
pub const FREQSPREAD: usize = 4;
pub const DETUNE: usize = 5;
pub const VOICES: usize = 6;
/// The number of params.
pub const PARAMS: usize = 7;

/// The engine's cap on the voice loop (`ceil(voices)` passes per frame).
pub const MAX_VOICES: f64 = 4096.0;

/// The error a `voices` param beyond [`MAX_VOICES`] finishes the processor with.
pub const VOICES_OUT_OF_RANGE: ProcessorError = ProcessorError {
    processor: "supersaw",
    what: "voices out of range",
};

/// The connection inputs of the seven params for one quantum, in descriptor order.
pub type SupersawInputs = [ParamInputs; PARAMS];

/// No param connected.
pub const NO_INPUTS: SupersawInputs = [
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
    ParamInputs::NONE,
];

/// `polyBlep(phase, dt)` (worklets.mjs:54-71).
fn poly_blep(phase: f64, dt: f64) -> f64 {
    let dt = jsmath::min(dt, 1.0 - dt);
    let invdt = 1.0 / dt;
    if phase < dt {
        let p = phase * invdt;
        2.0 * p - jsmath::pow(p, 2.0) - 1.0
    } else if phase > 1.0 - dt {
        let p = (phase - 1.0) * invdt;
        jsmath::pow(p, 2.0) + 2.0 * p + 1.0
    } else {
        0.0
    }
}

/// `waveshapes.sawblep(phase, dt)`: `(2 * phase - 1) - polyBlep(phase, dt)`.
pub(crate) fn saw_blep(phase: f64, dt: f64) -> f64 {
    let v = 2.0 * phase - 1.0;
    v - poly_blep(phase, dt)
}

/// The factor of `applySemitoneDetuneToFrequency(frequency, detune) = frequency * Math.pow(2,
/// detune / 12)`: `Math.pow(2, detune / 12)`.
fn semitone_factor(detune: f64) -> f64 {
    jsmath::pow(2.0, detune / 12.0)
}

/// The processor's state: one phase per voice, drawn on first use.
#[derive(Debug, Clone, PartialEq)]
pub struct Supersaw {
    phase: Vec<f64>,
    random: Mulberry32,
    /// The semitone factor of the last `detune` input (by its bits): the per-sample `Math.pow`
    /// is pure, so an unchanged input reuses it.
    detune_factor: Option<(u32, f64)>,
    /// Each voice's semitone factor (`getDetuner`) for the `voices` and `freqspread` inputs
    /// (by their bits) it was computed for, filled as the voices are reached.
    voice_factors: (u32, u32, Vec<f64>),
}

impl Supersaw {
    /// `initialize({ seed })`: no phases yet, `randomFromSeed(seed)` (superdough passes the hap's
    /// `_voiceSeed`, a number, so this is always `mulberry32(seed)`).
    pub fn new(seed: u32) -> Supersaw {
        Supersaw {
            phase: Vec::new(),
            random: random_from_seed(seed),
            detune_factor: None,
            voice_factors: (0, 0, Vec::new()),
        }
    }

    /// One `process()` call at `currentTime` `ct` with the seven param blocks (descriptor order),
    /// adding into `out` (zeroed by the caller). `Ok(false)` from `end + 0.5` s on; zeros and
    /// `Ok(true)` before the first quantum after `begin`, from `end` on and while `begin` is
    /// negative; [`VOICES_OUT_OF_RANGE`] when an active quantum's `ceil(voices)` exceeds
    /// [`MAX_VOICES`].
    pub fn process(
        &mut self,
        ct: f64,
        p: &[[f32; QUANTUM]; PARAMS],
        out: &mut [[f32; QUANTUM]; 2],
    ) -> Result<bool, ProcessorError> {
        let begin = p[BEGIN][0];
        let end = p[END][0];
        let begin_defined = begin >= 0.0;
        let end_defined = end >= 0.0;
        if end_defined && ct >= f64::from(end) + 0.5 {
            return Ok(false);
        }
        if (end_defined && worklet::reached(ct, end))
            || !worklet::started(ct, begin)
            || !begin_defined
        {
            return Ok(true);
        }
        let voices = f64::from(p[VOICES][0]);
        if voices.ceil() > MAX_VOICES {
            return Err(VOICES_OUT_OF_RANGE);
        }
        for i in 0..QUANTUM {
            let detune = f64::from(p[DETUNE][i]);
            let freqspread = f64::from(p[FREQSPREAD][i]);
            let panspread = f64::from(p[PANSPREAD][i]) * 0.5 + 0.5;
            let mut gain_l = (1.0 - panspread).sqrt();
            let mut gain_r = panspread.sqrt();
            let detune_bits = p[DETUNE][i].to_bits();
            let detune_factor = match self.detune_factor {
                Some((bits, f)) if bits == detune_bits => f,
                _ => {
                    let f = semitone_factor(detune / 100.0);
                    self.detune_factor = Some((detune_bits, f));
                    f
                }
            };
            let freq = f64::from(p[FREQUENCY][i]) * detune_factor;
            let key = (p[VOICES][0].to_bits(), p[FREQSPREAD][i].to_bits());
            if (self.voice_factors.0, self.voice_factors.1) != key {
                self.voice_factors = (key.0, key.1, Vec::new());
            }
            // `getDetuner(voices, freqspread)`.
            let (scale, center) = if voices < 2.0 {
                (None, 0.0)
            } else {
                (Some(freqspread / (voices - 1.0)), freqspread * 0.5)
            };
            let mut n = 0.0f64;
            let mut k = 0usize;
            while n < voices {
                let factors = &mut self.voice_factors.2;
                if k == factors.len() {
                    let detuner = match scale {
                        None => 0.0,
                        Some(scale) => n * scale - center,
                    };
                    factors.push(semitone_factor(detuner));
                }
                let freq_voice = freq * factors[k];
                let dt = jsmath::frac(freq_voice * INVSR);
                if k == self.phase.len() {
                    self.phase.push(self.random.next());
                }
                let phase = self.phase[k];
                let v = saw_blep(phase, dt);
                out[0][i] = (f64::from(out[0][i]) + v * gain_l) as f32;
                out[1][i] = (f64::from(out[1][i]) + v * gain_r) as f32;
                let mut pn = phase + dt;
                if pn >= 1.0 {
                    pn -= 1.0;
                }
                self.phase[k] = pn;
                std::mem::swap(&mut gain_l, &mut gain_r);
                n += 1.0;
                k += 1;
            }
        }
        Ok(true)
    }
}

/// The supersaw's `AudioWorkletNode`: the processor, its seven params and its lifecycle (no
/// input, so it finishes at its first `false`).
#[derive(Debug, Clone)]
pub struct SupersawNode {
    pub proc: Supersaw,
    /// In descriptor order: `begin`, `end`, `frequency`, `panspread`, `freqspread`, `detune`
    /// (a-rate) and `voices` (k-rate), defaults -1, -1, 440, 0.4, 0.2, 0, 5, full ranges.
    pub params: [Param; PARAMS],
    life: Lifecycle,
    error: Option<ProcessorError>,
}

impl SupersawNode {
    /// `new AudioWorkletNode(ac, 'supersaw-oscillator', { outputChannelCount: [2],
    /// processorOptions: { seed } })`.
    pub fn new(seed: u32) -> SupersawNode {
        SupersawNode {
            proc: Supersaw::new(seed),
            params: [
                worklet::param(-1.0, Rate::A),
                worklet::param(-1.0, Rate::A),
                worklet::param(440.0, Rate::A),
                worklet::param(0.4, Rate::A),
                worklet::param(0.2, Rate::A),
                worklet::param(0.0, Rate::A),
                worklet::param(5.0, Rate::K),
            ],
            life: Lifecycle::new(),
            error: None,
        }
    }

    /// Whether the processor has finished (its node outputs a silent zero bus).
    pub fn gone(&self) -> bool {
        self.life.gone()
    }

    /// The error the processor finished with, reported once.
    pub fn take_error(&mut self) -> Option<ProcessorError> {
        self.error.take()
    }

    /// One quantum starting at frame `q` into `out` (overwritten). The params are rendered
    /// (pulling their connections, `inputs`) only while the processor lives. Returns the output's
    /// silence hint: false while the processor lives, true (zeros) once it finished.
    pub fn render(
        &mut self,
        q: u64,
        inputs: &SupersawInputs,
        out: &mut [[f32; QUANTUM]; 2],
    ) -> bool {
        *out = [[0.0; QUANTUM]; 2];
        if !self.life.begin_quantum(false) {
            return true;
        }
        let mut blocks = [[0.0f32; QUANTUM]; PARAMS];
        for ((p, input), block) in self.params.iter_mut().zip(inputs).zip(blocks.iter_mut()) {
            worklet::fill_block(p, q, input, block);
        }
        let r = self.proc.process(worklet::current_time(q), &blocks, out);
        if let Err(e) = r {
            *out = [[0.0; QUANTUM]; 2];
            self.error = Some(e);
        }
        self.life.finish_quantum(r, false);
        false
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;
    use crate::js::Mulberry32;

    use crate::QUANTUM_U64 as Q;

    /// Constant param blocks: begin, end, frequency, panspread, freqspread, detune, voices.
    fn blocks(
        begin: f32,
        end: f32,
        freq: f32,
        pan: f32,
        spread: f32,
        voices: f32,
    ) -> [[f32; QUANTUM]; PARAMS] {
        [
            [begin; QUANTUM],
            [end; QUANTUM],
            [freq; QUANTUM],
            [pan; QUANTUM],
            [spread; QUANTUM],
            [0.0; QUANTUM],
            [voices; QUANTUM],
        ]
    }

    fn run(
        s: &mut Supersaw,
        ct: f64,
        p: &[[f32; QUANTUM]; PARAMS],
    ) -> (Result<bool, ProcessorError>, [[f32; QUANTUM]; 2]) {
        let mut out = [[0.0f32; QUANTUM]; 2];
        let r = s.process(ct, p, &mut out);
        (r, out)
    }

    fn silent(out: &[[f32; QUANTUM]; 2]) -> bool {
        out.iter().flatten().all(|x| x.to_bits() == 0)
    }

    /// A literal transcription of the JS `process` loop (after the gates), written separately
    /// from [`Supersaw::process`]: getDetuner as a closure, `this.phase[n] ?? this.random()`.
    fn js_loop(
        phase: &mut Vec<Option<f64>>,
        random: &mut Mulberry32,
        p: &[[f32; QUANTUM]; PARAMS],
        out: &mut [[f32; QUANTUM]; 2],
    ) {
        let pv = |arr: &[f32; QUANTUM], n: usize| f64::from(arr[n]);
        let voices = f64::from(p[VOICES][0]);
        for i in 0..QUANTUM {
            let detune = pv(&p[DETUNE], i);
            let freqspread = pv(&p[FREQSPREAD], i);
            let panspread = pv(&p[PANSPREAD], i) * 0.5 + 0.5;
            let mut gl = (1.0 - panspread).sqrt();
            let mut gr = panspread.sqrt();
            let mut freq = pv(&p[FREQUENCY], i);
            freq *= jsmath::pow(2.0, (detune / 100.0) / 12.0);
            let detuner: Box<dyn Fn(f64) -> f64> = if voices < 2.0 {
                Box::new(|_| 0.0)
            } else {
                let scale = freqspread / (voices - 1.0);
                let center = freqspread * 0.5;
                Box::new(move |n| n * scale - center)
            };
            let mut n = 0usize;
            while (n as f64) < voices {
                let fv = freq * jsmath::pow(2.0, detuner(n as f64) / 12.0);
                let dt = jsmath::frac(fv * INVSR);
                if phase.len() <= n {
                    phase.resize(n + 1, None);
                }
                let ph = match phase[n] {
                    Some(x) => x,
                    None => random.next(),
                };
                phase[n] = Some(ph);
                // sawblep
                let d = jsmath::min(dt, 1.0 - dt);
                let inv = 1.0 / d;
                let blep = if ph < d {
                    let x = ph * inv;
                    2.0 * x - jsmath::pow(x, 2.0) - 1.0
                } else if ph > 1.0 - d {
                    let x = (ph - 1.0) * inv;
                    jsmath::pow(x, 2.0) + 2.0 * x + 1.0
                } else {
                    0.0
                };
                let v = (2.0 * ph - 1.0) - blep;
                out[0][i] = (f64::from(out[0][i]) + v * gl) as f32;
                out[1][i] = (f64::from(out[1][i]) + v * gr) as f32;
                let mut pn = ph + dt;
                if pn >= 1.0 {
                    pn -= 1.0;
                }
                phase[n] = Some(pn);
                std::mem::swap(&mut gl, &mut gr);
                n += 1;
            }
        }
    }

    #[test]
    fn transcription_matches_over_four_quanta() {
        for voices in [1.0f32, 2.5, 5.0] {
            let mut s = Supersaw::new(1234);
            let mut phase = Vec::new();
            let mut random = Mulberry32::new(1234);
            let mut p = blocks(0.0, 10.0, 220.0, 0.6, 0.3, voices);
            for k in 1..=4u64 {
                // A detune block with events: a ramp of cents inside the quantum.
                for i in 0..QUANTUM {
                    p[DETUNE][i] = (k as f32) * 13.5 - (i as f32) * 0.37;
                }
                let (r, out) = run(&mut s, worklet::current_time(k * Q), &p);
                assert_eq!(r, Ok(true));
                let mut expect = [[0.0f32; QUANTUM]; 2];
                js_loop(&mut phase, &mut random, &p, &mut expect);
                for c in 0..2 {
                    for i in 0..QUANTUM {
                        assert_eq!(
                            out[c][i].to_bits(),
                            expect[c][i].to_bits(),
                            "voices {voices} quantum {k} channel {c} frame {i}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn begin_gate_on_the_float32_begin() {
        // begin exactly a quantum start (as a float32 that equals the double quantum time
        // widened): the quantum does not start, the next one does.
        let q0 = 384u64;
        let ct0 = worklet::current_time(q0);
        let b = ct0 as f32;
        let p = blocks(b, 10.0, 440.0, 0.0, 0.2, 1.0);
        if f64::from(b) >= ct0 {
            let mut s = Supersaw::new(1);
            let (r, out) = run(&mut s, ct0, &p);
            assert_eq!(r, Ok(true));
            assert!(silent(&out), "ct == f32(begin) or below it does not start");
            assert!(s.phase.is_empty(), "no phase drawn before the start");
        }
        // One float32 ulp below the quantum start: that quantum starts.
        let below = f32::from_bits((ct0 as f32).to_bits() - 1);
        assert!(f64::from(below) < ct0);
        let mut s = Supersaw::new(1);
        let (r, out) = run(&mut s, ct0, &blocks(below, 10.0, 440.0, 0.0, 0.2, 1.0));
        assert_eq!(r, Ok(true));
        assert!(
            !silent(&out),
            "f32(begin) below the quantum start starts in it"
        );
        // The exact rule: started is ct > f64(begin).
        let at = 384.0f64 / 44100.0;
        assert_eq!(worklet::started(at, at as f32), at > f64::from(at as f32));
        // The supersaw-boundary onsets: frame 384 rounds up (starts at 512), frame 10624 rounds
        // down (starts at 10624).
        let up = 384.0f64 / 44100.0;
        assert!(f64::from(up as f32) > up);
        assert!(!worklet::started(worklet::current_time(384), up as f32));
        assert!(worklet::started(worklet::current_time(512), up as f32));
        let down = 10624.0f64 / 44100.0;
        assert!(f64::from(down as f32) < down);
        assert!(worklet::started(worklet::current_time(10624), down as f32));
    }

    #[test]
    fn negative_begin_never_starts_and_end_gates() {
        let mut s = Supersaw::new(1);
        let (r, out) = run(&mut s, 1.0, &blocks(-0.5, 10.0, 440.0, 0.0, 0.2, 1.0));
        assert_eq!(r, Ok(true));
        assert!(silent(&out));
        // The default begin -1 never starts.
        let (r, out) = run(&mut s, 1.0, &blocks(-1.0, -1.0, 440.0, 0.0, 0.2, 1.0));
        assert_eq!(r, Ok(true));
        assert!(silent(&out));
        // end: zeros and true at end, false from end + 0.5 on.
        let end = 0.25f32;
        let p = blocks(0.0, end, 440.0, 0.0, 0.2, 1.0);
        let mut s = Supersaw::new(1);
        assert!(!silent(&run(&mut s, f64::from(end) - 0.001, &p).1));
        let (r, out) = run(&mut s, f64::from(end), &p);
        assert_eq!(r, Ok(true));
        assert!(silent(&out), "ct >= end outputs zeros");
        let (r, _) = run(&mut s, f64::from(end) + 0.4999, &p);
        assert_eq!(r, Ok(true));
        let (r, out) = run(&mut s, f64::from(end) + 0.5, &p);
        assert_eq!(r, Ok(false), "the 0.5 s grace ends");
        assert!(silent(&out));
        // A negative end never ends.
        let mut s = Supersaw::new(1);
        let (r, out) = run(&mut s, 100.0, &blocks(0.0, -1.0, 440.0, 0.0, 0.2, 1.0));
        assert_eq!(r, Ok(true));
        assert!(!silent(&out));
    }

    #[test]
    fn phases_are_drawn_in_voice_order_on_the_first_active_frame() {
        for (voices, passes) in [(1.0f32, 1usize), (2.5, 3), (5.0, 5)] {
            let mut s = Supersaw::new(99);
            let mut m = Mulberry32::new(99);
            let draws: Vec<f64> = (0..passes).map(|_| m.next()).collect();
            let p = blocks(0.0, 10.0, 0.0, 0.0, 0.0, voices);
            // frequency 0: dt 0, the phases stay at their draws.
            assert_eq!(run(&mut s, 0.01, &p).0, Ok(true));
            assert_eq!(s.phase, draws, "voices {voices}");
        }
    }

    #[test]
    fn one_voice_has_no_detune_equal_gains_and_the_blep_saw() {
        let mut s = Supersaw::new(5);
        let first = Mulberry32::new(5).next();
        let p = blocks(0.0, 10.0, 441.0, 0.0, 0.2, 1.0);
        let (_, out) = run(&mut s, 0.01, &p);
        let dt = jsmath::frac(441.0 * INVSR);
        let g = 0.5f64.sqrt();
        let v = saw_blep(first, dt);
        assert_eq!(out[0][0].to_bits(), ((v * g) as f32).to_bits());
        assert_eq!(out[1][0], out[0][0]);
        assert_eq!(s.phase[0], {
            let mut x = first;
            for _ in 0..QUANTUM {
                x += dt;
                if x >= 1.0 {
                    x -= 1.0;
                }
            }
            x
        });
    }

    #[test]
    fn detuner_and_swapped_pan_gains() {
        // Two voices with spread: voice 0 at -spread/2, voice 1 at +spread/2 semitones; voice 0
        // gains (sqrt(1 - p), sqrt(p)), voice 1 swapped.
        let mut s = Supersaw::new(3);
        let mut m = Mulberry32::new(3);
        let (ph0, ph1) = (m.next(), m.next());
        let p = blocks(0.0, 10.0, 300.0, 0.6, 0.4, 2.0);
        let (_, out) = run(&mut s, 0.01, &p);
        let pan = f64::from(0.6f32) * 0.5 + 0.5;
        let (gl, gr) = ((1.0 - pan).sqrt(), pan.sqrt());
        let sp = f64::from(0.4f32);
        let f0 = 300.0 * jsmath::pow(2.0, (0.0 * (sp / 1.0) - sp * 0.5) / 12.0);
        let f1 = 300.0 * jsmath::pow(2.0, (1.0 * (sp / 1.0) - sp * 0.5) / 12.0);
        let v0 = saw_blep(ph0, jsmath::frac(f0 * INVSR));
        let v1 = saw_blep(ph1, jsmath::frac(f1 * INVSR));
        let l = ((f64::from((v0 * gl) as f32)) + v1 * gr) as f32;
        let r = ((f64::from((v0 * gr) as f32)) + v1 * gl) as f32;
        assert_eq!(
            (out[0][0].to_bits(), out[1][0].to_bits()),
            (l.to_bits(), r.to_bits())
        );
        assert_ne!(out[0][0], out[1][0]);
    }

    #[test]
    fn the_output_is_the_float32_sum_after_every_voice() {
        // Find a frame where adding in double then rounding once differs from the stored sum.
        let mut s = Supersaw::new(2);
        let mut m = Mulberry32::new(2);
        let ph: Vec<f64> = (0..5).map(|_| m.next()).collect();
        let p = blocks(0.0, 10.0, 0.0, 0.4, 0.2, 5.0);
        let (_, out) = run(&mut s, 0.01, &p);
        let pan = f64::from(0.4f32) * 0.5 + 0.5;
        let (mut gl, mut gr) = ((1.0 - pan).sqrt(), pan.sqrt());
        let (mut stored, mut double) = (0.0f32, 0.0f64);
        for x in &ph {
            // frequency 0: dt 0, so sawblep is 2 * phase - 1.
            let v = 2.0 * x - 1.0;
            stored = (f64::from(stored) + v * gl) as f32;
            double += v * gl;
            std::mem::swap(&mut gl, &mut gr);
        }
        assert_eq!(out[0][0].to_bits(), stored.to_bits());
        // The case must discriminate (seed 2 was chosen so it does).
        assert_ne!(
            stored.to_bits(),
            (double as f32).to_bits(),
            "seed 2 gives a rounding difference"
        );
    }

    #[test]
    fn fractional_voices_run_ceil_passes_with_the_fractional_scale() {
        let mut s = Supersaw::new(8);
        let p = blocks(0.0, 10.0, 200.0, 0.0, 0.3, 2.5);
        assert_eq!(run(&mut s, 0.01, &p).0, Ok(true));
        assert_eq!(s.phase.len(), 3);
        // voices < 2: one pass, no detune; voices 1.5 runs 2 passes with detuner 0.
        let mut s = Supersaw::new(8);
        assert_eq!(
            run(&mut s, 0.01, &blocks(0.0, 10.0, 200.0, 0.0, 0.3, 1.5)).0,
            Ok(true)
        );
        assert_eq!(s.phase.len(), 2);
        // Zero or negative voices: no pass, zeros.
        let mut s = Supersaw::new(8);
        let (r, out) = run(&mut s, 0.01, &blocks(0.0, 10.0, 200.0, 0.0, 0.3, -3.0));
        assert_eq!(r, Ok(true));
        assert!(silent(&out) && s.phase.is_empty());
    }

    #[test]
    fn dt_zero_takes_no_blep_branch_and_negative_frequencies_wrap() {
        assert_eq!(poly_blep(0.5, 0.0), 0.0);
        assert_eq!(poly_blep(0.0, 0.0), 0.0);
        assert_eq!(saw_blep(0.25, 0.0), -0.5);
        // dt = frac(-x) lies in [0, 1).
        let dt = jsmath::frac(-100.0 * INVSR);
        assert!((0.0..1.0).contains(&dt));
        // Both branches at the wrap: phase just after 0 and just before 1.
        let d = 0.01;
        assert_eq!(poly_blep(0.005, d), {
            let p = 0.005 * (1.0 / d);
            2.0 * p - p * p - 1.0
        });
        assert_eq!(poly_blep(0.995, d), {
            let p = (0.995 - 1.0) * (1.0 / d);
            p * p + 2.0 * p + 1.0
        });
        // dt above one half uses 1 - dt.
        assert_eq!(poly_blep(0.1, 0.7), poly_blep(0.1, 1.0 - 0.7));
    }

    #[test]
    fn voices_beyond_the_cap_finish_the_processor() {
        let mut s = Supersaw::new(1);
        let (r, out) = run(&mut s, 0.01, &blocks(0.0, 10.0, 440.0, 0.0, 0.2, 5000.0));
        assert_eq!(r, Err(VOICES_OUT_OF_RANGE));
        assert!(silent(&out));
        let mut s = Supersaw::new(1);
        let (r, _) = run(&mut s, 0.01, &blocks(0.0, 10.0, 440.0, 0.0, 0.2, 4096.0));
        assert_eq!(r, Ok(true));
        assert_eq!(s.phase.len(), 4096);
        // 4095.5 passes 4096 times; 4096.5 is over the cap.
        let mut s = Supersaw::new(1);
        assert_eq!(
            run(&mut s, 0.01, &blocks(0.0, 10.0, 440.0, 0.0, 0.2, 4096.5)).0,
            Err(VOICES_OUT_OF_RANGE)
        );
        // The cap is only checked in an active quantum.
        let mut s = Supersaw::new(1);
        assert_eq!(
            run(&mut s, 0.0, &blocks(0.0, 10.0, 440.0, 0.0, 0.2, 5000.0)).0,
            Ok(true)
        );
    }

    #[test]
    fn node_params_lifecycle_and_errors() {
        let mut n = SupersawNode::new(4);
        let d: Vec<f32> = n.params.iter().map(|p| p.default_value()).collect();
        assert_eq!(d, vec![-1.0, -1.0, 440.0, 0.4, 0.2, 0.0, 5.0]);
        assert!(
            n.params
                .iter()
                .all(|p| (p.min_value(), p.max_value()) == (-f32::MAX, f32::MAX))
        );
        assert_eq!(n.params[VOICES].rate(), Rate::K);
        assert!(n.params[..VOICES].iter().all(|p| p.rate() == Rate::A));
        // Defaults: begin -1, so the processor runs (zeros, not silent) and never starts.
        let mut out = [[1.0f32; QUANTUM]; 2];
        assert!(!n.render(0, &NO_INPUTS, &mut out));
        assert!(silent(&out));
        // begin 0, end 0.01: active, then zeros, then gone at end + 0.5 (silent from then on).
        n.params[BEGIN].set_value(0.0, 0).unwrap();
        n.params[END].set_value(0.01, 0).unwrap();
        n.params[VOICES].set_value(1.0, 0).unwrap();
        assert!(!n.render(128, &NO_INPUTS, &mut out));
        assert!(!silent(&out));
        let gone_q = ((0.51f64 * 44100.0 / 128.0).ceil() as u64) * 128;
        assert!(!n.render(gone_q - 128, &NO_INPUTS, &mut out));
        assert!(!n.gone());
        assert!(
            !n.render(gone_q, &NO_INPUTS, &mut out),
            "the quantum returning false is not silent"
        );
        assert!(n.gone());
        assert!(n.render(gone_q + 128, &NO_INPUTS, &mut out));
        assert!(silent(&out));
        // voices over the cap: an error once, zeros for good.
        let mut n = SupersawNode::new(4);
        n.params[BEGIN].set_value(0.0, 0).unwrap();
        n.params[VOICES].set_value(1e6, 0).unwrap();
        assert!(!n.render(128, &NO_INPUTS, &mut out));
        assert_eq!(n.take_error(), Some(VOICES_OUT_OF_RANGE));
        assert_eq!(n.take_error(), None);
        assert!(n.gone());
        assert!(n.render(256, &NO_INPUTS, &mut out));
    }

    #[test]
    fn a_rate_detune_events_and_nan_inputs() {
        // A NaN connection into detune is replaced by the default 0 (the connected path).
        let mut n = SupersawNode::new(2);
        n.params[BEGIN].set_value(0.0, 0).unwrap();
        n.params[VOICES].set_value(1.0, 0).unwrap();
        let mut inputs = NO_INPUTS;
        inputs[DETUNE] = ParamInputs {
            bufs: vec![[f32::NAN; QUANTUM]],
            connected: true,
        };
        let mut a = [[0.0f32; QUANTUM]; 2];
        n.render(128, &inputs, &mut a);
        let mut m = SupersawNode::new(2);
        m.params[BEGIN].set_value(0.0, 0).unwrap();
        m.params[VOICES].set_value(1.0, 0).unwrap();
        let mut b = [[0.0f32; QUANTUM]; 2];
        m.render(128, &NO_INPUTS, &mut b);
        assert_eq!(a, b);
        // A detune ramp inside the quantum changes the frequency per frame.
        let mut e = SupersawNode::new(2);
        e.params[BEGIN].set_value(0.0, 0).unwrap();
        e.params[VOICES].set_value(1.0, 0).unwrap();
        e.params[DETUNE]
            .linear_ramp_to(1200.0, 256.0 / 44100.0, 0)
            .unwrap();
        let mut c = [[0.0f32; QUANTUM]; 2];
        e.render(128, &NO_INPUTS, &mut c);
        assert_ne!(c, b);
    }
}
