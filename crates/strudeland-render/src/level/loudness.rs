//! Integrated loudness, ITU-R BS.1770-4 (the gated measure EBU R128 uses): a port of
//! `integratedLufs` in `src/lib/render/loudness.ts`, f64 arithmetic on f32 input like the
//! TypeScript (`Float64Array` state and sums). All channels weigh 1.

/// K-weighting constants of `loudness.ts` (the analogue prototypes whose bilinear transform at
/// 48 kHz reproduces the published table).
const SHELF_F0: f64 = 1681.974450955533;
const SHELF_GAIN_DB: f64 = 3.999843853973347;
const SHELF_Q: f64 = 0.7071752369554196;
const SHELF_VB_EXP: f64 = 0.4996667741545416;
const HP_F0: f64 = 38.13547087602444;
const HP_Q: f64 = 0.5003270373238773;

/// The absolute gate, in LUFS.
pub const ABSOLUTE_GATE_LUFS: f64 = -70.0;
/// The relative gate, in LU below the loudness of the blocks above the absolute gate.
pub const RELATIVE_GATE_LU: f64 = 10.0;

/// A direct-form-I biquad, `a0 = 1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Biquad {
    pub b: [f64; 3],
    /// `a[0] = 1`.
    pub a: [f64; 3],
}

/// The K-weighting biquads (shelf, then high-pass) at `sample_rate` (`kWeightingCoeffs`).
pub fn k_weighting(sample_rate: f64) -> (Biquad, Biquad) {
    let k = (std::f64::consts::PI * SHELF_F0 / sample_rate).tan();
    let vh = 10f64.powf(SHELF_GAIN_DB / 20.0);
    let vb = vh.powf(SHELF_VB_EXP);
    let a0 = 1.0 + k / SHELF_Q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / SHELF_Q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / SHELF_Q + k * k) / a0,
        ],
        a: [
            1.0,
            2.0 * (k * k - 1.0) / a0,
            (1.0 - k / SHELF_Q + k * k) / a0,
        ],
    };
    let k = (std::f64::consts::PI * HP_F0 / sample_rate).tan();
    let a0 = 1.0 + k / HP_Q + k * k;
    let highpass = Biquad {
        b: [1.0, -2.0, 1.0],
        a: [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / HP_Q + k * k) / a0],
    };
    (shelf, highpass)
}

