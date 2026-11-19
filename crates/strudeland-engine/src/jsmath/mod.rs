//! JavaScript math as superdough's AudioWorklet processors see it in Chromium's V8, bit for bit.
//!
//! At the V8 Chromium 156.0.8078.4 pins (`b976f35`), `Math.sin cos exp expm1 log1p` are LLVM
//! libc (as is V8's `legacy::pow`, which `Math.pow` does not use), `Math.cosh` is V8's own code
//! on those, `Math.tanh` is the host's `std::tanh`, and `Math.pow` is V8's `math::pow` special
//! cases in front of the host's `std::pow` (`use_std_math_pow`). V8's values are correctly rounded
//! for `sin cos exp expm1 log1p` on every point of the V8 math reference
//! (`tests/refs/v8-math.json`, evaluated in Chromium by `bun run engine:refs`), so:
//!
//! - `sin cos exp expm1 log1p`: correctly rounded ports of CORE-MATH (`sin`, `cos`, `exp`,
//!   `expm1`, `log1p` submodules);
//! - `pow`: V8's `math::pow` special cases (`v8`) in front of the host C library's `pow` (`host`),
//!   as V8 does (`use_std_math_pow`): host-dependent in Chromium itself, so it matches Chromium on
//!   the same host (bit-exact on macOS, the reference's host);
//! - `tanh`: the host C library's `tanh` (`host`), as V8 calls it; macOS's is not correctly
//!   rounded everywhere, so a correctly rounded port misses V8 there;
//! - `cosh`: a port of V8's `cosh` (`v8`) on the ported `exp` and `expm1`.
//!
//! Rust `std`'s transcendentals are never used (the host functions are declared against the C
//! symbols); `tests/v8_math.rs` proves every
//! function against the reference and records `libm`'s and `std`'s match rates for the record.
//!
//! The rest is exact JS semantics: `Math.round` in V8's form, `ToInt32` (`x | 0`), the
//! fractional parts the processors use, and `Math.min`/`Math.max`, which propagate NaN and order
//! `-0 < +0` (Rust's `f64::min`/`max` return the other argument for a NaN). `sqrt`, `abs`,
//! `floor`, `ceil`, `trunc`, `mul_add` (fused, exact) and `%` are IEEE-exact and are used directly
//! as the `f64` methods.

mod cos;
mod exp;
mod expm1;
mod host;
mod log1p;
mod sin;
mod v8;

/// `(function, implementation)` for the transcendentals: `"core-math"` (a correctly rounded
/// port), `"v8"` (a port of V8's own code), `"host"` (the C library's function, as V8 calls it) or
/// `"libm"` (where the `libm` crate already matches every vector), as decided from
/// `plans/0011-engine-effects/evidence/v8-math-choice.json`. `std` is never an entry.
pub const IMPLEMENTATION: &[(&str, &str)] = &[
    ("sin", "core-math"),
    ("cos", "core-math"),
    ("tanh", "host"),
    ("exp", "core-math"),
    ("expm1", "core-math"),
    ("log1p", "core-math"),
    ("cosh", "v8"),
    ("pow", "host"),
];

/// `Math.sin`.
pub fn sin(x: f64) -> f64 {
    sin::cr_sin(x)
}

/// `Math.cos`.
pub fn cos(x: f64) -> f64 {
    cos::cr_cos(x)
}

/// `Math.tanh`.
pub fn tanh(x: f64) -> f64 {
    host::host_tanh(x)
}

/// `Math.exp`.
pub fn exp(x: f64) -> f64 {
    exp::cr_exp(x)
}

/// `Math.expm1`.
pub fn expm1(x: f64) -> f64 {
    expm1::cr_expm1(x)
}

/// `Math.log1p`.
pub fn log1p(x: f64) -> f64 {
    log1p::cr_log1p(x)
}

/// `Math.cosh`.
pub fn cosh(x: f64) -> f64 {
    v8::cosh(x)
}

/// `Math.pow(x, y)` and `x ** y`.
pub fn pow(x: f64, y: f64) -> f64 {
    v8::math_pow(x, y)
}

