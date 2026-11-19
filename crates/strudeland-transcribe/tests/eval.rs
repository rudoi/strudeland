//! The evaluation tiers: every case of each tier through `transcribe`, scored by COnP (onset
//! within 50 ms, pitch within 50 cents, one to one), micro-averaged per tier, against a pinned
//! minimum. The pins are the measured F1 minus 0.03, rounded down to a multiple of 0.01 (the
//! measured table is in the crate README). A change that lowers a measured score is a defect to
//! fix, never a pin to lower. `cargo run --release -p strudeland-transcribe --features eval --example eval` prints
//! the table.

use strudeland_transcribe::Resolved;
use strudeland_transcribe::eval::cases::{
    CaseResult, TIERS, Tier, TierResult, run_tier, tier_cases,
};

/// Measured 1.000.
const MIN_F1_CLEAN: f64 = 0.97;
/// Measured 0.997.
const MIN_F1_VIBRATO_GLIDE: f64 = 0.96;
/// Measured 1.000.
const MIN_F1_NOISY: f64 = 0.97;
/// Measured 1.000.
const MIN_F1_DETUNED_JITTERED: f64 = 0.97;
/// Measured 1.000.
const MIN_F1_NO_GRID: f64 = 0.97;
/// Measured 0.998.
const MIN_F1_HUMMED: f64 = 0.96;
/// The share of clean cases whose detected key is exactly the generator's; measured 0.60 (3 of
/// 5; the other two decline as too close, none is wrong).
const MIN_KEY_ACCURACY_CLEAN: f64 = 0.57;
/// The no-grid tier's inferred tempo is within this fraction of the truth.
const TEMPO_TOLERANCE: f64 = 0.02;
/// The no-grid tier's inferred phase is within this many seconds of the truth, modulo one beat.
const PHASE_TOLERANCE: f64 = 0.050;

fn summary(cases: &[CaseResult]) -> String {
    cases
        .iter()
        .map(|c| {
            format!(
                "seed {}: tp {} fp {} fn {} F1 {:.3} exact {} key {} ({})",
                c.seed,
                c.score.tp,
                c.score.fp,
                c.score.fn_,
                c.score.f1,
                c.exact,
                c.key_name.as_deref().unwrap_or("none"),
                c.truth_key
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn tier(tier: Tier, min_f1: f64) -> TierResult {
    let result = run_tier(tier).unwrap_or_else(|e| panic!("{}: {e}", tier.name()));
    let count = TIERS.iter().find(|t| t.0 == tier).map(|t| t.1);
    assert_eq!(Some(result.cases.len()), count, "{}", tier.name());
    assert!(
        result.score.f1 >= min_f1,
        "{}: F1 {:.3} (precision {:.3}, recall {:.3}) below the pin {min_f1}\n{}",
        tier.name(),
        result.score.f1,
        result.score.precision,
        result.score.recall,
        summary(&result.cases)
    );
    result
}

#[test]
fn tier_clean() {
    let r = tier(Tier::Clean, MIN_F1_CLEAN);
    for c in &r.cases {
        assert!(
            c.exact,
            "clean seed {}: the notation is not exact\n got  {:?}\n{}",
            c.seed,
            c.transcription.notation,
            summary(&r.cases)
        );
    }
    assert!(
        r.key_accuracy() >= MIN_KEY_ACCURACY_CLEAN,
        "clean: key accuracy {:.2} below the pin {MIN_KEY_ACCURACY_CLEAN}\n{}",
        r.key_accuracy(),
        summary(&r.cases)
    );
}

#[test]
fn tier_vibrato_glide() {
    tier(Tier::VibratoGlide, MIN_F1_VIBRATO_GLIDE);
}

#[test]
fn tier_noisy() {
    tier(Tier::Noisy, MIN_F1_NOISY);
}

#[test]
fn tier_detuned_jittered() {
    tier(Tier::DetunedJittered, MIN_F1_DETUNED_JITTERED);
}

#[test]
fn tier_no_grid() {
    let r = tier(Tier::NoGrid, MIN_F1_NO_GRID);
    for c in &r.cases {
        let grid = c.transcription.grid.as_ref();
        assert_eq!(
            grid.map(|g| (g.source.tempo, g.source.phase)),
            Some((Resolved::Inferred, Resolved::Inferred)),
            "no-grid seed {}",
            c.seed
        );
        let tempo = c.tempo_error.expect("a grid");
        assert!(
            tempo.abs() <= TEMPO_TOLERANCE,
            "no-grid seed {}: tempo off by {:.2}%",
            c.seed,
            tempo * 100.0
        );
        let phase = c.phase_error.expect("a grid");
        assert!(
            phase.abs() <= PHASE_TOLERANCE,
            "no-grid seed {}: phase off by {:.1} ms modulo one beat",
            c.seed,
            phase * 1000.0
        );
    }
}

#[test]
fn tier_hummed() {
    let r = tier(Tier::Hummed, MIN_F1_HUMMED);
    // the pickup cases start their notation with the pickup bar
    let cases = tier_cases(Tier::Hummed);
    let pickups: Vec<&CaseResult> = r
        .cases
        .iter()
        .zip(&cases)
        .filter(|(_, case)| case.case.settings.pickup)
        .map(|(c, _)| c)
        .collect();
    assert_eq!(pickups.len(), 2, "the hummed tier has two pickup cases");
    for c in pickups {
        let notation = c.transcription.notation.as_ref().expect("notation");
        assert_eq!(notation.first_bar, 0, "hummed seed {}", c.seed);
    }
}
