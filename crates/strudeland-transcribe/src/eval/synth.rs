//! A seeded generator of monophonic melodies with known notes, rendered as a voice-like signal.
//!
//! Everything is deterministic from the seed: the melody, the onset jitter and the noise. The
//! voice is a harmonic source with a static formant envelope and phase-continuous oscillation, so
//! note changes never click. Noise levels are in dB relative to the voice's RMS.

use std::f64::consts::TAU;
use std::path::Path;

use crate::util::midi_to_hz;

/// The default render rate in Hz.
pub const DEFAULT_RATE: u32 = 48_000;
/// The lowest generated MIDI note (C3): its f0 stays above 130 Hz.
pub const MIDI_MIN: i32 = 48;
/// The highest generated MIDI note (G5): its f0 stays below 800 Hz.
pub const MIDI_MAX: i32 = 79;
/// Steps (sixteenth notes) per 4/4 bar in the generated ground truth.
pub const STEPS_PER_BAR: u32 = 16;
/// Steps (sixteenth notes) per 12/8 bar (six quarter notes) in the generated ground truth.
pub const STEPS_PER_COMPOUND_BAR: u32 = 24;
/// The hummed tier's loudness wobble rates: each component's rate is drawn in this range, Hz.
const WOBBLE_HZ: (f64, f64) = (2.0, 8.0);
/// Sinusoids summed into the loudness wobble.
const WOBBLE_COMPONENTS: usize = 3;

const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR: [i32; 7] = [0, 2, 3, 5, 7, 8, 10];
/// Harmonic weights are refreshed every this many samples.
const WEIGHT_BLOCK: usize = 64;
/// Formants of an open vowel: centre Hz, bandwidth Hz, gain.
const FORMANTS: [(f64, f64, f64); 3] = [
    (700.0, 130.0, 1.0),
    (1220.0, 150.0, 0.7),
    (2600.0, 250.0, 0.4),
];

/// A SplitMix64 generator (Vigna), enough for a test fixture and free of any RNG crate.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A float in `[0, 1)` with 53 random bits.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// A float in `[lo, hi)`.
    pub fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }

    /// An integer in `lo..=hi` (`lo <= hi`).
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        let span = (hi - lo + 1).max(1) as u64;
        lo + (self.next_u64() % span) as i64
    }
}

/// How the voice is rendered and how hard the case is.
#[derive(Debug, Clone, PartialEq)]
pub struct SynthSettings {
    /// Harmonics of the voice (1 is a pure sine).
    pub harmonics: usize,
    /// Shape the harmonics with a static vowel formant envelope.
    pub formants: bool,
    /// Vibrato depth in cents (a half-range: 50 is +-50 cents); 0 is none.
    pub vibrato_cents: f64,
    pub vibrato_hz: f64,
    /// Each note starts 100 cents flat and glides to pitch over this many ms; 0 is none.
    pub attack_glide_ms: f64,
    /// Between adjacent notes the pitch moves over this many ms from the previous note; 0 is none.
    pub portamento_ms: f64,
    /// Allow repeated notes of the same pitch in the melody.
    pub repeated_notes: bool,
    /// Breath noise shaped by the note envelope, in dB relative to the voice RMS.
    pub breath_noise_db: Option<f64>,
    /// A constant noise floor, in dB relative to the voice RMS.
    pub noise_floor_db: Option<f64>,
    /// A global detune in cents (positive is sharp).
    pub detune_cents: f64,
    /// Each onset moves by a uniform amount within +-this many ms.
    pub onset_jitter_ms: f64,
    /// A metronome click on every beat, in dB relative to the voice RMS.
    pub click_bleed_db: Option<f64>,
    /// Silence before bar 1, in seconds; bar 1 starts exactly here.
    pub lead_in_seconds: f64,
    /// 12/8 (six quarter notes, 24 sixteenths, notes in whole eighths) instead of 4/4.
    pub compound: bool,
    /// A pickup: one note in the bar before bar 1 (bar 0), ending on bar 1's downbeat.
    pub pickup: bool,
    /// The lowest and highest generated MIDI note.
    pub midi_range: (i32, i32),
    /// The gain ramp at the start of each note, in milliseconds (a hummed attack is soft).
    pub attack_ms: f64,
    /// The fade at the end of each note, in milliseconds.
    pub release_ms: f64,
    /// A slow loudness wobble (a sum of sinusoids at 2 to 8 Hz), its peak in dB either way; 0 is
    /// none.
    pub amp_wobble_db: f64,
    /// A slow pitch drift within each note (a smooth walk through three random offsets), its
    /// peak in cents either way; 0 is none.
    pub pitch_drift_cents: f64,
}

