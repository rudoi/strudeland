//! The export post-roll bound (`exportFilterTailSec` and `exportDecay` in `src/lib/render/`,
//! compiled into the host bundle) pinned to the engine it budgets for:
//!
//! - parity: the bound's constants, read from the `tail_frames` of host one-shot plans, equal
//!   the engine's (`strudeland_engine::decay`), within one frame;
//! - completeness: one-shots of every control family, rendered past `tail_frames`, leave
//!   nothing above the floor after the bound.
//!
//! The floors are never loosened to absorb a defect.

use strudeland_engine::capability::unsupported_signatures;
use strudeland_engine::{EngineJob, SAMPLE_RATE, decay, render};
use strudeland_host::{ExportPreset, Host, LengthRequest, PlannedRender, RenderPlanRequest};

const SR: f64 = SAMPLE_RATE as f64;
/// The window rendered after the bound: several times the longest single delay repeat.
const EXTRA: u64 = 5 * SAMPLE_RATE as u64;
/// The planner budgets delay echoes down to -60 dB (`ln(1000)` repeats' worth): a TS decision
/// (echoes below it are dropped by design), not an engine constant.
const DELAY_CUT: f64 = 1e-3;
/// The DJ filter cases' own floor (dBFS). The planner allows the DJ filter a flat 0.5 s; the
/// user kept that tail as it is, so exports do not change. The measured leftover after the
/// bound is -76.9 dBFS at most (a gain-1 sawtooth), and this floor sits about 6 dB above it,
/// so a regression still fails. The render README documents the difference.
const DJF_FLOOR_DBFS: f64 = -70.0;

fn song(voice: &str) -> String {
    format!("const meta = {{ bpm: 120 }}\n$: {voice}\n")
}

fn plan(host: &Host, voice: &str) -> PlannedRender {
    let request = RenderPlanRequest {
        length: LengthRequest::Length { bars: 1 },
        instruments: Default::default(),
        preset: Some(ExportPreset::OneShot),
        include_events: None,
        chunk: None,
        timeout_sec: None,
    };
    let planned = host
        .plan_render(&song(voice), &request)
        .unwrap_or_else(|e| panic!("{voice}: plan failed: {e}"));
    assert!(
        planned.errors.is_empty(),
        "{voice}: plan errors {:?}",
        planned.errors
    );
    assert_eq!(planned.chunks.len(), 1, "{voice}: one chunk");
    planned
}

fn tail(host: &Host, voice: &str) -> u64 {
    plan(host, voice)
        .export
        .unwrap_or_else(|| panic!("{voice}: a one-shot plan has an export"))
        .tail_frames
}

/// Renders the job as planned and again `extra` frames longer; the first `frames` of both must be
/// bit-equal. Returns the long render and the planned `frames`.
fn render_past(name: &str, job_text: &str, extra: u64) -> (Vec<Vec<f32>>, u64) {
    let job = EngineJob::from_json(job_text).unwrap_or_else(|e| panic!("{name}: bad job: {e:?}"));
    let refused = unsupported_signatures(&job);
    assert!(refused.is_empty(), "{name}: unsupported {refused:?}");
    let frames = job.frames;
    let short = render(&job).unwrap_or_else(|e| panic!("{name}: render failed: {e:?}"));
    let mut longer = job.clone();
    longer.frames += extra;
    let long = render(&longer).unwrap_or_else(|e| panic!("{name}: long render failed: {e:?}"));
    assert!(short.errors.is_empty(), "{name}: errors {:?}", short.errors);
    assert!(long.errors.is_empty(), "{name}: errors {:?}", long.errors);
    assert_eq!(
        short.channels.len(),
        long.channels.len(),
        "{name}: channels"
    );
    for (c, (a, b)) in short.channels.iter().zip(&long.channels).enumerate() {
        assert_eq!(a.len() as u64, frames, "{name}: channel {c} length");
        assert_eq!(
            b.len() as u64,
            frames + extra,
            "{name}: long channel {c} length"
        );
        let same = a
            .iter()
            .zip(&b[..frames as usize])
            .all(|(x, y)| x.to_bits() == y.to_bits());
        assert!(
            same,
            "{name}: channel {c} differs before the bound when rendered longer"
        );
    }
    (long.channels, frames)
}

