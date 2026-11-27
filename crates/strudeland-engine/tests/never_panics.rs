//! The library never panics on job input: malformed jobs are `JobError`s, extreme control values
//! render finite audio or report `RenderError`s. Deterministic (a fixed xorshift generator); every
//! stage that implements a key adds its extremes here.

use serde_json::{Value, json};
use strudeland_engine::{ALLOWED_KEYS, EngineJob, ErrorKind, allowed_sounds, render};

mod common;
use common::{PauseHap, base_job};

fn base() -> Value {
    base_job(
        8820,
        2,
        json!([{ "s": "sine", "note": 60 }]),
        [
            PauseHap {
                suspend_sec: 0.0,
                t: 0.0,
                duration: 0.05,
            },
            PauseHap {
                suspend_sec: 0.1,
                t: 0.12,
                duration: 0.05,
            },
        ],
    )
}

/// Every path to a scalar or container inside `v`.
fn paths(v: &Value, prefix: &mut Vec<Value>, out: &mut Vec<Vec<Value>>) {
    out.push(prefix.clone());
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                prefix.push(json!(k));
                paths(x, prefix, out);
                prefix.pop();
            }
        }
        Value::Array(xs) => {
            for (i, x) in xs.iter().enumerate() {
                prefix.push(json!(i));
                paths(x, prefix, out);
                prefix.pop();
            }
        }
        _ => {}
    }
}

fn at<'a>(v: &'a mut Value, path: &[Value]) -> &'a mut Value {
    path.iter().fold(v, |v, p| match p {
        Value::String(k) => &mut v[k.as_str()],
        Value::Number(i) => &mut v[i.as_u64().unwrap() as usize],
        _ => unreachable!(),
    })
}

fn remove(v: &mut Value, path: &[Value]) {
    let (last, parent) = path.split_last().unwrap();
    match (at(v, parent), last) {
        (Value::Object(m), Value::String(k)) => {
            m.remove(k);
        }
        (Value::Array(xs), Value::Number(i)) => {
            xs.remove(i.as_u64().unwrap() as usize);
        }
        _ => {}
    }
}

/// Parses, validates and (for small valid jobs) renders; asserts finite output.
fn exercise(text: &str) -> Option<usize> {
    let job = EngineJob::from_json(text).ok()?;
    if job.frames > 200_000 || job.channels.count > 64 {
        return Some(0);
    }
    let r = render(&job).expect("a validated job renders");
    assert_eq!(r.channels.len(), job.channels.count as usize);
    for c in &r.channels {
        assert_eq!(c.len() as u64, job.frames);
        assert!(
            c.iter().all(|x| x.is_finite()),
            "non-finite sample for {text}"
        );
    }
    Some(r.errors.len() + r.unsupported.len())
}

const REPLACEMENTS: &[&str] = &[
    "null",
    "true",
    "\"x\"",
    "[]",
    "{}",
    "-1",
    "0",
    "1.5",
    "1e308",
    "-1e308",
    "4294967296",
    "-2147483649",
    "{\"$num\":\"NaN\"}",
    "{\"$num\":\"-0\"}",
    "{\"$unsupported\":\"function\"}",
    "[1,2,3]",
    "{\"$set\":[]}",
    "{\"$set\":[1,{\"$set\":[]}]}",
    "{\"$set\":1}",
    "{\"$set\":[],\"x\":1}",
];

#[test]
fn malformed_jobs_never_panic() {
    let base = base();
    let mut all = Vec::new();
    paths(&base, &mut Vec::new(), &mut all);
    let mut rejected = 0;
    for path in all.iter().filter(|p| !p.is_empty()) {
        let mut v = base.clone();
        remove(&mut v, path);
        if exercise(&v.to_string()).is_none() {
            rejected += 1;
        }
        for r in REPLACEMENTS {
            let mut v = base.clone();
            *at(&mut v, path) = serde_json::from_str(r).unwrap();
            if exercise(&v.to_string()).is_none() {
                rejected += 1;
            }
        }
    }
    assert!(rejected > 100, "most mutations are rejected ({rejected})");
    for text in [
        "",
        "null",
        "[]",
        "{}",
        "1e999",
        "{\"schema\":1e999}",
        "\u{feff}{}",
        "[[[[[[[[[[[[",
    ] {
        assert!(EngineJob::from_json(text).is_err());
    }
    let mut v = base.clone();
    v["pauses"][0]["haps"] = json!([]);
    v["pauses"][1]["haps"] = json!([]);
    v["values"] = json!([]);
    assert_eq!(exercise(&v.to_string()), Some(0));
    let mut v = base.clone();
    v["pauses"][0]["haps"][0]["v"] = json!(1);
    assert_eq!(exercise(&v.to_string()), None, "value index out of range");
    // A huge channel count with no frames was a multi-gigabyte allocation and an abort.
    for count in [65536u64, u32::MAX as u64] {
        let mut v = base.clone();
        v["frames"] = json!(0);
        v["channels"]["count"] = json!(count);
        assert_eq!(
            exercise(&v.to_string()),
            None,
            "{count} channels is rejected"
        );
    }
    let mut v = base.clone();
    v["frames"] = json!(0);
    v["pauses"] = json!([{ "suspendSec": 0, "haps": [] }]);
    v["channels"]["count"] = json!(65535);
    let job = EngineJob::from_json(&v.to_string()).expect("65535 channels is valid");
    let r = render(&job).expect("renders");
    assert_eq!(r.channels.len(), 65535);
    let mut v = base.clone();
    v["pauses"][0]["haps"] = Value::Array(vec![v["pauses"][0]["haps"][0].clone(); 2000]);
    assert!(exercise(&v.to_string()).is_some(), "many haps at once");
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() % xs.len() as u64) as usize]
    }
}

const KEYS: &[&str] = &[
    "note",
    "freq",
    "octave",
    "n",
    "gain",
    "velocity",
    "attack",
    "decay",
    "sustain",
    "release",
    "duration",
    "clip",
    "bank",
    "s",
    "resonance",
    "cutoff",
    "hcutoff",
    "bandf",
    "room",
    "roomsize",
    "delaytime",
    "delay",
    "delayfeedback",
    "partials",
    "fmi",
    "fmh",
    "pan",
    "postgain",
    "orbit",
    "phases",
    "vib",
    "vibmod",
    "penv",
    "pattack",
    "pdecay",
    "psustain",
    "prelease",
    "pcurve",
    "panchor",
    "detune",
    "hresonance",
    "bandq",
    "lpenv",
    "hpenv",
    "bpenv",
    "lpattack",
    "lprelease",
    "hpdecay",
    "bpsustain",
    "fanchor",
    "ftype",
    "dry",
    "noise",
    "density",
    "fmi2",
    "fmi11",
    "fmh2",
    "fmwave",
    "fmwave2",
    "fmattack",
    "fmenv",
    "fmrelease2",
    "channels",
    "delaysync",
    "tremolo",
    "tremolodepth",
    "tremoloshape",
    "tremolosync",
    "lprate",
    "lpdepth",
    "hpsync",
    "bpshape",
    "pwrate",
    "pwsweep",
    "compressor",
    "compressorRatio",
    "compressorRelease",
    "vowel",
    "phaserrate",
    "phaserdepth",
    "phasersweep",
    "FXrelease",
    "duckorbit",
    "duckonset",
    "duckattack",
    "duckdepth",
    "bus",
    "busgain",
];

