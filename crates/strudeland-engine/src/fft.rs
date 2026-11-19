// Port of Blink (Chromium 156.0.8078.4) platform/audio/rustfft_ffi.rs, platform/audio/fft_frame.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! `FFTFrame` as Blink builds it at this tag: a real FFT of even size `N` done with a half-size
//! complex `rustfft` transform plus Blink's own packing (`rustfft_ffi.rs`), with the DC bin in
//! `real[0]` and the Nyquist bin in `imag[0]`. Chromium vendors the same `rustfft` (6.4.1, default
//! features, so its NEON planner on arm64) and `num-complex` (0.4.6) as this crate pins, and Rust
//! never contracts float operations, so the transforms are bit-identical to Chromium's.
//!
//! Used by the periodic-wave tables (`DoInverseFFT`); the convolver will use `DoFFT` too.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// A real FFT frame of even size (`FFTFrame` with Blink's `RustFft` even strategy).
pub struct FftFrame {
    size: usize,
    half_size: usize,
    limit: usize,
    middle_index: Option<usize>,
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    twiddles: Vec<Complex<f32>>,
    complex: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    scale: f32,
    /// Packed real parts: `real[0]` is DC, `real[k]` bin `k` for `1 <= k < size / 2`.
    pub real: Vec<f32>,
    /// Packed imaginary parts: `imag[0]` is the (real) Nyquist bin.
    pub imag: Vec<f32>,
}

impl std::fmt::Debug for FftFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FftFrame")
            .field("size", &self.size)
            .finish()
    }
}

