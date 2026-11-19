//! The evaluation set: 25 seeded cases in six tiers, run through [`transcribe`] and scored.
//!
//! `tests/eval.rs` and `examples/eval.rs` both build their cases here, so the pinned tests and
//! the printed table cannot drift apart. Case `i` (0-based over the whole set, tiers in
//! [`TIERS`] order) is generated from seed `1000 + i` and rendered at 48 kHz, so the resampler is
//! in the loop. The grid comes from the case (a sidecar's hint) except in the no-grid tier, whose
//! hint is empty.

use super::score::{Score, ScoredNote, conp, exact_notation, micro_average};
use super::synth::{Case, DEFAULT_RATE, SplitMix64, SynthSettings};
use crate::grid::{GridHint, Hinted};
use crate::key::Key;
use crate::report::Transcription;
use crate::{Options, TranscribeError, transcribe};

/// The first case's seed; case `i` uses `FIRST_SEED + i`.
pub const FIRST_SEED: u64 = 1000;

/// A difficulty tier of the evaluation set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Harmonic voice with formants, no vibrato, no noise, no detune, exact grid timing.
    Clean,
    /// +-50 cents vibrato at 5.5 Hz, 60 ms attack glides, portamento, repeated notes.
    VibratoGlide,
    /// Breath noise at -30 dB, a noise floor at -45 dB, metronome click bleed at -35 dB.
    Noisy,
    /// A global detune uniform within +-35 cents, onset jitter uniform within +-20 ms.
    DetunedJittered,
    /// The clean voice with an empty grid hint: tempo and phase are inferred.
    NoGrid,
    /// A low hummed voice: soft attacks, a slow loudness wobble and pitch drift within notes,
    /// repeated notes, a small detune and onset jitter; 4/4 and 12/8, two cases with a pickup.
    Hummed,
}

/// The tiers in set order, with their case counts (25 in all).
pub const TIERS: [(Tier, usize); 6] = [
    (Tier::Clean, 5),
    (Tier::VibratoGlide, 4),
    (Tier::Noisy, 4),
    (Tier::DetunedJittered, 4),
    (Tier::NoGrid, 3),
    (Tier::Hummed, 5),
];

/// Portamento between adjacent notes in the vibrato and glide tier, in milliseconds.
pub const PORTAMENTO_MS: f64 = 60.0;
/// The detuned tier's detune range, either way, in cents.
pub const DETUNE_RANGE_CENTS: f64 = 35.0;
/// The detuned tier's onset jitter, either way, in milliseconds.
pub const JITTER_MS: f64 = 20.0;
/// The hummed tier's loudness wobble peak range, in dB either way: a swing of 3 to 6 dB, as
/// held notes of a real hummed memo measured (1 to 3 dB either way).
pub const HUM_WOBBLE_DB: (f64, f64) = (1.5, 3.0);
/// The hummed tier's pitch drift peak range, in cents either way.
pub const HUM_DRIFT_CENTS: (f64, f64) = (30.0, 40.0);
/// The hummed tier's attack, in milliseconds.
pub const HUM_ATTACK_MS: f64 = 60.0;
/// The hummed tier's release, in milliseconds.
pub const HUM_RELEASE_MS: f64 = 30.0;
/// The hummed tier's detune range, either way, in cents.
pub const HUM_DETUNE_CENTS: f64 = 20.0;
/// The hummed tier's onset jitter, either way, in milliseconds.
pub const HUM_JITTER_MS: f64 = 15.0;
/// The hummed tier's MIDI range: a low voice, G2 to E4.
pub const HUM_MIDI_RANGE: (i32, i32) = (43, 64);

