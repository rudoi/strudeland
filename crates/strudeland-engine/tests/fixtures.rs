//! The corpus fixture refs: every `tests/refs/fixture-<name>--<mode>-<k>.job.json` (one per chunk
//! of the plan `render:check` uses, written by `bun run engine:refs --write` from
//! `tests/render/corpus/`) is rendered by the engine. Each render has exactly `job.frames` frames
//! and `job.channels.count` channels, only finite samples, is bit-identical when repeated, and
//! survives a WAV round trip. For `rust: "supported"` fixtures the render has no errors and is
//! compared with its sibling superdough reference envelope (`*.ref.json`: RMS in dB per 10 ms
//! window and peak per channel): over every window whose reference RMS is above
//! `fixtures.floorDb`, the absolute dB differences have a mean <= `fixtures.windowMeanDb` and a
//! 95th percentile <= `fixtures.windowP95Db`, and each channel's peak differs by at most
//! `fixtures.peakDb` (`tests/refs/thresholds.json`). This is a coarse level-and-shape guard; the
//! sample-level and feature accuracy of fixtures is judged by `render:check --engine rust`. The
//! unsupported fixture kept on purpose (`noise-seed`) must be refused with
//! its expected unsupported signatures (silence plus `unsupported` errors) and are not
//! compared; every job of a supported fixture has no unsupported signature.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use strudeland_engine::wav::{read_f32, write_f32};
use strudeland_engine::{EngineJob, render, unsupported_signatures};

mod common;
use common::{corpus_path, load_job, read_json, refs_dir};

/// Fixtures that are `rust: "unsupported"` but keep refs on purpose, with the unsupported
/// signatures (`s`, sorted keys) their jobs must have.
const KEPT_UNSUPPORTED: &[(&str, &[(&str, &str)])] = &[("noise-seed", &[("z_noise", "")])];

/// Samples per RMS window of a reference envelope (10 ms at 44100 Hz).
const WINDOW_FRAMES: usize = 441;
/// The dB value of digital silence in a reference envelope.
const SILENCE_DB: f64 = -200.0;

fn read_ref(name: &str) -> Value {
    read_json(&refs_dir().join(name))
}

/// One fixture case of `index.json`.
struct Case {
    fixture: String,
    jobs: Vec<String>,
    envelopes: Vec<String>,
}

fn fixture_cases() -> Vec<Case> {
    let index = read_ref("index.json");
    index["cases"]
        .as_object()
        .expect("index.json has cases")
        .iter()
        .filter(|(_, c)| c["kind"] == "fixture")
        .map(|(name, c)| {
            let list = |key: &str| -> Vec<String> {
                c[key]
                    .as_array()
                    .unwrap_or_else(|| panic!("{name}: {key} is not a list"))
                    .iter()
                    .map(|v| v.as_str().expect("file name").to_string())
                    .collect()
            };
            Case {
                fixture: name
                    .strip_prefix("fixture-")
                    .unwrap_or_else(|| panic!("fixture case {name} lacks the fixture- prefix"))
                    .to_string(),
                jobs: list("jobs"),
                envelopes: list("envelopes"),
            }
        })
        .collect()
}

/// `name -> (rust supported, kind)` from `tests/render/corpus.json`.
fn corpus() -> BTreeMap<String, (bool, String)> {
    read_json(&corpus_path())["fixtures"]
        .as_array()
        .expect("corpus.json has fixtures")
        .iter()
        .map(|f| {
            (
                f["name"].as_str().expect("name").to_string(),
                (
                    f["rust"] == "supported",
                    f["kind"].as_str().unwrap_or("").to_string(),
                ),
            )
        })
        .collect()
}

/// `20*log10(rms)` of `x`, [`SILENCE_DB`] for digital silence (or an empty slice).
fn rms_db(x: &[f32]) -> f64 {
    if x.is_empty() {
        return SILENCE_DB;
    }
    let sq: f64 = x.iter().map(|&v| f64::from(v) * f64::from(v)).sum();
    if sq == 0.0 {
        return SILENCE_DB;
    }
    20.0 * (sq / x.len() as f64).sqrt().log10()
}

