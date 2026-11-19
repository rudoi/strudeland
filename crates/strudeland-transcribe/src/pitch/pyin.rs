//! pYIN: probabilistic YIN with HMM smoothing.
//!
//! A fresh implementation of M. Mauch and S. Dixon, "pYIN: A fundamental frequency estimator
//! using probabilistic threshold distributions", ICASSP 2014, with the defaults librosa publishes
//! for `librosa.pyin` (a beta(2, 18) threshold prior over 100 thresholds, `no_trough_prob` 0.01,
//! `switch_prob` 0.01, a triangular local transition, `(1 - voiced) / n_bins` unvoiced
//! observations). Values live in `consts.rs`.
//!
//! Per frame:
//!
//! 1. The YIN difference function `d(tau)` over a window of `FRAME / 2` samples, from FFT
//!    autocorrelation and running energies.
//! 2. The cumulative mean normalized difference (CMND).
//! 3. Troughs of the CMND inside the lag range, each refined by parabolic interpolation.
//! 4. For every threshold of the prior, the first trough below it takes that threshold's prior
//!    mass (the paper's rule); thresholds no trough falls below give `NO_TROUGH_PROB` of their mass
//!    to the global minimum. The masses are the candidates' probabilities.
//!
//! Then an HMM over `N_BINS` pitch bins of `BIN_CENTS`, each with a voiced and an unvoiced copy,
//! is decoded with a banded Viterbi (pitch moves at most `BAND_BINS` bins per frame, so the cost
//! is states x band, never states squared), and a voiced frame's f0 is refined to the
//! interpolated candidate nearest its decoded bin's centre.
//!
//! The YIN window of a frame is placed so that the compared span (the window plus the longest
//! lag) is centred on the frame time; the RMS covers the `FRAME` samples centred on it.
//! Everything is deterministic: no randomness, fixed iteration order, ties go to the lower index.

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;

use super::{PitchFrame, PitchTrack, PitchTracker};
use crate::consts::{
    BAND_BINS, BETA_A, BETA_B, BIN_CENTS, FMAX, FMIN, FRAME, HOP, N_BINS, N_THRESHOLDS,
    NO_TROUGH_PROB, VOICING_SWITCH,
};
#[cfg(test)]
use crate::util::finite_f64;

/// Voiced states are `0..N_BINS`, unvoiced `N_BINS..N_STATES`.
const N_STATES: usize = 2 * N_BINS;
/// The transition band's width: `BAND_BINS` either side of the source bin.
const BAND_WIDTH: usize = 2 * BAND_BINS + 1;
/// The YIN integration window (librosa's `win_length = frame_length // 2`).
const YIN_WINDOW: usize = FRAME / 2;

/// The pYIN tracker. Its parameters are the crate's constants.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PyinTracker;

/// A pitch candidate of one frame: cents above `FMIN` and its probability.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Candidate {
    cents: f64,
    prob: f64,
}

/// Per-frame results of the YIN stage, flat: frame `t`'s candidates are
/// `cands[starts[t]..starts[t + 1]]`.
struct Observations {
    cands: Vec<Candidate>,
    starts: Vec<usize>,
    voiced_prob: Vec<f64>,
    rms: Vec<f64>,
}

#[cfg(test)]
impl PyinTracker {
    /// The track of f32 samples, non-finite ones as silence.
    pub(crate) fn track(&self, samples: &[f32], rate: u32) -> PitchTrack {
        self.track_signal(&finite_f64(samples), rate)
    }
}

impl PitchTracker for PyinTracker {
    fn track_signal(&self, signal: &[f64], rate: u32) -> PitchTrack {
        let hop_seconds = HOP as f32 / rate.max(1) as f32;
        if rate == 0 || signal.len() < FRAME {
            return PitchTrack {
                hop_seconds,
                frames: Vec::new(),
            };
        }
        let n_frames = frame_count(signal.len());
        let obs = observe(signal, rate, n_frames);
        let states = decode(&obs, n_frames);
        let frames = states
            .iter()
            .enumerate()
            .map(|(t, &state)| {
                let cands = &obs.cands[obs.starts[t]..obs.starts[t + 1]];
                PitchFrame {
                    f0: (state < N_BINS).then(|| refine(state, cands) as f32),
                    voiced_prob: obs.voiced_prob[t] as f32,
                    rms: obs.rms[t] as f32,
                }
            })
            .collect();
        PitchTrack {
            hop_seconds,
            frames,
        }
    }
}

/// Frames of a signal of `len >= FRAME` samples: one per hop, frame `i` centred at `i * HOP`.
fn frame_count(len: usize) -> usize {
    1 + len / HOP
}

/// The lag range `(min, max)` in samples at `rate`: `floor(rate / FMAX)` to `ceil(rate / FMIN)`,
/// capped so the window plus the lag fits a frame. `None` when the range is too narrow for a
/// trough.
fn lag_range(rate: u32) -> Option<(usize, usize)> {
    let r = f64::from(rate);
    let min = ((r / FMAX).floor() as usize).max(1);
    let max = ((r / FMIN).ceil() as usize).min(FRAME - YIN_WINDOW - 1);
    (max >= min + 2).then_some((min, max))
}

/// The centre of pitch bin `b` in cents above `FMIN`. Bin `b` covers `[b, b + 1) * BIN_CENTS`.
fn bin_centre_cents(b: usize) -> f64 {
    (b as f64 + 0.5) * BIN_CENTS
}

