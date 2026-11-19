//! `jsmath` against the V8 math reference: `tests/refs/v8-math.json` holds V8's `Math.*` results
//! (evaluated inside an AudioWorklet in the Chromium the refs pin, by `bun run engine:refs`) for
//! the grid of `scripts/engine/v8-math-cases.ts`. Every number is an f64 bit string; two results
//! match when the bit patterns are equal or both are NaN (`-0` and `+0` differ).
//!
//! `candidate_match_rates` also measures `libm`, Rust's `std` (the host's libm: recorded only, never
//! chosen) and `jsmath`'s own port (CORE-MATH, LLVM libc or V8; JS semantics for `round` and
//! `toInt32`), per function, and asserts that the implementation `jsmath::IMPLEMENTATION` names
//! matches every vector.
//!
//! The `distort:<name>` groups (`x`, `k = expm1(d)` as V8 computes it) check the `shaper`'s
//! distortion curves (`distortion_algorithm`), which are built on `jsmath`, the same way.
//!
//! `pow` and `tanh` are the host C library's (`"host"`), as Chromium's are: their bit-exact check
//! (and that of the `distort:*` groups, which call `tanh`) applies where the reference was
//! generated, macOS (`host_is_judged`); other hosts record their match rates only. With
//! `V8_MATH_EVIDENCE=<path>` it writes the rates as JSON
//! (`plans/0011-engine-effects/evidence/v8-math-choice.json`). Nothing here is gated by platform.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};
use strudeland_engine::{DISTORTION_NAMES, distortion_algorithm, jsmath};

mod common;

/// Whether the host functions (`pow`, `tanh`) are held to the reference: only on macOS, the
/// reference's host (Chromium's `Math.pow` and `Math.tanh` are the host C library's too).
fn host_is_judged() -> bool {
    cfg!(target_os = "macos")
}

/// Reference functions `jsmath` does not implement: the `Math` functions the headless host runs
/// natively (judged there, against the same reference) and the `**` operator (`powOp`). Their
/// groups are well-formed-checked here and otherwise ignored.
const REFERENCE_ONLY: [&str; 15] = [
    "tan", "asin", "acos", "atan", "atan2", "sinh", "asinh", "acosh", "atanh", "log", "log2",
    "log10", "cbrt", "hypot", "powOp",
];

/// Whether a reference function takes two arguments (`ys` present).
fn is_binary(func: &str) -> bool {
    matches!(func, "pow" | "atan2" | "hypot" | "powOp") || func.starts_with("distort:")
}

/// The functions `jsmath` is checked on, in the evidence's order, with the reference's `fn` name.
const FUNCTIONS: [(&str, &str); 10] = [
    ("sin", "sin"),
    ("cos", "cos"),
    ("tanh", "tanh"),
    ("exp", "exp"),
    ("expm1", "expm1"),
    ("log1p", "log1p"),
    ("cosh", "cosh"),
    ("pow", "pow"),
    ("round", "round"),
    ("to_int32", "toInt32"),
];

struct Group {
    id: String,
    func: String,
    xs: Vec<f64>,
    ys: Option<Vec<f64>>,
    out: Vec<f64>,
}

fn reference_path() -> PathBuf {
    common::refs_dir().join("v8-math.json")
}

fn reference() -> Value {
    let text = std::fs::read_to_string(reference_path())
        .expect("tests/refs/v8-math.json exists: run `bun run engine:refs --write v8-math`");
    serde_json::from_str(&text).expect("v8-math.json is JSON")
}

fn hex_ok(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn decode(v: &Value, what: &str) -> Vec<f64> {
    v.as_array()
        .unwrap_or_else(|| panic!("{what} is an array"))
        .iter()
        .map(|h| {
            let s = h.as_str().unwrap_or_else(|| panic!("{what}: a string"));
            assert!(hex_ok(s), "{what}: {s:?} is not 16 lowercase hex digits");
            f64::from_bits(u64::from_str_radix(s, 16).unwrap())
        })
        .collect()
}

fn groups() -> Vec<Group> {
    reference()["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .map(|g| {
            let id = g["id"].as_str().expect("id").to_string();
            Group {
                func: g["fn"].as_str().expect("fn").to_string(),
                xs: decode(&g["xs"], &format!("{id}.xs")),
                ys: (!g["ys"].is_null()).then(|| decode(&g["ys"], &format!("{id}.ys"))),
                out: decode(&g["out"], &format!("{id}.out")),
                id,
            }
        })
        .collect()
}

fn same(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
}

type Unary = fn(f64) -> f64;
type Binary = fn(f64, f64) -> f64;

/// A candidate implementation of one function.
#[derive(Clone, Copy)]
enum Imp {
    U(Unary),
    B(Binary),
}

impl Imp {
    fn eval(self, x: f64, y: Option<f64>) -> f64 {
        match self {
            Imp::U(f) => f(x),
            Imp::B(f) => f(x, y.expect("a binary group has ys")),
        }
    }
}

