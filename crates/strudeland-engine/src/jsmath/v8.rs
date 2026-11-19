// Port of V8 (Chromium 156.0.8078.4, V8 b976f35) src/base/ieee754 and src/numbers/ieee754, cosh and math::pow.
// Copyright the V8 project authors. BSD-3-Clause; see LICENSE-v8.
//
// `cosh` is V8's `base::ieee754::cosh` (fdlibm lineage, see the fdlibm notice in LICENSE-v8),
// whose `exp` and `expm1` are LLVM libc's correctly rounded functions at this revision; the
// correctly rounded CORE-MATH ports give the same values. `math_pow` is V8's `math::pow`: the
// special cases ECMAScript and the optimizing compilers need, then, with `use_std_math_pow` on
// (its default on every OS but AIX), `std::pow`: the host C library's `pow` (the `host` submodule). V8 builds with `-ffp-contract=off`, so nothing here is fused.

use super::{exp::cr_exp as exp, expm1::cr_expm1 as expm1, host::host_pow};

/// V8 `base::ieee754::cosh`.
///
/// - `|x|` in `[0, ln2/2]`: `1 + expm1(|x|)^2 / (2 * exp(|x|))` (`1 + expm1(|x|)` below 2^-55);
/// - `[ln2/2, 22]`: `(exp(|x|) + 1/exp(|x|)) / 2`;
/// - `[22, log(maxdouble)]`: `exp(|x|) / 2`;
/// - up to the overflow threshold: `exp(|x|/2)/2 * exp(|x|/2)`;
/// - NaN and the infinities: `x * x`; above the threshold: `+inf`.
pub(super) fn cosh(x: f64) -> f64 {
    const KCOSH_OVERFLOW: f64 = 710.475_860_073_943_9;
    const ONE: f64 = 1.0;
    const HALF: f64 = 0.5;
    const HUGE: f64 = 1.0e300;

    // high word of |x|
    let ix = ((x.to_bits() >> 32) as u32 & 0x7FFF_FFFF) as i32;

    // |x| in [0, 0.5*log2]: 1 + expm1(|x|)^2 / (2*exp(|x|))
    if ix < 0x3FD6_2E43 {
        let t = expm1(x.abs());
        let w = ONE + t;
        // for |x| < 2^-55, cosh(x) = 1
        if ix < 0x3C80_0000 {
            return w;
        }
        return ONE + (t * t) / (w + w);
    }

    // |x| in [0.5*log2, 22]: (exp(|x|) + 1/exp(|x|)) / 2
    if ix < 0x4036_0000 {
        let t = exp(x.abs());
        return HALF * t + HALF / t;
    }

    // |x| in [22, log(maxdouble)]: exp(|x|) / 2
    if ix < 0x4086_2E42 {
        return HALF * exp(x.abs());
    }

    // |x| in [log(maxdouble), overflow threshold]
    if x.abs() <= KCOSH_OVERFLOW {
        let w = exp(HALF * x.abs());
        let t = HALF * w;
        return t * w;
    }

    // x is INF or NaN
    if ix >= 0x7FF0_0000 {
        return x * x;
    }

    // |x| > overflow threshold
    HUGE * HUGE
}

/// V8 `math::pow`: NaN `y` gives NaN; `|x| == 1` with an infinite `y` gives NaN (ECMAScript, where
/// C's `pow` gives 1); `y == 2` is `x * x` and `y == 0.5` is `sqrt(x + 0)` (`+inf` for
/// `x = -inf`), as the optimizing compilers do; everything else is the C library's `pow`.
pub(super) fn math_pow(x: f64, y: f64) -> f64 {
    if y.is_nan() {
        return f64::NAN;
    }
    if y.is_infinite() && (x == 1.0 || x == -1.0) {
        return f64::NAN;
    }
    // libm pow distinguishes quiet and signalling NaNs; JS does not
    let x = if x.is_nan() { f64::NAN } else { x };
    if y == 2.0 {
        return x * x;
    }
    if y == 0.5 {
        if x.is_infinite() {
            return f64::INFINITY;
        }
        // + 0 so that -0 ** 0.5 is +0
        return (x + 0.0).sqrt();
    }
    // v8_flags.use_std_math_pow (on for every OS but AIX): std::pow, the C library's
    host_pow(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(x: f64) -> u64 {
        x.to_bits()
    }

    #[test]
    fn cosh_branches_and_specials() {
        assert_eq!(bits(cosh(0.0)), bits(1.0));
        assert_eq!(bits(cosh(-0.0)), bits(1.0));
        assert_eq!(bits(cosh(1e-17)), bits(1.0), "below 2^-55");
        // the two sides of ln2/2 and of 22 and of log(maxdouble)
        for x in [0.34, 0.35, 21.9, 22.1, 709.7, 709.8, 710.0, 710.47] {
            let y = cosh(x);
            assert!(y.is_finite() && y >= 1.0, "cosh({x}) = {y}");
            assert_eq!(bits(cosh(-x)), bits(y), "even at {x}");
        }
        assert_eq!(cosh(710.48), f64::INFINITY);
        assert_eq!(cosh(711.0), f64::INFINITY);
        assert_eq!(cosh(f64::INFINITY), f64::INFINITY);
        assert_eq!(cosh(f64::NEG_INFINITY), f64::INFINITY);
        assert!(cosh(f64::NAN).is_nan());
        // the middle branch is (e + 1/e) / 2 on the ported exp
        let t = exp(2.0);
        assert_eq!(bits(cosh(2.0)), bits(0.5 * t + 0.5 / t));
    }

    #[test]
    fn math_pow_special_cases() {
        assert!(math_pow(2.0, f64::NAN).is_nan());
        assert!(math_pow(1.0, f64::NAN).is_nan(), "JS: 1 ** NaN is NaN");
        assert!(math_pow(1.0, f64::INFINITY).is_nan());
        assert!(math_pow(-1.0, f64::NEG_INFINITY).is_nan());
        assert!(math_pow(f64::NAN, 2.0).is_nan());
        assert_eq!(bits(math_pow(f64::NAN, 0.0)), bits(1.0));
        assert_eq!(bits(math_pow(-3.0, 2.0)), bits(9.0));
        let x = 1.000_000_1;
        assert_eq!(bits(math_pow(x, 2.0)), bits(x * x), "y = 2 is x * x");
        assert_eq!(
            bits(math_pow(2.0, 0.5)),
            bits(2.0f64.sqrt()),
            "y = 0.5 is sqrt"
        );
        assert_eq!(bits(math_pow(-0.0, 0.5)), bits(0.0), "-0 ** 0.5 is +0");
        assert_eq!(math_pow(f64::NEG_INFINITY, 0.5), f64::INFINITY);
        assert!(math_pow(-4.0, 0.5).is_nan());
        assert_eq!(bits(math_pow(3.0, 1.0)), bits(3.0));
        assert_eq!(bits(math_pow(0.0, 0.0)), bits(1.0));
    }
}