fn peak(channels: &[Vec<f32>], range: std::ops::Range<usize>) -> f64 {
    channels
        .iter()
        .flat_map(|c| c[range.clone()].iter())
        .fold(0.0f64, |m, &x| m.max(f64::from(x.abs())))
}

fn dbfs(x: f64) -> f64 {
    20.0 * x.log10()
}

/// The frames the bound gives a delay of `time` and `feedback` with `delay(1)`: the first tap
/// plus the repeats down to the -60 dB cut, each one render quantum late.
fn echo(time: f64, feedback: f64) -> f64 {
    let tc = time.min(decay::MAX_DELAY_TIME);
    let repeats = if feedback <= 0.0 {
        0.0
    } else {
        ((1.0 / DELAY_CUT).ln() / (1.0 / feedback.min(decay::MAX_FEEDBACK)).ln()).ceil()
    };
    (tc + repeats * (tc + decay::FEEDBACK_EXTRA_FRAMES as f64 / SR)) * SR
}

#[test]
fn the_bound_uses_the_engine_constants() {
    let host = Host::new().unwrap();
    const D: &str = r#"s("sine").note(48).release(1)"#;
    let cases: Vec<(String, f64)> = vec![
        (
            format!("{D}.lpf(400).lfo({{ c: 'cutoff', r: 1, dep: 0.5 }})"),
            decay::MAX_TAIL_TIME * SR,
        ),
        (format!("{D}.bpf(1).bpq(10000)"), decay::MAX_TAIL_TIME * SR),
        (
            format!("{D}.room(1).roomsize(0.5)"),
            decay::IR_DURATION_FACTOR * 0.5 * SR,
        ),
        (
            format!("{D}.room(1).roomsize(4)"),
            decay::IR_DURATION_FACTOR * 4.0 * SR,
        ),
        (
            format!("{D}.delay(1).delaytime(0.5).delayfeedback(0)"),
            echo(0.5, 0.0),
        ),
        (
            format!("{D}.delay(1).delaytime(0.5).delayfeedback(0.5)"),
            echo(0.5, 0.5),
        ),
        (
            format!("{D}.delay(1).delaytime(0.1).delayfeedback(0.98)"),
            echo(0.1, 0.98),
        ),
        // Equal to the 0.98 case through the feedback ceiling.
        (
            format!("{D}.delay(1).delaytime(0.1).delayfeedback(0.99)"),
            echo(0.1, 0.99),
        ),
        // The delay time is clamped to the engine's maximum.
        (
            format!("{D}.delay(1).delaytime(1.5).delayfeedback(0.5)"),
            echo(1.5, 0.5),
        ),
        (format!(r#"{D}.vowel("a")"#), SR),
    ];

    let mut lines = Vec::new();
    let mut mismatches = Vec::new();
    // The dry voice's absolute tail pins the TS sample rate and the release.
    let dry = tail(&host, D);
    let dry_expected = SR;
    lines.push(format!("{D:70} tail {dry:9} expected {dry_expected:12.3}"));
    if (dry as f64 - dry_expected).abs() > 1.0 {
        mismatches.push(format!("{D}: tail {dry}, expected {dry_expected}"));
    }
    for (voice, expected) in &cases {
        let delta = tail(&host, voice) as f64 - dry as f64;
        lines.push(format!(
            "{voice:70} dF   {delta:9} expected {expected:12.3} diff {:+.3}",
            delta - expected
        ));
        if (delta - expected).abs() > 1.0 {
            mismatches.push(format!("{voice}: dF {delta}, expected {expected}"));
        }
    }

    // The bound's +1 s per vowel covers the slowest formant's 6·Q/f.
    let (letter, worst) = decay::FORMANTS
        .iter()
        .map(|(letter, freqs, _, qs)| {
            let w = freqs
                .iter()
                .zip(qs)
                .map(|(f, q)| 6.0 * q / f)
                .fold(0.0f64, f64::max);
            (*letter, w)
        })
        .fold(("", 0.0f64), |a, b| if b.1 > a.1 { b } else { a });
    lines.push(format!(
        "vowel allowance: max 6·Q/f {worst:.4} s ('{letter}') against 1 s"
    ));
    if worst > 1.0 {
        mismatches.push(format!(
            "vowel '{letter}': 6·Q/f {worst} s exceeds the 1 s allowance"
        ));
    }

    eprintln!("{}", lines.join("\n"));
    assert!(
        mismatches.is_empty(),
        "parity mismatches:\n{}",
        mismatches.join("\n")
    );
}

#[derive(Clone, Copy)]
enum Floor {
    /// Blink's `MAX_TAIL_AMPLITUDE`.
    Tail,
    /// -60 dB under the render's peak before the bound.
    Delay,
    /// [`DJF_FLOOR_DBFS`].
    Djf,
}

#[test]
fn one_shots_decay_below_the_floor_after_the_bound() {
    let host = Host::new().unwrap();
    const B: &str = r#"note(36).s("sawtooth").gain(1)"#;
    const SINE: &str = r#"s("sine").note(60).gain(1)"#;
    let mut cases: Vec<(String, Floor)> = [
        ".release(0.01)",
        ".release(0.5)",
        ".release(4)",
        ".lpf(1500).lpq(15)",
        ".lpf(40).lpq(30)",
        ".hpf(1500).hpq(15)",
        ".hpf(20000).hpq(30)",
        ".bpf(1500).bpq(10)",
        ".bpf(40).bpq(20)",
        ".lpf(40).lpq(30).ftype('24db')",
        ".bpf(40).bpq(20).ftype('24db')",
        ".lpf(300).lpq(20).lpenv(-4)",
        ".lpf(400).lpq(20).lfo({ c: 'cutoff', r: 1, dep: 0.5 })",
        ".lpf(200).lpq(20).ftype('ladder')",
        r#".vowel("a")"#,
        r#".vowel("ue")"#,
        ".phaser(1)",
        ".phaser(1).phaserdepth(1)",
    ]
    .iter()
    .map(|c| (format!("{B}{c}"), Floor::Tail))
    .collect();
    for d in ["0.03", "0.05", "0.53", "0.55"] {
        cases.push((format!("{B}.djf({d})"), Floor::Djf));
        cases.push((format!("{SINE}.djf({d})"), Floor::Djf));
    }
    for c in [
        ".room(1).roomsize(0.5)",
        ".room(1)",
        ".room(1).roomsize(10)",
    ] {
        cases.push((format!("{B}{c}"), Floor::Tail));
    }
    for c in [
        ".delay(1).delaytime(0.1).delayfeedback(0.98)",
        ".delay(1).delaytime(0.1).delayfeedback(0.99)",
        ".delay(1).delaytime(0.5).delayfeedback(0.5)",
        ".delay(1).delaytime(1.5).delayfeedback(0.9)",
    ] {
        cases.push((format!("{B}{c}"), Floor::Delay));
    }
    assert_eq!(cases.len(), 33, "the case table");

    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (voice, floor_kind) in &cases {
        let planned = plan(&host, voice);
        let tail_frames = planned.export.as_ref().unwrap().tail_frames;
        let (long, frames) = render_past(voice, &planned.chunks[0].json, EXTRA);
        let (frames, extra) = (frames as usize, EXTRA as usize);
        let before = peak(&long, 0..frames);
        let after = peak(&long, frames..frames + extra);
        let floor = match floor_kind {
            Floor::Tail => decay::MAX_TAIL_AMPLITUDE,
            Floor::Delay => DELAY_CUT * before,
            Floor::Djf => 10f64.powf(DJF_FLOOR_DBFS / 20.0),
        };
        let pass = after <= floor;
        rows.push(format!(
            "{voice:75} tail {tail_frames:8} before {:7.1} after {:7.1} floor {:6.1} {}",
            dbfs(before),
            dbfs(after),
            dbfs(floor),
            if pass { "pass" } else { "FAIL" }
        ));
        if !pass {
            failures.push(format!(
                "{voice}: {:.1} dBFS after the bound, floor {:.1} dBFS",
                dbfs(after),
                dbfs(floor)
            ));
        }
    }
    eprintln!("{}", rows.join("\n"));
    assert!(
        failures.is_empty(),
        "cases above their floor after the bound:\n{}",
        failures.join("\n")
    );
}