/// A direct-form-I biquad's state, run one sample at a time.
struct Filter {
    b: [f64; 3],
    a: [f64; 3],
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Filter {
    fn new(f: &Biquad) -> Self {
        Filter {
            b: f.b,
            a: f.a,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// The next output for input `x0`.
    fn run(&mut self, x0: f64) -> f64 {
        let [b0, b1, b2] = self.b;
        let [_, a1, a2] = self.a;
        let y0 = b0 * x0 + b1 * self.x1 + b2 * self.x2 - a1 * self.y1 - a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x0;
        self.y2 = self.y1;
        self.y1 = y0;
        y0
    }
}

/// A block's loudness from its channel-summed mean square.
fn block_loudness(sum_z: f64) -> f64 {
    -0.691 + 10.0 * sum_z.log10()
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Gated integrated loudness in LUFS. `-inf` when undefined: no channels, fewer frames than one
/// 400 ms block, no block above the -70 LUFS absolute gate, or no block above the relative gate.
/// `NaN` when a sample is not finite, the channels differ in length or the sample rate is 0.
pub fn integrated_lufs(channels: &[Vec<f32>], sample_rate: u32) -> f64 {
    if sample_rate == 0 {
        return f64::NAN;
    }
    let Some(first) = channels.first() else {
        return f64::NEG_INFINITY;
    };
    let n = first.len();
    if channels.iter().any(|c| c.len() != n) {
        return f64::NAN;
    }
    let sr = f64::from(sample_rate);
    // round(0.4 * sr) and round(0.1 * sr) as in the TS (`Math.round`; both are positive).
    let block = (0.4 * sr).round() as usize;
    let step = ((0.1 * sr).round() as usize).max(1);
    if block == 0 || n < block {
        return f64::NEG_INFINITY;
    }
    let count = (n - block) / step + 1;

    let (shelf, highpass) = k_weighting(sr);
    let mut sum_z = vec![0.0f64; count];
    // The running sum of squares at each block's start (`lo[j]`, index `j * step`) and end
    // (`hi[j]`, index `j * step + block`): the only prefix sums a block reads.
    let mut lo = vec![0.0f64; count];
    let mut hi = vec![0.0f64; count];
    for ch in channels {
        let (mut k1, mut k2) = (Filter::new(&shelf), Filter::new(&highpass));
        let mut acc = 0.0f64;
        // `p` is the prefix index: the sum of the first `p` squares.
        let (mut next_lo, mut next_hi) = (0usize, 0usize);
        let mut store = |p: usize, acc: f64| {
            if p == next_lo * step
                && let Some(slot) = lo.get_mut(next_lo)
            {
                *slot = acc;
                next_lo += 1;
            }
            if p == next_hi * step + block
                && let Some(slot) = hi.get_mut(next_hi)
            {
                *slot = acc;
                next_hi += 1;
            }
        };
        store(0, acc);
        for (i, &x) in ch.iter().enumerate() {
            let v = k2.run(k1.run(f64::from(x)));
            acc += v * v;
            store(i + 1, acc);
        }
        if next_lo != count || next_hi != count {
            return f64::NAN;
        }
        for ((z, h), l) in sum_z.iter_mut().zip(&hi).zip(&lo) {
            *z += (h - l) / block as f64;
        }
    }
    if sum_z.iter().any(|z| z.is_nan()) {
        return f64::NAN;
    }

    let absolute: Vec<f64> = sum_z
        .into_iter()
        .filter(|&z| block_loudness(z) > ABSOLUTE_GATE_LUFS)
        .collect();
    if absolute.is_empty() {
        return f64::NEG_INFINITY;
    }
    let relative_gate = block_loudness(mean(&absolute)) - RELATIVE_GATE_LU;
    let kept: Vec<f64> = absolute
        .into_iter()
        .filter(|&z| block_loudness(z) > relative_gate)
        .collect();
    if kept.is_empty() {
        return f64::NEG_INFINITY;
    }
    block_loudness(mean(&kept))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f64, db: f64, frames: usize, sr: u32) -> Vec<f32> {
        let amp = 10f64.powf(db / 20.0);
        (0..frames)
            .map(|i| {
                (amp * (2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(sr)).sin()) as f32
            })
            .collect()
    }

    #[test]
    fn level_k_weighting_reproduces_the_published_48k_table() {
        // BS.1770-4 Tables 1 and 2 (48 kHz).
        let (shelf, hp) = k_weighting(48_000.0);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        assert!(close(shelf.b[0], 1.53512485958697), "{shelf:?}");
        assert!(close(shelf.b[1], -2.69169618940638), "{shelf:?}");
        assert!(close(shelf.b[2], 1.19839281085285), "{shelf:?}");
        assert!(close(shelf.a[1], -1.69065929318241), "{shelf:?}");
        assert!(close(shelf.a[2], 0.73248077421585), "{shelf:?}");
        assert!(close(hp.a[1], -1.99004745483398), "{hp:?}");
        assert!(close(hp.a[2], 0.99007225036621), "{hp:?}");
        assert_eq!(hp.b, [1.0, -2.0, 1.0]);
    }

    #[test]
    fn level_loudness_undefined_cases() {
        assert_eq!(integrated_lufs(&[], 44_100), f64::NEG_INFINITY);
        // Shorter than one block (17640 frames).
        assert_eq!(
            integrated_lufs(&[sine(1000.0, -20.0, 17_639, 44_100)], 44_100),
            f64::NEG_INFINITY
        );
        assert!(integrated_lufs(&[sine(1000.0, -20.0, 17_640, 44_100)], 44_100).is_finite());
        assert_eq!(
            integrated_lufs(&vec![vec![0.0; 44_100]; 2], 44_100),
            f64::NEG_INFINITY
        );
        assert_eq!(
            integrated_lufs(&[sine(1000.0, -75.0, 88_200, 44_100)], 44_100),
            f64::NEG_INFINITY
        );
    }

    #[test]
    fn level_loudness_errors_are_nan() {
        let mut x = sine(1000.0, -20.0, 44_100, 44_100);
        x[100] = f32::NAN;
        assert!(integrated_lufs(&[x.clone()], 44_100).is_nan());
        x[100] = f32::INFINITY;
        assert!(integrated_lufs(&[x], 44_100).is_nan());
        assert!(integrated_lufs(&[vec![0.0; 20_000], vec![0.0; 20_001]], 44_100).is_nan());
        assert!(integrated_lufs(&[vec![0.0; 20_000]], 0).is_nan());
    }

    #[test]
    fn level_loudness_stereo_is_mono_plus_3_db() {
        let x = sine(1000.0, -23.0, 441_000, 44_100);
        let mono = integrated_lufs(std::slice::from_ref(&x), 44_100);
        let stereo = integrated_lufs(&[x.clone(), x], 44_100);
        assert!(
            (stereo - mono - 10.0 * 2f64.log10()).abs() < 1e-9,
            "{mono} {stereo}"
        );
    }
}