fn jsmath_of(func: &str) -> Option<Imp> {
    Some(match func {
        "sin" => Imp::U(jsmath::sin),
        "cos" => Imp::U(jsmath::cos),
        "tanh" => Imp::U(jsmath::tanh),
        "exp" => Imp::U(jsmath::exp),
        "expm1" => Imp::U(jsmath::expm1),
        "log1p" => Imp::U(jsmath::log1p),
        "cosh" => Imp::U(jsmath::cosh),
        "pow" => Imp::B(jsmath::pow),
        "round" => Imp::U(jsmath::round),
        "toInt32" => Imp::U(|x| f64::from(jsmath::to_int32(x))),
        other => Imp::B(distortion_algorithm(other.strip_prefix("distort:")?)?),
    })
}

fn libm_of(func: &str) -> Imp {
    match func {
        "sin" => Imp::U(libm::sin),
        "cos" => Imp::U(libm::cos),
        "tanh" => Imp::U(libm::tanh),
        "exp" => Imp::U(libm::exp),
        "expm1" => Imp::U(libm::expm1),
        "log1p" => Imp::U(libm::log1p),
        "cosh" => Imp::U(libm::cosh),
        "pow" => Imp::B(libm::pow),
        // the nearest library functions to JS's own semantics, for the record
        "round" => Imp::U(libm::round),
        "toInt32" => Imp::U(|x| f64::from(libm::trunc(x) as i32)),
        _ => unreachable!("{func}"),
    }
}

fn std_of(func: &str) -> Imp {
    match func {
        "sin" => Imp::U(f64::sin),
        "cos" => Imp::U(f64::cos),
        "tanh" => Imp::U(f64::tanh),
        "exp" => Imp::U(f64::exp),
        "expm1" => Imp::U(f64::exp_m1),
        "log1p" => Imp::U(f64::ln_1p),
        "cosh" => Imp::U(f64::cosh),
        "pow" => Imp::B(f64::powf),
        "round" => Imp::U(f64::round),
        "toInt32" => Imp::U(|x| f64::from(x as i32)),
        _ => unreachable!("{func}"),
    }
}

/// `jsmath`'s own implementation where it is not the `libm` crate: a CORE-MATH, LLVM libc or V8
/// port, or JS semantics written out (`round`, `toInt32`).
fn port_of(func: &str) -> Option<Imp> {
    if chosen(func) == "libm" {
        None
    } else {
        jsmath_of(func)
    }
}

/// The implementation the evidence names as chosen: `IMPLEMENTATION`'s entry, or `"js"` for the
/// JS-semantics functions (`round`, `toInt32`).
fn chosen(func: &str) -> &'static str {
    jsmath::IMPLEMENTATION
        .iter()
        .find(|(n, _)| *n == func)
        .map(|(_, i)| *i)
        .unwrap_or("js")
}

fn fmt_bits(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}

#[derive(Default)]
struct Rate {
    matches: usize,
    samples: usize,
    mismatches: Vec<Value>,
}

impl Rate {
    fn add(&mut self, g: &Group, i: usize, got: f64) {
        self.samples += 1;
        if same(got, g.out[i]) {
            self.matches += 1;
        } else if self.mismatches.len() < 10 {
            self.mismatches.push(json!({
                "group": g.id,
                "x": fmt_bits(g.xs[i]),
                "y": g.ys.as_ref().map(|ys| fmt_bits(ys[i])),
                "v8": fmt_bits(g.out[i]),
                "got": fmt_bits(got),
            }));
        }
    }

    fn rate(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.matches as f64 / self.samples as f64
        }
    }

    fn json(&self) -> Value {
        // a full match is the JSON number 1 (the artifacts test reads it so)
        let rate = if self.matches == self.samples {
            json!(1)
        } else {
            json!(self.rate())
        };
        json!({ "matches": self.matches, "rate": rate, "firstMismatches": self.mismatches })
    }
}

#[test]
fn v8_math_reference_is_well_formed() {
    let r = reference();
    assert_eq!(r["schema"], 1, "v8-math.json schema");
    let p = &r["provenance"];
    assert_eq!(p["schema"], 1, "provenance schema");
    for k in ["inputsSha256", "playwright", "chromium", "platform"] {
        assert!(
            p[k].as_str().is_some_and(|s| !s.is_empty()),
            "provenance.{k}"
        );
    }
    let gs = groups();
    assert!(!gs.is_empty(), "no groups");
    let mut ids = std::collections::BTreeSet::new();
    for g in &gs {
        assert!(ids.insert(g.id.clone()), "duplicate group id {}", g.id);
        assert!(!g.xs.is_empty(), "{}: empty", g.id);
        assert_eq!(g.out.len(), g.xs.len(), "{}: out length", g.id);
        assert_eq!(
            g.ys.is_some(),
            is_binary(&g.func),
            "{}: ys present iff the function is binary",
            g.id
        );
        if let Some(ys) = &g.ys {
            assert_eq!(ys.len(), g.xs.len(), "{}: ys length", g.id);
        }
        assert!(
            jsmath_of(&g.func).is_some() || REFERENCE_ONLY.contains(&g.func.as_str()),
            "{}: unknown fn {}",
            g.id,
            g.func
        );
    }
    for (_, func) in FUNCTIONS {
        assert!(gs.iter().any(|g| g.func == func), "no group for {func}");
    }
    for name in DISTORTION_NAMES {
        let func = format!("distort:{name}");
        assert!(gs.iter().any(|g| g.func == func), "no group for {func}");
    }
    for func in REFERENCE_ONLY {
        assert!(gs.iter().any(|g| g.func == func), "no group for {func}");
        assert!(jsmath_of(func).is_none(), "{func} is reference-only");
    }
    assert!(is_binary("atan2") && is_binary("hypot") && is_binary("powOp"));
    assert!(!is_binary("tan") && !is_binary("sin"));
    assert!(jsmath_of("distort:nope").is_none());
    assert!(jsmath_of("nope").is_none());
}