fn extreme_values() -> Vec<Value> {
    [
        "0",
        "-0.0001",
        "1e-300",
        "1e-9",
        "1e9",
        "1e30",
        "1e300",
        "-1e300",
        "-1",
        "1e6",
        "\"\"",
        "\"abc\"",
        "\"c4\"",
        "\"c-\"",
        "\"0x10\"",
        "null",
        "true",
        "[]",
        "[1e9,-1e9]",
        "{}",
        "{\"$num\":\"NaN\"}",
        "{\"$num\":\"Infinity\"}",
        "{\"$num\":\"-Infinity\"}",
        "{\"$num\":\"-0\"}",
        "{\"$set\":[]}",
        "{\"$set\":[1,\"a\"]}",
    ]
    .iter()
    .map(|t| serde_json::from_str(t).unwrap())
    .collect()
}

/// The hap `(t, duration)` pairs most sweeps render each case at.
const DEFAULT_TIMES: &[(f64, f64)] = &[(0.0, 0.05), (0.013, 0.0), (0.0, 0.2), (0.1, 5e-324)];

/// Renders every case at every `(t, duration)` of `times`; none may panic.
fn sweep(cases: &[Value], times: &[(f64, f64)]) {
    for c in cases {
        for &(t, d) in times {
            assert!(
                exercise(&with_value(c.clone(), t, d)).is_some(),
                "{c} t={t} d={d}"
            );
        }
    }
}

fn with_value(map: Value, t: f64, duration: f64) -> String {
    let mut v = base();
    v["values"] = json!([map]);
    v["pauses"][0]["haps"][0]["t"] = json!(t);
    v["pauses"][0]["haps"][0]["duration"] = json!(duration);
    v.to_string()
}

#[test]
fn extreme_control_values_render_finite_audio() {
    let values = extreme_values();
    for key in KEYS {
        for x in &values {
            for s in ["sine", "sawtooth", "user"] {
                let mut map = json!({ "s": s, "note": 60, "partials": [1, 0.5] });
                if *key == "partials" || s == "sine" {
                    map.as_object_mut().unwrap().remove("partials");
                }
                map[*key] = x.clone();
                assert!(
                    exercise(&with_value(map, 0.0, 0.05)).is_some(),
                    "{s} {key}={x}"
                );
            }
        }
    }
    for sound in [
        "sine", "sawtooth", "square", "triangle", "user", "one", "white", "pink", "brown",
        "crackle", "~",
    ] {
        assert!(exercise(&with_value(json!({ "s": sound }), 0.0, 0.05)).is_some());
    }
    // Extreme hap times and durations.
    for (t, d) in [
        (0.0, 0.0),
        (0.0, 1e300),
        (1e300, 1.0),
        (0.19, 0.0),
        (0.0, 5e-324),
    ] {
        assert!(
            exercise(&with_value(json!({ "s": "sine" }), t, d)).is_some(),
            "t={t} d={d}"
        );
    }
    // Random combinations.
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..400 {
        let mut map = json!({
            "s": rng.pick(&["sine", "one", "pink", "triangle", "sawtooth", "user", "crackle"])
        });
        for _ in 0..4 {
            map[*rng.pick(KEYS)] = rng.pick(&values).clone();
        }
        let t = *rng.pick(&[0.0, 0.01, 0.15, 1e9]);
        let d = *rng.pick(&[0.0, 0.05, 1e9]);
        assert!(exercise(&with_value(map.clone(), t, d)).is_some(), "{map}");
    }
}

#[test]
fn extreme_envelopes_render_finite_audio() {
    let nan = json!({ "$num": "NaN" });
    let inf = json!({ "$num": "Infinity" });
    let cases = [
        json!({ "s": "sine", "attack": 1e9 }),
        json!({ "s": "one", "attack": 1e9, "decay": 1e9, "sustain": 1e9, "release": 1e9 }),
        json!({ "s": "sine", "attack": nan, "decay": nan, "sustain": nan, "release": nan }),
        json!({ "s": "sine", "attack": nan }),
        json!({ "s": "one", "release": inf }),
        json!({ "s": "one", "attack": inf, "sustain": -1e30 }),
        json!({ "s": "one", "attack": 1e-300, "decay": 1e-300, "sustain": 0.5 }),
        json!({ "s": "one", "attack": 0, "decay": 0, "sustain": 0, "release": 0 }),
        json!({ "s": "one", "sustain": 1e300 }),
        json!({ "s": "one", "sustain": -1e300 }),
    ];
    sweep(
        &cases,
        &[(0.0, 0.05), (0.013, 0.0), (0.0, 1e9), (0.1, 5e-324)],
    );
}

#[test]
fn extreme_oscillator_controls_render_finite_audio() {
    let nan = json!({ "$num": "NaN" });
    let inf = json!({ "$num": "Infinity" });
    let cases = [
        json!({ "s": "sine", "freq": 1e-300 }),
        json!({ "s": "sawtooth", "freq": 5e-324 }),
        json!({ "s": "square", "freq": 1e30 }),
        json!({ "s": "triangle", "freq": -1e30 }),
        json!({ "s": "sine", "freq": -22050 }),
        json!({ "s": "sawtooth", "partials": 1e15 }),
        json!({ "s": "sawtooth", "partials": -5 }),
        json!({ "s": "user", "partials": [1e38, -1e38] }),
        json!({ "s": "user", "partials": [1, 1], "phases": [nan, inf] }),
        json!({ "s": "square", "n": 100000, "phases": "0.25" }),
        json!({ "s": "sine", "vib": 1e9, "vibmod": 1e9 }),
        json!({ "s": "sine", "vib": 1e-9, "vibmod": -1e9 }),
        json!({ "s": "sawtooth", "vib": 20000, "vibmod": 1e30 }),
        json!({ "s": "sine", "penv": 1e9, "pattack": 1e-9 }),
        json!({ "s": "triangle", "penv": -1e9, "pcurve": 1, "panchor": 0.5 }),
        json!({ "s": "sine", "penv": 1e30, "pcurve": 1 }),
        json!({ "s": "sawtooth", "penv": 12, "pcurve": 1, "pattack": 0, "pdecay": 0, "prelease": 0 }),
        json!({ "s": "sine", "penv": nan, "panchor": inf }),
        json!({ "s": "user", "partials": [1], "vib": 8, "penv": 24, "pcurve": 0 }),
    ];
    sweep(&cases, DEFAULT_TIMES);
}

#[test]
fn extreme_filter_pan_and_level_controls_render_finite_audio() {
    let nan = json!({ "$num": "NaN" });
    let inf = json!({ "$num": "Infinity" });
    let cases = [
        json!({ "s": "sawtooth", "cutoff": 1000, "resonance": 1e6 }),
        json!({ "s": "sawtooth", "hcutoff": 1000, "hresonance": 1e6 }),
        json!({ "s": "sawtooth", "bandf": 1000, "bandq": 1e6 }),
        json!({ "s": "sawtooth", "bandf": 1000, "bandq": 1e-40 }),
        json!({ "s": "sawtooth", "cutoff": 1000, "resonance": -1e6 }),
        json!({ "s": "sawtooth", "hcutoff": 500, "hresonance": -3e38 }),
        json!({ "s": "sawtooth", "cutoff": 1e9 }),
        json!({ "s": "sawtooth", "cutoff": 0 }),
        json!({ "s": "sawtooth", "hcutoff": 0, "bandf": 0 }),
        json!({ "s": "sawtooth", "hcutoff": 30000, "bandf": 22050 }),
        json!({ "s": "sine", "gain": 1e30 }),
        json!({ "s": "sine", "gain": 1e30, "cutoff": 2000, "resonance": 40 }),
        json!({ "s": "sine", "gain": 1e30, "postgain": 1e30 }),
        json!({ "s": "sine", "postgain": 3e38, "dry": 3e38, "gain": 3e38 }),
        json!({ "s": "sine", "pan": 1e9 }),
        json!({ "s": "sine", "pan": -1e9 }),
        json!({ "s": "sine", "pan": nan }),
        json!({ "s": "sine", "pan": inf }),
        json!({ "s": "sawtooth", "cutoff": 300, "lpenv": 1e9 }),
        json!({ "s": "sawtooth", "cutoff": 300, "lpenv": -1e9, "fanchor": 1e9 }),
        json!({ "s": "sawtooth", "cutoff": 300, "lpenv": nan, "fanchor": inf }),
        json!({ "s": "sawtooth", "cutoff": 300, "lpenv": 4, "lpattack": 1e-300, "lpdecay": 0,
                "lpsustain": -1e30, "lprelease": 1e300 }),
        json!({ "s": "sawtooth", "hcutoff": 1e-300, "hpenv": 1000 }),
        json!({ "s": "sawtooth", "bandf": 5e-324, "bpenv": -1000, "ftype": "24db" }),
        json!({ "s": "sawtooth", "cutoff": 20, "resonance": 50, "ftype": 2, "lpenv": 8 }),
        json!({ "s": "one", "cutoff": 100, "resonance": 60, "release": 0.01 }),
    ];
    sweep(&cases, DEFAULT_TIMES);
}

