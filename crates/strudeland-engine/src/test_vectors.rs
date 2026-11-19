//! Test helpers for the JS vectors in `tests/vectors/` (`bun run engine:vectors`).

use crate::job::Value;
use serde_json::Value as J;

/// The parsed vector file `name` (schema 1).
pub fn load(name: &str) -> J {
    let path = format!("{}/tests/vectors/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("cannot read {path}: {e}; run `bun run engine:vectors --write`")
    });
    let v: J = serde_json::from_str(&text).unwrap();
    assert_eq!(v["schema"], 1, "{name} schema");
    v
}

/// A vector member decoded as an engine value (`$num` wrappers included).
pub fn decode(v: &J) -> Value {
    Value::from_json(v.clone()).unwrap()
}

/// A vector number (`$num` wrappers included).
pub fn num(v: &J) -> f64 {
    match decode(v) {
        Value::Num(x) => x,
        other => panic!("not a number: {other:?}"),
    }
}

/// Whether `a` and `b` are at most `ulps` units in the last place apart (NaN equals NaN; signed
/// zeros are equal).
pub fn within_ulps(a: f64, b: f64, ulps: u64) -> bool {
    if a.is_nan() || b.is_nan() {
        return a.is_nan() && b.is_nan();
    }
    if a == b {
        return true;
    }
    if a.is_sign_negative() != b.is_sign_negative() {
        return false;
    }
    a.to_bits().abs_diff(b.to_bits()) <= ulps
}
