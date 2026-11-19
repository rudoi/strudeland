// Port of Blink (Chromium 156.0.8078.4) platform/audio/stereo_panner.cc, modules/webaudio/stereo_panner_handler.cc, modules/webaudio/stereo_panner_node.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! `StereoPannerNode`: the equal-power panning law for mono and stereo input.
//!
//! The pan param is a-rate (range `[-1, 1]`, default 0). While it has sample-accurate values the
//! node pans frame by frame with `f64` gains (`PanWithSampleAccurateValues`); otherwise with one
//! value for the quantum and `f32` gains (`PanToTargetValue`), whose stereo branch uses macOS
//! `vDSP_vsma` (a fused multiply-add per frame, measured from Accelerate on the reference
//! machine) and `vDSP_vsmul` (a plain multiply). `fdlibm` `cos`/`sin` are `libm`'s.

use crate::QUANTUM;
use crate::param::{Param, ParamInputs, Rate};

use crate::SAMPLE_RATE_F64 as SR;
/// `kPiOverTwoDouble`.
const PI_OVER_TWO: f64 = std::f64::consts::FRAC_PI_2;

/// `ClampTo(x, -1.0, 1.0)` (NaN passes through).
fn clamp_pan(x: f64) -> f64 {
    x.clamp(-1.0, 1.0)
}

/// The two output channels of a panner.
pub type Stereo = [[f32; QUANTUM]; 2];

/// A `StereoPannerNode` with its `AudioHandler` silence state (tail and latency 0).
#[derive(Debug, Clone)]
pub struct StereoPanner {
    /// `pan` (a-rate, default 0, range `[-1, 1]`).
    pub pan: Param,
    last_non_silent_time: f64,
}

impl Default for StereoPanner {
    fn default() -> Self {
        StereoPanner::new()
    }
}

impl StereoPanner {
    pub fn new() -> StereoPanner {
        StereoPanner {
            pan: Param::new(0.0, -1.0, 1.0, Rate::A),
            last_non_silent_time: 0.0,
        }
    }

    /// `ProcessIfNecessary` + `StereoPannerHandler::Process` for the quantum at `quantum_start`:
    /// `input` is one (mono) or two channels; writes `out`. Returns the output's silence hint
    /// (silent input past the zero tail: zeros, only the param evaluated).
    pub fn process(
        &mut self,
        quantum_start: u64,
        input: &[[f32; QUANTUM]],
        input_silent: bool,
        out: &mut Stereo,
    ) -> bool {
        self.process_with(quantum_start, input, input_silent, out, &ParamInputs::NONE)
    }

    /// [`StereoPanner::process`] with the connections into `pan`.
    pub fn process_with(
        &mut self,
        quantum_start: u64,
        input: &[[f32; QUANTUM]],
        input_silent: bool,
        out: &mut Stereo,
        inputs: &ParamInputs,
    ) -> bool {
        let (bufs, connected) = (&inputs.bufs, inputs.connected);
        if input_silent && self.last_non_silent_time < quantum_start as f64 / SR {
            out[0].fill(0.0);
            out[1].fill(0.0);
            let mut scratch = [0.0f32; QUANTUM];
            self.pan
                .fill_quantum_connected(quantum_start, &mut scratch, bufs, connected);
            return true;
        }
        let sample_accurate = self
            .pan
            .has_sample_accurate_values(quantum_start, connected);
        if sample_accurate && self.pan.rate() == Rate::A {
            let mut values = [0.0f32; QUANTUM];
            self.pan
                .fill_quantum_connected(quantum_start, &mut values, bufs, connected);
            pan_with_sample_accurate_values(input, &values, out);
        } else {
            let value = if sample_accurate {
                self.pan.final_value_of_bufs(quantum_start, bufs, connected)
            } else {
                self.pan.audio_value(quantum_start)
            };
            pan_to_target_value(input, value, out);
        }
        if !input_silent {
            self.last_non_silent_time = crate::quantum_end_time(quantum_start);
        }
        false
    }
}