#[test]
fn extreme_noise_and_fm_controls_render_finite_audio() {
    let nan = json!({ "$num": "NaN" });
    let inf = json!({ "$num": "Infinity" });
    let cases = [
        json!({ "s": "sine", "fmi": 1e9 }),
        json!({ "s": "sine", "fmi": -1e9, "fmh": 1e9 }),
        json!({ "s": "sine", "fmi": 1e9, "fmi2": 1e9, "fmi3": 1e9, "fmh3": 1e-30 }),
        json!({ "s": "sawtooth", "fmi": 1e38, "fmh": 1e-38 }),
        json!({ "s": "sine", "fmi": 3e38, "fmh": 1e9 }),
        json!({ "s": "sine", "fmi": 1, "fmi11": 1e9 }),
        json!({ "s": "sine", "fmi": 1, "fmi2": 1, "fmi12": 1e9, "fmi88": 5 }),
        json!({ "s": "sine", "fmi": 1e9, "fmwave": "crackle" }),
        json!({ "s": "sine", "fmi": 1, "fmwave": "custom" }),
        json!({ "s": "sine", "fmi": nan, "fmi2": inf }),
        json!({ "s": "sine", "fmi": 2, "fmattack": inf, "fmenv": "lin" }),
        json!({ "s": "sine", "fmi": 2, "fmattack": 0, "fmdecay": 0, "fmsustain": -1e30 }),
        json!({ "s": "sine", "fmi": 2, "fmsustain": nan, "fmrelease": 1e300 }),
        json!({ "s": "crackle", "density": 1e300 }),
        json!({ "s": "crackle", "density": -1e300 }),
        json!({ "s": "crackle", "density": nan }),
        json!({ "s": "crackle", "density": inf }),
        json!({ "s": "crackle", "density": 5e-324 }),
        json!({ "s": "sawtooth", "noise": 1e9 }),
        json!({ "s": "sawtooth", "noise": -1e9 }),
        json!({ "s": "sawtooth", "noise": 1e300 }),
        json!({ "s": "triangle", "noise": -3, "fmi": 4, "fmwave": "pink" }),
        json!({ "s": "sine", "noise": inf }),
        json!({ "s": "white", "gain": 1e30, "postgain": 1e30 }),
    ];
    sweep(&cases, DEFAULT_TIMES);
}

#[test]
fn extreme_delay_controls_render_finite_audio() {
    let nan = json!({ "$num": "NaN" });
    let inf = json!({ "$num": "Infinity" });
    let cases = [
        json!({ "s": "sine", "delay": 0.5, "delaytime": 1e9 }),
        json!({ "s": "sine", "delay": 0.5, "delaytime": -1 }),
        json!({ "s": "sine", "delay": 0.5, "delaytime": nan }),
        json!({ "s": "sine", "delay": 0.5, "delaytime": inf }),
        json!({ "s": "sine", "delay": 0.5, "delaytime": 1e39 }),
        json!({ "s": "sine", "delay": 0.5, "delaytime": 5e-324 }),
        json!({ "s": "sine", "delay": 0.5, "delaytime": "0.01" }),
        json!({ "s": "sine", "delay": 0.5, "delaysync": 1e300 }),
        json!({ "s": "sine", "delay": 0.5, "delayfeedback": 5 }),
        json!({ "s": "sine", "delay": 0.5, "delayfeedback": 1e300 }),
        json!({ "s": "sine", "delay": 0.5, "delayfeedback": nan }),
        json!({ "s": "sine", "delay": 0.5, "delayfeedback": -1 }),
        json!({ "s": "sine", "delay": 1e30, "delaytime": 0.001, "delayfeedback": 0.98 }),
        json!({ "s": "one", "delay": 3e38, "delaytime": 0, "delayfeedback": 0.98, "gain": 1e30 }),
        json!({ "s": "sine", "delay": inf }),
        json!({ "s": "sine", "delay": 0.5, "delaytime": 0.001, "pan": 0.9 }),
        json!({ "s": "white", "delay": 1, "delaytime": 0.002, "delayfeedback": 1, "pan": 0.1 }),
    ];
    sweep(&cases, &[(0.0, 0.05), (0.013, 0.0), (0.0, 0.2)]);
    // A mono delay on orbit 1 that a stereo hap joins in the next pause; the guard changes
    // time and feedback between the pauses; a long render keeps the feedback ringing.
    let mut v = base();
    v["frames"] = json!(88200);
    v["values"] = json!([
        { "s": "sawtooth", "note": 48, "delay": 0.8, "delaytime": 0.3, "delayfeedback": 0.98 },
        { "s": "sine", "note": 72, "pan": 0.8, "delay": 1, "delaytime": 1.5, "delayfeedback": 2 }
    ]);
    v["pauses"][1]["haps"][0]["v"] = json!(1);
    assert_eq!(exercise(&v.to_string()), Some(0));
}

#[test]
fn extreme_reverb_controls_render_finite_audio() {
    let values = extreme_values();
    // Every room key over the extreme values, on top of a reverb send (roomsize short so the
    // IRs stay small, except where roomsize itself is the extreme).
    for key in ["room", "roomsize", "roomfade", "roomlp", "roomdim"] {
        for x in &values {
            let mut map = json!({ "s": "sine", "room": 0.5, "roomsize": 0.05 });
            map[key] = x.clone();
            assert!(exercise(&with_value(map, 0.0, 0.05)).is_some(), "{key}={x}");
        }
    }
    let nan = json!({ "$num": "NaN" });
    let inf = json!({ "$num": "Infinity" });
    let ninf = json!({ "$num": "-Infinity" });
    let cases = [
        json!({ "s": "sine", "room": 1e9 }),
        json!({ "s": "sine", "room": 1e39 }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 0 }),
        json!({ "s": "sine", "room": 0.5, "roomsize": -3 }),
        json!({ "s": "sine", "room": 0.5, "roomsize": nan }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 1e9 }),
        json!({ "s": "sine", "room": 0.5, "roomsize": inf }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 5e-324 }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 0.01, "roomfade": 1e300 }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 0.01, "roomfade": inf }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 0.01, "roomfade": -1e300 }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 0.01, "roomlp": ninf }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 0.01, "roomlp": 1e300, "roomdim": -1e300 }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 0.01, "roomlp": 1e-300, "roomdim": 1e-300 }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 0.01, "roomdim": ninf }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 0.01, "roomdim": nan }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 0, "delay": 0.5, "delaytime": 0.01 }),
        json!({ "s": "sine", "room": 0.5, "roomsize": 0.01, "dry": 1e40, "delay": 0.5 }),
    ];
    sweep(&cases, &[(0.0, 0.05), (0.013, 0.0)]);
    // Requests on one orbit across pauses: a good IR, a failing one, a changed one.
    let mut v = base();
    v["values"] = json!([
        { "s": "sawtooth", "room": 0.9, "roomsize": 0.02, "roomlp": 300, "roomdim": 20000 },
        { "s": "sine", "room": 0.9, "roomsize": 0, "delay": 1, "delaytime": 0.05 }
    ]);
    v["pauses"][1]["haps"][0]["v"] = json!(1);
    assert!(exercise(&v.to_string()).is_some());
}

