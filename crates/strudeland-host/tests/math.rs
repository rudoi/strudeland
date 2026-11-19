//! `Math` in the embedded V8 against the Chromium reference
//! (`crates/strudeland-engine/tests/refs/v8-math.json`): every `Math` property is classified
//! once (`strudeland_host::math`), and every non-exact function the host runs, natively or through
//! a `jsmath` override, matches Chromium bit for bit on every reference point. Judged on macOS
//! (the reference platform; `jsmath::pow`/`tanh` call the host C library); other hosts print the
//! match rates without asserting.
//!
//! `HOST_MATH_EVIDENCE=<path>` writes the verification table (one row per `Math` property and
//! per operator group: class, groups, points, mismatches, V8 version).

mod common;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;
use strudeland_host::Host;
use strudeland_host::math::{CONSTANTS, EXACT, OPTIONAL, OVERRIDDEN, THROWN, VERIFIED_NATIVE};
use strudeland_host::natives::RANDOM_MESSAGE;

fn judged() -> bool {
    cfg!(target_os = "macos")
}

/// The non-exact, non-overridden `Math` functions: each must end up in `VERIFIED_NATIVE` or,
/// ported into `jsmath`, in `OVERRIDDEN`.
const VERIFICATION_SET: [&str; 14] = [
    "tan", "asin", "acos", "atan", "atan2", "sinh", "asinh", "acosh", "atanh", "log", "log2",
    "log10", "cbrt", "hypot",
];

/// Reference functions that are not `Math` properties: the `**` operator and `x | 0`.
const OPERATORS: [&str; 2] = ["powOp", "toInt32"];

/// Why the `**` groups would not be asserted: no override can replace an operator. Empty while
/// `**` matches; set (with the evidence and the crate README's known differences) if it does not.
const POW_OP_SKIP: Option<&str> = None;

fn host() -> &'static Host {
    static HOST: OnceLock<Host> = OnceLock::new();
    HOST.get_or_init(|| Host::new().unwrap())
}

fn eval(source: &str) -> String {
    host()
        .eval_script(source)
        .unwrap_or_else(|e| panic!("{source}: {e}"))
}

fn math_names() -> &'static Vec<String> {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        eval("Object.getOwnPropertyNames(Math).join(',')")
            .split(',')
            .map(str::to_string)
            .collect()
    })
}

const CLASSES: [(&str, &[&str]); 5] = [
    ("overridden", OVERRIDDEN),
    ("thrown", THROWN),
    ("constant", CONSTANTS),
    ("exact", EXACT),
    ("verified native", VERIFIED_NATIVE),
];

fn class_of(name: &str) -> Option<&'static str> {
    CLASSES
        .iter()
        .find(|(_, names)| names.contains(&name))
        .map(|(class, _)| *class)
}

#[test]
fn every_math_property_is_classified_exactly_once() {
    let names = math_names();
    for name in names {
        let classes: Vec<&str> = CLASSES
            .iter()
            .filter(|(_, list)| list.contains(&name.as_str()))
            .map(|(class, _)| *class)
            .collect();
        assert_eq!(classes.len(), 1, "Math.{name} is in {classes:?}");
    }
    for (class, list) in CLASSES {
        for name in list {
            let present = names.iter().any(|n| n == name);
            assert!(
                present || OPTIONAL.contains(name),
                "Math.{name} ({class}) is listed but absent"
            );
        }
        let mut sorted = list.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), list.len(), "{class} lists a name twice");
    }
    // Every name of the verification set is verified native or overridden through jsmath.
    for name in VERIFICATION_SET {
        assert!(
            VERIFIED_NATIVE.contains(&name) || OVERRIDDEN.contains(&name),
            "{name} is neither verified native nor overridden"
        );
    }
    for name in VERIFIED_NATIVE {
        assert!(
            VERIFICATION_SET.contains(name),
            "{name} is verified native but not in the verification set"
        );
    }
}

#[test]
fn overridden_matches_the_natives() {
    let mut installed: Vec<&str> = strudeland_host::natives::UNARY_OVERRIDES
        .iter()
        .map(|(n, _)| *n)
        .chain(
            strudeland_host::natives::BINARY_OVERRIDES
                .iter()
                .map(|(n, _)| *n),
        )
        .collect();
    installed.sort_unstable();
    let mut listed = OVERRIDDEN.to_vec();
    listed.sort_unstable();
    assert_eq!(installed, listed);
}

#[test]
fn constants_match_std_bit_for_bit() {
    use std::f64::consts;
    let expected: [(&str, f64); 8] = [
        ("E", consts::E),
        ("LN10", consts::LN_10),
        ("LN2", consts::LN_2),
        ("LOG10E", consts::LOG10_E),
        ("LOG2E", consts::LOG2_E),
        ("PI", consts::PI),
        ("SQRT1_2", consts::FRAC_1_SQRT_2),
        ("SQRT2", consts::SQRT_2),
    ];
    assert_eq!(expected.len(), CONSTANTS.len());
    for (name, value) in expected {
        assert!(CONSTANTS.contains(&name), "{name}");
        let source = format!(
            "{{ const v = new DataView(new ArrayBuffer(8)); v.setFloat64(0, Math.{name}); \
             v.getBigUint64(0).toString(16).padStart(16, '0') }}"
        );
        assert_eq!(
            eval(&source),
            format!("{:016x}", value.to_bits()),
            "Math.{name}"
        );
    }
}