/// The window RMS values in dB of one channel, windows of [`WINDOW_FRAMES`] (the last may be
/// shorter).
fn window_rms_db(x: &[f32]) -> Vec<f64> {
    x.chunks(WINDOW_FRAMES).map(rms_db).collect()
}

/// `20*log10(peak)`, [`SILENCE_DB`] for 0.
fn peak_db(peak: f64) -> f64 {
    if peak > 0.0 {
        20.0 * peak.log10()
    } else {
        SILENCE_DB
    }
}

/// The nearest-rank 95th percentile of `v` (`None` when empty).
fn p95(v: &[f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let rank = ((0.95 * s.len() as f64).ceil() as usize).clamp(1, s.len());
    Some(s[rank - 1])
}

/// The tolerances of the `fixtures` section of `thresholds.json`.
#[derive(Clone, Copy, Debug)]
struct Tolerances {
    floor_db: f64,
    window_mean_db: f64,
    window_p95_db: f64,
    peak_db: f64,
}

impl Tolerances {
    fn load() -> Self {
        let f = &read_ref("thresholds.json")["fixtures"];
        let get = |k: &str| {
            f[k].as_f64()
                .unwrap_or_else(|| panic!("thresholds.json fixtures.{k}"))
        };
        Tolerances {
            floor_db: get("floorDb"),
            window_mean_db: get("windowMeanDb"),
            window_p95_db: get("windowP95Db"),
            peak_db: get("peakDb"),
        }
    }
}

/// The measured envelope differences of one channel.
#[derive(Debug, PartialEq)]
struct ChannelDelta {
    /// Windows above the floor in the reference.
    windows: usize,
    mean_db: f64,
    p95_db: f64,
    /// `None` when both peaks are below the floor.
    peak_db: Option<f64>,
}

/// Compares one rendered channel with the reference envelope's window RMS values and peak.
fn channel_delta(
    rendered: &[f32],
    ref_rms_db: &[f64],
    ref_peak: f64,
    floor_db: f64,
) -> ChannelDelta {
    let ours = window_rms_db(rendered);
    let diffs: Vec<f64> = ref_rms_db
        .iter()
        .zip(&ours)
        .filter(|(r, _)| **r > floor_db)
        .map(|(r, o)| (o - r).abs())
        .collect();
    let peak = rendered
        .iter()
        .fold(0.0f64, |m, &v| m.max(f64::from(v).abs()));
    let (rp, ep) = (peak_db(ref_peak), peak_db(peak));
    ChannelDelta {
        windows: diffs.len(),
        mean_db: if diffs.is_empty() {
            0.0
        } else {
            diffs.iter().sum::<f64>() / diffs.len() as f64
        },
        p95_db: p95(&diffs).unwrap_or(0.0),
        peak_db: (rp > floor_db || ep > floor_db).then(|| (ep - rp).abs()),
    }
}

impl ChannelDelta {
    fn problems(&self, t: Tolerances) -> Vec<String> {
        let mut out = Vec::new();
        if self.mean_db > t.window_mean_db {
            out.push(format!(
                "window mean {:.3} dB > {} dB",
                self.mean_db, t.window_mean_db
            ));
        }
        if self.p95_db > t.window_p95_db {
            out.push(format!(
                "window p95 {:.3} dB > {} dB",
                self.p95_db, t.window_p95_db
            ));
        }
        if let Some(p) = self.peak_db.filter(|p| *p > t.peak_db) {
            out.push(format!("peak {p:.3} dB > {} dB", t.peak_db));
        }
        out
    }
}

/// The structural checks every fixture job must pass (frames, channels, finite, repeatable, WAV
/// round trip); returns the first render.
fn structural(file: &str, job: &EngineJob) -> Result<strudeland_engine::Rendered, String> {
    let a = render(job).map_err(|e| format!("render: {e}"))?;
    if a.channels.len() != job.channels.count as usize {
        return Err(format!(
            "{} channels, the job has {}",
            a.channels.len(),
            job.channels.count
        ));
    }
    if let Some(c) = a.channels.iter().position(|c| c.len() as u64 != job.frames) {
        return Err(format!(
            "channel {c} has {} frames, the job has {}",
            a.channels[c].len(),
            job.frames
        ));
    }
    if let Some((c, i)) = a
        .channels
        .iter()
        .enumerate()
        .find_map(|(c, ch)| ch.iter().position(|v| !v.is_finite()).map(|i| (c, i)))
    {
        return Err(format!("non-finite sample at channel {c} frame {i}"));
    }
    let b = render(job).map_err(|e| format!("repeat render: {e}"))?;
    let same = a.channels.len() == b.channels.len()
        && a.channels.iter().zip(&b.channels).all(|(x, y)| {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.to_bits() == q.to_bits())
        });
    if !same {
        return Err("a repeat render is not bit-identical".into());
    }
    if a.errors != b.errors || a.unsupported != b.unsupported {
        return Err("a repeat render reports different errors".into());
    }
    let mut bytes = Vec::new();
    write_f32(&mut bytes, &a.channels, 44100).map_err(|e| format!("wav write: {e}"))?;
    let back = read_f32(&bytes).map_err(|e| format!("wav read: {e}"))?;
    let round_trips = back.sample_rate == 44100
        && back.channels.len() == a.channels.len()
        && back.channels.iter().zip(&a.channels).all(|(x, y)| {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.to_bits() == q.to_bits())
        });
    if !round_trips {
        return Err(format!("{file}: the WAV round trip changed the render"));
    }
    Ok(a)
}