/// `Math.round` in V8's form: `ceil(x) - (ceil(x) - 0.5 > x ? 1 : 0)`. Halves go toward `+inf`
/// (`round(-2.5)` is `-2`), `round(-0.5)` and `round(-0.4)` are `-0`, and NaN, the infinities and
/// `|x| >= 2^52` come back unchanged.
pub fn round(x: f64) -> f64 {
    let c = x.ceil();
    c - if c - 0.5 > x { 1.0 } else { 0.0 }
}

/// `x | 0` (ECMAScript `ToInt32`): NaN and the infinities give 0; otherwise truncate toward zero
/// and reduce modulo `2^32` into the `i32` range.
pub fn to_int32(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    // `trunc` and `rem_euclid` are exact on integers; the result is in [0, 2^32).
    let m = x.trunc().rem_euclid(4_294_967_296.0);
    (m as u32) as i32
}

/// `x - (x | 0)` (worklets.mjs `ffrac`): negative for a negative non-integer `x`.
pub fn ffrac(x: f64) -> f64 {
    x - f64::from(to_int32(x))
}

/// `x - Math.floor(x)`: in `[0, 1)` for finite `x`.
pub fn frac(x: f64) -> f64 {
    x - x.floor()
}

/// `Math.min(a, b)`: NaN when either is NaN; `min(-0, +0)` is `-0`.
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}

/// `Math.max(a, b)`: NaN when either is NaN; `max(-0, +0)` is `+0`.
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_positive() { a } else { b }
    } else if a > b {
        a
    } else {
        b
    }
}