#[test]
fn random_throws_and_clocks_are_zero() {
    let err = host()
        .eval_script("Math.random()")
        .expect_err("Math.random() must throw");
    assert!(err.to_string().contains(RANDOM_MESSAGE), "{err}");
    assert_eq!(eval("Date.now()"), "0");
    assert_eq!(eval("performance.now()"), "0");
}

#[derive(Deserialize)]
struct Reference {
    groups: Vec<Group>,
}

#[derive(Deserialize)]
struct Group {
    id: String,
    #[serde(rename = "fn")]
    func: String,
    xs: Vec<String>,
    ys: Option<Vec<String>>,
    out: Vec<String>,
}

/// One group evaluated in the embedded V8.
struct GroupResult {
    id: String,
    func: String,
    points: usize,
    /// `(index, x, y, expected, actual)` as bit strings.
    mismatches: Vec<(usize, String, Option<String>, String, String)>,
}

/// The JS function a reference `fn` names, or `None` for a group the host does not run
/// (`distort:*`, engine-internal).
fn js_function(func: &str, names: &[String]) -> Option<String> {
    match func {
        "powOp" => Some("(x, y) => x ** y".into()),
        "toInt32" => Some("(x) => x | 0".into()),
        "hypot" => Some("(x, y) => Math.hypot(x, y)".into()),
        _ if names.iter().any(|n| n == func) => Some(format!("Math.{func}")),
        _ => None,
    }
}

fn bits_of(hex: &str) -> u64 {
    u64::from_str_radix(hex, 16).unwrap_or_else(|e| panic!("{hex}: {e}"))
}

fn same(expected: &str, actual: &str) -> bool {
    let (e, a) = (
        f64::from_bits(bits_of(expected)),
        f64::from_bits(bits_of(actual)),
    );
    bits_of(expected) == bits_of(actual) || (e.is_nan() && a.is_nan())
}

fn results() -> &'static Vec<GroupResult> {
    static RESULTS: OnceLock<Vec<GroupResult>> = OnceLock::new();
    RESULTS.get_or_init(|| {
        let path = common::repo_root().join("crates/strudeland-engine/tests/refs/v8-math.json");
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let reference: Reference = serde_json::from_str(&text).unwrap();
        let names = math_names();
        reference
            .groups
            .iter()
            .filter_map(|g| {
                let f = js_function(&g.func, names)?;
                Some(run_group(g, &f))
            })
            .collect()
    })
}

