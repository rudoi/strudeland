//! The export post-roll bound (`exportFilterTailSec` and `exportDecay` in `src/lib/render/`,
//! compiled into the host bundle) pinned to the engine it budgets for:
//!
//! - parity: the bound's constants, read from the `tail_frames` of host one-shot plans, equal
//!   the engine's (`strudeland_engine::decay`), within one frame;
//! - completeness: one-shots of every control family, rendered past `tail_frames`, leave
//!   nothing above the floor after the bound, and each one sounds before it (so a case cannot
//!   pass by being silent), and a delay or reverb case rings past the dry voice's own budget.
//!
//! The floors are judged on the raw engine output, before any leveling (a one-shot export is
//! later leveled to -23 LUFS, which moves the leftover with it). They are never loosened to
//! absorb a defect.

use strudeland_engine::{EngineJob, SAMPLE_RATE, decay, render, unsupported_signatures};
use strudeland_host::{ExportPreset, Host, LengthRequest, PlannedRender, RenderPlanRequest};

const SR: f64 = SAMPLE_RATE as f64;
/// The window rendered after the bound: several times the longest single delay repeat.
const EXTRA: u64 = 5 * SAMPLE_RATE as u64;
/// The planner budgets delay echoes down to -60 dB (`ln(1000)` repeats' worth): a TS decision
/// (echoes below it are dropped by design), not an engine constant.
const DELAY_CUT: f64 = 1e-3;
/// The sawtooth DJ filter cases' own floor (dBFS). The planner allows the DJ filter a flat
/// 0.5 s; the user kept that tail as it is, so exports do not change. The measured leftover
/// after the bound is -76.9 dBFS at most (a gain-1 sawtooth), and this floor sits about 6 dB
/// above it, so a regression still fails. The render README documents the difference.
const DJF_FLOOR_DBFS: f64 = -70.0;
/// The sine DJ filter cases' own floor (dBFS): the measured leftover (-90.7 dBFS, a gain-1
/// sine) plus 6 dB, rounded to 0.5 dB.
const DJF_SINE_FLOOR_DBFS: f64 = -84.5;
/// A case sounds when its peak before the bound is this far above its floor.
const SOUNDING_DB: f64 = 10.0;
/// A delay case's floor is relative, so it sounds when its peak before the bound is above this.
const DELAY_SOUNDING_DBFS: f64 = -40.0;
/// A [`Floor::Relative`] case sounds when its peak before the bound is above this. The 10 dB
/// rule does not apply: a relative floor moves with the source, so a quiet case is not vacuous
/// (its drop is judged at any level), and these cases pass too little of the source to sound
/// 10 dB above an absolute floor at any gain (their leftover scales 1:1 with it).
const RELATIVE_SOUNDING_DBFS: f64 = -100.0;

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
        // Blink's `SetPreDelayTime`: `(unsigned)(kPreDelay * sampleRate)` frames, 264.
        (
            format!(r#"{D}.compressor("-20:12:6:0.01:0.1")"#),
            f64::from((decay::COMPRESSOR_PRE_DELAY * SAMPLE_RATE as f32) as u32),
        ),
    ];
    assert_eq!(cases.len() + 1, 12, "the parity table");

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

#[derive(Clone, Copy, PartialEq)]
enum Floor {
    /// Blink's `MAX_TAIL_AMPLITUDE`.
    Tail,
    /// -60 dB under the render's peak before the bound.
    Delay,
    /// [`DJF_FLOOR_DBFS`].
    Djf,
    /// [`DJF_SINE_FLOOR_DBFS`].
    DjfSine,
    /// This many dB under the render's peak before the bound: a known gap pinned as measured.
    /// At djf(0.03) the 0.5 s allowance ends while the tail is still near its peak, so the
    /// export cuts it (the user's decision: exports stay as they are). The value sits about
    /// 0.5 dB inside the measured drop, so a regression that cuts more still fails.
    Relative(f64),
}

struct Case {
    voice: String,
    gain: f64,
    floor: Floor,
    /// A delay or reverb: its wet signal must outlast the dry voice's own budget.
    wet: bool,
}