/// The bin containing `cents` above `FMIN`, `None` outside the tracked range.
fn bin_of(cents: f64) -> Option<usize> {
    if !cents.is_finite() || cents < 0.0 {
        return None;
    }
    let b = (cents / BIN_CENTS).floor();
    (b < N_BINS as f64).then_some(b as usize)
}

fn cents_to_hz(cents: f64) -> f64 {
    FMIN * (cents / 1200.0).exp2()
}

fn hz_to_cents(hz: f64) -> f64 {
    1200.0 * (hz / FMIN).log2()
}

/// The regularized incomplete beta function `I_x(a, b)` for integer `a, b >= 1`: the binomial
/// tail `sum_{j=a}^{a+b-1} C(a+b-1, j) x^j (1-x)^(a+b-1-j)`.
fn beta_cdf(x: f64, a: u32, b: u32) -> f64 {
    let x = x.clamp(0.0, 1.0);
    let n = a + b - 1;
    let mut sum = 0.0;
    let mut binom = 1.0f64; // C(n, j), built up from j = 0
    for j in 0..=n {
        if j >= a {
            sum += binom * x.powi(j as i32) * (1.0 - x).powi((n - j) as i32);
        }
        binom = binom * f64::from(n - j) / f64::from(j + 1);
    }
    sum.clamp(0.0, 1.0)
}

/// The thresholds `k / N_THRESHOLDS` for `k = 1..=N_THRESHOLDS`, and the prior's cumulative mass
/// `cum[k] = P(threshold <= k / N_THRESHOLDS)` (`cum[0] = 0`).
fn threshold_tables() -> (Vec<f64>, Vec<f64>) {
    let (a, b) = (BETA_A as u32, BETA_B as u32);
    let thresholds: Vec<f64> = (1..=N_THRESHOLDS)
        .map(|k| k as f64 / N_THRESHOLDS as f64)
        .collect();
    let mut cum = Vec::with_capacity(N_THRESHOLDS + 1);
    cum.push(0.0);
    for &s in &thresholds {
        // a running maximum: rounding near 1 must not give a threshold negative mass
        let prev = cum.last().copied().unwrap_or(0.0);
        cum.push(beta_cdf(s, a, b).max(prev));
    }
    (thresholds, cum)
}

/// The log transition weights by target: entry `target * BAND_WIDTH + (source + BAND_BINS -
/// target)` is `ln P(target | source)` for `|source - target| <= BAND_BINS`, negative infinity
/// outside the range. Each source's row is a triangle over the band, truncated at the range's
/// ends and normalized to 1.
fn transition_table() -> Vec<f64> {
    let weight = |k: usize| (BAND_BINS + 1 - k) as f64 / (BAND_BINS + 1) as f64;
    let norms: Vec<f64> = (0..N_BINS)
        .map(|src| {
            let lo = src.saturating_sub(BAND_BINS);
            let hi = (src + BAND_BINS).min(N_BINS - 1);
            (lo..=hi).map(|dst| weight(src.abs_diff(dst))).sum()
        })
        .collect();
    let mut table = vec![f64::NEG_INFINITY; N_BINS * BAND_WIDTH];
    for dst in 0..N_BINS {
        let lo = dst.saturating_sub(BAND_BINS);
        let hi = (dst + BAND_BINS).min(N_BINS - 1);
        for src in lo..=hi {
            table[dst * BAND_WIDTH + src + BAND_BINS - dst] =
                (weight(src.abs_diff(dst)) / norms[src]).ln();
        }
    }
    table
}

/// Buffers reused across frames by the YIN stage.
struct Yin {
    forward: std::sync::Arc<dyn realfft::RealToComplex<f64>>,
    inverse: std::sync::Arc<dyn realfft::ComplexToReal<f64>>,
    frame: Vec<f64>,
    fft_in: Vec<f64>,
    spec_x: Vec<Complex<f64>>,
    spec_a: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
    acf: Vec<f64>,
    energy: Vec<f64>,
    cmnd: Vec<f64>,
    troughs: Vec<(usize, f64)>,
}

impl Yin {
    fn new() -> Self {
        let mut planner = RealFftPlanner::<f64>::new();
        let forward = planner.plan_fft_forward(FRAME);
        let inverse = planner.plan_fft_inverse(FRAME);
        let scratch_len = forward.get_scratch_len().max(inverse.get_scratch_len());
        Self {
            frame: vec![0.0; FRAME],
            fft_in: forward.make_input_vec(),
            spec_x: forward.make_output_vec(),
            spec_a: forward.make_output_vec(),
            scratch: vec![Complex::new(0.0, 0.0); scratch_len],
            acf: inverse.make_output_vec(),
            energy: vec![0.0; FRAME + 1],
            cmnd: vec![0.0; FRAME],
            troughs: Vec::with_capacity(FRAME / 2),
            forward,
            inverse,
        }
    }