/// `StereoPanner::PanWithSampleAccurateValues`: per-frame `f64` gains, outputs cast to `f32`.
pub fn pan_with_sample_accurate_values(
    input: &[[f32; QUANTUM]],
    values: &[f32; QUANTUM],
    out: &mut Stereo,
) {
    let (l, r) = out.split_at_mut(1);
    let (dl, dr) = (&mut l[0], &mut r[0]);
    if input.len() == 1 {
        let src = &input[0];
        for i in 0..QUANTUM {
            let x = f64::from(src[i]);
            let pan = clamp_pan(f64::from(values[i]));
            let radian = (pan * 0.5 + 0.5) * PI_OVER_TWO;
            let gain_l = libm::cos(radian);
            let gain_r = libm::sin(radian);
            dl[i] = (x * gain_l) as f32;
            dr[i] = (x * gain_r) as f32;
        }
    } else {
        let (sl, sr) = (&input[0], &input[1]);
        for i in 0..QUANTUM {
            let (xl, xr) = (f64::from(sl[i]), f64::from(sr[i]));
            let pan = clamp_pan(f64::from(values[i]));
            let radian = (if pan <= 0.0 { pan + 1.0 } else { pan }) * PI_OVER_TWO;
            let gain_l = libm::cos(radian);
            let gain_r = libm::sin(radian);
            if pan <= 0.0 {
                dl[i] = (xl + xr * gain_l) as f32;
                dr[i] = (xr * gain_r) as f32;
            } else {
                dl[i] = (xl * gain_l) as f32;
                dr[i] = (xr + xl * gain_r) as f32;
            }
        }
    }
}

