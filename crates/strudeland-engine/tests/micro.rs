//! The micro cases: every Chromium reference `tests/refs/<name>.job.json` + `<name>.wav` (written
//! by `bun run engine:refs --write` from `tests/render/engine-micro/<name>.js`) is rendered by the
//! engine and compared sample by sample (no alignment search) with the reference: the null
//! residual `20*log10(rms(a - b) / rms(a))` over all samples of all channels (a = Chromium) must
//! be below the case's `maxResidualDb` in `tests/refs/thresholds.json`. Frame and channel counts
//! must equal the reference's, and the engine's errors must be exactly the case's `expectErrors`
//! (none when absent). Every micro case has a threshold, refs and a source, and vice versa, and
//! no micro job has an unsupported signature.
//!
//! `thresholds.json` (schema 2) may mark a case `platform: "macos"` (judged only where
//! `platform_applies`; skipped elsewhere with a printed `SKIP`) or `floor: { runs: 8, reason }`
//! (a measured Chromium floor: `index.json` then carries the case's `floorDb`, and the threshold
//! must lie above it).

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::Value;
use strudeland_engine::controls::unsupported_signatures;
use strudeland_engine::wav::read_f32;
use strudeland_engine::{ErrorKind, render};

mod common;
use common::{refs_dir, repo_root, try_load_job};

fn sources_dir() -> PathBuf {
    repo_root().join("tests/render/engine-micro")
}

fn read_json(name: &str) -> Value {
    common::read_json(&refs_dir().join(name))
}

/// The micro cases of `index.json`, sorted.
fn indexed_cases() -> BTreeSet<String> {
    let index = read_json("index.json");
    index["cases"]
        .as_object()
        .expect("index.json has cases")
        .iter()
        .filter(|(_, c)| c["kind"] == "micro")
        .map(|(n, _)| n.clone())
        .collect()
}

fn threshold_cases() -> BTreeSet<String> {
    read_json("thresholds.json")["cases"]
        .as_object()
        .expect("thresholds.json has cases")
        .keys()
        .cloned()
        .collect()
}

fn source_cases() -> BTreeSet<String> {
    std::fs::read_dir(sources_dir())
        .expect("the micro-case sources exist")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter_map(|f| f.strip_suffix(".js").map(str::to_string))
        .collect()
}

/// `20*log10(rms(a - b) / rms(a))` over every sample of every channel, `a` the reference.
/// `-inf` when both are silent (or equal), `+inf` when only the reference is silent. `None` when
/// the shapes differ.
fn residual_db(a: &[Vec<f32>], b: &[Vec<f32>]) -> Option<f64> {
    if a.len() != b.len() || a.iter().zip(b).any(|(x, y)| x.len() != y.len()) {
        return None;
    }
    let (mut ref_sq, mut diff_sq) = (0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b) {
        for (&p, &q) in x.iter().zip(y) {
            let (p, q) = (f64::from(p), f64::from(q));
            ref_sq += p * p;
            diff_sq += (p - q) * (p - q);
        }
    }
    if diff_sq.is_nan() {
        return Some(f64::NAN);
    }
    Some(match (ref_sq == 0.0, diff_sq == 0.0) {
        (_, true) => f64::NEG_INFINITY,
        (true, false) => f64::INFINITY,
        // the sample counts cancel in the ratio of the two rms values
        (false, false) => 10.0 * (diff_sq / ref_sq).log10(),
    })
}

fn kind_name(k: ErrorKind) -> &'static str {
    match k {
        ErrorKind::Hap => "hap",
        ErrorKind::Ir => "ir",
        ErrorKind::Unsupported => "unsupported",
        ErrorKind::Other => "other",
    }
}

/// The evidence directories a threshold may cite.
const EVIDENCE_PREFIXES: [&str; 2] = [
    "plans/0010-rust-engine-core/evidence/",
    "plans/0011-engine-effects/evidence/",
];

/// Whether a case gated on platform `p` is judged here. Only `"macos"` exists (the compressor
/// cases call the platform's libm); any other value is a malformed threshold.
fn platform_applies(p: &str) -> bool {
    match p {
        "macos" => cfg!(target_os = "macos"),
        other => panic!("unknown platform {other:?} in thresholds.json"),
    }
}