    /// Fills `self.cmnd[0..=max]` for the frame in `self.frame`. `false` when an FFT failed (the
    /// frame then gets no candidates).
    fn cmnd(&mut self, max: usize) -> bool {
        // autocorrelation of the window against the frame: r(tau) = sum_j a_j x_{j + tau}
        self.fft_in.copy_from_slice(&self.frame);
        if self
            .forward
            .process_with_scratch(&mut self.fft_in, &mut self.spec_x, &mut self.scratch)
            .is_err()
        {
            return false;
        }
        self.fft_in[..YIN_WINDOW].copy_from_slice(&self.frame[..YIN_WINDOW]);
        self.fft_in[YIN_WINDOW..].fill(0.0);
        if self
            .forward
            .process_with_scratch(&mut self.fft_in, &mut self.spec_a, &mut self.scratch)
            .is_err()
        {
            return false;
        }
        for (x, a) in self.spec_x.iter_mut().zip(&self.spec_a) {
            *x *= a.conj();
        }
        // the spectrum of a real correlation is real at DC and Nyquist
        if let Some(first) = self.spec_x.first_mut() {
            first.im = 0.0;
        }
        if let Some(last) = self.spec_x.last_mut() {
            last.im = 0.0;
        }
        if self
            .inverse
            .process_with_scratch(&mut self.spec_x, &mut self.acf, &mut self.scratch)
            .is_err()
        {
            return false;
        }
        let scale = 1.0 / FRAME as f64;

        // running energies: energy[k] = sum_{j < k} x_j^2
        let span = YIN_WINDOW + max + 1;
        self.energy[0] = 0.0;
        for k in 0..span {
            self.energy[k + 1] = self.energy[k] + self.frame[k] * self.frame[k];
        }
        let e0 = self.energy[YIN_WINDOW];
        self.cmnd[0] = 1.0;
        let mut running = 0.0;
        for tau in 1..=max {
            let et = self.energy[tau + YIN_WINDOW] - self.energy[tau];
            let d = (e0 + et - 2.0 * self.acf[tau] * scale).max(0.0);
            running += d;
            self.cmnd[tau] = if running > 0.0 {
                d * tau as f64 / running
            } else {
                1.0
            };
        }
        true
    }
}

/// Runs the YIN stage over every frame: candidates, voiced probability and RMS.
fn observe(signal: &[f64], rate: u32, n_frames: usize) -> Observations {
    let mut obs = Observations {
        cands: Vec::with_capacity(n_frames * 4),
        starts: Vec::with_capacity(n_frames + 1),
        voiced_prob: Vec::with_capacity(n_frames),
        rms: Vec::with_capacity(n_frames),
    };
    obs.starts.push(0);
    let lags = lag_range(rate);
    let (thresholds, cum) = threshold_tables();
    let count_at_or_below = |x: f64| thresholds.partition_point(|&s| s <= x);
    let mut yin = Yin::new();
    let len = signal.len() as isize;
    let fill = |dst: &mut [f64], start: isize| {
        for (j, v) in dst.iter_mut().enumerate() {
            let i = start + j as isize;
            *v = if (0..len).contains(&i) {
                signal[i as usize]
            } else {
                0.0
            };
        }
    };

    for t in 0..n_frames {
        let centre = (t * HOP) as isize;

        // RMS over the FRAME samples centred on the frame time
        let lo = (centre - (FRAME / 2) as isize).clamp(0, len) as usize;
        let hi = (centre + (FRAME / 2) as isize).clamp(0, len) as usize;
        let energy: f64 = signal[lo..hi].iter().map(|v| v * v).sum();
        obs.rms.push((energy / FRAME as f64).sqrt());

        let mut voiced = 0.0;
        if let Some((min, max)) = lags {
            let offset = (YIN_WINDOW + max).div_ceil(2) as isize;
            fill(&mut yin.frame, centre - offset);
            if yin.cmnd(max) {
                let y = &yin.cmnd;
                yin.troughs.clear();
                if y[min] < y[min + 1] {
                    yin.troughs.push((min, y[min]));
                }
                for tau in min + 1..max {
                    if y[tau] < y[tau - 1] && y[tau] <= y[tau + 1] {
                        yin.troughs.push((tau, y[tau]));
                    }
                }
                // the global minimum: the first trough of the lowest height
                let global = yin.troughs.iter().enumerate().fold(
                    None::<(usize, f64)>,
                    |best, (i, &(_, h))| match best {
                        Some((_, bh)) if bh <= h => best,
                        _ => Some((i, h)),
                    },
                );
                let mut earlier_min = f64::INFINITY;
                for (i, &(tau, h)) in yin.troughs.iter().enumerate() {
                    // thresholds s with h < s <= every earlier trough: this trough is the first
                    // below them
                    let below = count_at_or_below(h);
                    let upto = if earlier_min.is_finite() {
                        count_at_or_below(earlier_min)
                    } else {
                        N_THRESHOLDS
                    };
                    let mut prob = if upto > below {
                        cum[upto] - cum[below]
                    } else {
                        0.0
                    };
                    if let Some((g, gh)) = global
                        && g == i
                    {
                        prob += NO_TROUGH_PROB * cum[count_at_or_below(gh)];
                    }
                    earlier_min = earlier_min.min(h);
                    if prob <= 0.0 {
                        continue;
                    }
                    let lag = tau as f64 + parabolic_shift(y[tau - 1], y[tau], y[tau + 1]);
                    if lag <= 0.0 {
                        continue;
                    }
                    let cents = hz_to_cents(f64::from(rate) / lag);
                    if bin_of(cents).is_some() {
                        obs.cands.push(Candidate { cents, prob });
                        voiced += prob;
                    }
                }
            }
        }
        obs.voiced_prob.push(voiced.clamp(0.0, 1.0));
        obs.starts.push(obs.cands.len());
    }
    obs
}