#[test]
fn extreme_shaper_and_transient_controls_never_panic() {
    let nan = json!({ "$num": "NaN" });
    let inf = json!({ "$num": "Infinity" });
    let cases = [
        json!({ "s": "sawtooth", "crush": 0 }),
        json!({ "s": "sawtooth", "crush": 1e30 }),
        json!({ "s": "sawtooth", "crush": -1e30 }),
        json!({ "s": "sawtooth", "crush": 3.5 }),
        json!({ "s": "sawtooth", "coarse": 1e9 }),
        json!({ "s": "sawtooth", "coarse": -1e9 }),
        json!({ "s": "sawtooth", "coarse": 3e38 }),
        json!({ "s": "sawtooth", "shape": -1 }),
        json!({ "s": "one", "shape": -1 }),
        json!({ "s": "sawtooth", "shape": 1e30, "shapevol": 1e30 }),
        json!({ "s": "sawtooth", "shape": -1e30, "shapevol": -1e30 }),
        json!({ "s": "sawtooth", "distort": 1e9 }),
        json!({ "s": "sawtooth", "distort": 3e38, "distorttype": 4 }),
        json!({ "s": "sawtooth", "distort": -1e30, "distorttype": 8 }),
        json!({ "s": "sawtooth", "distort": 709, "distorttype": "chebyshev" }),
        json!({ "s": "sawtooth", "distort": 50, "distorttype": "diode", "distortvol": 1e30 }),
        json!({ "s": "sawtooth", "distort": 50, "distorttype": "asym" }),
        json!({ "s": "sawtooth", "distort": 2, "distorttype": -1 }),
        json!({ "s": "sawtooth", "distort": 2, "distorttype": nan }),
        json!({ "s": "sawtooth", "distort": 2, "distorttype": inf }),
        json!({ "s": "sawtooth", "distort": 2, "distorttype": 1e300 }),
        json!({ "s": "sawtooth", "distort": 2, "distorttype": "nope" }),
        json!({ "s": "sawtooth", "transient": 1e30 }),
        json!({ "s": "sawtooth", "transient": -1e30, "transsustain": 1e30 }),
        json!({ "s": "sawtooth", "transient": nan, "transsustain": -1e30 }),
        json!({ "s": "one", "transient": 1, "transsustain": -1, "gain": 1e30 }),
        json!({ "s": "sawtooth", "transient": 1, "release": inf }),
        json!({ "s": "sawtooth", "transient": 1, "crush": 1, "coarse": 3, "shape": 0.99,
                "distort": 3, "distorttype": "fold", "pan": 0.3 }),
    ];
    sweep(&cases, DEFAULT_TIMES);
    // shape(-1): at 0.3 the output stays finite; a full-scale constant input (|x| = 1 after
    // Gain(0.3) * gain 1/0.3) makes the denominator 0: reported, never rendered.
    let r = exercise(&with_value(
        json!({ "s": "one", "gain": 1, "shape": -1, "attack": 0, "sustain": 1 }),
        0.0,
        0.05,
    ));
    assert!(r.is_some());
    let v = with_value(
        json!({ "s": "one", "gain": 1.0 / 0.3, "shape": -1, "attack": 0, "sustain": 1 }),
        0.0,
        0.05,
    );
    let job = EngineJob::from_json(&v).unwrap();
    let r = render(&job).unwrap();
    assert!(r.channels.iter().all(|c| c.iter().all(|x| x.is_finite())));
    assert!(
        r.errors.iter().any(|e| e.message.contains("non-finite")),
        "{:?}",
        r.errors
    );
}

#[test]
fn extreme_ladder_and_djf_controls_never_panic() {
    let nan = json!({ "$num": "NaN" });
    let inf = json!({ "$num": "Infinity" });
    let ninf = json!({ "$num": "-Infinity" });
    let ladder = |extra: Value| {
        let mut m = json!({ "s": "sawtooth", "cutoff": 900, "ftype": "ladder" });
        for (k, v) in extra.as_object().unwrap() {
            m[k] = v.clone();
        }
        m
    };
    let cases = [
        ladder(json!({ "resonance": 1e6 })),
        ladder(json!({ "resonance": -1e6 })),
        ladder(json!({ "resonance": 3e38 })),
        ladder(json!({ "drive": 1e9 })),
        ladder(json!({ "drive": -1e9 })),
        ladder(json!({ "drive": 3e38, "resonance": 40 })),
        ladder(json!({ "drive": 1e39 })),
        ladder(json!({ "drive": nan.clone() })),
        ladder(json!({ "drive": null })),
        ladder(json!({ "drive": "abc" })),
        ladder(json!({ "cutoff": -1e9 })),
        ladder(json!({ "cutoff": 1e30, "resonance": 1e6 })),
        ladder(json!({ "cutoff": nan.clone() })),
        ladder(json!({ "cutoff": null, "hcutoff": 1e9, "bandf": -1e9, "bandq": 1e6 })),
        ladder(json!({ "lpenv": 1e9, "lpattack": 0, "resonance": 30 })),
        ladder(json!({ "lpenv": -1e300, "fanchor": 1e300 })),
        ladder(json!({ "transient": 1, "coarse": 3, "gain": 1e30, "pan": 0.2 })),
        json!({ "s": "sawtooth", "djf": nan.clone() }),
        json!({ "s": "sawtooth", "djf": inf.clone() }),
        json!({ "s": "sawtooth", "djf": ninf.clone() }),
        json!({ "s": "sawtooth", "djf": "0.2" }),
        json!({ "s": "sawtooth", "djf": "abc", "delay": 0.5 }),
        json!({ "s": "sawtooth", "djf": true }),
        json!({ "s": "sawtooth", "djf": 1e39, "room": 0.5, "roomsize": 0.02 }),
        json!({ "s": "sawtooth", "djf": -1e300 }),
        json!({ "s": "one", "djf": 0.01, "gain": 1e30 }),
        json!({ "s": "white", "djf": 0.99, "ftype": "ladder", "hcutoff": 100 }),
    ];
    sweep(&cases, DEFAULT_TIMES);
    // A non-finite djf (or one that overflows a float) throws at `.value =`: reported, and the
    // render goes on.
    for djf in [nan, inf, json!(1e39)] {
        let v = with_value(json!({ "s": "sawtooth", "djf": djf }), 0.0, 0.05);
        let r = render(&EngineJob::from_json(&v).unwrap()).unwrap();
        assert!(
            r.errors.iter().any(|e| e.message.contains("TypeError")),
            "{djf}: {:?}",
            r.errors
        );
    }
}