impl Default for SynthSettings {
    fn default() -> Self {
        Self {
            harmonics: 16,
            formants: true,
            vibrato_cents: 0.0,
            vibrato_hz: 5.5,
            attack_glide_ms: 0.0,
            portamento_ms: 0.0,
            repeated_notes: false,
            breath_noise_db: None,
            noise_floor_db: None,
            detune_cents: 0.0,
            onset_jitter_ms: 0.0,
            click_bleed_db: None,
            lead_in_seconds: 2.0,
            compound: false,
            pickup: false,
            midi_range: (MIDI_MIN, MIDI_MAX),
            attack_ms: 10.0,
            release_ms: 10.0,
            amp_wobble_db: 0.0,
            pitch_drift_cents: 0.0,
        }
    }
}

/// A note of the ground truth, in memo seconds (jitter and the lead-in included).
#[derive(Debug, Clone, PartialEq)]
pub struct TruthNote {
    pub start: f64,
    pub end: f64,
    pub midi: i32,
    /// 1-based bar; 0 is the pickup bar before bar 1.
    pub bar: u32,
    /// Start step within the bar on the sixteenth grid, before jitter.
    pub step: u32,
    /// Length in sixteenth steps, before jitter.
    pub steps: u32,
}

/// One generated melody with its settings and ground truth.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub seed: u64,
    pub settings: SynthSettings,
    pub bpm: f64,
    pub beats_per_bar: f64,
    pub bars: u32,
    /// 0 is C.
    pub tonic_pc: u8,
    pub minor: bool,
    /// Bar 1's downbeat in memo seconds: the lead-in.
    pub origin_seconds: f64,
    pub truth: Vec<TruthNote>,
}