/// The offset of a parabola's vertex through `(-1, left)`, `(0, mid)`, `(1, right)`, in `(-1, 1)`;
/// 0 when the points are flat or the vertex falls outside.
fn parabolic_shift(left: f64, mid: f64, right: f64) -> f64 {
    let a = left + right - 2.0 * mid;
    let b = (right - left) / 2.0;
    if b.abs() < a.abs() { -b / a } else { 0.0 }
}

/// Fills the voiced log emissions of frame `t` into `voiced` and returns the unvoiced one: one
/// minus the voiced probability, spread over the `N_BINS` bins (librosa's rule).
fn emissions(obs: &Observations, t: usize, acc: &mut [f64], voiced: &mut [f64]) -> f64 {
    let ln_floor = f64::MIN_POSITIVE.ln();
    let cands = &obs.cands[obs.starts[t]..obs.starts[t + 1]];
    // `acc` is all zeros between calls: only the bins of a frame's candidates are touched
    for c in cands {
        if let Some(b) = bin_of(c.cents) {
            acc[b] += c.prob;
        }
    }
    voiced.fill(ln_floor);
    for c in cands {
        if let Some(b) = bin_of(c.cents) {
            let p = acc[b];
            if p > 0.0 {
                voiced[b] = p.ln();
            }
        }
    }
    for c in cands {
        if let Some(b) = bin_of(c.cents) {
            acc[b] = 0.0;
        }
    }
    let unvoiced = (1.0 - obs.voiced_prob[t]) / N_BINS as f64;
    if unvoiced > 0.0 {
        unvoiced.ln()
    } else {
        ln_floor
    }
}

/// The most likely state per frame, by a banded Viterbi in the log domain. Backpointers are one
/// flat `u16` array, `N_STATES` per frame.
fn decode(obs: &Observations, n_frames: usize) -> Vec<usize> {
    if n_frames == 0 {
        return Vec::new();
    }
    let trans = transition_table();
    let ln_switch = VOICING_SWITCH.ln();
    let ln_stay = (1.0 - VOICING_SWITCH).ln();
    let ln_init = -(N_STATES as f64).ln();

    let mut acc = vec![0.0; N_BINS];
    let mut ev = vec![0.0; N_BINS];
    let mut delta = vec![0.0; N_STATES];
    let mut next = vec![0.0; N_STATES];
    // the best predecessor of each pitch bin for a voiced and for an unvoiced target
    let mut into_v = vec![0.0; N_BINS];
    let mut into_u = vec![0.0; N_BINS];
    let mut src_v = vec![0u16; N_BINS];
    let mut src_u = vec![0u16; N_BINS];
    let mut back = vec![0u16; n_frames * N_STATES];

    let eu = emissions(obs, 0, &mut acc, &mut ev);
    for b in 0..N_BINS {
        delta[b] = ln_init + ev[b];
        delta[N_BINS + b] = ln_init + eu;
    }

    for t in 1..n_frames {
        let eu = emissions(obs, t, &mut acc, &mut ev);
        for b in 0..N_BINS {
            let (from_v, from_u) = (delta[b], delta[N_BINS + b]);
            let (stay, switch) = (from_v + ln_stay, from_u + ln_switch);
            (into_v[b], src_v[b]) = if stay >= switch {
                (stay, b as u16)
            } else {
                (switch, (N_BINS + b) as u16)
            };
            let (switch, stay) = (from_v + ln_switch, from_u + ln_stay);
            (into_u[b], src_u[b]) = if stay >= switch {
                (stay, (N_BINS + b) as u16)
            } else {
                (switch, b as u16)
            };
        }
        let row_back = &mut back[t * N_STATES..(t + 1) * N_STATES];
        for dst in 0..N_BINS {
            let lo = dst.saturating_sub(BAND_BINS);
            let hi = (dst + BAND_BINS).min(N_BINS - 1);
            let weights = &trans[dst * BAND_WIDTH + lo + BAND_BINS - dst..][..hi - lo + 1];
            let (mut best_v, mut arg_v) = (f64::NEG_INFINITY, lo);
            let (mut best_u, mut arg_u) = (f64::NEG_INFINITY, lo);
            for (k, &w) in weights.iter().enumerate() {
                let src = lo + k;
                let v = into_v[src] + w;
                if v > best_v {
                    (best_v, arg_v) = (v, src);
                }
                let u = into_u[src] + w;
                if u > best_u {
                    (best_u, arg_u) = (u, src);
                }
            }
            next[dst] = best_v + ev[dst];
            next[N_BINS + dst] = best_u + eu;
            row_back[dst] = src_v[arg_v];
            row_back[N_BINS + dst] = src_u[arg_u];
        }
        // keep the scores near 0 so long takes keep their precision
        let top = next.iter().fold(f64::NEG_INFINITY, |m, &v| m.max(v));
        if top.is_finite() {
            for v in &mut next {
                *v -= top;
            }
        }
        std::mem::swap(&mut delta, &mut next);
    }

    let mut state = 0;
    for (s, &v) in delta.iter().enumerate() {
        if v > delta[state] {
            state = s;
        }
    }
    let mut states = vec![0; n_frames];
    states[n_frames - 1] = state;
    for t in (1..n_frames).rev() {
        state = usize::from(back[t * N_STATES + state]);
        states[t - 1] = state;
    }
    states
}