/// Checks one case; `Ok(residual)` or the reasons it fails.
fn check_case(name: &str, threshold: &Value) -> Result<f64, String> {
    let max = threshold["maxResidualDb"]
        .as_f64()
        .ok_or("no numeric maxResidualDb")?;
    let job = try_load_job(&format!("{name}.job.json")).map_err(|e| format!("job: {e}"))?;
    let wav = std::fs::read(refs_dir().join(format!("{name}.wav"))).map_err(|e| e.to_string())?;
    let reference = read_f32(&wav).map_err(|e| format!("wav: {e}"))?;
    if reference.sample_rate != 44100 {
        return Err(format!("reference is {} Hz", reference.sample_rate));
    }
    let rendered = render(&job).map_err(|e| format!("render: {e}"))?;
    let (rc, ec) = (reference.channels.len(), rendered.channels.len());
    if rc != ec {
        return Err(format!("{ec} channels, the reference has {rc}"));
    }
    let (rf, ef) = (reference.channels[0].len(), rendered.channels[0].len());
    if rf != ef {
        return Err(format!("{ef} frames, the reference has {rf}"));
    }
    let mut expected: Vec<(i64, String)> = threshold["expectErrors"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|e| {
                    (
                        e["bar"].as_i64().unwrap_or(i64::MIN),
                        e["engineKind"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let mut got: Vec<(i64, String)> = rendered
        .errors
        .iter()
        .map(|e| (e.bar.unwrap_or(i64::MIN), kind_name(e.kind).to_string()))
        .collect();
    expected.sort();
    expected.dedup();
    got.sort();
    got.dedup();
    if got != expected {
        return Err(format!(
            "errors (bar, kind) {got:?}, expected {expected:?}: {:?}",
            rendered.errors
        ));
    }
    let r = residual_db(&reference.channels, &rendered.channels).ok_or("shape mismatch")?;
    if r.is_nan() || r >= max {
        return Err(format!("residual {r:.2} dB, max {max} dB"));
    }
    Ok(r)
}

#[test]
fn every_micro_case_has_refs_a_threshold_and_a_source() {
    let (indexed, thresholds, sources) = (indexed_cases(), threshold_cases(), source_cases());
    assert!(!sources.is_empty(), "no micro cases");
    assert_eq!(
        indexed, sources,
        "micro refs vs tests/render/engine-micro: run `bun run engine:refs --write`"
    );
    assert_eq!(
        thresholds, sources,
        "thresholds.json cases vs tests/render/engine-micro: every case needs a threshold"
    );
}

#[test]
fn micro_jobs_have_no_unsupported_signature() {
    let cases = indexed_cases();
    assert!(!cases.is_empty(), "no micro cases");
    for name in &cases {
        let job = common::load_job(&format!("{name}.job.json"));
        let sigs = unsupported_signatures(&job);
        assert!(sigs.is_empty(), "{name}: {sigs:?}");
    }
}

#[test]
fn micro_cases_are_within_their_null_thresholds() {
    let thresholds = read_json("thresholds.json");
    let cases = thresholds["cases"].as_object().expect("cases");
    assert!(!cases.is_empty(), "no thresholds");
    let mut failures = Vec::new();
    for (name, t) in cases {
        if let Some(p) = t.get("platform").and_then(Value::as_str)
            && !platform_applies(p)
        {
            println!("SKIP {name} (platform {p})");
            continue;
        }
        match check_case(name, t) {
            Ok(r) => println!("{name}: residual {r:.2} dB (max {} dB)", t["maxResidualDb"]),
            Err(e) => {
                println!("{name}: FAIL {e}");
                failures.push(format!("{name}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_threshold_is_well_formed() {
    let thresholds = read_json("thresholds.json");
    assert_eq!(thresholds["schema"], 2);
    for (name, t) in thresholds["cases"].as_object().unwrap() {
        let max = t["maxResidualDb"].as_f64();
        assert!(
            max.is_some_and(|m| m.is_finite() && m < 0.0),
            "{name}: maxResidualDb"
        );
        let ev = t["evidence"].as_str().unwrap_or("");
        assert!(
            EVIDENCE_PREFIXES.iter().any(|p| ev.starts_with(p)),
            "{name}: evidence {ev:?}"
        );
        assert!(
            repo_root().join(ev).exists(),
            "{name}: evidence {ev} does not exist"
        );
        if let Some(p) = t.get("platform") {
            assert_eq!(p, "macos", "{name}: platform is \"macos\" or absent");
        }
        for key in t.as_object().unwrap().keys() {
            assert!(
                [
                    "maxResidualDb",
                    "evidence",
                    "expectErrors",
                    "floor",
                    "platform"
                ]
                .contains(&key.as_str()),
                "{name}: unknown key {key}"
            );
        }
        if let Some(errors) = t.get("expectErrors") {
            let errors = errors.as_array().expect("expectErrors is an array");
            assert!(!errors.is_empty(), "{name}: empty expectErrors");
            for e in errors {
                assert!(e["bar"].as_i64().is_some(), "{name}: expectErrors bar");
                assert!(
                    ["hap", "ir", "unsupported", "other"]
                        .contains(&e["engineKind"].as_str().unwrap_or("")),
                    "{name}: expectErrors engineKind"
                );
            }
        }
    }
    let f = &thresholds["fixtures"];
    for key in ["floorDb", "windowMeanDb", "windowP95Db", "peakDb"] {
        assert!(f[key].as_f64().is_some(), "fixtures.{key}");
    }
}

/// `floor` in `thresholds.json` iff `floorDb` in `index.json`; `runs` is 8, the reason names its
/// sounding inputs, and the threshold lies above the floor.
fn floor_problems(thresholds: &Value, index: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    let cases = thresholds["cases"].as_object().cloned().unwrap_or_default();
    let indexed = index["cases"].as_object().cloned().unwrap_or_default();
    for (name, t) in &cases {
        let floor_db = indexed.get(name).and_then(|c| c.get("floorDb"));
        match (t.get("floor"), floor_db) {
            (None, None) => {}
            (Some(_), None) => problems.push(format!("{name}: floor declared, no floorDb")),
            (None, Some(_)) => problems.push(format!("{name}: floorDb without a floor")),
            (Some(f), Some(db)) => {
                if f["runs"] != 8 {
                    problems.push(format!("{name}: floor.runs {}, expected 8", f["runs"]));
                }
                let reason = f["reason"].as_str().unwrap_or("");
                let n = reason
                    .strip_suffix(" sounding inputs)")
                    .and_then(|r| r.rsplit_once('('))
                    .and_then(|(_, n)| n.parse::<u32>().ok());
                if n.is_none_or(|n| n < 3) {
                    problems.push(format!("{name}: floor.reason {reason:?}"));
                }
                match (db.as_f64(), t["maxResidualDb"].as_f64()) {
                    (Some(db), Some(max)) if max > db => {}
                    _ => problems.push(format!("{name}: maxResidualDb must exceed floorDb {db}")),
                }
            }
        }
    }
    for (name, c) in &indexed {
        if c.get("floorDb").is_some() && !cases.contains_key(name) {
            problems.push(format!("{name}: floorDb without a threshold"));
        }
    }
    problems
}

#[test]
fn floor_cases_are_declared() {
    let thresholds = read_json("thresholds.json");
    let index = read_json("index.json");
    let problems = floor_problems(&thresholds, &index);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    // The declared floor cases: the vowel's formant sum and the modulated carrier frequency.
    let floors: BTreeSet<String> = thresholds["cases"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, t)| t.get("floor").is_some())
        .map(|(n, _)| n.clone())
        .collect();
    assert_eq!(
        floors,
        BTreeSet::from(["mod-lfo-note".to_string(), "vowel-formants".to_string()])
    );
    // The platform-gated cases are exactly the compressor ones (they call the platform libm).
    let gated: BTreeSet<String> = thresholds["cases"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, t)| t.get("platform").is_some())
        .map(|(n, _)| n.clone())
        .collect();
    let compressor: BTreeSet<String> = ["fresh", "params", "stereo", "warmup"]
        .iter()
        .map(|s| format!("compressor-{s}"))
        .collect();
    assert_eq!(gated, compressor);
    // The bus case is exact: the bus graph has no nondeterministic junction (one feeder).
    assert_eq!(
        thresholds["cases"]["bus-sound"],
        serde_json::json!({
            "maxResidualDb": -140,
            "evidence": "plans/0011-engine-effects/evidence/engine-null-micro-buses.json"
        })
    );
    // The one case that expects errors from this stage: the unknown vowel's hap error.
    assert_eq!(
        thresholds["cases"]["vowel-unknown"]["expectErrors"],
        serde_json::json!([{ "bar": 0, "engineKind": "hap" }])
    );
}

#[test]
fn the_floor_check_catches_each_rule() {
    use serde_json::json;
    let t = |case: Value| json!({ "cases": { "a": case } });
    let ok = json!({ "maxResidualDb": -90, "evidence": "e",
        "floor": { "runs": 8, "reason": "vowel: five formants (5 sounding inputs)" } });
    let idx = |db: Option<f64>| match db {
        Some(db) => json!({ "cases": { "a": { "kind": "micro", "floorDb": db } } }),
        None => json!({ "cases": { "a": { "kind": "micro" } } }),
    };
    assert!(floor_problems(&t(ok.clone()), &idx(Some(-120.0))).is_empty());
    assert_eq!(floor_problems(&t(ok.clone()), &idx(None)).len(), 1);
    let plain = json!({ "maxResidualDb": -140, "evidence": "e" });
    assert!(floor_problems(&t(plain.clone()), &idx(None)).is_empty());
    assert_eq!(floor_problems(&t(plain), &idx(Some(-120.0))).len(), 1);
    let mut bad = ok.clone();
    bad["floor"]["runs"] = json!(4);
    assert_eq!(floor_problems(&t(bad), &idx(Some(-120.0))).len(), 1);
    let mut bad = ok.clone();
    bad["floor"]["reason"] = json!("two inputs (2 sounding inputs)");
    assert_eq!(floor_problems(&t(bad), &idx(Some(-120.0))).len(), 1);
    assert_eq!(
        floor_problems(&t(ok), &idx(Some(-80.0))).len(),
        1,
        "threshold below the floor"
    );
}

#[test]
fn platform_gate_is_macos_only() {
    assert_eq!(platform_applies("macos"), cfg!(target_os = "macos"));
    assert!(std::panic::catch_unwind(|| platform_applies("linux")).is_err());
}

#[test]
fn residual_of_equal_buffers_is_minus_infinity() {
    let a = vec![vec![0.5f32, -0.25, 0.0], vec![0.1, 0.2, 0.3]];
    assert_eq!(residual_db(&a, &a), Some(f64::NEG_INFINITY));
    let silent = vec![vec![0.0f32; 4]; 2];
    assert_eq!(residual_db(&silent, &silent), Some(f64::NEG_INFINITY));
}

#[test]
fn residual_is_the_db_ratio_over_all_channels() {
    let a = vec![vec![1.0f32; 100], vec![0.0; 100]];
    let mut b = a.clone();
    for v in &mut b[1] {
        *v = 0.1;
    }
    // diff power 0.1^2 * 100 against reference power 1 * 100 (channel 1 of a is silent); 0.1 is
    // an f32, so -20 dB holds to f32 precision
    let r = residual_db(&a, &b).unwrap();
    assert!((r - (-20.0)).abs() < 1e-5, "{r}");
    let half: Vec<Vec<f32>> = a
        .iter()
        .map(|c| c.iter().map(|v| v * 0.5).collect())
        .collect();
    assert!((residual_db(&a, &half).unwrap() - 20.0 * 0.5f64.log10()).abs() < 1e-9);
}

#[test]
fn residual_against_a_silent_reference_is_infinite() {
    let silent = vec![vec![0.0f32; 4]];
    assert_eq!(
        residual_db(&silent, &[vec![0.0, 1e-9, 0.0, 0.0]]),
        Some(f64::INFINITY)
    );
}

#[test]
fn residual_needs_equal_shapes_and_propagates_nan() {
    let a = vec![vec![1.0f32; 4]];
    assert_eq!(residual_db(&a, &[vec![1.0; 3]]), None);
    assert_eq!(residual_db(&a, &[vec![1.0; 4], vec![1.0; 4]]), None);
    assert!(
        residual_db(&a, &[vec![1.0, f32::NAN, 1.0, 1.0]])
            .unwrap()
            .is_nan()
    );
}