impl Case {
    /// Builds the melody for `seed`: a random major or minor key, 90..=130 bpm (quarter notes)
    /// in 4/4 or, with `compound`, 12/8, 8..=16 bars of 4..=8 notes each, in the settings' MIDI
    /// range (48..=79 by default). In 12/8 the notes are whole eighths. With `pickup` one more
    /// note, the dominant below the first, fills the end of bar 0 (one or two eighths); its draws
    /// come after the melody's, so the melody is the one generated without it.
    ///
    /// The melody behaves tonally, as sung melodies do: it starts on the tonic and ends on it
    /// (the final note at least a beat long), each bar has a chord of the key (four-bar phrases of
    /// I, then IV, vi or ii, then V, then I; the last bar is I), and notes on the bar's strong
    /// beats (steps 0 and 8) and each phrase's last note are tones of that chord, the chord tone
    /// nearest the walk. The other notes walk the scale by one or two steps. The key itself is a
    /// random draw.
    pub fn generate(seed: u64, settings: &SynthSettings) -> Case {
        let mut rng = SplitMix64::new(seed);
        let tonic_pc = rng.range(0, 11) as u8;
        let minor = rng.range(0, 1) == 1;
        let bpm = rng.range(90, 130) as f64;
        let bars = rng.range(8, 16) as u32;
        let (beats_per_bar, spb, unit) = if settings.compound {
            (6.0, STEPS_PER_COMPOUND_BAR, 2)
        } else {
            (4.0, STEPS_PER_BAR, 1)
        };
        let step_seconds = 60.0 / bpm / 4.0;
        let origin = settings.lead_in_seconds;

        let scale = if minor { MINOR } else { MAJOR };
        let (lowest, highest) = settings.midi_range;
        let ladder: Vec<i32> = (lowest..=highest)
            .filter(|m| {
                let pc = (m - i32::from(tonic_pc)).rem_euclid(12);
                scale.contains(&pc)
            })
            .collect();
        let top = ladder.len() as i64 - 1;
        // the scale degree (0 is the tonic) of a ladder index
        let degree_of = |i: i64| -> usize {
            let pc = (ladder[i as usize] - i32::from(tonic_pc)).rem_euclid(12);
            scale.iter().position(|s| *s == pc).unwrap_or(0)
        };
        // the ladder index nearest `from` whose degree is one of `tones` (the lower on a tie),
        // other than `avoid`
        let nearest = |from: i64, tones: &[usize], avoid: Option<i64>| -> i64 {
            (0..=top)
                .filter(|&i| tones.contains(&degree_of(i)) && Some(i) != avoid)
                .min_by_key(|&i| ((i - from).abs(), i))
                .unwrap_or(from.clamp(0, top))
        };
        let start = rng.range(top / 3, 2 * top / 3);
        let mut at = nearest(start, &[0], None);

        // nominal notes: (bar, step, steps, midi)
        let mut nominal: Vec<(u32, u32, u32, i32)> = Vec::new();
        for bar in 1..=bars {
            let last_bar = bar == bars;
            let phrase_bar = (bar - 1) % 4;
            let chord_root = match phrase_bar {
                _ if last_bar => 0,
                0 | 3 => 0,
                2 => 4,
                _ => [3usize, 5, 1][rng.range(0, 2) as usize],
            };
            let chord = [chord_root, (chord_root + 2) % 7, (chord_root + 4) % 7];
            let n = rng.range(4, 8) as usize;
            // a random composition of the bar into n parts of at least one unit (a step, or an
            // eighth in 12/8); the final note of the melody gets at least a beat (a dotted
            // quarter in 12/8)
            let units = spb / unit;
            let max_cut = if last_bar {
                units - if settings.compound { 3 } else { 4 }
            } else {
                units - 1
            };
            let mut cuts: Vec<u32> = Vec::new();
            while cuts.len() < n - 1 {
                let c = rng.range(1, i64::from(max_cut)) as u32;
                if !cuts.contains(&c) {
                    cuts.push(c);
                }
            }
            cuts.sort_unstable();
            let mut edges = vec![0];
            edges.extend(cuts.iter().map(|c| c * unit));
            edges.push(spb);
            let parts = edges.len() - 1;
            for (k, w) in edges.windows(2).enumerate() {
                let (start, mut len) = (w[0], w[1] - w[0]);
                let first = bar == 1 && k == 0;
                let last_in_bar = k + 1 == parts;
                let final_note = last_bar && last_in_bar;
                let deltas: &[i64] = if settings.repeated_notes {
                    &[-2, -1, 0, 0, 1, 2]
                } else {
                    &[-2, -1, 1, 2]
                };
                let d = deltas[rng.range(0, deltas.len() as i64 - 1) as usize];
                let before = at;
                let avoid = (!settings.repeated_notes && !first).then_some(before);
                if first {
                    // the melody starts on the tonic
                } else if final_note {
                    at = nearest(before + d, &[0], avoid);
                } else if start == 0 || start == spb / 2 {
                    at = nearest(before + d, &chord, avoid);
                } else if phrase_bar == 3 && last_in_bar {
                    at = nearest(before + d, &[0, 2, 4], avoid);
                } else {
                    at += d;
                    if at < 0 {
                        at = -at;
                    }
                    if at > top {
                        at = 2 * top - at;
                    }
                    at = at.clamp(0, top);
                    if at == before && !settings.repeated_notes {
                        // a reflection at the range's edge landed on the same pitch
                        at = if before < top { before + 1 } else { before - 1 };
                    }
                }
                // sometimes the last step (eighth) of a long note is a rest
                if !final_note && len >= 2 * unit && rng.next_f64() < 0.15 {
                    len -= unit;
                }
                nominal.push((bar, start, len, ladder[at as usize]));
            }
        }

        // onset jitter: one draw per note, offsets follow the next note when adjacent
        let mut jitter: Vec<f64> = nominal
            .iter()
            .map(|_| {
                if settings.onset_jitter_ms > 0.0 {
                    rng.uniform(-settings.onset_jitter_ms, settings.onset_jitter_ms) / 1000.0
                } else {
                    0.0
                }
            })
            .collect();
        if settings.pickup {
            // the dominant below the first note (above when the range ends), an eighth or a
            // quarter before bar 1
            let first = ladder.iter().position(|&m| m == nominal[0].3).unwrap_or(0) as i64;
            let at = nearest(first - 2, &[4], Some(first));
            let len = 2 * rng.range(1, 2) as u32;
            nominal.insert(0, (0, spb - len, len, ladder[at as usize]));
            let j = if settings.onset_jitter_ms > 0.0 {
                rng.uniform(-settings.onset_jitter_ms, settings.onset_jitter_ms) / 1000.0
            } else {
                0.0
            };
            jitter.insert(0, j);
        }
        let abs_step =
            |bar: u32, step: u32| ((i64::from(bar) - 1) * i64::from(spb) + i64::from(step)) as f64;
        let truth = nominal
            .iter()
            .enumerate()
            .map(|(i, &(bar, step, steps, midi))| {
                let start = origin + abs_step(bar, step) * step_seconds + jitter[i];
                let end_step = abs_step(bar, step) + f64::from(steps);
                let adjacent = nominal
                    .get(i + 1)
                    .is_some_and(|&(b, s, _, _)| abs_step(b, s) == end_step);
                let end_jitter = if adjacent { jitter[i + 1] } else { jitter[i] };
                TruthNote {
                    start,
                    end: origin + end_step * step_seconds + end_jitter,
                    midi,
                    bar,
                    step,
                    steps,
                }
            })
            .collect();

        Case {
            seed,
            settings: settings.clone(),
            bpm,
            beats_per_bar,
            bars,
            tonic_pc,
            minor,
            origin_seconds: origin,
            truth,
        }
    }

