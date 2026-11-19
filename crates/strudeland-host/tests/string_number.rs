//! The `String(number)` differential: `String(x)` and `JSON.stringify([x])` in the embedded V8
//! against Chromium's output and the engine's `number_to_string`, over the 200 000 doubles of
//! `scripts/host/string-number-lib.ts`.
//!
//! The comparison is ignored by default: it needs the files the probe writes. Run:
//!
//! ```text
//! bun scripts/host/string-number.probe.ts
//! cargo test --release -p strudeland-host --test string_number -- --ignored --nocapture
//! ```

mod common;

use common::repo_root;
use serde_json::Value;
use strudeland_engine::number_to_string;
use strudeland_host::Host;

/// The first differences between two output lists over the same inputs.
#[derive(Debug, PartialEq)]
struct Comparison {
    count: usize,
    length_mismatch: bool,
    /// `(index, expected, actual)`.
    first: Vec<(usize, String, String)>,
}

const LIMIT: usize = 20;

fn compare(expected: &[String], actual: &[String]) -> Comparison {
    let mut count = 0;
    let mut first = Vec::new();
    for (i, (e, a)) in expected.iter().zip(actual).enumerate() {
        if e != a {
            count += 1;
            if first.len() < LIMIT {
                first.push((i, e.clone(), a.clone()));
            }
        }
    }
    Comparison {
        count,
        length_mismatch: expected.len() != actual.len(),
        first,
    }
}

fn report(label: &str, hex: &[String], c: &Comparison) -> String {
    let mut text = format!(
        "{label}: {} difference{}{}",
        c.count,
        if c.count == 1 { "" } else { "s" },
        if c.length_mismatch {
            " (length mismatch)"
        } else {
            ""
        }
    );
    for (i, e, a) in &c.first {
        let h = hex.get(*i).map_or("?", String::as_str);
        text.push_str(&format!("\n  #{i} 0x{h}: expected {e} got {a}"));
    }
    text
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} is not an array"))
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn read_json(name: &str) -> Value {
    let path = repo_root().join("target").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; run bun scripts/host/string-number.probe.ts first",
            path.display()
        )
    });
    serde_json::from_str(&text).unwrap()
}

/// Evaluates `String(x)` and `JSON.stringify([x])` for every input in the embedded V8 (with the
/// host's natives installed), in chunks to keep each script small.
fn embedded(host: &Host, hex: &[String]) -> (Vec<String>, Vec<String>) {
    let mut string = Vec::with_capacity(hex.len());
    let mut json = Vec::with_capacity(hex.len());
    for chunk in hex.chunks(20_000) {
        let list = serde_json::to_string(chunk).unwrap();
        let source = format!(
            "const dv = new DataView(new ArrayBuffer(8));
             const out = [];
             for (const h of {list}) {{
               dv.setBigUint64(0, BigInt('0x' + h));
               const x = dv.getFloat64(0);
               out.push(String(x), JSON.stringify([x]));
             }}
             JSON.stringify(out)"
        );
        let out: Vec<String> = serde_json::from_str(&host.eval_script(&source).unwrap()).unwrap();
        for pair in out.chunks(2) {
            string.push(pair[0].clone());
            json.push(pair[1].clone());
        }
    }
    (string, json)
}

fn hex_to_f64(h: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(h, 16).unwrap())
}

#[test]
#[ignore = "needs target/string-number-{inputs,chromium}.json from bun scripts/host/string-number.probe.ts"]
fn string_and_json_of_numbers_match_chromium_and_the_engine() {
    let inputs = read_json("string-number-inputs.json");
    let chromium = read_json("string-number-chromium.json");
    let hex = strings(&inputs, "hex");
    assert_eq!(hex.len(), 200_000, "the input set");
    let host = Host::new().unwrap();
    let (string, json) = embedded(&host, &hex);
    let engine_string: Vec<String> = hex
        .iter()
        .map(|h| number_to_string(hex_to_f64(h)))
        .collect();
    let engine_json: Vec<String> = hex
        .iter()
        .map(|h| {
            let x = hex_to_f64(h);
            if x.is_finite() {
                format!("[{}]", number_to_string(x))
            } else {
                "[null]".into()
            }
        })
        .collect();

    let checks = [
        (
            "String(x): Chromium vs embedded V8",
            compare(&strings(&chromium, "string"), &string),
        ),
        (
            "JSON.stringify([x]): Chromium vs embedded V8",
            compare(&strings(&chromium, "json"), &json),
        ),
        (
            "String(x): engine number_to_string vs embedded V8",
            compare(&engine_string, &string),
        ),
        (
            "JSON.stringify([x]): engine number_to_string vs embedded V8",
            compare(&engine_json, &json),
        ),
    ];
    println!(
        "string_number: {} inputs; Chromium {}; embedded V8 {}",
        hex.len(),
        chromium["chromium"].as_str().unwrap_or("?"),
        strudeland_host::v8_version(),
    );
    let mut failed = false;
    for (label, c) in &checks {
        println!("{}", report(label, &hex, c));
        failed |= c.count > 0 || c.length_mismatch;
    }
    assert!(!failed, "the outputs differ (listed above)");
}

fn list(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn compare_finds_equal_outputs() {
    let c = compare(&list(&["1", "2"]), &list(&["1", "2"]));
    assert_eq!(
        c,
        Comparison {
            count: 0,
            length_mismatch: false,
            first: vec![]
        }
    );
    assert_eq!(report("String", &[], &c), "String: 0 differences");
}

#[test]
fn compare_lists_one_difference() {
    let hex = list(&["3ff0000000000000", "4000000000000000"]);
    let c = compare(&list(&["1", "2"]), &list(&["1", "2.0"]));
    assert_eq!(c.count, 1);
    assert_eq!(c.first, vec![(1, "2".to_string(), "2.0".to_string())]);
    assert!(!c.length_mismatch);
    assert_eq!(
        report("String", &hex, &c),
        "String: 1 difference\n  #1 0x4000000000000000: expected 2 got 2.0"
    );
}

#[test]
fn compare_flags_a_length_mismatch() {
    let c = compare(&list(&["1", "2", "3"]), &list(&["1", "x"]));
    assert!(c.length_mismatch);
    assert_eq!(c.count, 1);
    assert!(report("JSON", &[], &c).starts_with("JSON: 1 difference (length mismatch)"));
}

#[test]
fn compare_keeps_the_first_twenty() {
    let expected: Vec<String> = (0..30).map(|i| i.to_string()).collect();
    let actual: Vec<String> = (0..30).map(|i| format!("{i}!")).collect();
    let c = compare(&expected, &actual);
    assert_eq!(c.count, 30);
    assert_eq!(c.first.len(), LIMIT);
}