impl Tier {
    /// The tier's name in the table and in failure messages.
    pub fn name(self) -> &'static str {
        match self {
            Tier::Clean => "clean",
            Tier::VibratoGlide => "vibrato + glide",
            Tier::Noisy => "noisy",
            Tier::DetunedJittered => "detuned + jittered",
            Tier::NoGrid => "no-grid inference",
            Tier::Hummed => "hummed",
        }
    }

    /// The tier's synth settings for the case of `seed`, the tier's case number `place` (0-based
    /// within the tier). The detune and the hummed draws come from the seed, the hummed tier's
    /// meter and pickup from the place.
    pub fn settings(self, seed: u64, place: usize) -> SynthSettings {
        let base = SynthSettings::default();
        match self {
            Tier::Clean | Tier::NoGrid => base,
            Tier::VibratoGlide => SynthSettings {
                vibrato_cents: 50.0,
                vibrato_hz: 5.5,
                attack_glide_ms: 60.0,
                portamento_ms: PORTAMENTO_MS,
                repeated_notes: true,
                ..base
            },
            Tier::Noisy => SynthSettings {
                breath_noise_db: Some(-30.0),
                noise_floor_db: Some(-45.0),
                click_bleed_db: Some(-35.0),
                ..base
            },
            Tier::DetunedJittered => {
                // a stream of its own, so the melody does not depend on the detune draw
                let mut rng = SplitMix64::new(seed ^ 0x5EED_DE7E_0000_0000);
                SynthSettings {
                    detune_cents: rng.uniform(-DETUNE_RANGE_CENTS, DETUNE_RANGE_CENTS),
                    onset_jitter_ms: JITTER_MS,
                    ..base
                }
            }
            Tier::Hummed => {
                let mut rng = SplitMix64::new(seed ^ 0x4855_4D4D_4544_7E57);
                // by the case's place in the tier: 4/4, 12/8, 4/4 with a pickup, 12/8 with a
                // pickup, 12/8
                SynthSettings {
                    compound: matches!(place, 1 | 3 | 4),
                    pickup: matches!(place, 2 | 3),
                    midi_range: HUM_MIDI_RANGE,
                    attack_ms: HUM_ATTACK_MS,
                    release_ms: HUM_RELEASE_MS,
                    amp_wobble_db: rng.uniform(HUM_WOBBLE_DB.0, HUM_WOBBLE_DB.1),
                    pitch_drift_cents: rng.uniform(HUM_DRIFT_CENTS.0, HUM_DRIFT_CENTS.1),
                    detune_cents: rng.uniform(-HUM_DETUNE_CENTS, HUM_DETUNE_CENTS),
                    onset_jitter_ms: HUM_JITTER_MS,
                    repeated_notes: true,
                    ..base
                }
            }
        }
    }
}

/// One case of the set.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalCase {
    pub tier: Tier,
    /// 0-based over the whole set.
    pub index: usize,
    /// 0-based within its tier.
    pub place: usize,
    pub case: Case,
}

impl EvalCase {
    pub fn seed(&self) -> u64 {
        self.case.seed
    }

    /// The grid hint the case is transcribed with: the generator's grid as a sidecar gives it,
    /// or nothing for the no-grid tier.
    pub fn hint(&self) -> GridHint {
        match self.tier {
            Tier::NoGrid => GridHint::default(),
            _ => GridHint {
                bpm: Some(Hinted::sidecar(self.case.bpm)),
                beats_per_bar: Some(Hinted::sidecar(self.case.beats_per_bar)),
                origin_seconds: Some(Hinted::sidecar(self.case.origin_seconds)),
                ..GridHint::default()
            },
        }
    }
}

/// All 25 cases, tiers in [`TIERS`] order.
pub fn cases() -> Vec<EvalCase> {
    let mut out = Vec::new();
    for (tier, count) in TIERS {
        for place in 0..count {
            let index = out.len();
            let seed = FIRST_SEED + index as u64;
            out.push(EvalCase {
                tier,
                index,
                place,
                case: Case::generate(seed, &tier.settings(seed, place)),
            });
        }
    }
    out
}

