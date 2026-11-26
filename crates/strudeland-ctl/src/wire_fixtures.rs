//! The golden wire examples (`tests/wire/`) for tests in other crates.
//!
//! Compiled for ctl's own unit tests and under the `wire-fixtures` feature, which other crates turn
//! on only as a dev-dependency, so no shipped binary carries the examples.

use serde_json::Value;

use crate::methods::Method;

/// `errors.json`: every error code by its Rust name.
pub const ERRORS: &str = include_str!("../tests/wire/errors.json");
/// `limits.json`: `eventsLimitDefault` and `eventsLimitMax`.
pub const LIMITS: &str = include_str!("../tests/wire/limits.json");

/// One state-independent refusal of a method's params.
#[derive(Debug, Clone, PartialEq)]
pub struct Invalid {
    pub name: String,
    pub params: Value,
    pub code: i64,
    pub message: String,
    /// The method's `answeredBy` unless the entry overrides it.
    pub answered_by: String,
}

/// The example file of `method`, as committed.
pub fn text(method: Method) -> &'static str {
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

/// The `value` of the entry called `name` in the example's `list` (`params` or `results`).
fn named(method: Method, list: &str, kind: &str, name: &str) -> Value {
    let ex = example(method);
    ex[list]
        .as_array()
        .and_then(|list| list.iter().find(|e| e["name"] == name))
        .map(|e| e["value"].clone())
        .unwrap_or_else(|| panic!("no {kind} example '{name}' for {method}"))
}

/// The params example called `name`.
pub fn params(method: Method, name: &str) -> Value {
    named(method, "params", "params", name)
}

/// The result example called `name`.
pub fn result(method: Method, name: &str) -> Value {
    named(method, "results", "result", name)
}

/// Every invalid entry of the method, in file order.
pub fn invalid(method: Method) -> Vec<Invalid> {
    let ex = example(method);
    let default = ex["answeredBy"].as_str().unwrap_or_default().to_string();
    let field = |e: &Value, key: &str| {
        e[key]
            .as_str()
            .unwrap_or_else(|| panic!("an invalid entry of {method} has no string '{key}'"))
            .to_string()
    };
    ex["invalid"]
        .as_array()
        .unwrap_or_else(|| panic!("the {method} example has no invalid list"))
        .iter()
        .map(|e| Invalid {
            name: field(e, "name"),
            params: e["params"].clone(),
            code: e["code"]
                .as_i64()
                .unwrap_or_else(|| panic!("an invalid entry of {method} has no integer code")),
            message: field(e, "message"),
            answered_by: e["answeredBy"].as_str().unwrap_or(&default).to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_entries_are_found() {
        assert_eq!(params(Method::Events, "bars")["limit"], 10);
        let app: Vec<_> = invalid(Method::ProjectOpen)
            .into_iter()
            .filter(|e| e.answered_by == "app")
            .collect();
        assert_eq!(app.len(), 1, "{app:?}");
    }

    #[test]
    #[should_panic(expected = "no result example 'nope' for state")]
    fn a_missing_name_panics_naming_it() {
        result(Method::State, "nope");
    }
}