    /// Sixteenth steps in one bar: 16 in 4/4, 24 in 12/8.
    pub fn steps_per_bar(&self) -> u32 {
        (self.beats_per_bar * 4.0).round() as u32
    }

    /// Seconds in one bar.
    pub fn bar_seconds(&self) -> f64 {
        self.beats_per_bar * 60.0 / self.bpm
    }

    /// The rendered length in seconds: lead-in, the bars and a short tail.
    pub fn duration_seconds(&self) -> f64 {
        self.origin_seconds + f64::from(self.bars) * self.bar_seconds() + 0.5
    }

    /// Renders the voice mono at `rate` Hz. The peak stays below full scale.
    pub fn render(&self, rate: u32) -> Vec<f32> {
        let rate = rate.max(1);
        let sr = f64::from(rate);
        let s = &self.settings;
        let n = (self.duration_seconds() * sr).ceil() as usize;
        let mut voice = vec![0.0f64; n];
        let mut env = vec![0.0f64; n];
        let harmonics = s.harmonics.max(1);
        let mut weights = vec![0.0f64; harmonics];
        let mut phase = 0.0f64;
        let mut idx = 0usize;
        // the hummed imperfections draw from a stream of their own, so they do not move the
        // other layers' noise
        let mut hum = SplitMix64::new(self.seed ^ 0x4855_4D4D_4544_0000);
        let wobble: Vec<(f64, f64, f64)> = if s.amp_wobble_db > 0.0 {
            (0..WOBBLE_COMPONENTS)
                .map(|_| {
                    (
                        hum.uniform(WOBBLE_HZ.0, WOBBLE_HZ.1),
                        hum.uniform(0.0, TAU),
                        hum.uniform(0.5, 1.0),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let wobble_norm: f64 = wobble.iter().map(|w| w.2).sum();
        let drift: Vec<[f64; 3]> = if s.pitch_drift_cents > 0.0 {
            let d = s.pitch_drift_cents;
            self.truth
                .iter()
                .map(|_| [hum.uniform(-d, d), hum.uniform(-d, d), hum.uniform(-d, d)])
                .collect()
        } else {
            Vec::new()
        };
        let (attack, release) = (s.attack_ms / 1000.0, s.release_ms / 1000.0);

        for i in 0..n {
            let t = i as f64 / sr;
            while idx < self.truth.len() && self.truth[idx].end <= t {
                idx += 1;
            }
            let Some(note) = self.truth.get(idx).filter(|note| t >= note.start) else {
                continue;
            };
            let el = t - note.start;
            let prev = idx.checked_sub(1).map(|p| &self.truth[p]);
            let adjacent = prev.is_some_and(|p| p.end >= note.start - 1e-9);
            let mut semis = f64::from(note.midi) + s.detune_cents / 100.0;
            if let (true, Some(p)) = (s.portamento_ms > 0.0 && adjacent, prev) {
                let span = s.portamento_ms / 1000.0;
                if el < span {
                    semis += f64::from(p.midi - note.midi) * (1.0 - el / span);
                }
            } else if s.attack_glide_ms > 0.0 {
                let span = s.attack_glide_ms / 1000.0;
                if el < span {
                    semis -= 1.0 - el / span;
                }
            }
            if s.vibrato_cents > 0.0 {
                semis += s.vibrato_cents / 100.0 * (TAU * s.vibrato_hz * t).sin();
            }
            if let Some(&[a, m, b]) = drift.get(idx) {
                // a smooth walk from a through m to b over the note
                let u = (el / (note.end - note.start)).clamp(0.0, 1.0);
                let ease = |x: f64| (1.0 - (std::f64::consts::PI * x).cos()) / 2.0;
                let cents = if u < 0.5 {
                    a + (m - a) * ease(2.0 * u)
                } else {
                    m + (b - m) * ease(2.0 * u - 1.0)
                };
                semis += cents / 100.0;
            }
            let freq = midi_to_hz(semis);
            phase = (phase + TAU * freq / sr) % TAU;

            if i % WEIGHT_BLOCK == 0 {
                for (k, w) in weights.iter_mut().enumerate() {
                    let fk = freq * (k + 1) as f64;
                    *w = if fk < 0.45 * sr {
                        let shape = if s.formants { formant_gain(fk) } else { 1.0 };
                        shape / (k + 1) as f64
                    } else {
                        0.0
                    };
                }
            }
            // sin(k phase) by the Chebyshev recurrence: one sin and one cos per sample
            let (sin1, cos1) = phase.sin_cos();
            let (mut prev_s, mut cur_s) = (0.0, sin1);
            let mut sum = 0.0;
            for w in &weights {
                sum += w * cur_s;
                let next = 2.0 * cos1 * cur_s - prev_s;
                prev_s = cur_s;
                cur_s = next;
            }
            let gain = (el / attack).min(1.0) * ((note.end - t) / release).clamp(0.0, 1.0);
            let loudness = if wobble.is_empty() {
                1.0
            } else {
                let w: f64 = wobble
                    .iter()
                    .map(|&(hz, phi, amp)| amp * (TAU * hz * t + phi).sin())
                    .sum();
                10f64.powf(s.amp_wobble_db * w / wobble_norm / 20.0)
            };
            voice[i] = sum * gain * loudness;
            env[i] = gain;
        }

        // peak-normalize the voice, then add the other layers relative to its RMS
        let peak = voice.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let scale = if peak > 0.0 { 0.5 / peak } else { 1.0 };
        let (sum_sq, count) = voice
            .iter()
            .filter(|v| **v != 0.0)
            .fold((0.0, 0usize), |(a, c), v| (a + (v * scale).powi(2), c + 1));
        let rms = if count > 0 {
            (sum_sq / count as f64).sqrt()
        } else {
            0.0
        };
        let level = |db: f64| rms * 10f64.powf(db / 20.0);

        let mut rng = SplitMix64::new(self.seed ^ 0xA5A5_5A5A_D1CE_F00D);
        let mut out: Vec<f64> = voice.iter().map(|v| v * scale).collect();
        if let Some(db) = s.breath_noise_db {
            let amp = level(db);
            for (o, e) in out.iter_mut().zip(&env) {
                *o += amp * e * rng.uniform(-1.0, 1.0);
            }
        }
        if let Some(db) = s.noise_floor_db {
            let amp = level(db);
            for o in &mut out {
                *o += amp * rng.uniform(-1.0, 1.0);
            }
        }
        if let Some(db) = s.click_bleed_db {
            let amp = level(db);
            let beat = 60.0 / self.bpm;
            let len = (0.005 * sr) as usize;
            let mut k = 0u32;
            loop {
                let at = self.origin_seconds + f64::from(k) * beat;
                let first = (at * sr).round() as usize;
                if first >= n {
                    break;
                }
                for j in 0..len.min(n - first) {
                    let tj = j as f64 / sr;
                    out[first + j] += amp * (TAU * 2000.0 * tj).sin() * (-tj / 0.001).exp();
                }
                k += 1;
            }
        }
        let top = out.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let guard = if top > 0.95 { 0.95 / top } else { 1.0 };
        out.iter().map(|v| (v * guard) as f32).collect()
    }
}

fn formant_gain(freq: f64) -> f64 {
    0.15 + FORMANTS
        .iter()
        .map(|&(centre, bw, g)| g * (-0.5 * ((freq - centre) / bw).powi(2)).exp())
        .sum::<f64>()
}

/// Writes `samples` as a mono 32-bit float WAV.
pub fn write_wav(path: &Path, samples: &[f32], rate: u32) -> Result<(), hound::Error> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        writer.write_sample(s)?;
    }
    writer.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::zero_crossing_hz;

    #[test]
    fn splitmix64_known_vector() {
        let mut rng = SplitMix64::new(0);
        assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(rng.next_u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn splitmix64_float_helpers_stay_in_range() {
        let mut rng = SplitMix64::new(7);
        for _ in 0..1000 {
            let f = rng.next_f64();
            assert!((0.0..1.0).contains(&f));
            let u = rng.uniform(-3.0, 5.0);
            assert!((-3.0..5.0).contains(&u));
            let r = rng.range(2, 4);
            assert!((2..=4).contains(&r));
        }
        assert_eq!(rng.range(5, 5), 5);
    }

    #[test]
    fn synth_is_seeded_and_deterministic() {
        let settings = SynthSettings {
            vibrato_cents: 40.0,
            breath_noise_db: Some(-30.0),
            noise_floor_db: Some(-45.0),
            onset_jitter_ms: 15.0,
            ..SynthSettings::default()
        };
        let a = Case::generate(1000, &settings);
        let b = Case::generate(1000, &settings);
        assert_eq!(a, b);
        let (ra, rb) = (a.render(16_000), b.render(16_000));
        assert!(
            ra.iter().zip(&rb).all(|(x, y)| x.to_bits() == y.to_bits()) && ra.len() == rb.len(),
            "same seed renders bitwise equal"
        );
        let c = Case::generate(1001, &settings);
        assert_ne!(
            a.truth, c.truth,
            "a different seed gives a different melody"
        );
        assert_ne!(ra, c.render(16_000));
    }

    #[test]
    fn synth_ground_truth_matches_requested_notes() {
        for seed in 1000..1020 {
            let case = Case::generate(seed, &SynthSettings::default());
            assert!(
                (8..=16).contains(&case.bars),
                "seed {seed}: bars {}",
                case.bars
            );
            assert!(
                (90.0..=130.0).contains(&case.bpm),
                "seed {seed}: bpm {}",
                case.bpm
            );
            assert_eq!(case.beats_per_bar, 4.0);
            let step = 60.0 / case.bpm / 4.0;
            for bar in 1..=case.bars {
                let count = case.truth.iter().filter(|n| n.bar == bar).count();
                assert!(
                    (4..=8).contains(&count),
                    "seed {seed} bar {bar}: {count} notes"
                );
            }
            for n in &case.truth {
                assert!(
                    (MIDI_MIN..=MIDI_MAX).contains(&n.midi),
                    "seed {seed}: midi {}",
                    n.midi
                );
                assert!(n.steps >= 1 && n.step + n.steps <= STEPS_PER_BAR);
                let nominal =
                    case.origin_seconds + f64::from((n.bar - 1) * STEPS_PER_BAR + n.step) * step;
                assert!((n.start - nominal).abs() < 1e-9, "no jitter: on the grid");
                assert!((n.end - n.start - f64::from(n.steps) * step).abs() < 1e-9);
            }
            let notes: Vec<_> = case.truth.windows(2).collect();
            for w in notes {
                assert!(
                    w[0].end <= w[1].start + 1e-9,
                    "seed {seed}: notes do not overlap"
                );
                assert_ne!(w[0].midi, w[1].midi, "seed {seed}: no repeats unless asked");
            }
            let pcs: Vec<u8> = case
                .truth
                .iter()
                .map(|n| (n.midi.rem_euclid(12)) as u8)
                .collect();
            let scale = if case.minor { MINOR } else { MAJOR };
            for pc in pcs {
                let degree = (i32::from(pc) - i32::from(case.tonic_pc)).rem_euclid(12);
                assert!(
                    scale.contains(&degree),
                    "seed {seed}: pc {pc} is in the key"
                );
            }
            assert_eq!(case.origin_seconds, 2.0);
        }
    }

    #[test]
    fn melodies_behave_tonally() {
        for seed in 1000..1040 {
            for repeated_notes in [false, true] {
                let settings = SynthSettings {
                    repeated_notes,
                    ..SynthSettings::default()
                };
                let case = Case::generate(seed, &settings);
                let scale = if case.minor { MINOR } else { MAJOR };
                let degree = |midi: i32| {
                    let pc = (midi - i32::from(case.tonic_pc)).rem_euclid(12);
                    scale.iter().position(|s| *s == pc).unwrap()
                };
                let (first, last) = (&case.truth[0], case.truth.last().unwrap());
                assert_eq!(degree(first.midi), 0, "seed {seed}: starts on the tonic");
                assert_eq!(degree(last.midi), 0, "seed {seed}: ends on the tonic");
                assert!(
                    last.steps >= 4,
                    "seed {seed}: the final note is a beat or longer"
                );
                assert_eq!(last.bar, case.bars);
                // bars on the tonic chord: a phrase's first and last bar, and the final bar
                for n in &case.truth {
                    let tonic_bar =
                        (n.bar - 1) % 4 == 0 || (n.bar - 1) % 4 == 3 || n.bar == case.bars;
                    if tonic_bar && (n.step == 0 || n.step == 8) {
                        assert!(
                            [0, 2, 4].contains(&degree(n.midi)),
                            "seed {seed} bar {} step {}: a tonic chord tone",
                            n.bar,
                            n.step
                        );
                    }
                }
            }
        }
        let keys: std::collections::BTreeSet<(u8, bool)> = (1000..1040)
            .map(|s| {
                let c = Case::generate(s, &SynthSettings::default());
                (c.tonic_pc, c.minor)
            })
            .collect();
        assert!(keys.len() >= 15, "{} keys over 40 seeds", keys.len());
        assert!(keys.iter().any(|k| k.1) && keys.iter().any(|k| !k.1));
    }

    #[test]
    fn repeated_notes_setting_produces_repeats() {
        let settings = SynthSettings {
            repeated_notes: true,
            ..SynthSettings::default()
        };
        let repeats = (1000..1005)
            .flat_map(|s| Case::generate(s, &settings).truth)
            .collect::<Vec<_>>()
            .windows(2)
            .filter(|w| w[0].midi == w[1].midi)
            .count();
        assert!(repeats > 0, "no repeated pitches generated");
    }

    #[test]
    fn jitter_stays_within_the_requested_range_and_keeps_order() {
        let settings = SynthSettings {
            onset_jitter_ms: 20.0,
            ..SynthSettings::default()
        };
        let case = Case::generate(1003, &settings);
        let step = 60.0 / case.bpm / 4.0;
        let mut moved = false;
        for n in &case.truth {
            let nominal =
                case.origin_seconds + f64::from((n.bar - 1) * STEPS_PER_BAR + n.step) * step;
            let d = (n.start - nominal).abs();
            assert!(d <= 0.020 + 1e-9, "jitter {d} is within 20 ms");
            moved |= d > 1e-6;
            assert!(n.end > n.start);
        }
        assert!(moved);
        for w in case.truth.windows(2) {
            assert!(w[0].end <= w[1].start + 1e-9);
        }
    }

    #[test]
    fn compound_bars_and_pickups() {
        for seed in 1000..1010 {
            let settings = SynthSettings {
                compound: true,
                ..SynthSettings::default()
            };
            let case = Case::generate(seed, &settings);
            assert_eq!((case.beats_per_bar, case.steps_per_bar()), (6.0, 24));
            for n in &case.truth {
                assert!(n.step % 2 == 0, "seed {seed}: whole eighths");
                assert!(n.step + n.steps <= 24);
                assert!((1..=case.bars).contains(&n.bar));
            }
            let last = case.truth.last().unwrap();
            assert!(
                last.steps >= 6,
                "seed {seed}: the final note is a dotted quarter"
            );
            let step = 60.0 / case.bpm / 4.0;
            for n in &case.truth {
                let nominal = case.origin_seconds + f64::from((n.bar - 1) * 24 + n.step) * step;
                assert!((n.start - nominal).abs() < 1e-9);
            }
            // a pickup: one more note at the end of bar 0, the melody unchanged
            let with = Case::generate(
                seed,
                &SynthSettings {
                    pickup: true,
                    ..settings.clone()
                },
            );
            assert_eq!(with.truth[1..], case.truth[..]);
            let p = &with.truth[0];
            assert_eq!(p.bar, 0);
            assert!(p.steps == 2 || p.steps == 4);
            assert_eq!(p.step + p.steps, 24);
            assert!((p.end - with.origin_seconds).abs() < 1e-9);
            assert_ne!(p.midi, with.truth[1].midi);
        }
        // the default is 4/4 with no pickup
        let case = Case::generate(1000, &SynthSettings::default());
        assert_eq!((case.beats_per_bar, case.steps_per_bar()), (4.0, 16));
        assert!(case.truth.iter().all(|n| n.bar >= 1));
    }

    #[test]
    fn hummed_imperfections_change_only_what_they_name() {
        let case = Case::generate(1003, &SynthSettings::default());
        let plain = case.render(16_000);
        let with = |settings: SynthSettings| {
            Case {
                settings,
                ..case.clone()
            }
            .render(16_000)
        };
        for (label, settings) in [
            (
                "wobble",
                SynthSettings {
                    amp_wobble_db: 3.0,
                    ..SynthSettings::default()
                },
            ),
            (
                "drift",
                SynthSettings {
                    pitch_drift_cents: 35.0,
                    ..SynthSettings::default()
                },
            ),
            (
                "soft attack",
                SynthSettings {
                    attack_ms: 60.0,
                    release_ms: 30.0,
                    ..SynthSettings::default()
                },
            ),
        ] {
            let a = with(settings.clone());
            assert_eq!(a.len(), plain.len(), "{label}");
            assert_ne!(a, plain, "{label}");
            assert_eq!(a, with(settings), "{label}: seeded");
        }
        // the drift stays within its peak: a sine's pitch over a long note
        let settings = SynthSettings {
            harmonics: 1,
            formants: false,
            pitch_drift_cents: 35.0,
            ..SynthSettings::default()
        };
        let case = Case::generate(1004, &settings);
        let samples = case.render(DEFAULT_RATE);
        for note in case.truth.iter().filter(|n| n.end - n.start >= 0.2) {
            let hz = zero_crossing_hz(&samples, DEFAULT_RATE, note.start + 0.03, note.end - 0.03);
            let cents = 1200.0 * (hz / midi_to_hz(f64::from(note.midi))).log2();
            assert!(cents.abs() <= 36.0, "midi {}: {cents:.1} cents", note.midi);
        }
    }

    #[test]
    fn peak_is_below_full_scale() {
        let settings = SynthSettings {
            vibrato_cents: 50.0,
            breath_noise_db: Some(-10.0),
            noise_floor_db: Some(-10.0),
            click_bleed_db: Some(0.0),
            ..SynthSettings::default()
        };
        for rate in [16_000, 44_100, 48_000] {
            let samples = Case::generate(1001, &settings).render(rate);
            let peak = samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.1 && peak < 1.0, "rate {rate}: peak {peak}");
            assert!(samples.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn render_has_silent_lead_in_and_sound_in_notes() {
        let case = Case::generate(1002, &SynthSettings::default());
        let rate = 16_000;
        let samples = case.render(rate);
        let lead = (case.origin_seconds * f64::from(rate)) as usize;
        assert!(
            samples[..lead - 1].iter().all(|v| *v == 0.0),
            "silent lead-in"
        );
        assert_eq!(
            samples.len(),
            (case.duration_seconds() * f64::from(rate)).ceil() as usize
        );
        let first = &case.truth[0];
        let mid = ((first.start + first.end) / 2.0 * f64::from(rate)) as usize;
        assert!(samples[mid..mid + 160].iter().any(|v| v.abs() > 0.01));
    }

    #[test]
    fn detune_shifts_every_note_by_the_requested_cents() {
        for detune in [0.0, 30.0, -25.0] {
            let settings = SynthSettings {
                harmonics: 1,
                formants: false,
                detune_cents: detune,
                ..SynthSettings::default()
            };
            let case = Case::generate(1004, &settings);
            let samples = case.render(DEFAULT_RATE);
            let mut measured = 0;
            for note in case.truth.iter().filter(|n| n.end - n.start >= 0.2) {
                let hz =
                    zero_crossing_hz(&samples, DEFAULT_RATE, note.start + 0.03, note.end - 0.03);
                let expected = midi_to_hz(f64::from(note.midi) + detune / 100.0);
                let cents = 1200.0 * (hz / expected).log2();
                assert!(
                    cents.abs() < 1.0,
                    "detune {detune}: midi {} measured {hz:.2} Hz, {cents:.2} cents off",
                    note.midi
                );
                measured += 1;
            }
            assert!(
                measured >= 3,
                "enough long notes were measured ({measured})"
            );
        }
    }

    #[test]
    fn write_wav_round_trips_through_hound() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.wav");
        let samples = Case::generate(1000, &SynthSettings::default()).render(16_000);
        write_wav(&path, &samples, 16_000).unwrap();
        let mut reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.sample_format, hound::SampleFormat::Float);
        assert_eq!(spec.bits_per_sample, 32);
        let back: Vec<f32> = reader.samples::<f32>().map(Result::unwrap).collect();
        assert_eq!(back, samples);
    }
}