/// The cases of one tier.
pub fn tier_cases(tier: Tier) -> Vec<EvalCase> {
    cases().into_iter().filter(|c| c.tier == tier).collect()
}

/// How a detected key relates to the case's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// The same tonic and mode.
    Exact,
    /// A closely related key: the relative, the dominant or subdominant (same mode), or the
    /// parallel key.
    Related,
    /// Another key.
    Wrong,
    /// No key was chosen.
    None,
}

impl KeyOutcome {
    /// Compares `detected` with the key of tonic `tonic_pc` in `minor` or major.
    pub fn of(detected: Option<&Key>, tonic_pc: u8, minor: bool) -> Self {
        let Some(key) = detected else {
            return KeyOutcome::None;
        };
        let (pc, is_minor) = (i32::from(key.tonic_pc % 12), key.scale == "minor");
        if key.scale != "minor" && key.scale != "major" {
            return KeyOutcome::Wrong;
        }
        let truth = i32::from(tonic_pc % 12);
        let up = (pc - truth).rem_euclid(12);
        if is_minor == minor {
            match up {
                0 => KeyOutcome::Exact,
                5 | 7 => KeyOutcome::Related,
                _ => KeyOutcome::Wrong,
            }
        } else if up == 0 || up == if minor { 3 } else { 9 } {
            // parallel, or relative (a minor key's relative major is 3 above, a major key's
            // relative minor 9 above)
            KeyOutcome::Related
        } else {
            KeyOutcome::Wrong
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            KeyOutcome::Exact => "exact",
            KeyOutcome::Related => "related",
            KeyOutcome::Wrong => "wrong",
            KeyOutcome::None => "none",
        }
    }
}

/// One case's result.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseResult {
    pub tier: Tier,
    pub seed: u64,
    pub score: Score,
    /// The notation equals the generator's own, rendered with the transcription's key.
    pub exact: bool,
    pub key: KeyOutcome,
    /// The detected key's name, or `None`.
    pub key_name: Option<String>,
    /// The case's key's name.
    pub truth_key: String,
    /// The resolved tempo's relative error, `(bpm - true) / true`; `None` without a grid.
    pub tempo_error: Option<f64>,
    /// The resolved origin minus the true one, modulo one true beat, in -beat/2..beat/2 seconds;
    /// `None` without a grid.
    pub phase_error: Option<f64>,
    pub transcription: Transcription,
}

/// Transcribes `case` with its hint and default options, and scores it.
pub fn run_case(case: &EvalCase) -> Result<CaseResult, TranscribeError> {
    let samples = case.case.render(DEFAULT_RATE);
    let t = transcribe(&samples, DEFAULT_RATE, &case.hint(), &Options::default())?;
    let reference: Vec<ScoredNote> = case.case.truth.iter().map(Into::into).collect();
    let estimate: Vec<ScoredNote> = t.notes.iter().map(Into::into).collect();
    let truth_key = super::score::case_key(&case.case);
    let beat = 60.0 / case.case.bpm;
    let (tempo_error, phase_error) = match &t.grid {
        Some(g) => {
            let d = (g.origin - case.case.origin_seconds).rem_euclid(beat);
            let d = if d > beat / 2.0 { d - beat } else { d };
            (Some((g.bpm - case.case.bpm) / case.case.bpm), Some(d))
        }
        None => (None, None),
    };
    Ok(CaseResult {
        tier: case.tier,
        seed: case.seed(),
        score: conp(&reference, &estimate),
        exact: exact_notation(t.notation.as_ref(), &case.case, t.key.as_ref()),
        key: KeyOutcome::of(t.key.as_ref(), case.case.tonic_pc, case.case.minor),
        key_name: t.key.as_ref().map(|k| k.name.clone()),
        truth_key: truth_key.name,
        tempo_error,
        phase_error,
        transcription: t,
    })
}

/// A tier's results and their micro-averaged score.
#[derive(Debug, Clone, PartialEq)]
pub struct TierResult {
    pub tier: Tier,
    pub cases: Vec<CaseResult>,
    pub score: Score,
}