fn saw(gain: f64) -> String {
    format!(r#"note(36).s("sawtooth").gain({gain})"#)
}

fn sine(gain: f64) -> String {
    format!(r#"s("sine").note(60).gain({gain})"#)
}

fn db_to_amp(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

#[test]
fn one_shots_decay_below_the_floor_after_the_bound() {
    let host = Host::new().unwrap();
    let case = |base: String, gain: f64, fx: &str, floor: Floor, wet: bool| Case {
        voice: format!("{base}{fx}"),
        gain,
        floor,
        wet,
    };
    let mut cases: Vec<Case> = [
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
        ".lpf(300).lpq(20).lpenv(4).fanchor(0.5)",
        ".hpf(500).hpq(15).hpenv(4)",
        ".bpf(500).bpq(10).bpenv(-3)",
        // The built-in filter LFO: the bound sweeps the cutoff down to 30 Hz.
        ".lpf(400).lpq(20).lprate(1)",
        ".lpf(400).lpq(20).lpdepth(1).lpshape(2).lpskew(0.2)",
        ".lpf(400).lpq(20).lpenv(2).lprate(1)",
        ".hpf(500).hpq(15).hprate(2)",
        ".bpf(500).bpq(10).bprate(1)",
        ".lpf(400).lpq(20).lfo({ c: 'cutoff', r: 1, dep: 0.5 })",
        ".lpf(200).lpq(20).ftype('ladder')",
        r#".vowel("a")"#,
        r#".vowel("ue")"#,
        ".phaser(1)",
        ".phaser(1).phaserdepth(1)",
        // FXrelease extends the voice's own need past its release.
        ".lpf(600).lpq(12).release(0.05).fxr(0.4)",
        r#".vowel("a").release(0.05).fxr(2)"#,
        r#".compressor("-20:12:6:0.01:0.1")"#,
    ]
    .iter()
    .map(|fx| case(saw(1.0), 1.0, fx, Floor::Tail, false))
    .collect();
    // djf(0.03) is the known gap: measured drops of 4.8 dB (sawtooth) and 9.6 dB (sine).
    cases.push(case(
        saw(1.0),
        1.0,
        ".djf(0.03)",
        Floor::Relative(4.0),
        false,
    ));
    cases.push(case(
        sine(1.0),
        1.0,
        ".djf(0.03)",
        Floor::Relative(9.0),
        false,
    ));
    for d in ["0.05", "0.53", "0.55"] {
        let fx = format!(".djf({d})");
        cases.push(case(saw(1.0), 1.0, &fx, Floor::Djf, false));
        cases.push(case(sine(1.0), 1.0, &fx, Floor::DjfSine, false));
    }
    for fx in [
        ".room(1).roomsize(0.5)",
        ".room(1)",
        ".room(1).roomsize(10)",
    ] {
        cases.push(case(saw(1.0), 1.0, fx, Floor::Tail, true));
    }
    for fx in [
        ".delay(1).delaytime(0.1).delayfeedback(0.98)",
        ".delay(1).delaytime(0.1).delayfeedback(0.99)",
        ".delay(1).delaytime(0.5).delayfeedback(0.5)",
        ".delay(1).delaytime(1.5).delayfeedback(0.9)",
    ] {
        cases.push(case(saw(1.0), 1.0, fx, Floor::Delay, true));
    }
    assert_eq!(cases.len(), 44, "the case table");

    // The dry voice the wet cases add their effect to: its own budget ends at its job's frames.
    let dry_voice = saw(1.0);
    let dry_plan = plan(&host, &dry_voice);
    let dry_tail = dry_plan.export.as_ref().unwrap().tail_frames;
    let (dry_long, dry_frames) = render_past(&dry_voice, &dry_plan.chunks[0].json, EXTRA);
    let dry_frames = dry_frames as usize;
    let dry_before = peak(&dry_long, 0..dry_frames);

    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for Case {
        voice,
        gain,
        floor: floor_kind,
        wet,
    } in &cases
    {
        let planned = plan(&host, voice);
        let tail_frames = planned.export.as_ref().unwrap().tail_frames;
        let (long, frames) = render_past(voice, &planned.chunks[0].json, EXTRA);
        let (frames, extra) = (frames as usize, EXTRA as usize);
        let before = peak(&long, 0..frames);
        let after = peak(&long, frames..frames + extra);
        let floor = match floor_kind {
            Floor::Tail => decay::MAX_TAIL_AMPLITUDE,
            Floor::Delay => DELAY_CUT * before,
            Floor::Djf => db_to_amp(DJF_FLOOR_DBFS),
            Floor::DjfSine => db_to_amp(DJF_SINE_FLOOR_DBFS),
            Floor::Relative(db) => before * db_to_amp(-db),
        };
        let mut problems = Vec::new();
        if after > floor {
            problems.push(format!(
                "{:.1} dBFS after the bound, floor {:.1} dBFS",
                dbfs(after),
                dbfs(floor)
            ));
        }
        let sounding = match floor_kind {
            Floor::Delay => db_to_amp(DELAY_SOUNDING_DBFS),
            Floor::Relative(_) => db_to_amp(RELATIVE_SOUNDING_DBFS),
            _ => floor * db_to_amp(SOUNDING_DB),
        };
        if before < sounding {
            problems.push(format!(
                "silent: {:.1} dBFS before the bound, needs {:.1} dBFS",
                dbfs(before),
                dbfs(sounding)
            ));
        }
        let mut wet_db = String::new();
        if *wet {
            let ring = DELAY_CUT * dry_before;
            let wet_peak = peak(&long, dry_frames.min(frames)..frames);
            wet_db = format!(" wet {:7.1}", dbfs(wet_peak));
            if wet_peak <= ring {
                problems.push(format!(
                    "no wet signal after the dry budget: {:.1} dBFS in frames {dry_frames}..{frames}, needs {:.1} dBFS",
                    dbfs(wet_peak),
                    dbfs(ring)
                ));
            }
            if tail_frames <= dry_tail {
                problems.push(format!(
                    "tail {tail_frames} frames is not past the dry tail {dry_tail}"
                ));
            }
        }
        rows.push(format!(
            "{voice:75} gain {gain:3} tail {tail_frames:8} before {:7.1} after {:7.1} floor {:6.1} margin {:6.1}{wet_db} {}",
            dbfs(before),
            dbfs(after),
            dbfs(floor),
            dbfs(floor) - dbfs(after),
            if problems.is_empty() { "pass" } else { "FAIL" }
        ));
        failures.extend(problems.into_iter().map(|p| format!("{voice}: {p}")));
    }
    eprintln!(
        "dry {dry_voice}: tail {dry_tail} frames {dry_frames} before {:.1}",
        dbfs(dry_before)
    );
    eprintln!("{}", rows.join("\n"));
    assert!(
        failures.is_empty(),
        "cases that fail after the bound:\n{}",
        failures.join("\n")
    );
}
