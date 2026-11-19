//! True peak (ITU-R BS.1770-4 Annex 2): 4x oversampling with the Annex 2 48-tap interpolation
//! filter, then the maximum absolute value. Arithmetic is f64 on f32 input; no attenuation stage is
//! needed in floating point.
//!
//! The coefficients are the table "One set of filter coefficients (for the order 48, 4-phase, FIR
//! interpolating)" of Rec. ITU-R BS.1770-4 (10/2015), Annex 2, section 3, copied from the ITU's
//! published PDF and checked by the tests against the EBU Tech 3341 true-peak cases (15 to 23).
//! The filter is defined in normalised frequency, so the same table serves 44.1 kHz.

/// Taps per phase.
pub const TAPS_PER_PHASE: usize = 12;

/// The four phases of the Annex 2 filter: `PHASES[k][j]` is phase `k`, tap `j` (`h[4*j + k]`).
/// Output `y[4*i + k] = sum_j PHASES[k][j] * x[i - j]`.
pub const PHASES: [[f64; TAPS_PER_PHASE]; 4] = [
    [
        0.0017089843750,
        0.0109863281250,
        -0.0196533203125,
        0.0332031250000,
        -0.0594482421875,
        0.1373291015625,
        0.9721679687500,
        -0.1022949218750,
        0.0476074218750,
        -0.0266113281250,
        0.0148925781250,
        -0.0083007812500,
    ],
    [
        -0.0291748046875,
        0.0292968750000,
        -0.0517578125000,
        0.0891113281250,
        -0.1665039062500,
        0.4650878906250,
        0.7797851562500,
        -0.2003173828125,
        0.1015625000000,
        -0.0582275390625,
        0.0330810546875,
        -0.0189208984375,
    ],
    [
        -0.0189208984375,
        0.0330810546875,
        -0.0582275390625,
        0.1015625000000,
        -0.2003173828125,
        0.7797851562500,
        0.4650878906250,
        -0.1665039062500,
        0.0891113281250,
        -0.0517578125000,
        0.0292968750000,
        -0.0291748046875,
    ],
    [
        -0.0083007812500,
        0.0148925781250,
        -0.0266113281250,
        0.0476074218750,
        -0.1022949218750,
        0.9721679687500,
        0.1373291015625,
        -0.0594482421875,
        0.0332031250000,
        -0.0196533203125,
        0.0109863281250,
        0.0017089843750,
    ],
];

/// The phases with their taps reversed, so a window `x[b-11..=b]` is a plain dot product.
const REVERSED: [[f64; TAPS_PER_PHASE]; 4] = reverse_phases();

// Evaluated at compile time: an out-of-range index would fail the build, not panic.
#[allow(clippy::indexing_slicing)]
const fn reverse_phases() -> [[f64; TAPS_PER_PHASE]; 4] {
    let mut out = [[0.0; TAPS_PER_PHASE]; 4];
    let mut k = 0;
    while k < 4 {
        let mut j = 0;
        while j < TAPS_PER_PHASE {
            out[k][j] = PHASES[k][TAPS_PER_PHASE - 1 - j];
            j += 1;
        }
        k += 1;
    }
    out
}

/// One channel's interpolation filter state: the last 12 input samples, oldest first. Each
/// sample is stored twice (at `idx` and `idx + 12`), so the window is always the contiguous
/// `buf[idx + 1..idx + 13]` after a push.
#[derive(Debug, Clone)]
struct Interpolator {
    buf: [f64; 2 * TAPS_PER_PHASE],
    idx: usize,
    /// A sample was pushed.
    fed: bool,
}

impl Interpolator {
    fn new() -> Self {
        Interpolator {
            buf: [0.0; 2 * TAPS_PER_PHASE],
            idx: 0,
            fed: false,
        }
    }

    /// Pushes the next input sample and returns the 4 oversampled outputs of its position, as
    /// absolute values.
    fn push(&mut self, x: f64) -> [f64; 4] {
        let i = self.idx;
        self.fed = true;
        if let Some(slot) = self.buf.get_mut(i) {
            *slot = x;
        }
        if let Some(slot) = self.buf.get_mut(i + TAPS_PER_PHASE) {
            *slot = x;
        }
        self.idx = (i + 1) % TAPS_PER_PHASE;
        let w = self.buf.get(i + 1..i + 1 + TAPS_PER_PHASE).unwrap_or(&[]);
        let mut y = [0.0f64; 4];
        for (yk, rk) in y.iter_mut().zip(&REVERSED) {
            let mut acc = 0.0f64;
            for (c, s) in rk.iter().zip(w) {
                acc += c * s;
            }
            *yk = acc.abs();
        }
        y
    }
}

