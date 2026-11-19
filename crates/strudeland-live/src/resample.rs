//! The 44.1 kHz engine quantum to the device rate.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, Resampler, SincInterpolationParameters, WindowFunction};
use strudeland_engine::{QUANTUM, SAMPLE_RATE};

/// Samples in one engine quantum (interleaved stereo).
const QUANTUM_SAMPLES: usize = QUANTUM * 2;

/// Resamples engine quanta to the device rate; the identity at 44100 Hz.
pub struct QuantumResampler {
    device_rate: u32,
    inner: Option<Inner>,
}

struct Inner {
    resampler: Async<f32>,
    out: Vec<f32>,
}

impl QuantumResampler {
    pub fn new(device_rate: u32) -> Result<QuantumResampler, String> {
        if device_rate == SAMPLE_RATE {
            return Ok(QuantumResampler {
                device_rate,
                inner: None,
            });
        }
        if device_rate == 0 {
            return Err("device rate is 0".into());
        }
        let resampler = Async::<f32>::new_sinc(
            f64::from(device_rate) / f64::from(SAMPLE_RATE),
            1.0,
            &SincInterpolationParameters::new(128, WindowFunction::BlackmanHarris2),
            QUANTUM,
            2,
            FixedAsync::Input,
        )
        .map_err(|e| format!("resampler: {e}"))?;
        let out = vec![0.0; resampler.output_frames_max() * 2];
        Ok(QuantumResampler {
            device_rate,
            inner: Some(Inner { resampler, out }),
        })
    }

    /// The most device-rate frames one `process` call can produce.
    pub fn output_frames_max(&self) -> usize {
        match &self.inner {
            None => QUANTUM,
            Some(i) => i.resampler.output_frames_max(),
        }
    }

    /// The resampler's delay in engine (44.1 kHz) frames; 0 without a resampler.
    pub fn delay_engine_frames(&self) -> f64 {
        match &self.inner {
            None => 0.0,
            Some(i) => {
                i.resampler.output_delay() as f64 * f64::from(SAMPLE_RATE)
                    / f64::from(self.device_rate)
            }
        }
    }

    /// Replaces the contents of `out` with the device-rate frames of one quantum. Allocates
    /// nothing as long as `out` has the capacity (`2 * output_frames_max()` samples).
    pub fn process(&mut self, q: &[f32; QUANTUM_SAMPLES], out: &mut Vec<f32>) {
        out.clear();
        match &mut self.inner {
            None => out.extend_from_slice(q),
            Some(inner) => {
                let input = InterleavedSlice::new(&q[..], 2, QUANTUM).expect("input shape");
                let frames = inner.out.len() / 2;
                let mut output =
                    InterleavedSlice::new_mut(&mut inner.out[..], 2, frames).expect("output shape");
                // An error is never reached for a well-formed quantum: the shapes are fixed above.
                if let Ok((_, n)) = inner
                    .resampler
                    .process_into_buffer(&input, &mut output, None)
                {
                    out.extend_from_slice(&inner.out[..n * 2]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_quanta(freq: f64, count: usize) -> Vec<[f32; QUANTUM_SAMPLES]> {
        let mut n = 0usize;
        (0..count)
            .map(|_| {
                let mut q = [0.0f32; QUANTUM_SAMPLES];
                for f in 0..QUANTUM {
                    let v = (2.0 * std::f64::consts::PI * freq * n as f64 / 44100.0).sin() as f32;
                    q[2 * f] = v;
                    q[2 * f + 1] = v;
                    n += 1;
                }
                q
            })
            .collect()
    }

    #[test]
    fn identity_at_44100() {
        let mut r = QuantumResampler::new(44100).unwrap();
        assert_eq!(r.delay_engine_frames(), 0.0);
        let q = sine_quanta(1000.0, 1).remove(0);
        let mut out = Vec::new();
        r.process(&q, &mut out);
        assert_eq!(out, q.to_vec());
    }

    #[test]
    fn keeps_a_sines_frequency_at_48k() {
        let mut r = QuantumResampler::new(48000).unwrap();
        let mut left = Vec::new();
        let mut out = Vec::with_capacity(r.output_frames_max() * 2);
        for q in sine_quanta(1000.0, 200) {
            r.process(&q, &mut out);
            left.extend(out.chunks(2).map(|f| f[0]));
        }
        // skip the filter's start-up, then measure rising zero crossings
        let body = &left[2000..];
        let crossings: Vec<usize> = (1..body.len())
            .filter(|&i| body[i - 1] < 0.0 && body[i] >= 0.0)
            .collect();
        assert!(crossings.len() > 100);
        let spacing =
            (crossings[crossings.len() - 1] - crossings[0]) as f64 / (crossings.len() - 1) as f64;
        let expected = 48.0;
        assert!(
            (spacing - expected).abs() / expected < 0.005,
            "spacing {spacing}"
        );
        // the output count follows the ratio
        let ratio = left.len() as f64 / (200.0 * QUANTUM as f64);
        assert!((ratio - 48000.0 / 44100.0).abs() < 0.01, "ratio {ratio}");
    }

    #[test]
    fn delay_is_output_delay_in_engine_frames() {
        let r = QuantumResampler::new(48000).unwrap();
        let expected =
            r.inner.as_ref().unwrap().resampler.output_delay() as f64 * 44100.0 / 48000.0;
        assert_eq!(r.delay_engine_frames(), expected);
        assert!(expected > 0.0);
    }

    #[test]
    fn process_does_not_grow_the_output_vec() {
        let mut r = QuantumResampler::new(96000).unwrap();
        let mut out = Vec::with_capacity(r.output_frames_max() * 2);
        let cap = out.capacity();
        for q in sine_quanta(440.0, 50) {
            r.process(&q, &mut out);
            assert_eq!(out.capacity(), cap);
        }
    }

    #[test]
    fn rejects_a_zero_rate() {
        assert!(QuantumResampler::new(0).is_err());
    }
}