fn run_group(g: &Group, f: &str) -> GroupResult {
    let quote = |v: &[String]| {
        v.iter()
            .map(|h| format!("'{h}'"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let ys =
        g.ys.as_deref()
            .map_or_else(|| "null".to_string(), |ys| format!("[{}]", quote(ys)));
    let source = format!(
        "{{ const v = new DataView(new ArrayBuffer(8));
           const dec = (h) => {{ v.setBigUint64(0, BigInt('0x' + h)); return v.getFloat64(0); }};
           const enc = (x) => {{ v.setFloat64(0, x); return v.getBigUint64(0).toString(16).padStart(16, '0'); }};
           const xs = [{xs}].map(dec);
           const ys = {ys};
           const f = {f};
           xs.map((x, i) => enc(ys === null ? f(x) : f(x, dec(ys[i])))).join(',') }}",
        xs = quote(&g.xs),
    );
    let text = host()
        .eval_script(&source)
        .unwrap_or_else(|e| panic!("{}: {e}", g.id));
    let actual: Vec<&str> = text.split(',').collect();
    assert_eq!(actual.len(), g.out.len(), "{}: output length", g.id);
    let mismatches = g
        .out
        .iter()
        .zip(&actual)
        .enumerate()
        .filter(|(_, (e, a))| !same(e, a))
        .map(|(i, (e, a))| {
            let y = g.ys.as_ref().map(|ys| ys[i].clone());
            (i, g.xs[i].clone(), y, e.clone(), (*a).to_string())
        })
        .collect();
    GroupResult {
        id: g.id.clone(),
        func: g.func.clone(),
        points: g.out.len(),
        mismatches,
    }
}

fn hex_value(hex: &str) -> String {
    format!("{:e}", f64::from_bits(bits_of(hex)))
}

#[test]
fn every_function_the_host_runs_matches_the_chromium_reference() {
    let results = results();
    // Every function in the verification set, overridden or operator has reference groups.
    for name in VERIFICATION_SET.iter().chain(OVERRIDDEN).chain(&OPERATORS) {
        assert!(
            results.iter().any(|r| r.func == *name),
            "no reference group runs {name}"
        );
    }
    let mut failures = Vec::new();
    for r in results {
        let total = r.mismatches.len();
        println!("{}: {}/{} match", r.id, r.points - total, r.points);
        if total == 0 {
            continue;
        }
        if r.func == "powOp"
            && let Some(reason) = POW_OP_SKIP
        {
            println!("{}: not asserted: {reason}", r.id);
            continue;
        }
        let first: Vec<String> = r
            .mismatches
            .iter()
            .take(5)
            .map(|(i, x, y, e, a)| {
                let args = match y {
                    Some(y) => format!("({}, {})", hex_value(x), hex_value(y)),
                    None => format!("({})", hex_value(x)),
                };
                format!(
                    "#{i} {}{args}: chromium {e} ({}), host {a} ({})",
                    r.func,
                    hex_value(e),
                    hex_value(a)
                )
            })
            .collect();
        failures.push(format!(
            "{} ({}): {total} of {} points differ; first: {}",
            r.id,
            class_of(&r.func).unwrap_or("operator"),
            r.points,
            first.join("; ")
        ));
    }
    if judged() {
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    } else if !failures.is_empty() {
        println!("not judged on this platform:\n{}", failures.join("\n"));
    }
}

/// Writes the verification table to `HOST_MATH_EVIDENCE` when set.
#[test]
fn writes_the_verification_table() {
    let Ok(path) = std::env::var("HOST_MATH_EVIDENCE") else {
        return;
    };
    let version = strudeland_host::v8_version();
    let results = results();
    let mut rows: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for r in results {
        let row = rows.entry(r.func.clone()).or_default();
        row.0 += 1;
        row.1 += r.points;
        row.2 += r.mismatches.len();
    }
    let mut names: Vec<String> = math_names().clone();
    for name in OPTIONAL {
        if !names.iter().any(|n| n == name) {
            names.push((*name).to_string());
        }
    }
    let mut lines = vec![
        format!(
            "Embedded V8 {version}; reference `crates/strudeland-engine/tests/refs/v8-math.json`."
        ),
        String::new(),
        "| Name | Class | Groups | Points | Mismatches | V8 |".to_string(),
        "|---|---|---|---|---|---|".to_string(),
    ];
    let row = |name: &str, class: &str| {
        let present =
            name == "powOp" || name == "toInt32" || math_names().iter().any(|n| n == name);
        match rows.get(name) {
            Some((g, p, m)) => format!("| `{name}` | {class} | {g} | {p} | {m} | {version} |"),
            None if present => {
                // Not in the reference: constants are compared with std, `random` is checked to
                // throw, exact functions are defined by the spec.
                let why = match class {
                    "constant" => "bit pattern equals `std::f64::consts`",
                    "thrown" => "throws (checked)",
                    "exact" => "exact by the spec, not compared",
                    _ => "not compared",
                };
                format!("| `{name}` | {class} | 0 | 0 | {why} | {version} |")
            }
            None => {
                format!("| `{name}` | {class} | 0 | 0 | absent in the embedded V8 | {version} |")
            }
        }
    };
    let display = |name: &str| match name {
        "powOp" => "`**` operator",
        "toInt32" => "`x \\| 0` operator",
        _ => class_of(name).unwrap_or("unclassified"),
    };
    for (class, list) in CLASSES {
        for name in list {
            lines.push(row(name, class));
        }
    }
    for name in names.iter().filter(|n| class_of(n).is_none()) {
        lines.push(row(name, "unclassified"));
    }
    for name in OPERATORS {
        lines.push(row(name, display(name)));
    }
    lines.push(String::new());
    // Relative paths are relative to the repo root, as the plan's command line gives them.
    let path = std::path::PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        common::repo_root().join(path)
    };
    std::fs::write(&path, lines.join("\n")).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("wrote {}", path.display());
}

#[test]
fn the_comparison_catches_a_wrong_bit_and_a_signed_zero() {
    // Math.sqrt(4) is 2 (0x4000000000000000); claim 0x4000000000000001, and claim +0 for -0.
    let group = Group {
        id: "self-check".into(),
        func: "sqrt".into(),
        xs: vec![
            "4010000000000000".into(),
            "8000000000000000".into(),
            "7ff8000000000000".into(),
        ],
        ys: None,
        out: vec![
            "4000000000000001".into(),
            "0000000000000000".into(),
            "7ff8000000000001".into(),
        ],
    };
    let r = run_group(&group, "Math.sqrt");
    let wrong: Vec<usize> = r.mismatches.iter().map(|m| m.0).collect();
    // Index 2: both NaN (different payloads) count as equal.
    assert_eq!(wrong, [0, 1]);
    assert_eq!(r.mismatches[0].4, "4000000000000000");
    assert_eq!(r.mismatches[1].4, "8000000000000000");
}

#[test]
fn distort_groups_are_not_run_here() {
    assert_eq!(js_function("distort:soft", math_names()), None);
    assert_eq!(
        js_function("powOp", math_names()).as_deref(),
        Some("(x, y) => x ** y")
    );
    assert_eq!(
        js_function("tan", math_names()).as_deref(),
        Some("Math.tan")
    );
}