#[test]
fn extreme_synth_worklet_controls_never_panic() {
    let nan = json!({ "$num": "NaN" });
    let inf = json!({ "$num": "Infinity" });
    let with = |s: &str, extra: Value| {
        let mut m = json!({ "s": s, "note": 45 });
        for (k, v) in extra.as_object().unwrap() {
            m[k] = v.clone();
        }
        m
    };
    let mut cases = Vec::new();
    for s in ["supersaw", "pulse"] {
        for extra in [
            json!({ "unison": 1e300 }),
            json!({ "unison": -1e300 }),
            json!({ "unison": nan.clone() }),
            json!({ "unison": null }),
            json!({ "unison": 2.5, "spread": 1e300 }),
            json!({ "unison": 7, "spread": -1e300 }),
            json!({ "unison": 7, "spread": nan.clone() }),
            json!({ "detune": 1e30 }),
            json!({ "detune": -1e30 }),
            json!({ "detune": 1e39 }),
            json!({ "detune": nan.clone(), "n": 3 }),
            json!({ "pw": 1e30 }),
            json!({ "pw": -1e30 }),
            json!({ "pw": inf.clone() }),
            json!({ "pw": "abc" }),
            json!({ "note": -1e9 }),
            json!({ "freq": 3e38 }),
            json!({ "freq": 1e39 }),
            json!({ "penv": 1e30, "pattack": 0 }),
            json!({ "penv": -1e300 }),
            json!({ "vib": 1e30, "vibmod": 1e30 }),
            json!({ "vib": 5, "vibmod": 3e38 }),
            json!({ "fmi": 1e30, "fmh": 1e30 }),
            json!({ "fmi": 2, "fmwave": "white" }),
            json!({ "fmi": 2, "fmh": nan.clone(), "fmattack": 0 }),
            json!({ "transient": 1, "pan": 0.9, "cutoff": 1e9, "ftype": "ladder" }),
            json!({ "release": inf.clone() }),
            json!({ "attack": 1e300, "release": 1e300 }),
        ] {
            cases.push(with(s, extra));
        }
    }
    sweep(&cases, DEFAULT_TIMES);
    // A non-finite `.value =` (NaN voices, an infinite pulse width, a detune past the float
    // range) throws a TypeError: reported, and the render goes on.
    for c in [
        with("supersaw", json!({ "unison": nan.clone() })),
        with("supersaw", json!({ "detune": 1e39 })),
        with("pulse", json!({ "pw": inf })),
        with("pulse", json!({ "pw": "abc" })),
    ] {
        let v = with_value(c.clone(), 0.0, 0.05);
        let r = render(&EngineJob::from_json(&v).unwrap()).unwrap();
        assert!(
            r.errors.iter().any(|e| e.message.contains("TypeError")),
            "{c}: {:?}",
            r.errors
        );
        assert!(
            r.channels.iter().all(|ch| ch.iter().all(|x| *x == 0.0)),
            "{c}"
        );
    }
}

#[test]
fn extreme_lfo_controls_never_panic() {
    let nan = json!({ "$num": "NaN" });
    let inf = json!({ "$num": "Infinity" });
    let ninf = json!({ "$num": "-Infinity" });
    let with = |s: &str, m: Value| {
        let mut base = json!({ "s": s, "note": 45 });
        for (k, v) in m.as_object().unwrap() {
            base[k] = v.clone();
        }
        base
    };
    let mut cases = Vec::new();
    // Tremolo: depth, rate, sync, shape, skew and phase extremes.
    for extra in [
        json!({ "tremolo": 4, "tremolodepth": 1e30 }),
        json!({ "tremolo": 4, "tremolodepth": -1e30 }),
        json!({ "tremolo": 4, "tremolodepth": nan.clone() }),
        json!({ "tremolo": 4, "tremolodepth": inf.clone() }),
        json!({ "tremolo": 1e30, "tremolodepth": 2 }),
        json!({ "tremolo": -1e30, "tremoloshape": "sine" }),
        json!({ "tremolo": 3e38, "tremoloskew": 1 }),
        json!({ "tremolosync": 1e300 }),
        json!({ "tremolosync": nan.clone() }),
        json!({ "tremolo": 4, "tremoloshape": 1e300 }),
        json!({ "tremolo": 4, "tremoloshape": -2.5 }),
        json!({ "tremolo": 4, "tremoloshape": ninf.clone() }),
        json!({ "tremolo": 4, "tremoloskew": 1e30, "tremolophase": -1e30 }),
        json!({ "tremolo": 4, "tremoloskew": 0, "tremolophase": 1e39 }),
    ] {
        cases.push(with("sine", extra.clone()));
        cases.push(with("pulse", extra));
    }
    // Filter LFOs on every filter kind, the 24 dB pair and the ladder.
    for f in ["lp", "hp", "bp"] {
        let cutoff = match f {
            "lp" => "cutoff",
            "hp" => "hcutoff",
            _ => "bandf",
        };
        for (k, v) in [
            ("rate", json!(1e30)),
            ("rate", json!(-1e30)),
            ("rate", json!(3e38)),
            ("rate", nan.clone()),
            ("sync", json!(1e300)),
            ("depth", json!(1e30)),
            ("depth", json!(-1e30)),
            ("depth", inf.clone()),
            ("depthfrequency", json!(1e39)),
            ("depthfrequency", json!(-1e9)),
            ("shape", json!(1e9)),
            ("shape", json!(2.5)),
            ("dc", json!(1e30)),
            ("skew", json!(-1e30)),
        ] {
            for ftype in [json!("12db"), json!("24db"), json!("ladder")] {
                let mut m = json!({ "ftype": ftype });
                m[cutoff] = json!(900);
                m[format!("{f}{k}")] = v.clone();
                m[format!("{f}rate")] = m.get(format!("{f}rate")).cloned().unwrap_or(json!(8));
                cases.push(with("sawtooth", m));
            }
        }
    }
    cases.push(with(
        "sawtooth",
        json!({ "cutoff": 1e30, "lprate": 5, "lpdepth": 1e30 }),
    ));
    cases.push(with(
        "sawtooth",
        json!({ "cutoff": -1e30, "lpdepthfrequency": 1e30 }),
    ));
    // Pulse width modulation.
    for extra in [
        json!({ "pwrate": 1e30 }),
        json!({ "pwrate": -1e30, "pwsweep": 1e30 }),
        json!({ "pwrate": 3e38, "pwsweep": -1e30 }),
        json!({ "pwrate": nan.clone() }),
        json!({ "pwsweep": inf.clone() }),
        json!({ "pwsweep": 1e39 }),
        json!({ "pwrate": 0, "pwsweep": 1 }),
        json!({ "pwrate": 44100, "pwsweep": 2, "pw": 1e30 }),
    ] {
        cases.push(with("pulse", extra));
    }
    sweep(&cases, DEFAULT_TIMES);
    // A non-finite `.value =` or GainNode option throws a TypeError: reported, the voice does
    // not sound, and the render goes on.
    for c in [
        with("sine", json!({ "tremolo": 4, "tremolodepth": nan.clone() })),
        with("sine", json!({ "tremolo": 4, "tremolodepth": inf.clone() })),
        with("sine", json!({ "tremolo": nan.clone() })),
        with("sawtooth", json!({ "cutoff": 800, "lprate": nan.clone() })),
        with("sawtooth", json!({ "cutoff": 800, "lpdepth": 1e39 })),
        with("pulse", json!({ "pwrate": nan })),
        with("pulse", json!({ "pwsweep": inf })),
    ] {
        let v = with_value(c.clone(), 0.0, 0.05);
        let r = render(&EngineJob::from_json(&v).unwrap()).unwrap();
        assert!(
            r.errors.iter().any(|e| e.message.contains("TypeError")),
            "{c}: {:?}",
            r.errors
        );
        assert!(
            r.channels.iter().all(|ch| ch.iter().all(|x| *x == 0.0)),
            "{c}"
        );
    }
    // A shape that names no LFO waveshape is a processor error: reported once per voice, the
    // voice plays on at the tremolo's intrinsic gain.
    let v = with_value(
        with(
            "sine",
            json!({ "tremolo": 4, "tremoloshape": 2.5, "tremolodepth": 0.5 }),
        ),
        0.0,
        0.2,
    );
    let r = render(&EngineJob::from_json(&v).unwrap()).unwrap();
    let errors: Vec<_> = r
        .errors
        .iter()
        .filter(|e| e.message.contains("processor error"))
        .collect();
    // Once per voice (the base job has two haps).
    assert_eq!(errors.len(), 2, "{:?}", r.errors);
    assert_ne!(errors[0].message, errors[1].message);
    assert!(r.channels.iter().any(|ch| ch.iter().any(|x| *x != 0.0)));
}