/// The f0 in Hz of a frame decoded to voiced bin `bin`: the candidate nearest the bin's centre
/// when one lies within a bin of it, else the centre.
fn refine(bin: usize, cands: &[Candidate]) -> f64 {
    let centre = bin_centre_cents(bin);
    let nearest = cands.iter().fold(None::<f64>, |best, c| match best {
        Some(b) if (b - centre).abs() <= (c.cents - centre).abs() => best,
        _ => Some(c.cents),
    });
    match nearest {
        Some(cents) if (cents - centre).abs() <= BIN_CENTS => cents_to_hz(cents),
        _ => cents_to_hz(centre),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::{ANALYSIS_RATE, MAX_TRANSITION_OCT_PER_SEC};
    use crate::eval::synth::{Case, SplitMix64, SynthSettings, TruthNote};
    use crate::util::hz_to_midi;

    const RATE: u32 = ANALYSIS_RATE;

    /// A synth case of consecutive notes `(fractional midi, seconds)` after a 0.2 s lead-in, at
    /// the analysis rate. The fraction becomes the global detune, so every note shares it.
    fn melody(notes: &[(f64, f64)], settings: SynthSettings) -> (Case, Vec<f32>) {
        let detune = notes.first().map_or(0.0, |n| n.0 - n.0.round()) * 100.0;
        let settings = SynthSettings {
            detune_cents: detune,
            lead_in_seconds: 0.2,
            ..settings
        };
        let mut t = settings.lead_in_seconds;
        let truth = notes
            .iter()
            .map(|&(midi, seconds)| {
                let note = TruthNote {
                    start: t,
                    end: t + seconds,
                    midi: midi.round() as i32,
                    bar: 1,
                    step: 0,
                    steps: 1,
                };
                t += seconds;
                note
            })
            .collect();
        let total: f64 = notes.iter().map(|n| n.1).sum();
        let case = Case {
            seed: 7,
            settings: settings.clone(),
            bpm: 60.0,
            beats_per_bar: 4.0,
            bars: (total / 4.0).ceil().max(1.0) as u32,
            tonic_pc: 0,
            minor: false,
            origin_seconds: settings.lead_in_seconds,
            truth,
        };
        let samples = case.render(RATE);
        (case, samples)
    }

    fn sine_settings() -> SynthSettings {
        SynthSettings {
            harmonics: 1,
            formants: false,
            ..SynthSettings::default()
        }
    }

    fn tone(hz: f64, seconds: f64, settings: SynthSettings) -> (Case, Vec<f32>) {
        melody(&[(hz_to_midi(hz), seconds)], settings)
    }

    /// The frames whose time lies in `[from, to]` seconds.
    fn frames_between(track: &PitchTrack, from: f64, to: f64) -> Vec<(f64, &PitchFrame)> {
        track
            .frames
            .iter()
            .enumerate()
            .map(|(i, f)| (i as f64 * f64::from(track.hop_seconds), f))
            .filter(|(t, _)| (from..=to).contains(t))
            .collect()
    }

    fn cents_between(hz: f64, reference: f64) -> f64 {
        1200.0 * (hz / reference).log2()
    }

    /// The largest absolute error in cents over the steady part of a tone, failing on any
    /// unvoiced steady frame.
    fn worst_error(label: &str, hz: f64, case: &Case, samples: &[f32]) -> f64 {
        let track = PyinTracker.track(samples, RATE);
        let note = &case.truth[0];
        let steady = frames_between(&track, note.start + 0.05, note.end - 0.05);
        assert!(steady.len() > 20, "{label}: steady frames");
        steady
            .iter()
            .map(|(t, f)| {
                let f0 =
                    f.f0.unwrap_or_else(|| panic!("{label}: unvoiced at {t:.2} s"));
                cents_between(f64::from(f0), hz).abs()
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn pure_tone_within_5_cents() {
        for hz in [110.0, 220.0, 440.0, 880.0] {
            let (case, samples) = tone(hz, 1.0, sine_settings());
            let worst = worst_error(&format!("{hz} Hz sine"), hz, &case, &samples);
            assert!(worst < 5.0, "{hz} Hz sine: worst error {worst:.3} cents");
        }
    }

    #[test]
    fn harmonic_tone_within_5_cents() {
        for hz in [130.81, 220.0, 392.0, 659.26] {
            let (case, samples) = tone(hz, 1.0, SynthSettings::default());
            let worst = worst_error(&format!("{hz} Hz voice"), hz, &case, &samples);
            assert!(worst < 5.0, "{hz} Hz voice: worst error {worst:.3} cents");
        }
    }

    #[test]
    fn silence_is_unvoiced() {
        let track = PyinTracker.track(&vec![0.0; RATE as usize], RATE);
        assert_eq!(track.frames.len(), frame_count(RATE as usize));
        for (i, f) in track.frames.iter().enumerate() {
            assert_eq!(f.f0, None, "frame {i}");
            assert_eq!(f.voiced_prob, 0.0, "frame {i}");
            assert_eq!(f.rms, 0.0, "frame {i}");
        }
    }

    #[test]
    fn noise_is_unvoiced() {
        for seed in [1u64, 2, 3] {
            let mut rng = SplitMix64::new(seed);
            let noise: Vec<f32> = (0..2 * RATE)
                .map(|_| rng.uniform(-0.5, 0.5) as f32)
                .collect();
            let track = PyinTracker.track(&noise, RATE);
            let voiced = track.frames.iter().filter(|f| f.f0.is_some()).count();
            assert_eq!(voiced, 0, "seed {seed}: {voiced} voiced noise frames");
            let mean =
                track.frames.iter().map(|f| f.voiced_prob).sum::<f32>() / track.frames.len() as f32;
            assert!(mean < 0.05, "seed {seed}: mean voiced_prob {mean}");
        }
    }

    #[test]
    fn weak_fundamental_has_no_octave_error() {
        // harmonics 1..=10 at 1/k, the fundamental 24 dB below the second harmonic
        for hz in [110.0, 196.0, 330.0] {
            let samples: Vec<f32> = (0..RATE as usize)
                .map(|i| {
                    let t = i as f64 / f64::from(RATE);
                    (1..=10)
                        .map(|k| {
                            let amp = if k == 1 {
                                0.5 * 10f64.powf(-24.0 / 20.0)
                            } else {
                                1.0 / k as f64
                            };
                            amp * (std::f64::consts::TAU * hz * k as f64 * t).sin()
                        })
                        .sum::<f64>() as f32
                        * 0.25
                })
                .collect();
            let track = PyinTracker.track(&samples, RATE);
            let steady = frames_between(&track, 0.1, 0.9);
            for (t, f) in steady {
                let f0 =
                    f.f0.unwrap_or_else(|| panic!("{hz} Hz: unvoiced at {t:.2} s"));
                let cents = cents_between(f64::from(f0), hz);
                assert!(
                    cents.abs() < 50.0,
                    "{hz} Hz: {f0} Hz at {t:.2} s ({cents:.0} cents)"
                );
            }
        }
    }

    #[test]
    fn vibrato_is_tracked() {
        let hz = 293.66;
        let settings = SynthSettings {
            vibrato_cents: 50.0,
            vibrato_hz: 5.5,
            ..SynthSettings::default()
        };
        let (case, samples) = tone(hz, 2.0, settings);
        let track = PyinTracker.track(&samples, RATE);
        let note = &case.truth[0];
        let steady = frames_between(&track, note.start + 0.1, note.end - 0.1);
        let cents: Vec<f64> = steady
            .iter()
            .map(|(t, f)| {
                let f0 = f.f0.unwrap_or_else(|| panic!("unvoiced at {t:.2} s"));
                cents_between(f64::from(f0), hz)
            })
            .collect();
        let mean = cents.iter().sum::<f64>() / cents.len() as f64;
        let (lo, hi) = cents
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), &c| {
                (l.min(c), h.max(c))
            });
        assert!(mean.abs() < 10.0, "mean {mean:.2} cents");
        assert!((hi - 50.0).abs() < 30.0, "top {hi:.1} cents");
        assert!((lo + 50.0).abs() < 30.0, "bottom {lo:.1} cents");
    }

    #[test]
    fn glide_follows_within_one_bin_per_frame() {
        // two notes a fifth apart, the second gliding up from the first over 300 ms
        let settings = SynthSettings {
            portamento_ms: 300.0,
            ..SynthSettings::default()
        };
        let (case, samples) = melody(&[(57.0, 0.6), (64.0, 1.0)], settings);
        let track = PyinTracker.track(&samples, RATE);
        let second = &case.truth[1];
        let glide_end = second.start + 0.3;
        let truth_midi = |t: f64| {
            if t < second.start {
                57.0
            } else if t < glide_end {
                64.0 - 7.0 * (1.0 - (t - second.start) / 0.3)
            } else {
                64.0
            }
        };
        let frames = frames_between(&track, case.truth[0].start + 0.05, second.end - 0.05);
        for (t, f) in &frames {
            let f0 = f.f0.unwrap_or_else(|| panic!("unvoiced at {t:.2} s"));
            let err = (hz_to_midi(f64::from(f0)) - truth_midi(*t)) * 100.0;
            // the frame averages its window, so judge against the closest truth within 10 ms
            let near = [-0.01, 0.0, 0.01]
                .iter()
                .map(|d| ((hz_to_midi(f64::from(f0)) - truth_midi(t + d)) * 100.0).abs())
                .fold(f64::INFINITY, f64::min);
            assert!(
                near <= BIN_CENTS,
                "at {t:.3} s: {err:.1} cents off the glide"
            );
        }
        assert!(frames.len() > 100);
    }

    #[test]
    fn bin_count_is_252() {
        assert_eq!(N_BINS, 252);
        assert_eq!(N_STATES, 504);
        // the bins tile FMIN up past FMAX in BIN_CENTS steps
        assert_eq!(bin_of(0.0), Some(0));
        assert_eq!(bin_of(-0.001), None);
        assert_eq!(bin_of(hz_to_cents(FMAX)), Some(N_BINS - 1));
        assert_eq!(bin_of(N_BINS as f64 * BIN_CENTS), None);
        assert_eq!(bin_of(f64::NAN), None);
        assert!((cents_to_hz(bin_centre_cents(0)) - FMIN * (10.0f64 / 1200.0).exp2()).abs() < 1e-9);
        assert!(cents_to_hz(bin_centre_cents(N_BINS - 1)) < FMAX * (20.0f64 / 1200.0).exp2());
        for b in 0..N_BINS {
            assert_eq!(bin_of(bin_centre_cents(b)), Some(b), "bin {b}");
        }
    }

    #[test]
    fn viterbi_band_is_21_bins() {
        let derived =
            MAX_TRANSITION_OCT_PER_SEC * 1200.0 * HOP as f64 / f64::from(RATE) / BIN_CENTS;
        assert_eq!(derived, 21.0);
        assert_eq!(BAND_BINS, 21);
        assert_eq!(BAND_WIDTH, 43);
        let trans = transition_table();
        assert_eq!(
            trans.len(),
            N_BINS * BAND_WIDTH,
            "states x band, not states squared"
        );
        // every source's row sums to 1 over the band and is a triangle peaking at the source
        for src in 0..N_BINS {
            let mut sum = 0.0;
            for dst in 0..N_BINS {
                let p = if src.abs_diff(dst) <= BAND_BINS {
                    trans[dst * BAND_WIDTH + src + BAND_BINS - dst].exp()
                } else {
                    0.0
                };
                sum += p;
                if dst.abs_diff(src) == BAND_BINS {
                    assert!(p > 0.0, "{src} -> {dst} is inside the band");
                }
            }
            assert!((sum - 1.0).abs() < 1e-12, "row {src} sums to {sum}");
        }
        let at = |src: usize, dst: usize| trans[dst * BAND_WIDTH + src + BAND_BINS - dst];
        assert!(at(100, 100) > at(100, 101));
        assert_eq!(at(100, 99), at(100, 101));
        // a target has every source within the band, fewer at the range's ends
        let sources = |dst: usize| {
            trans[dst * BAND_WIDTH..(dst + 1) * BAND_WIDTH]
                .iter()
                .filter(|v| v.is_finite())
                .count()
        };
        assert_eq!(sources(0), BAND_BINS + 1);
        assert_eq!(sources(N_BINS / 2), BAND_WIDTH);
        assert_eq!(sources(N_BINS - 1), BAND_BINS + 1);
    }

    #[test]
    fn refinement_beats_bin_width() {
        // 1000.0 cents above the lowest bin's edge: on the edge between bins 49 and 50, the
        // furthest a tone can be from a bin centre
        let hz = cents_to_hz(1000.0);
        assert_eq!(bin_of(1000.0), Some(50));
        assert_eq!((1000.0 - bin_centre_cents(49)), BIN_CENTS / 2.0);
        for settings in [sine_settings(), SynthSettings::default()] {
            let (case, samples) = tone(hz, 1.0, settings);
            let worst = worst_error("1000 cents", hz, &case, &samples);
            assert!(worst < 5.0, "worst error {worst:.3} cents");
        }
    }

    #[test]
    fn deterministic() {
        let settings = SynthSettings {
            vibrato_cents: 30.0,
            breath_noise_db: Some(-30.0),
            noise_floor_db: Some(-45.0),
            ..SynthSettings::default()
        };
        let samples = Case::generate(1000, &settings).render(RATE);
        let samples = &samples[..samples.len().min(8 * RATE as usize)];
        let (a, b) = (
            PyinTracker.track(samples, RATE),
            PyinTracker.track(samples, RATE),
        );
        assert_eq!(a.hop_seconds.to_bits(), b.hop_seconds.to_bits());
        assert_eq!(a.frames.len(), b.frames.len());
        for (i, (x, y)) in a.frames.iter().zip(&b.frames).enumerate() {
            assert_eq!(
                x.f0.map(f32::to_bits),
                y.f0.map(f32::to_bits),
                "frame {i} f0"
            );
            assert_eq!(
                x.voiced_prob.to_bits(),
                y.voiced_prob.to_bits(),
                "frame {i} voiced"
            );
            assert_eq!(x.rms.to_bits(), y.rms.to_bits(), "frame {i} rms");
        }
        assert!(a.frames.iter().any(|f| f.f0.is_some()));
    }

    #[test]
    fn short_input_gives_empty_track() {
        for len in [0, 1, FRAME - 1] {
            let track = PyinTracker.track(&vec![0.1; len], RATE);
            assert!(track.frames.is_empty(), "{len} samples");
            assert_eq!(track.hop_seconds, 0.01, "{len} samples");
        }
        let track = PyinTracker.track(&vec![0.1; FRAME], RATE);
        assert_eq!(track.frames.len(), 1 + FRAME / HOP);
        assert!(PyinTracker.track(&vec![0.1; FRAME], 0).frames.is_empty());
    }

    #[test]
    fn frame_count_matches_hop() {
        for len in [FRAME, FRAME + 1, 16_000, 16_159, 16_160, 33_333] {
            let track = PyinTracker.track(&vec![0.0; len], RATE);
            assert_eq!(track.frames.len(), 1 + len / HOP, "{len} samples");
        }
        let track = PyinTracker.track(&vec![0.0; 48_000], 48_000);
        assert_eq!(track.hop_seconds, HOP as f32 / 48_000.0);
        assert_eq!(track.frames.len(), 1 + 48_000 / HOP);
    }

    #[test]
    fn rms_matches_known_amplitude() {
        // the synth peak-normalizes a voice to 0.5, so a sine's RMS is 0.5 / sqrt(2); 437.5 Hz
        // fits exactly 28 cycles in a frame, so no partial cycle skews it
        let (case, samples) = tone(437.5, 1.0, sine_settings());
        let track = PyinTracker.track(&samples, RATE);
        let note = &case.truth[0];
        let expected = 0.5 / 2f64.sqrt();
        for (t, f) in frames_between(&track, note.start + 0.05, note.end - 0.05) {
            let err = (f64::from(f.rms) - expected).abs() / expected;
            assert!(err < 1e-3, "at {t:.2} s: rms {} for {expected:.5}", f.rms);
        }
        // the lead-in, a frame clear of the note, is silent
        assert_eq!(track.frames[5].rms, 0.0);
        // a frame centred on the note's start covers half a frame of it, less the energy the
        // synth's 10 ms linear attack ramp removes (two thirds of the ramp)
        let half = (note.start / 0.01).round() as usize;
        let ratio = f64::from(track.frames[half].rms).powi(2) / expected.powi(2);
        let covered = (FRAME as f64 / 2.0 - 2.0 / 3.0 * 0.010 * f64::from(RATE)) / FRAME as f64;
        assert!(
            (ratio - covered).abs() < 0.01,
            "half-covered frame energy ratio {ratio:.4}, expected {covered:.4}"
        );
    }

    #[test]
    fn frequencies_at_the_range_edges() {
        for hz in [62.0, 1000.0] {
            let (case, samples) = tone(hz, 1.0, sine_settings());
            let worst = worst_error(&format!("{hz} Hz"), hz, &case, &samples);
            assert!(worst < 5.0, "{hz} Hz: worst error {worst:.3} cents");
        }
        for hz in [40.0, 2000.0] {
            let (_, samples) = tone(hz, 1.0, sine_settings());
            let track = PyinTracker.track(&samples, RATE);
            for (i, f) in track.frames.iter().enumerate() {
                let Some(f0) = f.f0 else { continue };
                let f0 = f64::from(f0);
                assert!(
                    (FMIN..=cents_to_hz(N_BINS as f64 * BIN_CENTS)).contains(&f0),
                    "{hz} Hz: frame {i} reports {f0} Hz outside the range"
                );
                assert!(
                    cents_between(f0, hz).abs() > 100.0,
                    "{hz} Hz: frame {i} reports the out-of-range pitch ({f0} Hz)"
                );
            }
        }
    }

    #[test]
    fn beta_prior_matches_closed_form() {
        // I_x(2, 18) = 1 - (1-x)^19 - 19 x (1-x)^18
        for x in [0.0, 0.01, 0.05, 0.1, 0.3, 0.5, 0.99, 1.0] {
            let closed = 1.0 - (1.0f64 - x).powi(19) - 19.0 * x * (1.0f64 - x).powi(18);
            assert!((beta_cdf(x, 2, 18) - closed).abs() < 1e-12, "x = {x}");
        }
        assert_eq!(BETA_A.fract(), 0.0);
        assert_eq!(BETA_B.fract(), 0.0);
        let (thresholds, cum) = threshold_tables();
        assert_eq!(thresholds.len(), N_THRESHOLDS);
        assert_eq!(cum.len(), N_THRESHOLDS + 1);
        assert_eq!(thresholds[N_THRESHOLDS - 1], 1.0);
        assert!((cum[N_THRESHOLDS] - 1.0).abs() < 1e-12);
        assert!(cum.windows(2).all(|w| w[1] >= w[0]));
    }

    #[test]
    fn parabolic_shift_finds_the_vertex() {
        // y = (x - 0.3)^2 sampled at -1, 0, 1
        let y = |x: f64| (x - 0.3) * (x - 0.3);
        assert!((parabolic_shift(y(-1.0), y(0.0), y(1.0)) - 0.3).abs() < 1e-12);
        assert_eq!(parabolic_shift(1.0, 1.0, 1.0), 0.0);
        // a monotonic run has its vertex outside: no shift
        assert_eq!(parabolic_shift(3.0, 2.0, 1.0), 0.0);
    }

    #[test]
    fn non_finite_samples_do_not_poison_the_track() {
        let (_, mut samples) = tone(220.0, 1.0, sine_settings());
        samples[8000] = f32::NAN;
        samples[8001] = f32::INFINITY;
        let track = PyinTracker.track(&samples, RATE);
        for f in &track.frames {
            assert!(f.rms.is_finite() && f.voiced_prob.is_finite());
            assert!(f.f0.is_none_or(f32::is_finite));
        }
        assert!(track.frames.iter().filter(|f| f.f0.is_some()).count() > 50);
    }

    #[test]
    fn other_rates_track_and_do_not_panic() {
        for rate in [8_000u32, 44_100, 48_000] {
            let samples: Vec<f32> = (0..rate as usize)
                .map(|i| {
                    (0.5 * (std::f64::consts::TAU * 220.0 * i as f64 / f64::from(rate)).sin())
                        as f32
                })
                .collect();
            let track = PyinTracker.track(&samples, rate);
            let mid = &track.frames[track.frames.len() / 2];
            let f0 = f64::from(mid.f0.unwrap_or_else(|| panic!("{rate} Hz: unvoiced")));
            assert!(cents_between(f0, 220.0).abs() < 5.0, "{rate} Hz: {f0} Hz");
        }
        // a rate too low for any lag range still gives frames, all unvoiced
        let track = PyinTracker.track(&vec![0.3; 2048], 100);
        assert!(!track.frames.is_empty());
        assert!(track.frames.iter().all(|f| f.f0.is_none()));
    }
}