impl FftFrame {
    /// `FFTFrame(size)` for an even `size >= 2` (`None` otherwise: Blink's odd strategy is not
    /// ported, the engine only uses powers of two).
    pub fn new(size: usize) -> Option<FftFrame> {
        if size < 2 || !size.is_multiple_of(2) {
            return None;
        }
        let half_size = size / 2;
        let mut planner = FftPlanner::new();
        let forward = planner.plan_fft_forward(half_size);
        let inverse = planner.plan_fft_inverse(half_size);
        let scratch_len = forward
            .get_inplace_scratch_len()
            .max(inverse.get_inplace_scratch_len());
        let twiddles = (0..half_size)
            .map(|k| {
                let angle = -2.0 * std::f64::consts::PI * (k as f64) / (size as f64);
                let (sin, cos) = angle.sin_cos();
                Complex::new((0.5 * cos) as f32, (0.5 * sin) as f32)
            })
            .collect();
        Some(FftFrame {
            size,
            half_size,
            limit: (half_size - 1) / 2,
            middle_index: half_size.is_multiple_of(2).then_some(half_size / 2),
            forward,
            inverse,
            twiddles,
            complex: vec![Complex::new(0.0, 0.0); half_size],
            scratch: vec![Complex::new(0.0, 0.0); scratch_len],
            scale: 1.0 / (half_size as f32),
            real: vec![0.0; half_size],
            imag: vec![0.0; half_size],
        })
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// `DoFFT(data)`: the forward transform of `data` (`size` samples) into `real`/`imag`.
    pub fn do_fft(&mut self, data: &[f32]) {
        assert_eq!(data.len(), self.size, "do_fft: wrong input length");
        for (c, pair) in self.complex.iter_mut().zip(data.chunks_exact(2)) {
            *c = Complex::new(pair[0], pair[1]);
        }
        self.forward
            .process_with_scratch(&mut self.complex, &mut self.scratch);
        let z = &self.complex;
        let half = self.half_size;
        self.real[0] = z[0].re + z[0].im;
        self.imag[0] = z[0].re - z[0].im;
        for k in 1..=self.limit {
            let z_k = z[k];
            let z_mk = z[half - k];
            let f_even = Complex::new(0.5 * (z_k.re + z_mk.re), 0.5 * (z_k.im - z_mk.im));
            let f_odd_unscaled = Complex::new(z_k.im + z_mk.im, -(z_k.re - z_mk.re));
            let w_f_odd = self.twiddles[k] * f_odd_unscaled;
            let x_k = f_even + w_f_odd;
            let x_mk_conj = f_even - w_f_odd;
            self.real[k] = x_k.re;
            self.imag[k] = x_k.im;
            self.real[half - k] = x_mk_conj.re;
            self.imag[half - k] = -x_mk_conj.im;
        }
        if let Some(k) = self.middle_index {
            let z_k = z[k];
            self.real[k] = z_k.re;
            self.imag[k] = -z_k.im;
        }
    }

    /// `DoInverseFFT(data)`: the inverse transform of `real`/`imag` into `out` (`size` samples):
    /// the unnormalized half-size inverse scaled by `1 / (size / 2)`, so that the round trip
    /// with [`FftFrame::do_fft`] is the identity.
    pub fn do_inverse_fft(&mut self, out: &mut [f32]) {
        assert_eq!(out.len(), self.size, "do_inverse_fft: wrong output length");
        let half = self.half_size;
        let (real, imag) = (&self.real, &self.imag);
        let z = &mut self.complex;
        z[0] = Complex::new(0.5 * (real[0] + imag[0]), 0.5 * (real[0] - imag[0]));
        for k in 1..=self.limit {
            let x_k = Complex::new(real[k], imag[k]);
            let x_mk_conj = Complex::new(real[half - k], -imag[half - k]);
            let f_even = (x_k + x_mk_conj) * 0.5;
            let x_diff = x_k - x_mk_conj;
            let f_odd = self.twiddles[k].conj() * x_diff;
            let i_f_odd = Complex::new(-f_odd.im, f_odd.re);
            z[k] = f_even + i_f_odd;
            z[half - k] = (f_even - i_f_odd).conj();
        }
        if let Some(k) = self.middle_index {
            z[k] = Complex::new(real[k], -imag[k]);
        }
        self.inverse.process_with_scratch(z, &mut self.scratch);
        for (pair, c) in out.chunks_exact_mut(2).zip(z.iter()) {
            pair[0] = c.re * self.scale;
            pair[1] = c.im * self.scale;
        }
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    /// A naive DFT in f64: bins `0..=n/2`.
    fn dft(x: &[f32]) -> Vec<(f64, f64)> {
        let n = x.len();
        (0..=n / 2)
            .map(|k| {
                let (mut re, mut im) = (0.0, 0.0);
                for (j, &v) in x.iter().enumerate() {
                    let a = -2.0 * std::f64::consts::PI * (k * j) as f64 / n as f64;
                    re += f64::from(v) * a.cos();
                    im += f64::from(v) * a.sin();
                }
                (re, im)
            })
            .collect()
    }

    fn signal(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| ((i * 7919 % 97) as f32 / 48.0 - 1.0) * if i % 3 == 0 { 0.5 } else { 1.0 })
            .collect()
    }

    #[test]
    fn odd_and_tiny_sizes_are_refused() {
        assert!(FftFrame::new(0).is_none());
        assert!(FftFrame::new(1).is_none());
        assert!(FftFrame::new(7).is_none());
        assert_eq!(FftFrame::new(2).unwrap().size(), 2);
        assert_eq!(FftFrame::new(4096).unwrap().size(), 4096);
    }

    #[test]
    fn forward_matches_a_naive_dft_with_blink_packing() {
        for n in [4usize, 8, 64, 4096] {
            let x = signal(n);
            let mut f = FftFrame::new(n).unwrap();
            f.do_fft(&x);
            let want = dft(&x);
            let tol = 1e-3 * n as f64;
            assert!((f64::from(f.real[0]) - want[0].0).abs() < tol, "dc n={n}");
            assert!(
                (f64::from(f.imag[0]) - want[n / 2].0).abs() < tol,
                "nyquist n={n}"
            );
            for k in 1..n / 2 {
                assert!(
                    (f64::from(f.real[k]) - want[k].0).abs() < tol,
                    "re {k} n={n}"
                );
                assert!(
                    (f64::from(f.imag[k]) - want[k].1).abs() < tol,
                    "im {k} n={n}"
                );
            }
        }
    }

    #[test]
    fn inverse_undoes_forward() {
        // The half-size inverse is unnormalized and Blink scales it by 1 / (n / 2): the round
        // trip is the identity.
        for n in [2usize, 4, 16, 4096] {
            let x = signal(n);
            let mut f = FftFrame::new(n).unwrap();
            f.do_fft(&x);
            let mut y = vec![0.0; n];
            f.do_inverse_fft(&mut y);
            for (a, b) in x.iter().zip(&y) {
                assert!((a - b).abs() < 1e-5, "n={n}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn inverse_of_one_bin_is_a_cosine_or_sine() {
        let n = 64;
        let mut f = FftFrame::new(n).unwrap();
        f.imag[3] = -(n as f32); // the periodic-wave convention: imag scaled by -fftSize
        let mut y = vec![0.0; n];
        f.do_inverse_fft(&mut y);
        for (j, v) in y.iter().enumerate() {
            let want = 2.0 * (2.0 * std::f64::consts::PI * 3.0 * j as f64 / n as f64).sin();
            assert!((f64::from(*v) - want).abs() < 1e-4, "{j}: {v} vs {want}");
        }
    }

    #[test]
    fn transforms_are_deterministic() {
        let x = signal(4096);
        let mut a = FftFrame::new(4096).unwrap();
        let mut b = FftFrame::new(4096).unwrap();
        a.do_fft(&x);
        b.do_fft(&x);
        assert_eq!(a.real, b.real);
        assert_eq!(a.imag, b.imag);
        let (mut ya, mut yb) = (vec![0.0; 4096], vec![0.0; 4096]);
        a.do_inverse_fft(&mut ya);
        b.do_inverse_fft(&mut yb);
        assert_eq!(ya, yb);
    }

    #[test]
    #[should_panic(expected = "wrong input length")]
    fn wrong_lengths_panic() {
        FftFrame::new(8).unwrap().do_fft(&[0.0; 4]);
    }
}