#[test]
fn extreme_native_effect_controls_never_panic() {
    let nan = json!({ "$num": "NaN" });
    let inf = json!({ "$num": "Infinity" });
    let with = |s: &str, m: Value| {
        let mut base = json!({ "s": s, "note": 45 });
        for (k, v) in m.as_object().unwrap() {
            base[k] = v.clone();
        }
        base
    };
    let mut cases = Vec::new();
    for extra in [
        // Compressor: release 0, ratio 1, knee 0, attack 0, threshold -100 and out of range.
        json!({ "compressor": -100 }),
        json!({ "compressor": -20, "compressorRelease": 0 }),
        json!({ "compressor": -20, "compressorRatio": 1 }),
        json!({ "compressor": -20, "compressorKnee": 0 }),
        json!({ "compressor": -20, "compressorAttack": 0 }),
        json!({ "compressor": 0, "compressorRatio": 1, "compressorKnee": 0, "compressorAttack": 0, "compressorRelease": 0 }),
        json!({ "compressor": -1e30, "compressorRatio": 1e30, "compressorKnee": -1e30 }),
        json!({ "compressor": 1e30, "compressorAttack": 1e30, "compressorRelease": -1e30 }),
        json!({ "compressor": null, "gain": 1e30 }),
        // Phaser: sweep 0, huge rates and centres, depth extremes.
        json!({ "phaserrate": 4, "phasersweep": 0 }),
        json!({ "phaserrate": 1e30, "phasersweep": 1e30 }),
        json!({ "phaserrate": -1e30, "phasercenter": -1e30 }),
        json!({ "phaserrate": 4, "phasercenter": 1e30, "phaserdepth": 1e30 }),
        json!({ "phaserrate": 4, "phasersweep": 1e38 }),
        json!({ "phaserrate": 0, "phaserdepth": 1e-30 }),
        json!({ "phaserrate": 22050, "phasercenter": 21800 }),
        // Vowel: junk letters.
        json!({ "vowel": "zz" }),
        json!({ "vowel": "" }),
        json!({ "vowel": null }),
        json!({ "vowel": 3 }),
        json!({ "vowel": ["a"] }),
        json!({ "vowel": "constructor" }),
        json!({ "vowel": "ı", "gain": 1e30 }),
        // FXrelease extremes.
        json!({ "FXrelease": 1e30 }),
        json!({ "FXrelease": -1e30 }),
        json!({ "FXrelease": 1e300, "release": 1e299 }),
        json!({ "FXrelease": 5e-324 }),
        json!({ "FXrelease": nan.clone() }),
        json!({ "FXrelease": inf.clone() }),
        json!({ "FXrelease": 0.3, "release": nan.clone() }),
        // All together.
        json!({ "vowel": "a", "compressor": -30, "pan": 0.5, "phaserrate": 3, "FXrelease": 0.2, "cutoff": 500 }),
    ] {
        cases.push(with("sawtooth", extra.clone()));
        cases.push(with("supersaw", extra));
    }
    sweep(&cases, DEFAULT_TIMES);
    // Throws are reported as hap errors and the voice does not sound.
    for (c, needle) in [
        (
            with("sine", json!({ "compressor": nan.clone() })),
            "non-finite",
        ),
        (
            with(
                "sine",
                json!({ "compressor": -3, "compressorKnee": inf.clone() }),
            ),
            "non-finite",
        ),
        (
            with("sine", json!({ "vowel": "zz" })),
            "vowel: unknown vowel zz",
        ),
        (
            with(
                "sine",
                json!({ "phaserrate": 2, "phasersweep": nan.clone() }),
            ),
            "TypeError",
        ),
        (
            with("sine", json!({ "phaserrate": 2, "phasercenter": inf })),
            "TypeError",
        ),
    ] {
        let v = with_value(c.clone(), 0.0, 0.05);
        let r = render(&EngineJob::from_json(&v).unwrap()).unwrap();
        assert!(
            r.errors.iter().any(|e| e.message.contains(needle)),
            "{c}: {:?}",
            r.errors
        );
        assert!(
            r.channels.iter().all(|ch| ch.iter().all(|x| *x == 0.0)),
            "{c}"
        );
    }
    // A NaN FXrelease is not `> release`: no release gain, the voice sounds.
    let v = with_value(with("sine", json!({ "FXrelease": nan })), 0.0, 0.05);
    let r = render(&EngineJob::from_json(&v).unwrap()).unwrap();
    assert!(r.channels.iter().any(|ch| ch.iter().any(|x| *x != 0.0)));
}

#[test]
fn extreme_duck_controls_never_panic() {
    let nan = json!({ "$num": "NaN" });
    let inf = json!({ "$num": "Infinity" });
    let ninf = json!({ "$num": "-Infinity" });
    // Orbit 1 ducks orbit 2, which holds a DC voice; both haps of `base` duck.
    let job_of = |duck: &Value| {
        let mut v = base();
        let mut ducker = json!({ "s": "one", "gain": 0 });
        for (k, x) in duck.as_object().unwrap() {
            ducker[k] = x.clone();
        }
        v["values"] = json!([ducker, { "s": "one", "attack": 0, "sustain": 1, "orbit": 2 }]);
        v["orbits"] =
            json!([{ "orbit": 1, "channels": [0, 1] }, { "orbit": 2, "channels": [0, 1] }]);
        v["pauses"][0]["haps"] = json!([
            { "t": 0.0, "duration": 0.2, "cycle": 0, "v": 1, "event": 0 },
            { "t": 0.01, "duration": 0.05, "cycle": 0, "v": 0, "event": 1 }
        ]);
        v.to_string()
    };
    let ok = [
        json!({ "duckorbit": 2, "duckdepth": inf.clone() }),
        json!({ "duckorbit": 2, "duckdepth": 0 }),
        json!({ "duckorbit": 2, "duckdepth": 1e308 }),
        json!({ "duckorbit": 2, "duckattack": 0 }),
        json!({ "duckorbit": 2, "duckattack": -1e30 }),
        json!({ "duckorbit": 2, "duckattack": 1e300 }),
        json!({ "duckorbit": 2, "duckonset": -1 }),
        json!({ "duckorbit": 2, "duckonset": -1e30 }),
        json!({ "duckorbit": 2, "duckonset": 1e300 }),
        json!({ "duckorbit": 2, "duckonset": 5e-324, "duckattack": 5e-324 }),
        json!({ "duckorbit": [2, 2, 2], "duckonset": [0, 0.01], "duckdepth": [0.9, null] }),
        json!({ "duckorbit": [1, 2, 99], "duckattack": [] }),
        json!({ "duckorbit": 99 }),
        json!({ "duckorbit": "x" }),
        json!({ "duckorbit": [] }),
        json!({ "duckorbit": null }),
        json!({ "duckorbit": 1, "duckdepth": 0.99 }),
        json!({ "duckorbit": 2, "duckonset": "0.1", "duckattack": true, "duckdepth": {} }),
    ];
    for d in &ok {
        assert!(exercise(&job_of(d)).is_some(), "{d}");
    }
    // Non-finite results throw: reported as hap errors, nothing panics, the audio stays finite.
    for (d, needle) in [
        (
            json!({ "duckorbit": 2, "duckdepth": nan.clone() }),
            "float value is non-finite",
        ),
        (
            json!({ "duckorbit": 2, "duckdepth": ninf }),
            "float value is non-finite",
        ),
        (
            json!({ "duckorbit": 2, "duckdepth": -0.5 }),
            "float value is non-finite",
        ),
        (
            json!({ "duckorbit": 2, "duckattack": nan.clone() }),
            "double value is non-finite",
        ),
        (
            json!({ "duckorbit": 2, "duckonset": nan }),
            "double value is non-finite",
        ),
        (
            json!({ "duckorbit": 2, "duckonset": inf }),
            "double value is non-finite",
        ),
        (
            json!({ "duckorbit": 2, "duckonset": 1e308, "duckattack": 1e308 }),
            "double value is non-finite",
        ),
    ] {
        let text = job_of(&d);
        assert!(exercise(&text).is_some(), "{d}");
        let r = render(&EngineJob::from_json(&text).unwrap()).unwrap();
        assert!(
            r.errors
                .iter()
                .any(|e| e.kind == ErrorKind::Hap && e.message.contains(needle)),
            "{d}: {:?}",
            r.errors
        );
    }
}