/// superdough's `clamp(x, lo, hi)`: `Math.min(Math.max(x, lo), hi)`, so a NaN anywhere gives NaN.
pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    min(max(x, lo), hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(x: f64) -> u64 {
        x.to_bits()
    }

    fn same(a: f64, b: f64) -> bool {
        (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
    }

    /// The identities the callers skip `pow` on: `x ** 1` is `x` bit for bit (the lfo's default
    /// curve) and `10 ** ±0` is 1 (the transient's 0 dB).
    #[test]
    fn pow_identities_callers_rely_on() {
        for x in [
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.3,
            -0.7,
            5e-324,
            -2.2250738585072014e-308,
            1e300,
            f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert_eq!(bits(pow(x, 1.0)), bits(x), "{x:e} ** 1");
        }
        let mut r = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..10_000 {
            r ^= r << 13;
            r ^= r >> 7;
            r ^= r << 17;
            let x = f64::from_bits(r);
            if !x.is_nan() {
                assert_eq!(bits(pow(x, 1.0)), bits(x), "{x:e} ** 1");
            }
        }
        assert_eq!(bits(pow(10.0, 0.0)), bits(1.0));
        assert_eq!(bits(pow(10.0, -0.0)), bits(1.0));
    }

    #[test]
    fn round_takes_halves_toward_positive_infinity() {
        for (x, want) in [
            (2.5, 3.0),
            (-2.5, -2.0),
            (0.5, 1.0),
            (1.5, 2.0),
            (-1.5, -1.0),
            (0.49999999999999994, 0.0),
            (-0.49999999999999994, -0.0),
            (2.4, 2.0),
            (-2.6, -3.0),
        ] {
            assert_eq!(bits(round(x)), bits(want), "round({x})");
        }
    }

    #[test]
    fn round_keeps_signed_zeros_specials_and_large_values() {
        assert_eq!(bits(round(-0.5)), bits(-0.0), "round(-0.5) is -0");
        assert_eq!(bits(round(-0.4)), bits(-0.0), "round(-0.4) is -0");
        assert_eq!(bits(round(0.0)), bits(0.0));
        assert_eq!(bits(round(-0.0)), bits(-0.0));
        assert_eq!(bits(round(0.3)), bits(0.0), "round(0.3) is +0");
        assert!(round(f64::NAN).is_nan());
        assert_eq!(round(f64::INFINITY), f64::INFINITY);
        assert_eq!(round(f64::NEG_INFINITY), f64::NEG_INFINITY);
        let p52 = 4_503_599_627_370_496.0;
        assert_eq!(bits(round(p52)), bits(p52), "2^52");
        assert_eq!(bits(round(p52 + 1.0)), bits(p52 + 1.0), "2^52 + 1");
        assert_eq!(bits(round(-p52)), bits(-p52), "-2^52");
        // the largest double below 2^52 with a half: 2^52 - 0.5 rounds up
        assert_eq!(bits(round(p52 - 0.5)), bits(p52), "2^52 - 0.5");
    }

    #[test]
    fn to_int32_truncates_and_wraps() {
        let two32 = 4_294_967_296.0;
        let two31 = 2_147_483_648.0;
        assert_eq!(to_int32(two32 + 3.0), 3);
        assert_eq!(to_int32(two32 + 1.0), 1);
        assert_eq!(to_int32(-1.5), -1);
        assert_eq!(to_int32(1.9), 1);
        assert_eq!(to_int32(-1.9), -1);
        assert_eq!(to_int32(two31), i32::MIN);
        assert_eq!(to_int32(-two31), i32::MIN);
        assert_eq!(to_int32(-two31 - 1.0), i32::MAX);
        assert_eq!(to_int32(two31 - 1.0), i32::MAX);
        assert_eq!(to_int32(2_147_483_647.9), i32::MAX);
        assert_eq!(to_int32(two32), 0);
        assert_eq!(to_int32(-two32), 0);
        assert_eq!(to_int32(9_007_199_254_740_992.0), 0, "2^53");
        assert_eq!(to_int32(1e21), -559_939_584, "1e21 mod 2^32");
        assert_eq!(to_int32(-1e21), 559_939_584);
        assert_eq!(to_int32(-0.0), 0);
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_int32(f64::INFINITY), 0);
        assert_eq!(to_int32(f64::NEG_INFINITY), 0);
        assert_eq!(to_int32(f64::MAX), 0);
    }

    #[test]
    fn ffrac_and_frac() {
        assert_eq!(bits(ffrac(-1.5)), bits(-0.5));
        assert_eq!(bits(ffrac(2.75)), bits(0.75));
        assert_eq!(bits(ffrac(-0.25)), bits(-0.25));
        assert_eq!(bits(ffrac(3.0)), bits(0.0));
        assert!(ffrac(f64::NAN).is_nan());
        // beyond i32, `x | 0` wraps, so ffrac is not a fractional part
        assert_eq!(ffrac(4_294_967_296.0 + 0.5), 4_294_967_296.5);
        assert_eq!(bits(frac(-0.25)), bits(0.75));
        assert_eq!(bits(frac(2.75)), bits(0.75));
        assert_eq!(bits(frac(-2.0)), bits(0.0));
        assert!(frac(f64::INFINITY).is_nan());
    }

    #[test]
    fn min_max_propagate_nan_in_either_position() {
        for (a, b) in [(f64::NAN, 1.0), (1.0, f64::NAN), (f64::NAN, f64::NAN)] {
            assert!(min(a, b).is_nan(), "min({a}, {b})");
            assert!(max(a, b).is_nan(), "max({a}, {b})");
        }
        assert_eq!(min(1.0, 2.0), 1.0);
        assert_eq!(max(1.0, 2.0), 2.0);
        assert_eq!(min(f64::NEG_INFINITY, 0.0), f64::NEG_INFINITY);
        assert_eq!(max(f64::NEG_INFINITY, 0.0), 0.0);
    }

    #[test]
    fn min_max_order_signed_zeros() {
        assert_eq!(bits(min(-0.0, 0.0)), bits(-0.0));
        assert_eq!(bits(min(0.0, -0.0)), bits(-0.0));
        assert_eq!(bits(max(-0.0, 0.0)), bits(0.0));
        assert_eq!(bits(max(0.0, -0.0)), bits(0.0));
        assert_eq!(bits(min(0.0, 0.0)), bits(0.0));
        assert_eq!(bits(max(-0.0, -0.0)), bits(-0.0));
    }

    #[test]
    fn clamp_is_min_of_max() {
        assert_eq!(clamp(5.0, 0.0, 1.0), 1.0);
        assert_eq!(clamp(-5.0, 0.0, 1.0), 0.0);
        assert_eq!(clamp(0.5, 0.0, 1.0), 0.5);
        for (x, lo, hi) in [
            (f64::NAN, 0.0, 1.0),
            (0.5, f64::NAN, 1.0),
            (0.5, 0.0, f64::NAN),
        ] {
            assert!(clamp(x, lo, hi).is_nan(), "clamp({x}, {lo}, {hi})");
        }
        // lo > hi: hi wins (JS order)
        assert_eq!(clamp(0.5, 2.0, 1.0), 1.0);
        assert_eq!(bits(clamp(-0.0, 0.0, 1.0)), bits(0.0));
    }

    const GUARD: [f64; 10] = [
        0.0,
        -0.0,
        0.5,
        -0.75,
        1.0,
        2.5,
        -3.25,
        10.0,
        1e-8,
        0.123_456_789,
    ];

    fn guard(name: &str, ours: fn(f64) -> f64, port: fn(f64) -> f64) {
        for x in GUARD {
            assert!(same(ours(x), port(x)), "{name}({x})");
        }
    }

    fn chosen(name: &str) -> &'static str {
        IMPLEMENTATION
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, i)| *i)
            .unwrap_or_else(|| panic!("{name} has no IMPLEMENTATION entry"))
    }

    #[test]
    fn implementation_lists_the_transcendentals_once_never_std() {
        let names: Vec<&str> = IMPLEMENTATION.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            ["sin", "cos", "tanh", "exp", "expm1", "log1p", "cosh", "pow"]
        );
        for (n, i) in IMPLEMENTATION {
            assert!(["libm", "core-math", "v8", "host"].contains(i), "{n}: {i}");
        }
    }

    #[test]
    fn each_function_is_its_chosen_port() {
        // a guard against a wrong re-export; tests/v8_math.rs is the bit-for-bit proof
        assert_eq!(chosen("sin"), "core-math");
        guard("sin", sin, sin::cr_sin);
        assert_eq!(chosen("cos"), "core-math");
        guard("cos", cos, cos::cr_cos);
        assert_eq!(chosen("tanh"), "host");
        guard("tanh", tanh, host::host_tanh);
        assert_eq!(chosen("exp"), "core-math");
        guard("exp", exp, exp::cr_exp);
        assert_eq!(chosen("expm1"), "core-math");
        guard("expm1", expm1, expm1::cr_expm1);
        assert_eq!(chosen("log1p"), "core-math");
        guard("log1p", log1p, log1p::cr_log1p);
        assert_eq!(chosen("cosh"), "v8");
        guard("cosh", cosh, v8::cosh);
        assert_eq!(chosen("pow"), "host");
        for y in [3.0, 1.5, 1.0, -3.0, 0.25] {
            for x in GUARD {
                assert!(same(pow(x, y), host::host_pow(x, y)), "pow({x}, {y})");
            }
        }
        // V8's special cases sit in front of the C library's pow
        assert_eq!(bits(pow(3.0, 2.0)), bits(9.0));
        assert!(pow(1.0, f64::INFINITY).is_nan());
    }

    #[test]
    fn the_ports_agree_with_known_values() {
        // exact results every correct implementation gives
        assert_eq!(bits(exp(0.0)), bits(1.0));
        assert_eq!(bits(expm1(-0.0)), bits(-0.0));
        assert_eq!(bits(log1p(0.0)), bits(0.0));
        assert_eq!(log1p(-1.0), f64::NEG_INFINITY);
        assert!(log1p(-2.0).is_nan());
        assert_eq!(bits(sin(-0.0)), bits(-0.0));
        assert_eq!(bits(cos(0.0)), bits(1.0));
        assert_eq!(bits(tanh(-0.0)), bits(-0.0));
        assert_eq!(bits(tanh(40.0)), bits(1.0));
        assert_eq!(exp(1000.0), f64::INFINITY);
        assert_eq!(bits(exp(-1000.0)), bits(0.0));
        assert!(sin(f64::INFINITY).is_nan());
        assert_eq!(bits(pow(2.0, 10.0)), bits(1024.0));
    }
}