/// `StereoPanner::PanToTargetValue`: one pan value, `f32` gains; `Vsmul` is a float multiply,
/// `Vsma` (`dest += src * gain`) a fused multiply-add.
pub fn pan_to_target_value(input: &[[f32; QUANTUM]], pan_value: f32, out: &mut Stereo) {
    let target = clamp_pan(f64::from(pan_value)) as f32;
    let (l, r) = out.split_at_mut(1);
    let (dl, dr) = (&mut l[0], &mut r[0]);
    if input.len() == 1 {
        let radian = (f64::from(target) * 0.5 + 0.5) * PI_OVER_TWO;
        let gain_l = libm::cos(radian) as f32;
        let gain_r = libm::sin(radian) as f32;
        let src = &input[0];
        for i in 0..QUANTUM {
            dr[i] = src[i] * gain_r;
            dl[i] = src[i] * gain_l;
        }
    } else {
        let t = f64::from(target);
        let radian = (if target <= 0.0 { t + 1.0 } else { t }) * PI_OVER_TWO;
        let gain_l = libm::cos(radian) as f32;
        let gain_r = libm::sin(radian) as f32;
        let (sl, sr) = (&input[0], &input[1]);
        if target <= 0.0 {
            for i in 0..QUANTUM {
                dl[i] = sr[i].mul_add(gain_l, sl[i]);
                dr[i] = sr[i] * gain_r;
            }
        } else {
            for i in 0..QUANTUM {
                dr[i] = sl[i].mul_add(gain_r, sr[i]);
                dl[i] = sl[i] * gain_l;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    const HALF: f64 = std::f64::consts::FRAC_1_SQRT_2;

    fn mono(v: f32) -> [[f32; QUANTUM]; 1] {
        [[v; QUANTUM]]
    }

    #[test]
    fn mono_law() {
        let mut out = [[0.0f32; QUANTUM]; 2];
        // superdough's pan 0.5 -> param 0 -> 0.7071 per side.
        pan_to_target_value(&mono(1.0), 0.0, &mut out);
        assert!((f64::from(out[0][0]) - HALF).abs() < 1e-7);
        assert_eq!(out[0][0], out[1][0]);
        assert_eq!(out[0][0], libm::cos(PI_OVER_TWO * 0.5) as f32);
        // Hard left and right.
        pan_to_target_value(&mono(1.0), -1.0, &mut out);
        assert_eq!((out[0][5], out[1][5]), (1.0, 0.0));
        pan_to_target_value(&mono(1.0), 1.0, &mut out);
        assert_eq!(out[0][5], libm::cos(PI_OVER_TWO) as f32);
        assert_eq!(out[1][5], 1.0);
        // Equal power at any position.
        for p in [-0.8f32, -0.3, 0.2, 0.9] {
            pan_to_target_value(&mono(1.0), p, &mut out);
            let power = out[0][0] * out[0][0] + out[1][0] * out[1][0];
            assert!((power - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn stereo_laws() {
        let input = [[0.5f32; QUANTUM], [0.25f32; QUANTUM]];
        let mut out = [[0.0f32; QUANTUM]; 2];
        // pan <= 0: the left keeps its input and gains the right's cos share.
        let p = -0.5f32;
        pan_to_target_value(&input, p, &mut out);
        let theta = (f64::from(p) + 1.0) * PI_OVER_TWO;
        let (gl, gr) = (libm::cos(theta) as f32, libm::sin(theta) as f32);
        assert_eq!(out[0][0], 0.25f32.mul_add(gl, 0.5));
        assert_eq!(out[1][0], 0.25 * gr);
        // pan > 0: the right keeps its input and gains the left's sin share.
        let p = 0.5f32;
        pan_to_target_value(&input, p, &mut out);
        let theta = f64::from(p) * PI_OVER_TWO;
        let (gl, gr) = (libm::cos(theta) as f32, libm::sin(theta) as f32);
        assert_eq!(out[0][0], 0.5 * gl);
        assert_eq!(out[1][0], 0.5f32.mul_add(gr, 0.25));
        // pan 0 passes stereo through except for the tiny cos(pi/2) leak of the right channel.
        pan_to_target_value(&input, 0.0, &mut out);
        assert_eq!(out[1][0], 0.25);
        assert!((out[0][0] - 0.5).abs() < 1e-15);
        let leak = libm::cos(PI_OVER_TWO) as f32;
        assert!(leak > 0.0 && leak < 1e-16);
    }

    #[test]
    fn sample_accurate_gains_are_doubles() {
        let mut values = [0.0f32; QUANTUM];
        for i in 0..QUANTUM {
            values[i] = i as f32 / 64.0 - 1.0;
        }
        let x = 0.3f32;
        let mut out = [[0.0f32; QUANTUM]; 2];
        pan_with_sample_accurate_values(&mono(x), &values, &mut out);
        for i in [0, 17, 64, 127] {
            let pan = f64::from(values[i]);
            let radian = (pan * 0.5 + 0.5) * PI_OVER_TWO;
            assert_eq!(out[0][i], (f64::from(x) * libm::cos(radian)) as f32);
            assert_eq!(out[1][i], (f64::from(x) * libm::sin(radian)) as f32);
        }
        // The k-rate path rounds the gains first: the two paths can differ in the last bit.
        let mut differs = false;
        for p in 0..200 {
            let pan = p as f32 / 100.0 - 1.0;
            let mut a = [[0.0f32; QUANTUM]; 2];
            let mut b = [[0.0f32; QUANTUM]; 2];
            let input = [[0.123_456_7f32; QUANTUM]];
            pan_with_sample_accurate_values(&input, &[pan; QUANTUM], &mut a);
            pan_to_target_value(&input, pan, &mut b);
            assert!((a[0][0] - b[0][0]).abs() <= 2.0 * f32::EPSILON);
            differs |= a[0][0] != b[0][0] || a[1][0] != b[1][0];
        }
        assert!(differs);
        // Stereo, both signs.
        let input = [[0.5f32; QUANTUM], [-0.25f32; QUANTUM]];
        let mut values = [0.0f32; QUANTUM];
        values[0] = -0.4;
        values[1] = 0.4;
        pan_with_sample_accurate_values(&input, &values, &mut out);
        let t0 = (-0.4f32 as f64 + 1.0) * PI_OVER_TWO;
        assert_eq!(out[0][0], (0.5 + -0.25 * libm::cos(t0)) as f32);
        assert_eq!(out[1][0], (-0.25 * libm::sin(t0)) as f32);
        let t1 = 0.4f32 as f64 * PI_OVER_TWO;
        assert_eq!(out[0][1], (0.5 * libm::cos(t1)) as f32);
        assert_eq!(out[1][1], (-0.25 + 0.5 * libm::sin(t1)) as f32);
    }

    #[test]
    fn values_are_clamped() {
        let mut a = [[0.0f32; QUANTUM]; 2];
        let mut b = [[0.0f32; QUANTUM]; 2];
        pan_to_target_value(&mono(1.0), 3.0, &mut a);
        pan_to_target_value(&mono(1.0), 1.0, &mut b);
        assert_eq!(a, b);
        pan_with_sample_accurate_values(&mono(1.0), &[-7.0; QUANTUM], &mut a);
        pan_with_sample_accurate_values(&mono(1.0), &[-1.0; QUANTUM], &mut b);
        assert_eq!(a, b);
        assert_eq!(clamp_pan(0.25), 0.25);
        assert!(clamp_pan(f64::NAN).is_nan());
    }

    #[test]
    fn the_node_is_a_rate_only_while_its_param_is_sample_accurate() {
        // `.value = 0.4` at the pause frame 0: the first quantum is sample accurate (double
        // gains), the next ones k-rate (float gains).
        let mut p = StereoPanner::new();
        p.pan.set_value(0.4, 0).unwrap();
        let input = mono(0.123_456_7);
        let mut out = [[0.0f32; QUANTUM]; 2];
        assert!(!p.process(0, &input, false, &mut out));
        let mut want = [[0.0f32; QUANTUM]; 2];
        pan_with_sample_accurate_values(&input, &[0.4; QUANTUM], &mut want);
        assert_eq!(out, want);
        p.process(128, &input, false, &mut out);
        pan_to_target_value(&input, 0.4, &mut want);
        assert_eq!(out, want);
        // Silent input one quantum after the last signal: zeros, flagged silent.
        let zero = mono(0.0);
        assert!(!p.process(256, &zero, true, &mut out));
        assert!(p.process(384, &zero, true, &mut out));
        assert_eq!(out, [[0.0f32; QUANTUM]; 2]);
        assert_eq!(p.pan.value(), 0.4);
        // The default param is 0 (centre) and clamps `.value`.
        let mut p = StereoPanner::default();
        assert_eq!(p.pan.value(), 0.0);
        p.pan.set_value(-3.0, 0).unwrap();
        assert_eq!(p.pan.value(), -1.0);
    }

    /// The stereo k-rate branch against Accelerate's `vDSP_vsma` and `vDSP_vsmul`.
    #[cfg(target_os = "macos")]
    mod accelerate {
        use super::*;

        #[link(name = "Accelerate", kind = "framework")]
        unsafe extern "C" {
            fn vDSP_vsma(
                a: *const f32,
                ia: isize,
                b: *const f32,
                c: *const f32,
                ic: isize,
                d: *mut f32,
                id: isize,
                n: usize,
            );
            fn vDSP_vsmul(
                a: *const f32,
                ia: isize,
                b: *const f32,
                c: *mut f32,
                ic: isize,
                n: usize,
            );
        }

        #[test]
        fn vsma_is_fused_and_vsmul_plain() {
            let mut rng = crate::js::Mulberry32::new(4);
            for _ in 0..100 {
                let mut a = [0.0f32; QUANTUM];
                let mut c = [0.0f32; QUANTUM];
                for i in 0..QUANTUM {
                    a[i] = (rng.next() * 2.0 - 1.0) as f32;
                    c[i] = ((rng.next() * 2.0 - 1.0) * 1e-3) as f32;
                }
                let s = (rng.next() * 2.0 - 1.0) as f32;
                let mut d = [0.0f32; QUANTUM];
                let mut m = [0.0f32; QUANTUM];
                unsafe {
                    vDSP_vsma(a.as_ptr(), 1, &s, c.as_ptr(), 1, d.as_mut_ptr(), 1, QUANTUM);
                    vDSP_vsmul(a.as_ptr(), 1, &s, m.as_mut_ptr(), 1, QUANTUM);
                }
                for i in 0..QUANTUM {
                    assert_eq!(d[i], a[i].mul_add(s, c[i]));
                    assert_eq!(m[i], a[i] * s);
                }
            }
        }
    }
}