/// The 4 oversampled outputs of input position `b` (`y[4*b + k]`, `k = 0..4`), for every
/// `b in 0..len + 12` (the full convolution: `4 * len + 48` points, the last one always 0), as
/// absolute values. `visit(b, [|y0|, |y1|, |y2|, |y3|])`. The filter sees `sample(i, x[i])`.
fn for_each_position(
    x: &[f32],
    mut sample: impl FnMut(usize, f32) -> f32,
    mut visit: impl FnMut(usize, [f64; 4]),
) {
    if x.is_empty() {
        return;
    }
    // Window b is x[b - 11..=b], zero outside: the samples, then 12 zeros to flush the tail.
    let mut f = Interpolator::new();
    let samples = x.iter().enumerate().map(|(i, &s)| f64::from(sample(i, s)));
    let tail = std::iter::repeat_n(0.0, TAPS_PER_PHASE);
    for (b, s) in samples.chain(tail).enumerate() {
        visit(b, f.push(s));
    }
}

/// A streaming [`true_peak_linear`]: interleaved samples in, any number of pushes, the same
/// value out of [`TruePeakMeter::linear`] as the whole buffer would give.
#[derive(Debug, Clone)]
pub struct TruePeakMeter {
    channels: Vec<Interpolator>,
    /// The channel the next pushed sample belongs to (a push may end mid-frame).
    next: usize,
    non_finite: bool,
    peak: f64,
}

impl TruePeakMeter {
    /// A meter for `channels` interleaved channels.
    pub fn new(channels: usize) -> Self {
        TruePeakMeter {
            channels: vec![Interpolator::new(); channels],
            next: 0,
            non_finite: false,
            peak: 0.0,
        }
    }

    /// Pushes interleaved samples. A push need not hold whole frames: the next push carries on
    /// with the channel after the last sample pushed.
    pub fn push_interleaved(&mut self, samples: &[f32]) {
        let n = self.channels.len();
        if n == 0 {
            return;
        }
        for &x in samples {
            let c = self.next;
            self.next = (c + 1) % n;
            let Some(f) = self.channels.get_mut(c) else {
                continue;
            };
            self.non_finite |= !x.is_finite();
            for v in f.push(f64::from(x)) {
                self.peak = self.peak.max(v);
            }
        }
    }

    /// Flushes the filters' tails and returns the true peak (linear): `0.0` for no samples,
    /// `NaN` when a sample was not finite.
    pub fn linear(mut self) -> f64 {
        if self.non_finite {
            return f64::NAN;
        }
        // Like the whole-buffer measure, a channel without samples has no tail.
        for f in self.channels.iter_mut().filter(|f| f.fed) {
            for _ in 0..TAPS_PER_PHASE {
                for v in f.push(0.0) {
                    self.peak = self.peak.max(v);
                }
            }
        }
        self.peak
    }
}

/// `max |y[n]|` over every channel and every oversampled point; `0.0` for no samples. `NaN` when a
/// sample is not finite.
pub fn true_peak_linear(channels: &[Vec<f32>]) -> f64 {
    true_peak_linear_by(channels, |_, x| x)
}

/// [`true_peak_linear`] of the channels whose frame `i` is `sample(i, x)`, without a buffer.
pub(crate) fn true_peak_linear_by(
    channels: &[Vec<f32>],
    sample: impl Fn(usize, f32) -> f32 + Copy,
) -> f64 {
    let mut non_finite = false;
    let mut peak = 0.0f64;
    for ch in channels {
        let checked = |i, x| {
            let y = sample(i, x);
            non_finite |= !y.is_finite();
            y
        };
        for_each_position(ch, checked, |_, y| {
            for v in y {
                peak = peak.max(v);
            }
        });
    }
    if non_finite { f64::NAN } else { peak }
}

