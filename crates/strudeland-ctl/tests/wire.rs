//! The golden wire examples (`tests/wire/`) against the typed method table: every params and
//! result example round-trips through its method's types, the refusals are well formed, and
//! `errors.json` and `limits.json` equal the crate's constants.
//!
//! A `<method>.json` holds `params`, `results` and `invalid`, plus two optional lists:
//! `olderResults` (an older app's replies, with fields missing: they decode, and every key they
//! have serializes back equal) and `invalidApp` (refusals of a webview-answered method that the
//! app answers before forwarding; `control.rs` tests them). A method the app answers itself
//! (`answeredBy: "app"`) keeps all its refusals in `invalid`.
//!
//! Paths in the examples (`project.open`'s `/p` and `/empty`, `screenshot`'s `/tmp/a.png`) are
//! Unix-style: on Windows the app refuses them as not absolute before the webview sees them, so
//! no Rust test feeds an example path to a native absolute-path check. App-side tests that need
//! one that passes use a platform-absolute path (`std::env::temp_dir()`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use strudeland_ctl::*;

fn dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/wire"))
}

fn read(file: &str) -> Value {
    let path = dir().join(file);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{file}: {e}"))
}

/// JSON equality with numbers compared as `f64`, so the app's `120` equals Rust's `120.0`.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(v, w)| same(v, w))
        }
        _ => a == b,
    }
}

#[test]
fn same_compares_numbers_by_value() {
    assert!(same(&json!(120), &json!(120.0)));
    assert!(!same(&json!(1), &json!(2)));
    assert!(!same(&json!({}), &json!({"a": 1})));
    assert!(!same(&json!([1]), &json!([1, 1])));
    assert!(same(&json!({"a": [1, null]}), &json!({"a": [1.0, null]})));
    assert!(!same(&json!("1"), &json!(1)));
}

#[test]
fn one_file_per_method_plus_errors_and_limits() {
    let mut want: BTreeSet<String> = Method::ALL
        .iter()
        .map(|m| format!("{}.json", m.as_str()))
        .collect();
    want.insert("errors.json".into());
    want.insert("limits.json".into());
    let have: BTreeSet<String> = fs::read_dir(dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|f| f.ends_with(".json"))
        .collect();
    assert_eq!(have, want);
    for m in Method::ALL {
        let ex = read(&format!("{}.json", m.as_str()));
        assert_eq!(ex["method"], m.as_str());
        let by = if m.answered_by_app() {
            "app"
        } else {
            "webview"
        };
        assert_eq!(ex["answeredBy"], by, "{m}");
    }
}

#[test]
fn every_example_round_trips() {
    for m in Method::ALL {
        let ex = read(&format!("{}.json", m.as_str()));
        check(m, &ex);
    }
}

/// Every method's examples through its own marker; no `_` arm, so a new method must be added.
fn check(m: Method, ex: &Value) {
    match m {
        Method::Ping => check_rpc::<Ping>(m, ex),
        Method::State => check_rpc::<StateCall>(m, ex),
        Method::CodeGet => check_rpc::<CodeGet>(m, ex),
        Method::CodeSet => check_rpc::<CodeSet>(m, ex),
        Method::Eval => check_rpc::<EvalCall>(m, ex),
        Method::Play => check_rpc::<Play>(m, ex),
        Method::Stop => check_rpc::<Stop>(m, ex),
        Method::Events => check_rpc::<EventsCall>(m, ex),
        Method::Screenshot => check_rpc::<ScreenshotCall>(m, ex),
        Method::SongNew => check_rpc::<SongNewCall>(m, ex),
        Method::SongSwitch => check_rpc::<SongSwitchCall>(m, ex),
        Method::SongRename => check_rpc::<SongRenameCall>(m, ex),
        Method::SongDelete => check_rpc::<SongDeleteCall>(m, ex),
        Method::Save => check_rpc::<SaveCall>(m, ex),
        Method::ProjectOpen => check_rpc::<ProjectOpenCall>(m, ex),
        Method::InstrumentApply => check_rpc::<InstrumentApply>(m, ex),
    }
}

/// The entries of `list`, with their names checked unique.
fn entries<'a>(m: Method, ex: &'a Value, list: &str) -> &'a [Value] {
    let entries = ex[list]
        .as_array()
        .unwrap_or_else(|| panic!("{m}: no {list} list"));
    unique(m, list, entries)
}

/// The entries of `list` when the file has it, else none, with their names checked unique.
fn optional<'a>(m: Method, ex: &'a Value, list: &str) -> &'a [Value] {
    match ex.get(list) {
        None => &[],
        Some(_) => entries(m, ex, list),
    }
}

/// `entries`, with their names checked present and unique.
fn unique<'a>(m: Method, list: &str, entries: &'a [Value]) -> &'a [Value] {
    let mut names = BTreeSet::new();
    for e in entries {
        let name = e["name"]
            .as_str()
            .unwrap_or_else(|| panic!("{m}: a {list} entry has no name"));
        assert!(names.insert(name), "{m}: {list} name '{name}' is repeated");
    }
    entries
}