#[test]
fn jsmath_matches_the_v8_reference_bit_for_bit() {
    let mut mismatches = Vec::new();
    let mut checked = 0usize;
    for g in groups() {
        if REFERENCE_ONLY.contains(&g.func.as_str()) {
            continue;
        }
        let f = jsmath_of(&g.func).unwrap_or_else(|| panic!("{}: unknown fn {}", g.id, g.func));
        let host = chosen(&g.func) == "host" || g.func.starts_with("distort:");
        if host && !host_is_judged() {
            println!("SKIP {} (host {}, judged on macOS only)", g.id, g.func);
            continue;
        }
        for i in 0..g.xs.len() {
            let y = g.ys.as_ref().map(|ys| ys[i]);
            let got = f.eval(g.xs[i], y);
            checked += 1;
            if !same(got, g.out[i]) {
                mismatches.push(format!(
                    "{} {}(x={}{}): v8 {} ({}), jsmath {} ({})",
                    g.id,
                    g.func,
                    g.xs[i],
                    y.map(|y| format!(", y={y}")).unwrap_or_default(),
                    fmt_bits(g.out[i]),
                    g.out[i],
                    fmt_bits(got),
                    got
                ));
            }
        }
    }
    assert!(checked > 0, "nothing checked");
    assert!(
        mismatches.is_empty(),
        "{} of {checked} samples differ from V8; the first five:\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn candidate_match_rates() {
    let gs = groups();
    let mut evidence = BTreeMap::new();
    let mut failures = Vec::new();
    for (name, func) in FUNCTIONS {
        let (libm_imp, std_imp, port_imp) = (libm_of(func), std_of(func), port_of(func));
        let (mut l, mut s, mut p) = (Rate::default(), Rate::default(), Rate::default());
        for g in gs.iter().filter(|g| g.func == func) {
            for i in 0..g.xs.len() {
                let (x, y) = (g.xs[i], g.ys.as_ref().map(|ys| ys[i]));
                l.add(g, i, libm_imp.eval(x, y));
                s.add(g, i, std_imp.eval(x, y));
                if let Some(imp) = port_imp {
                    p.add(g, i, imp.eval(x, y));
                }
            }
        }
        assert!(l.samples > 0, "{func}: no samples");
        let pick = chosen(func);
        let chosen_rate = match pick {
            "libm" => &l,
            "core-math" | "v8" | "host" | "js" => &p,
            other => panic!("{func}: implementation {other}"),
        };
        let judged = pick != "host" || host_is_judged();
        if judged && chosen_rate.matches != chosen_rate.samples {
            failures.push(format!(
                "{func}: chosen {pick} matches {}/{}",
                chosen_rate.matches, chosen_rate.samples
            ));
        }
        println!(
            "{func}: {} samples, libm {:.6}, std {:.6}, port {}, chosen {pick}",
            l.samples,
            l.rate(),
            s.rate(),
            port_imp.map_or("-".to_string(), |_| format!("{:.6}", p.rate()))
        );
        evidence.insert(
            name,
            json!({
                "fn": func,
                "samples": l.samples,
                "libm": l.json(),
                "std": s.json(),
                "port": port_imp.map(|_| p.json()),
                "chosen": pick,
                "judged": judged,
            }),
        );
    }
    if let Ok(path) = std::env::var("V8_MATH_EVIDENCE") {
        let r = reference();
        let body = json!({
            "rule": "per function, jsmath must match every vector of the V8 math reference: \
                     correctly rounded CORE-MATH ports for sin cos exp expm1 log1p, V8's \
                     math::pow special cases in front of the host C library's pow (host, as \
                     V8's use_std_math_pow does; judged on macOS only), the host C library's \
                     tanh (host, as V8's Math.tanh calls std::tanh; judged on macOS only), V8's cosh on the ported \
                     exp and expm1 (libm only where it already matches every vector); Rust std \
                     is recorded, never chosen; round and toInt32 are JS semantics written out \
                     (js); the port field is jsmath's own implementation",
            "reference": "crates/strudeland-engine/tests/refs/v8-math.json",
            "provenance": r["provenance"],
            "host": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            "functions": evidence,
        });
        // relative paths are relative to the repo root, as the plan's command line gives them
        let path = PathBuf::from(&path);
        let path = if path.is_absolute() {
            path
        } else {
            common::repo_root().join(path)
        };
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&body).unwrap()),
        )
        .unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
        println!("wrote {}", path.display());
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