/// `20 * log10(true_peak_linear)`: `-inf` for silence (or no samples).
pub fn true_peak_db(channels: &[Vec<f32>]) -> f64 {
    20.0 * true_peak_linear(channels).log10()
}

/// [`true_peak_db`] of the channels whose frame `i` is `sample(i, x)`, without a buffer.
pub(crate) fn true_peak_db_by(
    channels: &[Vec<f32>],
    sample: impl Fn(usize, f32) -> f32 + Copy,
) -> f64 {
    20.0 * true_peak_linear_by(channels, sample).log10()
}

/// The largest of `frame_peaks`, as [`true_peak_linear`] of the same buffer: `NaN` when one is.
pub(crate) fn peak_of_frames(peaks: &[f64]) -> f64 {
    if peaks.iter().any(|p| p.is_nan()) {
        return f64::NAN;
    }
    peaks.iter().fold(0.0f64, |m, &p| m.max(p))
}

/// For frame `i`, the largest `|y_c[n]|` over channels `c` and the four oversampled points nearest
/// to sample `i` (`n` in `4i+22 ..= 4i+25`, at times `i - 0.375 .. i + 0.375`). The points before
/// the first of those (`n < 22`, the head) fold into frame 0 and the points after the last one
/// (the tail) into the last frame, so `max(frame_peaks) == true_peak_linear` and the limiter sees
/// every point the measure does. The length is the longest channel's.
pub fn frame_peaks(channels: &[Vec<f32>]) -> Vec<f64> {
    frame_peaks_by(channels, |_, x| x)
}

/// [`frame_peaks`] of the channels whose frame `i` is `sample(i, x)`, without a buffer.
pub(crate) fn frame_peaks_by(
    channels: &[Vec<f32>],
    sample: impl Fn(usize, f32) -> f32 + Copy,
) -> Vec<f64> {
    let frames = channels.iter().map(Vec::len).max().unwrap_or(0);
    let mut tp = vec![0.0f64; frames];
    let last = frames.saturating_sub(1);
    for ch in channels {
        for_each_position(ch, sample, |b, y| {
            let [y0, y1, y2, y3] = y;
            // Position b holds y[4b..4b+4]: phases 0 and 1 belong to frame b - 6, phases 2 and 3
            // to frame b - 5 (clamped into the buffer: head and tail fold into the ends).
            let lo = y0.max(y1);
            let hi = y2.max(y3);
            let f01 = b.saturating_sub(6).min(last);
            let f23 = b.saturating_sub(5).min(last);
            if let Some(slot) = tp.get_mut(f01) {
                *slot = max_nan(*slot, lo);
            }
            if let Some(slot) = tp.get_mut(f23) {
                *slot = max_nan(*slot, hi);
            }
        });
    }
    tp
}