fn check_rpc<M: Rpc>(m: Method, ex: &Value) {
    assert_eq!(M::METHOD, m);
    for p in entries(m, ex, "params") {
        let name = p["name"].as_str().unwrap_or_default();
        let value = &p["value"];
        let typed: M::Params = serde_json::from_value(value.clone())
            .unwrap_or_else(|e| panic!("{m} params '{name}': {e}"));
        let back = serde_json::to_value(&typed).unwrap();
        assert!(
            same(value, &back),
            "{m} params '{name}': {value} came back as {back}"
        );
    }
    let results = entries(m, ex, "results");
    assert!(!results.is_empty(), "{m}: no results");
    for r in results {
        let name = r["name"].as_str().unwrap_or_default();
        let value = &r["value"];
        let back = result_back::<M>(m, name, value);
        assert!(
            same(value, &back),
            "{m} result '{name}': {value} came back as {back}"
        );
    }
    let older = optional(m, ex, "olderResults");
    for r in older {
        let name = r["name"].as_str().unwrap_or_default();
        let value = &r["value"];
        let back = result_back::<M>(m, name, value);
        let (Value::Object(old), Value::Object(new)) = (value, &back) else {
            panic!("{m} older result '{name}': an older reply is an object");
        };
        for (k, v) in old {
            assert!(
                new.get(k).is_some_and(|w| same(v, w)),
                "{m} older result '{name}': '{k}' came back as {:?}",
                new.get(k)
            );
        }
    }
    unique(m, "results and olderResults", &[results, older].concat());
    let invalid = entries(m, ex, "invalid");
    let app = optional(m, ex, "invalidApp");
    assert!(
        app.is_empty() || !m.answered_by_app(),
        "{m}: the app answers the method, so its refusals belong in invalid"
    );
    check_refusals(m, invalid, "invalid");
    check_refusals(m, app, "invalidApp");
    unique(m, "invalid and invalidApp", &[invalid, app].concat());
}

/// `value` decoded as the method's result and serialized again.
fn result_back<M: Rpc>(m: Method, name: &str, value: &Value) -> Value {
    let typed: M::Result = serde_json::from_value(value.clone())
        .unwrap_or_else(|e| panic!("{m} result '{name}': {e}"));
    serde_json::to_value(&typed).unwrap()
}

fn check_refusals(m: Method, list: &[Value], kind: &str) {
    for i in list {
        let name = i["name"].as_str().unwrap_or_default();
        assert_eq!(i["code"], codes::INVALID_PARAMS, "{m} {kind} '{name}'");
        let message = i["message"].as_str().unwrap_or_default();
        assert!(
            message.starts_with("invalid params: "),
            "{m} {kind} '{name}': {message}"
        );
        assert!(!i["params"].is_null(), "{m} {kind} '{name}': null params");
    }
}

#[test]
fn errors_json_is_every_code() {
    let file: BTreeMap<String, i64> = serde_json::from_value(read("errors.json")).unwrap();
    let all: BTreeMap<String, i64> = codes::ALL
        .iter()
        .map(|&(name, code)| (name.to_string(), code))
        .collect();
    assert_eq!(all.len(), codes::ALL.len(), "a repeated name in codes::ALL");
    let values: BTreeSet<i64> = codes::ALL.iter().map(|&(_, code)| code).collect();
    assert_eq!(
        values.len(),
        codes::ALL.len(),
        "a repeated value in codes::ALL"
    );
    assert_eq!(file, all);
}

#[test]
fn limits_json_is_the_events_limits() {
    assert_eq!(
        read("limits.json"),
        json!({
            "eventsLimitDefault": EVENTS_LIMIT_DEFAULT,
            "eventsLimitMax": EVENTS_LIMIT_MAX,
        })
    );
}

/// `state`'s `full` sets every optional field, so a reader of it sees each one's shape.
#[test]
fn full_state_sets_every_optional_field() {
    let ex = read("state.json");
    let full = entries(Method::State, &ex, "results")
        .iter()
        .find(|r| r["name"] == "full")
        .expect("state has a full result");
    let s: AppState = serde_json::from_value(full["value"].clone()).unwrap();
    assert!(s.project.is_some());
    assert!(s.active.is_some());
    assert!(s.playing.is_some());
    assert!(s.eval_error.is_some());
    assert!(s.meta_error.is_some());
    assert!(s.seed.is_some());
    assert!(s.output_error.is_some());
    assert!(s.live.is_some());
    assert!(s.vim);
    assert_eq!(s.engine, EngineId::Rust);
    let playhead = s.playhead.expect("playhead");
    assert!(playhead.song_bar.is_some());
    assert!(playhead.section.is_some());
    assert!(s.transport.loop_section.is_some());
    assert!(s.transport.loop_through.is_some());
    assert!(s.transport.armed_section.is_some());
    assert!(s.songs.iter().any(|song| song.conflict.is_some()));
    let levels = s.levels.expect("levels");
    assert!(levels.peak_db.iter().any(Option::is_none));
    assert!(levels.peak_db.iter().any(Option::is_some));
}
