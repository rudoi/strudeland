//! The golden wire examples (`tests/wire/`) for tests in other crates.
//!
//! Compiled for ctl's own unit tests and under the `wire-fixtures` feature, which other crates turn
//! on only as a dev-dependency, so no shipped binary carries the examples.

use serde_json::Value;

use crate::methods::Method;

/// One state-independent refusal of a method's params.
#[derive(Debug, Clone, PartialEq)]
pub struct Invalid {
    pub name: String,
    pub params: Value,
    pub code: i64,
    pub message: String,
}

/// The example file of `method`, as committed.
fn text(method: Method) -> &'static str {
    match method {
        Method::Ping => include_str!("../tests/wire/ping.json"),
        Method::State => include_str!("../tests/wire/state.json"),
        Method::CodeGet => include_str!("../tests/wire/code.get.json"),
        Method::CodeSet => include_str!("../tests/wire/code.set.json"),
        Method::Eval => include_str!("../tests/wire/eval.json"),
        Method::Play => include_str!("../tests/wire/play.json"),
        Method::Stop => include_str!("../tests/wire/stop.json"),
        Method::Events => include_str!("../tests/wire/events.json"),
        Method::Screenshot => include_str!("../tests/wire/screenshot.json"),
        Method::SongNew => include_str!("../tests/wire/song.new.json"),
        Method::SongSwitch => include_str!("../tests/wire/song.switch.json"),
        Method::SongRename => include_str!("../tests/wire/song.rename.json"),
        Method::SongDelete => include_str!("../tests/wire/song.delete.json"),
        Method::Save => include_str!("../tests/wire/save.json"),
        Method::ProjectOpen => include_str!("../tests/wire/project.open.json"),
        Method::InstrumentApply => include_str!("../tests/wire/instrument.apply.json"),
    }
}

fn example(method: Method) -> Value {
    serde_json::from_str(text(method))
        .unwrap_or_else(|e| panic!("the {method} example is not JSON: {e}"))
}

/// The `value` of the entry called `name` in the first of the example's `lists` that has one.
fn named(method: Method, lists: &[&str], kind: &str, name: &str) -> Value {
    let ex = example(method);
    lists
        .iter()
        .filter_map(|list| ex[*list].as_array())
        .flatten()
        .find(|e| e["name"] == name)
        .map(|e| e["value"].clone())
        .unwrap_or_else(|| panic!("no {kind} example '{name}' for {method}"))
}

/// The result example called `name`: a current reply from `results`, else an older app's reply
/// from `olderResults`.
pub fn result(method: Method, name: &str) -> Value {
    named(method, &["results", "olderResults"], "result", name)
}

/// The entries of the example's `list` as refusals, in file order; an absent list is empty.
fn refusals(method: Method, list: &str) -> Vec<Invalid> {
    let ex = example(method);
    let field = |e: &Value, key: &str| {
        e[key]
            .as_str()
            .unwrap_or_else(|| panic!("an {list} entry of {method} has no string '{key}'"))
            .to_string()
    };
    ex[list]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|e| Invalid {
            name: field(e, "name"),
            params: e["params"].clone(),
            code: e["code"]
                .as_i64()
                .unwrap_or_else(|| panic!("an {list} entry of {method} has no integer code")),
            message: field(e, "message"),
        })
        .collect()
}

/// The method's `invalid` refusals, answered by whoever answers the method (`answeredBy`).
pub fn invalid(method: Method) -> Vec<Invalid> {
    refusals(method, "invalid")
}

/// The method's `invalidApp` refusals: a webview method's refusals the app answers before
/// forwarding. Empty for most methods.
pub fn invalid_app(method: Method) -> Vec<Invalid> {
    refusals(method, "invalidApp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_entries_are_found() {
        assert_eq!(result(Method::Events, "truncated")["truncated"], true);
        assert_eq!(result(Method::State, "minimal")["bpm"], 120);
        assert_eq!(invalid_app(Method::ProjectOpen).len(), 1);
        assert!(invalid_app(Method::State).is_empty());
    }

    #[test]
    #[should_panic(expected = "no result example 'nope' for state")]
    fn a_missing_name_panics_naming_it() {
        result(Method::State, "nope");
    }
}