/// `max` that keeps a `NaN`.
fn max_nan(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Annex 2 table as published (column per phase, row per tap), transcribed from the ITU
    /// PDF a second time, independently of `PHASES`.
    const ANNEX2_ROWS: [[f64; 4]; 12] = [
        [
            0.0017089843750,
            -0.0291748046875,
            -0.0189208984375,
            -0.0083007812500,
        ],
        [
            0.0109863281250,
            0.0292968750000,
            0.0330810546875,
            0.0148925781250,
        ],
        [
            -0.0196533203125,
            -0.0517578125000,
            -0.0582275390625,
            -0.0266113281250,
        ],
        [
            0.0332031250000,
            0.0891113281250,
            0.1015625000000,
            0.0476074218750,
        ],
        [
            -0.0594482421875,
            -0.1665039062500,
            -0.2003173828125,
            -0.1022949218750,
        ],
        [
            0.1373291015625,
            0.4650878906250,
            0.7797851562500,
            0.9721679687500,
        ],
        [
            0.9721679687500,
            0.7797851562500,
            0.4650878906250,
            0.1373291015625,
        ],
        [
            -0.1022949218750,
            -0.2003173828125,
            -0.1665039062500,
            -0.0594482421875,
        ],
        [
            0.0476074218750,
            0.1015625000000,
            0.0891113281250,
            0.0332031250000,
        ],
        [
            -0.0266113281250,
            -0.0582275390625,
            -0.0517578125000,
            -0.0196533203125,
        ],
        [
            0.0148925781250,
            0.0330810546875,
            0.0292968750000,
            0.0109863281250,
        ],
        [
            -0.0083007812500,
            -0.0189208984375,
            -0.0291748046875,
            0.0017089843750,
        ],
    ];

    #[test]
    fn level_true_peak_table_equals_the_published_columns() {
        for (j, row) in ANNEX2_ROWS.iter().enumerate() {
            for (k, &c) in row.iter().enumerate() {
                assert_eq!(PHASES[k][j].to_bits(), c.to_bits(), "phase {k} tap {j}");
            }
        }
    }

    #[test]
    fn level_true_peak_table_is_symmetric_and_phases_sum_near_one() {
        for j in 0..TAPS_PER_PHASE {
            assert_eq!(PHASES[3][j], PHASES[0][11 - j], "P3 = reverse(P0) at {j}");
            assert_eq!(PHASES[2][j], PHASES[1][11 - j], "P2 = reverse(P1) at {j}");
        }
        for (k, p) in PHASES.iter().enumerate() {
            let sum: f64 = p.iter().sum();
            assert!((sum - 1.0).abs() < 0.03, "phase {k} sums to {sum}");
        }
        // Every coefficient is a multiple of 2^-13 (the table's 13-bit resolution).
        for p in PHASES {
            for c in p {
                assert_eq!((c * 8192.0).fract(), 0.0, "{c}");
            }
        }
    }

    #[test]
    fn level_true_peak_impulse_reproduces_the_filter() {
        // A unit impulse at frame 0 yields h itself: y[4*j + k] = P_k[j].
        let mut ys = Vec::new();
        for_each_position(&[1.0], |_, x| x, |b, y| ys.push((b, y)));
        assert_eq!(ys.len(), 13);
        for (b, y) in ys {
            for (k, v) in y.into_iter().enumerate() {
                let want = PHASES[k].get(b).map_or(0.0, |c| c.abs());
                assert_eq!(v, want, "b {b} k {k}");
            }
        }
    }

    #[test]
    fn level_true_peak_quarter_rate_sine_at_45_degrees() {
        // A full-scale fs/4 sine at 45 degrees: samples are +-0.7071, the true peak is 1.0.
        let n = 4410;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                (std::f64::consts::FRAC_PI_2 * i as f64 + std::f64::consts::FRAC_PI_4).sin() as f32
            })
            .collect();
        let sample_peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((f64::from(sample_peak) - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        let db = true_peak_db(&[x]);
        assert!(db.abs() <= 0.3, "{db}");
    }

    #[test]
    fn level_true_peak_dc() {
        // A constant 0.5: every oversampled point away from the edges is 0.5 within 0.05 dB
        // (each phase sums to about 1).
        let want = 20.0 * 0.5f64.log10();
        let x = vec![0.5f32; 4410];
        let tp = frame_peaks(std::slice::from_ref(&x));
        for (i, p) in tp.iter().enumerate().take(4410 - 12).skip(12) {
            assert!((20.0 * p.log10() - want).abs() <= 0.05, "frame {i}: {p}");
        }
        // The abrupt start and end are steps, whose ringing the measure includes.
        assert!(true_peak_linear(&[x]) > 0.5);
        // Faded in and out (10 ms raised cosine), the whole measure is 0.5 within 0.05 dB.
        let fade = 441;
        let x: Vec<f32> = (0..4410)
            .map(|i| {
                let edge = i.min(4409 - i);
                let w = if edge < fade {
                    0.5 - 0.5 * (std::f64::consts::PI * edge as f64 / fade as f64).cos()
                } else {
                    1.0
                };
                (0.5 * w) as f32
            })
            .collect();
        let db = true_peak_db(&[x]);
        assert!((db - want).abs() <= 0.05, "{db}");
    }

    #[test]
    fn level_true_peak_silence_and_empty() {
        assert_eq!(true_peak_linear(&[]), 0.0);
        assert_eq!(true_peak_linear(&[vec![], vec![]]), 0.0);
        assert_eq!(true_peak_db(&vec![vec![0.0; 1000]; 2]), f64::NEG_INFINITY);
        assert_eq!(true_peak_db(&[]), f64::NEG_INFINITY);
        assert!(frame_peaks(&[]).is_empty());
        assert!(frame_peaks(&[vec![]]).is_empty());
        assert_eq!(frame_peaks(&[vec![0.0; 5]]), vec![0.0; 5]);
    }

    #[test]
    fn level_true_peak_non_finite_is_nan() {
        assert!(true_peak_linear(&[vec![0.0, f32::NAN, 0.0]]).is_nan());
        assert!(true_peak_linear(&[vec![0.0, f32::INFINITY, 0.0]]).is_nan());
        assert!(
            frame_peaks(&[vec![0.0, f32::NAN, 0.0]])
                .iter()
                .any(|v| v.is_nan())
        );
    }

    #[test]
    fn level_true_peak_meter_equals_the_whole_buffer_measure() {
        let mut state: u32 = 7;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (f64::from(state) / f64::from(u32::MAX) * 2.4 - 1.2) as f32
        };
        for (nch, frames) in [(1, 1), (2, 3), (2, 1000), (3, 777)] {
            let planar: Vec<Vec<f32>> = (0..nch)
                .map(|_| (0..frames).map(|_| next()).collect())
                .collect();
            let interleaved: Vec<f32> = (0..frames)
                .flat_map(|i| planar.iter().map(move |c| c[i]))
                .collect();
            let want = true_peak_linear(&planar);
            // Pushed whole, and in uneven whole-frame pieces.
            let mut whole = TruePeakMeter::new(nch);
            whole.push_interleaved(&interleaved);
            assert_eq!(whole.linear().to_bits(), want.to_bits(), "{nch}x{frames}");
            let mut pieces = TruePeakMeter::new(nch);
            for piece in interleaved.chunks(nch * 7) {
                pieces.push_interleaved(piece);
            }
            assert_eq!(pieces.linear().to_bits(), want.to_bits(), "{nch}x{frames}");
            // Odd-sized pieces that split frames: the next push carries on mid-frame.
            for size in [1, 5, 7, 13] {
                let mut split = TruePeakMeter::new(nch);
                for piece in interleaved.chunks(size) {
                    split.push_interleaved(piece);
                }
                assert_eq!(
                    split.linear().to_bits(),
                    want.to_bits(),
                    "{nch}x{frames} in pieces of {size}"
                );
            }
        }
        assert_eq!(TruePeakMeter::new(2).linear(), 0.0);
        let mut m = TruePeakMeter::new(2);
        m.push_interleaved(&[0.0, f32::NAN]);
        assert!(m.linear().is_nan());
    }

    #[test]
    fn level_frame_peaks_cover_every_point() {
        // Frame peaks' maximum equals the full measure, for short and long, mono and stereo input
        // (head and tail fold into the end frames).
        let cases: Vec<Vec<Vec<f32>>> = vec![
            vec![vec![1.0]],
            vec![vec![0.0, 0.0, 1.0]],
            vec![vec![1.0, -1.0, 1.0, -1.0, 0.3]],
            vec![
                (0..300)
                    .map(|i| ((i * 37 % 101) as f32 / 50.0) - 1.0)
                    .collect(),
                (0..300)
                    .map(|i| ((i * 53 % 97) as f32 / 40.0) - 1.2)
                    .collect(),
            ],
        ];
        for (n, c) in cases.iter().enumerate() {
            let tp = frame_peaks(c);
            assert_eq!(tp.len(), c[0].len(), "case {n}");
            let max = tp.iter().fold(0.0f64, |m, v| m.max(*v));
            assert_eq!(max, true_peak_linear(c), "case {n}");
        }
    }

    #[test]
    fn level_frame_peaks_are_the_points_near_each_sample() {
        // An isolated impulse at frame 20 of 41: frame 20 sees the main lobe (0.972), its
        // neighbours the side lobes; the frame's own value comes from phases at +-0.125 and
        // +-0.375 around it.
        let mut x = vec![0.0f32; 41];
        x[20] = 1.0;
        let tp = frame_peaks(&[x]);
        assert_eq!(tp[20], 0.9721679687500);
        assert_eq!(tp[19], 0.4650878906250);
        assert_eq!(tp[21], 0.4650878906250);
        assert_eq!(tp[0], 0.0);
        assert_eq!(tp[40], 0.0);
    }
}