#[test]
fn extreme_bus_controls_never_panic() {
    // A feeder (`bus`, `busgain`) and the bus sound reading `n`; both haps of `base` play.
    let job_of = |feeder: &Value, reader: &Value| {
        let mut v = base();
        v["values"] = json!([feeder, reader]);
        v["pauses"][0]["haps"] = json!([
            { "t": 0.0, "duration": 0.1, "cycle": 0, "v": 1, "event": 0 },
            { "t": 0.01, "duration": 0.05, "cycle": 0, "v": 0, "event": 1 }
        ]);
        v.to_string()
    };
    let values = extreme_values();
    for x in &values {
        // The value as the bus key of both sides, as busgain, and as the reader's own `bus`.
        let feeder = json!({ "s": "sawtooth", "bus": x.clone(), "dry": 0 });
        let reader = json!({ "s": "bus", "n": x.clone() });
        assert!(exercise(&job_of(&feeder, &reader)).is_some(), "bus {x}");
        let feeder = json!({ "s": "sawtooth", "bus": 1, "busgain": x.clone() });
        let reader = json!({ "s": "bus", "n": 1, "pan": 0.2, "bus": 2 });
        assert!(exercise(&job_of(&feeder, &reader)).is_some(), "busgain {x}");
        let reader = json!({ "s": "bus", "n": 1, "attack": x.clone(), "release": x.clone() });
        let feeder = json!({ "s": "one", "bus": 1, "busgain": 1e30, "gain": 1e30 });
        assert!(
            exercise(&job_of(&feeder, &reader)).is_some(),
            "envelope {x}"
        );
    }
    // A non-finite or overflowing busgain is a hap error; the reader then reads silence.
    for g in [
        json!({ "$num": "NaN" }),
        json!({ "$num": "-Infinity" }),
        json!(1e39),
    ] {
        let text = job_of(
            &json!({ "s": "sawtooth", "bus": 1, "busgain": g.clone() }),
            &json!({ "s": "bus", "n": 1 }),
        );
        let r = render(&EngineJob::from_json(&text).unwrap()).unwrap();
        assert!(
            r.errors
                .iter()
                .any(|e| e.kind == ErrorKind::Hap && e.message.contains("TypeError")),
            "{g}: {:?}",
            r.errors
        );
    }
    // A cycle (a reader of its own send bus) is refused, never rendered.
    let text = job_of(
        &json!({ "s": "sine", "bus": [1] }),
        &json!({ "s": "bus", "n": "1", "bus": 1 }),
    );
    let r = render(&EngineJob::from_json(&text).unwrap()).unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(!r.unsupported.is_empty());
    assert!(r.channels.iter().flatten().all(|x| *x == 0.0));
    // Gains that overflow through a two-level bus chain are reported, not rendered.
    let mut v = base();
    v["values"] = json!([
        { "s": "one", "gain": 1e30, "bus": "a", "dry": 0 },
        { "s": "bus", "n": "a", "postgain": 1e30, "bus": "b", "dry": 0 },
        { "s": "bus", "n": "b", "postgain": 1e30 }
    ]);
    v["pauses"][0]["haps"] = json!([
        { "t": 0.0, "duration": 0.05, "cycle": 0, "v": 2, "event": 0 },
        { "t": 0.0, "duration": 0.05, "cycle": 0, "v": 1, "event": 1 },
        { "t": 0.0, "duration": 0.05, "cycle": 0, "v": 0, "event": 2 }
    ]);
    assert!(exercise(&v.to_string()).unwrap() > 0);
}

/// The final sweep: every allowlisted key over the extreme values, on voices that use every
/// effect (filters, pan, delay and a short reverb), on every allowlisted sound.
#[test]
fn every_allowed_key_renders_finite_audio() {
    let values = extreme_values();
    let plain = json!({
        "s": "sine", "note": 60, "cutoff": 2000, "pan": 0.3, "delay": 0.5, "delaytime": 0.01,
        "room": 0.5, "roomsize": 0.02, "roomlp": 0
    });
    assert_eq!(
        exercise(&with_value(plain, 0.0, 0.05)),
        Some(0),
        "the base voice is supported"
    );
    let sounds = allowed_sounds();
    for (i, key) in ALLOWED_KEYS.iter().enumerate() {
        for (j, x) in values.iter().enumerate() {
            let s = sounds[(i + j) % sounds.len()];
            let mut map = json!({
                "s": s, "note": 60, "cutoff": 2000, "pan": 0.3, "delay": 0.5, "delaytime": 0.01,
                "room": 0.5, "roomsize": 0.02, "roomlp": 0
            });
            map[*key] = x.clone();
            let (t, d) = if j % 2 == 0 {
                (0.0, 0.05)
            } else {
                (0.013, 0.0)
            };
            assert!(exercise(&with_value(map, t, d)).is_some(), "{s} {key}={x}");
        }
    }
    // Two reverbs on one orbit's history: a long-ish IR swapped for a short one mid-tail, with
    // stereo and mono sends.
    let mut v = base();
    v["values"] = json!([
        { "s": "sawtooth", "room": 1e9, "roomsize": 0.15, "pan": 0.9 },
        { "s": "white", "room": 1e-30, "roomsize": 0.01, "roomlp": 1e-300, "dry": 0 }
    ]);
    v["pauses"][1]["haps"][0]["v"] = json!(1);
    assert!(exercise(&v.to_string()).is_some());
}

#[test]
fn odd_orbit_layouts_render_finite_audio() {
    let mut rng = Rng(0x5851_f42d_4c95_7f2d);
    let orbits = [-2.0, 0.0, 1.0, 1.5, 2.0, 17.0, 1e9];
    for _ in 0..200 {
        let count = *rng.pick(&[1u64, 2, 4, 6, 32]);
        let n_orbits = (rng.next() % 4) as usize;
        let specs: Vec<Value> = (0..n_orbits)
            .map(|_| {
                let n = 1 + (rng.next() % 4) as usize;
                let channels: Vec<u64> = (0..n)
                    .map(|_| *rng.pick(&[0u64, 1, 3, 5, 31, 64, 1 << 40]))
                    .collect();
                json!({ "orbit": *rng.pick(&orbits), "channels": channels })
            })
            .collect();
        let mut v = base();
        v["channels"] = json!({ "count": count, "multiChannelOrbits": count > 2 });
        v["orbits"] = json!(specs);
        v["values"] = json!([{ "s": "sine", "note": 60, "orbit": *rng.pick(&orbits), "pan": 0.2 }]);
        let errors = exercise(&v.to_string()).expect("valid job");
        if n_orbits == 0 {
            assert_eq!(errors, 2, "both haps miss their orbit: {v}");
        }
    }
}