/// Checks one chunk job of a supported fixture; the per-channel deltas, or the reasons it fails.
fn check_supported(
    job_file: &str,
    env_file: &str,
    t: Tolerances,
) -> Result<Vec<ChannelDelta>, String> {
    let job = load_job(job_file);
    let rendered = structural(job_file, &job)?;
    if !rendered.errors.is_empty() {
        return Err(format!("render errors: {:?}", rendered.errors));
    }
    if !rendered.unsupported.is_empty() {
        return Err(format!("unsupported: {:?}", rendered.unsupported));
    }
    let env = read_ref(env_file);
    if env["schema"] != 1 || env["windowFrames"].as_u64() != Some(WINDOW_FRAMES as u64) {
        return Err("envelope schema or windowFrames".into());
    }
    if env["frames"].as_u64() != Some(job.frames) {
        return Err("envelope frames differ from the job".into());
    }
    let rms = env["rmsDb"].as_array().ok_or("envelope rmsDb")?;
    let peaks = env["peak"].as_array().ok_or("envelope peak")?;
    if rms.len() != rendered.channels.len() || peaks.len() != rendered.channels.len() {
        return Err("envelope channel count differs from the render".into());
    }
    let mut deltas = Vec::new();
    let mut problems = Vec::new();
    for (c, ch) in rendered.channels.iter().enumerate() {
        let ref_rms: Vec<f64> = rms[c]
            .as_array()
            .ok_or("envelope rmsDb channel")?
            .iter()
            .map(|v| v.as_f64().unwrap_or(f64::NAN))
            .collect();
        if ref_rms.len() != ch.len().div_ceil(WINDOW_FRAMES) {
            return Err(format!(
                "envelope channel {c} has {} windows",
                ref_rms.len()
            ));
        }
        let d = channel_delta(
            ch,
            &ref_rms,
            peaks[c].as_f64().unwrap_or(f64::NAN),
            t.floor_db,
        );
        problems.extend(
            d.problems(t)
                .into_iter()
                .map(|p| format!("channel {c}: {p}")),
        );
        deltas.push(d);
    }
    if problems.is_empty() {
        Ok(deltas)
    } else {
        Err(problems.join("; "))
    }
}

/// Runs `f` on every item on all cores, keeping the order.
fn parallel<T: Sync, R: Send>(items: Vec<T>, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<R>>> =
        items.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..threads.min(items.len()) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    *slots[i].lock().unwrap() = Some(f(item));
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|m| m.into_inner().unwrap().expect("every item ran"))
        .collect()
}

