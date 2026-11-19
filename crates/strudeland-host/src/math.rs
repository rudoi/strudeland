//! The classification of every `Math` property in the host. The host README describes the
//! policy. Each own property of `Math` is in exactly one of the five lists.
//! `tests/math.rs` asserts it and compares the non-exact functions with the Chromium reference
//! (`crates/strudeland-engine/tests/refs/v8-math.json`).

/// Replaced by `jsmath` (installed by the natives: [`crate::natives::UNARY_OVERRIDES`] and
/// [`crate::natives::BINARY_OVERRIDES`]).
pub const OVERRIDDEN: &[&str] = &["sin", "cos", "tanh", "exp", "expm1", "log1p", "cosh", "pow"];

/// Throws (patterns must be deterministic).
pub const THROWN: &[&str] = &["random"];

/// The constants; the test compares each bit pattern with `std::f64::consts`.
pub const CONSTANTS: &[&str] = &[
    "E", "LN10", "LN2", "LOG10E", "LOG2E", "PI", "SQRT1_2", "SQRT2",
];

/// Functions whose result is the unique value the language spec defines (IEEE-754 correctly
/// rounded or integer-defined); not verified. `f16round` and `sumPrecise` may be absent from
/// the embedded V8.
pub const EXACT: &[&str] = &[
    "abs",
    "ceil",
    "floor",
    "trunc",
    "sign",
    "sqrt",
    "fround",
    "f16round",
    "clz32",
    "imul",
    "max",
    "min",
    "round",
    "sumPrecise",
];

/// The names of [`EXACT`] the embedded V8 need not have.
pub const OPTIONAL: &[&str] = &["f16round", "sumPrecise"];

/// Native functions that match the Chromium reference bit for bit on every reference point in
/// the embedded V8.
pub const VERIFIED_NATIVE: &[&str] = &[
    "tan", "asin", "acos", "atan", "atan2", "sinh", "asinh", "acosh", "atanh", "log", "log2",
    "log10", "cbrt", "hypot",
];