#[test]
fn extreme_fx_chains_never_panic() {
    let values = extreme_values();
    // Odd `FX` shapes: refused (`FX`) or empty, never a panic.
    for fx in [
        json!(3),
        json!(null),
        json!("ab"),
        json!([]),
        json!([null]),
        json!([3, {}]),
        json!([[]]),
        json!({ "0": { "cutoff": 500 } }),
        json!([{ "FX": [{ "delay": 0.5 }] }]),
        json!([{ "FX": 3, "cutoff": 500 }]),
        json!({ "$set": [{ "cutoff": 500 }] }),
        json!([{ "$unsupported": "function" }]),
    ] {
        let map = json!({ "s": "sawtooth", "note": 50, "FX": fx.clone() });
        assert!(exercise(&with_value(map, 0.0, 0.05)).is_some(), "FX = {fx}");
    }
    // Every chain key at every extreme value inside an entry with a local delay and reverb.
    let entry = json!({
        "cutoff": 2000, "pan": 0.3, "delay": 0.5, "delaytime": 0.01, "room": 0.5,
        "roomsize": 0.02, "roomlp": 0
    });
    let chain_keys = strudeland_engine::CHAIN_KEYS;
    assert!(!chain_keys.is_empty());
    for (i, key) in chain_keys.iter().enumerate() {
        for (j, x) in values.iter().enumerate() {
            let s = ["sine", "sawtooth", "supersaw", "white", "pulse", "one"][(i + j) % 6];
            let mut e = entry.clone();
            e[*key] = x.clone();
            let map = json!({ "s": s, "note": 50, "FXrelease": 0.05, "FX": [e, entry.clone()] });
            let (t, d) = if j % 2 == 0 {
                (0.0, 0.05)
            } else {
                (0.013, 0.0)
            };
            assert!(
                exercise(&with_value(map, t, d)).is_some(),
                "{s} FX.{key}={x}"
            );
        }
    }
    // Huge and odd local delays and reverbs, many entries, no FXrelease (a cut at
    // endWithRelease all the same), and a release that never ends.
    let many = vec![json!({ "delay": 0.5, "delaytime": 0.003, "room": 0.2, "roomsize": 0.01 }); 8];
    for extra in [
        json!({ "FX": [{ "delay": 1e30, "delaytime": 1e30, "delayfeedback": 1e30 }] }),
        json!({ "FX": [{ "delay": 1e-30, "delaytime": 5e-324, "delayfeedback": 0.98 }] }),
        json!({ "FX": [{ "delay": 1, "delaytime": 0.001, "delayfeedback": 0.999, "dry": 1e30 }] }),
        json!({ "FX": [{ "room": 1e30, "roomsize": 1e-30 }] }),
        json!({ "FX": [{ "room": 1, "roomsize": 0.02, "roomfade": 1e30, "roomlp": 1e30, "roomdim": -1e30 }] }),
        json!({ "FX": [{ "room": 1, "roomsize": 1e30 }] }),
        json!({ "FX": many }),
        json!({ "release": 1e30, "FX": [{ "delay": 0.5, "delaytime": 0.01 }] }),
        json!({ "FXrelease": 1e300, "FX": [{ "room": 0.5, "roomsize": 0.01 }] }),
        json!({ "ftype": "24db", "FX": [{ "cutoff": 500, "ftype": "ladder" }, { "bandf": 300, "pan": 0 }] }),
        json!({ "FX": [{ "cutoff": 500, "delay": 0.5, "lfo": modulator(json!({ "control": "delaytime", "depthabs": 1e30 })) }] }),
        json!({ "FX": [{ "delay": 0.5, "delaytime": 0.01 }], "lfo": modulator(json!({ "control": "delay", "fxi": 0, "depthabs": 1e30 })) }),
    ] {
        let mut map = json!({ "s": "sawtooth", "note": 50 });
        for (k, v) in extra.as_object().unwrap() {
            map[k] = v.clone();
        }
        for (t, d) in [(0.0, 0.05), (0.013, 0.0), (0.1, 5e-324)] {
            assert!(exercise(&with_value(map.clone(), t, d)).is_some(), "{map}");
        }
    }
}

/// A modulator value with one config under id 0.
fn modulator(config: Value) -> Value {
    json!({ "__ids": { "$set": [0] }, "0": config })
}

#[test]
fn extreme_modulator_configs_render_finite_audio() {
    let values = extreme_values();
    // A voice with something to target: gain, a lowpass, an orbit delay, a djf.
    let voice = |s: &str| {
        json!({ "s": s, "note": 60, "cutoff": 2000, "delay": 0.5, "delaytime": 0.01,
                "djf": 0.4, "pan": 0.3, "bus": 1 })
    };
    let targets = [
        "gain",
        "cutoff",
        "note",
        "delaytime",
        "djf",
        "pan",
        "lfo_0",
        "pw",
    ];
    let keys = [
        (
            "lfo",
            &[
                "rate",
                "sync",
                "depth",
                "depthabs",
                "dcoffset",
                "shape",
                "skew",
                "curve",
                "retrig",
                "phaseoffset",
                "fxi",
                "subControl",
            ][..],
        ),
        (
            "env",
            &[
                "attack",
                "decay",
                "sustain",
                "release",
                "depth",
                "depthabs",
                "acurve",
                "dcurve",
                "rcurve",
                "retrigger",
                "fxi",
            ][..],
        ),
        (
            "bmod",
            &["bus", "depth", "depthabs", "dc", "fxi", "subControl"][..],
        ),
    ];
    let mut n = 0;
    for (ty, subs) in keys {
        for (i, sub) in subs.iter().enumerate() {
            for (j, x) in values.iter().enumerate() {
                let target = targets[(i + j) % targets.len()];
                let s = ["sine", "pulse", "supersaw", "white"][(i + 2 * j) % 4];
                let mut map = voice(s);
                let mut config = json!({ "control": target });
                config[*sub] = x.clone();
                if ty == "bmod" && *sub != "bus" {
                    config["bus"] = json!(1);
                }
                map[ty] = modulator(config);
                // A second modulator of the first one (mod of mod), sometimes.
                if j % 3 == 0 {
                    map["env"] = modulator(json!({ "control": "lfo_0", "depthabs": x.clone() }));
                }
                let mut v = base();
                v["values"] = json!([map, { "s": "sine", "bus": 1, "dry": 0 }]);
                v["pauses"][1]["haps"][0]["v"] = json!(1);
                let text = v.to_string();
                assert!(
                    exercise(&text).is_some(),
                    "{ty}.{sub}={x} on {target} ({s})"
                );
                n += 1;
            }
        }
    }
    assert!(n > 500);
    // Odd `__ids` shapes are refused (a signature), never a panic.
    for ids in [
        json!(3),
        json!({ "$set": [] }),
        json!({ "$set": [{}, null, [1]] }),
        json!([0]),
    ] {
        let mut v = base();
        v["values"] = json!([{ "s": "sine", "lfo": { "__ids": ids, "0": { "control": "gain" } } }]);
        assert!(exercise(&v.to_string()).is_some());
    }
}