#[test]
fn every_fixture_with_refs_has_its_jobs_and_envelopes() {
    let corpus = corpus();
    let kept: BTreeSet<&str> = KEPT_UNSUPPORTED.iter().map(|(n, _)| *n).collect();
    let expected: BTreeSet<String> = corpus
        .iter()
        .filter(|(n, (supported, kind))| {
            (*supported && kind != "external") || kept.contains(n.as_str())
        })
        .map(|(n, _)| n.clone())
        .collect();
    let cases = fixture_cases();
    let got: BTreeSet<String> = cases.iter().map(|c| c.fixture.clone()).collect();
    assert_eq!(
        got, expected,
        "fixture refs vs corpus.json: run `bun run engine:refs --write`"
    );
    for c in &cases {
        assert!(!c.jobs.is_empty(), "{}: no jobs", c.fixture);
        assert_eq!(
            c.jobs.len(),
            c.envelopes.len(),
            "{}: jobs vs envelopes",
            c.fixture
        );
        for (j, e) in c.jobs.iter().zip(&c.envelopes) {
            assert!(j.starts_with(&format!("fixture-{}--", c.fixture)), "{j}");
            assert_eq!(
                e.strip_suffix(".ref.json"),
                j.strip_suffix(".job.json"),
                "{}: the envelope is not the job's sibling",
                c.fixture
            );
            assert!(refs_dir().join(j).is_file(), "{j} is missing");
            assert!(refs_dir().join(e).is_file(), "{e} is missing");
        }
    }
}

