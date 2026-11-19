//! The host C library's functions V8 delegates to at the pinned revision: `pow`, the function
//! V8's `math::pow` ends in (`use_std_math_pow` is on for every OS but AIX, so Chromium calls the
//! libc symbol `pow`), and `tanh` (`Math.tanh` is `std::tanh`). Declared directly against those
//! symbols, not through Rust's `f64::powf`/`f64::tanh`, so the engine calls what Chromium calls on
//! the same host (Apple's libm on macOS, the reference's host). Apple's `tanh` is not correctly
//! rounded everywhere (`plans/0011-engine-effects/evidence/v8-math-tanh-distort.json`).

unsafe extern "C" {
    #[link_name = "pow"]
    fn libc_pow(x: f64, y: f64) -> f64;
    #[link_name = "tanh"]
    fn libc_tanh(x: f64) -> f64;
}

/// The C library's `pow(x, y)`.
pub(super) fn host_pow(x: f64, y: f64) -> f64 {
    // SAFETY: `pow` is a pure C function of two doubles with no preconditions.
    unsafe { libc_pow(x, y) }
}

/// The C library's `tanh(x)`.
pub(super) fn host_tanh(x: f64) -> f64 {
    // SAFETY: `tanh` is a pure C function of one double with no preconditions.
    unsafe { libc_tanh(x) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_cases_of_the_c_library() {
        assert_eq!(host_pow(2.0, 10.0).to_bits(), 1024.0f64.to_bits());
        assert_eq!(host_pow(0.0, 0.0), 1.0);
        assert_eq!(
            host_pow(1.0, f64::NAN),
            1.0,
            "C: pow(1, NaN) is 1 (V8 corrects it)"
        );
        assert_eq!(host_pow(-8.0, 3.0), -512.0);
        assert!(host_pow(-8.0, 1.0 / 3.0).is_nan());
        assert_eq!(host_pow(0.0, -1.0), f64::INFINITY);
        assert_eq!(host_pow(2.0, -1075.0).to_bits(), 0);
    }

    #[test]
    fn exact_tanh_cases_of_the_c_library() {
        assert_eq!(host_tanh(0.0).to_bits(), 0);
        assert_eq!(host_tanh(-0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(host_tanh(40.0), 1.0);
        assert_eq!(host_tanh(f64::NEG_INFINITY), -1.0);
        assert!(host_tanh(f64::NAN).is_nan());
        assert_eq!(host_tanh(-1.5), -host_tanh(1.5));
    }
}