impl TierResult {
    /// Cases whose notation matched exactly.
    pub fn exact_count(&self) -> usize {
        self.cases.iter().filter(|c| c.exact).count()
    }

    /// Cases whose key came out as `outcome`.
    pub fn key_count(&self, outcome: KeyOutcome) -> usize {
        self.cases.iter().filter(|c| c.key == outcome).count()
    }

    /// The share of cases with the exact key, 0..=1 (0 for no cases).
    pub fn key_accuracy(&self) -> f64 {
        if self.cases.is_empty() {
            0.0
        } else {
            self.key_count(KeyOutcome::Exact) as f64 / self.cases.len() as f64
        }
    }
}

/// Runs and scores every case of `tier`.
pub fn run_tier(tier: Tier) -> Result<TierResult, TranscribeError> {
    let cases = tier_cases(tier)
        .iter()
        .map(run_case)
        .collect::<Result<Vec<_>, _>>()?;
    let scores: Vec<Score> = cases.iter().map(|c| c.score).collect();
    Ok(TierResult {
        tier,
        score: micro_average(&scores),
        cases,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::KeySource;

    fn key(tonic_pc: u8, minor: bool) -> Key {
        let scale = if minor { "minor" } else { "major" };
        Key {
            name: format!("{} {scale}", crate::key::root_name(tonic_pc)),
            tonic_pc,
            scale: scale.into(),
            confidence: None,
            runner_up: None,
            correlation: None,
            source: KeySource::Detected,
        }
    }

    #[test]
    fn twenty_five_cases_with_consecutive_seeds_in_tier_order() {
        let all = cases();
        assert_eq!(all.len(), 25);
        for (i, c) in all.iter().enumerate() {
            assert_eq!(c.index, i);
            assert_eq!(c.seed(), 1000 + i as u64);
            assert_eq!(
                c.case.settings,
                c.tier.settings(c.seed(), c.place),
                "case {i}"
            );
        }
        let tiers: Vec<Tier> = all.iter().map(|c| c.tier).collect();
        let mut expected = Vec::new();
        for (tier, n) in TIERS {
            expected.extend(std::iter::repeat_n(tier, n));
        }
        assert_eq!(tiers, expected);
        assert_eq!(tier_cases(Tier::NoGrid).len(), 3);
        assert_eq!(tier_cases(Tier::NoGrid)[0].seed(), 1017);
    }

    #[test]
    fn tier_settings_follow_the_table() {
        let clean = Tier::Clean.settings(1000, 0);
        assert_eq!(clean, SynthSettings::default());
        assert_eq!(Tier::NoGrid.settings(1017, 0), clean);
        let v = Tier::VibratoGlide.settings(1005, 0);
        assert_eq!((v.vibrato_cents, v.vibrato_hz), (50.0, 5.5));
        assert_eq!((v.attack_glide_ms, v.portamento_ms), (60.0, PORTAMENTO_MS));
        assert!(v.repeated_notes);
        let n = Tier::Noisy.settings(1009, 0);
        assert_eq!(
            (n.breath_noise_db, n.noise_floor_db, n.click_bleed_db),
            (Some(-30.0), Some(-45.0), Some(-35.0))
        );
        let mut detunes = Vec::new();
        for seed in 1013..1017 {
            let d = Tier::DetunedJittered.settings(seed, 0);
            assert_eq!(d.onset_jitter_ms, 20.0);
            assert!(
                d.detune_cents.abs() < 35.0,
                "seed {seed}: {}",
                d.detune_cents
            );
            assert_eq!(d, Tier::DetunedJittered.settings(seed, 0), "seeded");
            detunes.push(d.detune_cents);
        }
        detunes.dedup();
        assert_eq!(detunes.len(), 4, "each case draws its own detune");
        // hummed: 4/4, 12/8, 4/4 with a pickup, 12/8 with a pickup, 12/8
        let hummed: Vec<SynthSettings> = (0..5)
            .map(|k| Tier::Hummed.settings(1020 + k as u64, k))
            .collect();
        let shapes: Vec<(bool, bool)> = hummed.iter().map(|h| (h.compound, h.pickup)).collect();
        assert_eq!(
            shapes,
            [
                (false, false),
                (true, false),
                (false, true),
                (true, true),
                (true, false)
            ]
        );
        for h in &hummed {
            assert!((HUM_WOBBLE_DB.0..HUM_WOBBLE_DB.1).contains(&h.amp_wobble_db));
            assert!((HUM_DRIFT_CENTS.0..HUM_DRIFT_CENTS.1).contains(&h.pitch_drift_cents));
            assert!(h.detune_cents.abs() < HUM_DETUNE_CENTS);
            assert_eq!((h.attack_ms, h.release_ms), (HUM_ATTACK_MS, HUM_RELEASE_MS));
            assert_eq!(h.midi_range, HUM_MIDI_RANGE);
            assert!(h.repeated_notes && h.onset_jitter_ms == HUM_JITTER_MS);
        }
    }

    #[test]
    fn hints_carry_the_grid_except_no_grid() {
        for c in cases() {
            let h = c.hint();
            if c.tier == Tier::NoGrid {
                assert_eq!(h, GridHint::default(), "seed {}", c.seed());
            } else {
                assert_eq!(h.bpm, Some(Hinted::sidecar(c.case.bpm)));
                let beats = if c.case.settings.compound { 6.0 } else { 4.0 };
                assert_eq!(h.beats_per_bar, Some(Hinted::sidecar(beats)));
                assert_eq!(
                    h.origin_seconds,
                    Some(Hinted::sidecar(c.case.origin_seconds))
                );
            }
        }
    }

    #[test]
    fn key_outcomes() {
        // E major
        let cases = [
            (key(4, false), KeyOutcome::Exact),
            (key(11, false), KeyOutcome::Related), // dominant
            (key(9, false), KeyOutcome::Related),  // subdominant
            (key(1, true), KeyOutcome::Related),   // relative minor
            (key(4, true), KeyOutcome::Related),   // parallel minor
            (key(6, false), KeyOutcome::Wrong),
            (key(8, true), KeyOutcome::Wrong),
        ];
        for (k, want) in &cases {
            assert_eq!(
                KeyOutcome::of(Some(k), 4, false),
                *want,
                "{} vs E major",
                k.name
            );
        }
        // E minor: G major is its relative, B minor its dominant
        assert_eq!(
            KeyOutcome::of(Some(&key(7, false)), 4, true),
            KeyOutcome::Related
        );
        assert_eq!(
            KeyOutcome::of(Some(&key(11, true)), 4, true),
            KeyOutcome::Related
        );
        assert_eq!(
            KeyOutcome::of(Some(&key(4, true)), 4, true),
            KeyOutcome::Exact
        );
        assert_eq!(
            KeyOutcome::of(Some(&key(9, false)), 4, true),
            KeyOutcome::Wrong
        );
        assert_eq!(KeyOutcome::of(None, 4, true), KeyOutcome::None);
        let mut dorian = key(4, true);
        dorian.scale = "dorian".into();
        assert_eq!(KeyOutcome::of(Some(&dorian), 4, true), KeyOutcome::Wrong);
    }

    #[test]
    fn a_sidecar_case_has_no_grid_error() {
        // a case scored against itself: zero errors, exact, the key's outcome recorded
        let c = &tier_cases(Tier::Clean)[0];
        let r = run_case(c).unwrap();
        assert_eq!(r.tempo_error, Some(0.0));
        assert_eq!(r.phase_error, Some(0.0));
        assert_eq!(r.seed, 1000);
        assert_eq!(r.truth_key, super::super::score::case_key(&c.case).name);
    }
}