#[test]
fn supported_fixtures_render_within_the_envelope_guard() {
    let corpus = corpus();
    let t = Tolerances::load();
    let chunks: Vec<(String, String)> = fixture_cases()
        .into_iter()
        .filter(|c| corpus.get(&c.fixture).is_some_and(|(s, _)| *s))
        .flat_map(|c| c.jobs.into_iter().zip(c.envelopes))
        .collect();
    assert!(
        chunks.len() >= 36,
        "only {} supported chunk jobs",
        chunks.len()
    );
    let results = parallel(chunks.clone(), |(j, e)| check_supported(j, e, t));
    let mut failures = Vec::new();
    for ((job, _), r) in chunks.iter().zip(results) {
        match r {
            Ok(deltas) => {
                let worst =
                    |f: fn(&ChannelDelta) -> f64| deltas.iter().map(f).fold(0.0f64, f64::max);
                println!(
                    "{job}: window mean {:.3} dB, p95 {:.3} dB, peak {:.3} dB ({} windows)",
                    worst(|d| d.mean_db),
                    worst(|d| d.p95_db),
                    worst(|d| d.peak_db.unwrap_or(0.0)),
                    deltas.iter().map(|d| d.windows).sum::<usize>()
                );
            }
            Err(e) => {
                println!("{job}: FAIL {e}");
                failures.push(format!("{job}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn supported_fixture_jobs_have_no_unsupported_signature() {
    let corpus = corpus();
    let mut checked = 0;
    for c in fixture_cases() {
        if !corpus.get(&c.fixture).is_some_and(|(s, _)| *s) {
            continue;
        }
        for file in &c.jobs {
            let sigs = unsupported_signatures(&load_job(file));
            assert!(sigs.is_empty(), "{file}: {sigs:?}");
            checked += 1;
        }
    }
    assert!(checked >= 29, "only {checked} supported chunk jobs");
}

#[test]
fn kept_unsupported_fixtures_are_refused_with_their_signatures() {
    let cases = fixture_cases();
    for (name, expected) in KEPT_UNSUPPORTED {
        let case = cases
            .iter()
            .find(|c| c.fixture == *name)
            .unwrap_or_else(|| panic!("{name} has no refs"));
        let mut want: Vec<(String, String)> = expected
            .iter()
            .map(|(s, k)| (s.to_string(), k.to_string()))
            .collect();
        want.sort();
        for file in &case.jobs {
            let job = load_job(file);
            let mut got: Vec<(String, String)> = unsupported_signatures(&job)
                .into_iter()
                .map(|u| (u.s, u.keys.join(",")))
                .collect();
            got.sort();
            got.dedup();
            assert_eq!(got, want, "{file}: unsupported signatures");
            let rendered = structural(file, &job).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert!(rendered.errors.is_empty(), "{file}: {:?}", rendered.errors);
            assert_eq!(
                rendered.unsupported,
                unsupported_signatures(&job),
                "{file}: the render reports the detection's signatures"
            );
            let mut reported: Vec<(String, String)> = rendered
                .unsupported
                .iter()
                .map(|u| (u.s.clone(), u.keys.join(",")))
                .collect();
            reported.sort();
            reported.dedup();
            assert_eq!(reported, want, "{file}: reported signatures");
            assert!(
                rendered
                    .channels
                    .iter()
                    .all(|c| c.iter().all(|v| *v == 0.0)),
                "{file}: an unsupported job must render silence"
            );
        }
    }
}

#[test]
fn rms_db_of_silence_full_scale_and_a_partial_window() {
    assert_eq!(rms_db(&[]), SILENCE_DB);
    assert_eq!(rms_db(&[0.0; 441]), SILENCE_DB);
    assert!(rms_db(&[1.0; 441]).abs() < 1e-12);
    assert!((rms_db(&[0.5, -0.5]) - 20.0 * 0.5f64.log10()).abs() < 1e-12);
    let mut x = vec![1.0f32; 441];
    x.extend([0.5f32; 10]);
    let w = window_rms_db(&x);
    assert_eq!(w.len(), 2);
    assert!((w[1] - 20.0 * 0.5f64.log10()).abs() < 1e-12);
}

#[test]
fn p95_is_the_nearest_rank() {
    assert_eq!(p95(&[]), None);
    assert_eq!(p95(&[3.0]), Some(3.0));
    let v: Vec<f64> = (1..=100).map(f64::from).rev().collect();
    assert_eq!(p95(&v), Some(95.0));
    let v: Vec<f64> = (1..=10).map(f64::from).collect();
    assert_eq!(p95(&v), Some(10.0));
}

#[test]
fn channel_delta_skips_windows_below_the_floor() {
    // window 0 at full scale (0 dB ref), window 1 below the floor in the reference
    let mut x = vec![0.5f32; 441];
    x.extend([0.001f32; 441]);
    let d = channel_delta(&x, &[0.0, -80.0], 1.0, -60.0);
    assert_eq!(d.windows, 1);
    let half = -20.0 * 0.5f64.log10();
    assert!((d.mean_db - half).abs() < 1e-9, "{d:?}");
    assert!((d.p95_db - half).abs() < 1e-9);
    assert!((d.peak_db.unwrap() - half).abs() < 1e-9);
    let t = Tolerances {
        floor_db: -60.0,
        window_mean_db: 1.0,
        window_p95_db: 3.0,
        peak_db: 1.0,
    };
    let p = d.problems(t);
    assert_eq!(p.len(), 3, "{p:?}");
    let loose = Tolerances {
        window_mean_db: 10.0,
        window_p95_db: 10.0,
        peak_db: 10.0,
        ..t
    };
    assert!(d.problems(loose).is_empty());
}

#[test]
fn channel_delta_of_silence_against_silence_is_empty() {
    let d = channel_delta(&[0.0; 882], &[SILENCE_DB, SILENCE_DB], 0.0, -60.0);
    assert_eq!(
        d,
        ChannelDelta {
            windows: 0,
            mean_db: 0.0,
            p95_db: 0.0,
            peak_db: None
        }
    );
    // the engine sounding where the reference is silent shows in the peak
    let d = channel_delta(&[0.5; 441], &[SILENCE_DB], 0.0, -60.0);
    assert_eq!(d.windows, 0);
    assert_eq!(d.peak_db, Some(20.0 * 0.5f64.log10() - SILENCE_DB));
}

#[test]
fn kept_unsupported_list_names_corpus_fixtures_marked_unsupported() {
    let corpus = corpus();
    for (name, sigs) in KEPT_UNSUPPORTED {
        let (supported, _) = corpus
            .get(*name)
            .unwrap_or_else(|| panic!("{name} is not in corpus.json"));
        assert!(!supported, "{name} is marked supported");
        assert!(!sigs.is_empty());
    }
}
