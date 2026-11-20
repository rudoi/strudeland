//! Loudness: integrated loudness, true peak, the static gain and the true-peak limiter.
//!
//! [`prepare`] checks and measures the joined buffer and works out one static gain to bring it to
//! the target integrated loudness (one f32 rounding); [`Prepared::level`] applies the gain and the
//! true-peak limiter at a ceiling, and measures again. The encoder's correction loop prepares once
//! and levels once per pass. Silence (integrated loudness `-inf`) gets no gain and a warning. With
//! loudness off the buffer comes back bit for bit, borrowed.

pub mod limiter;
pub mod loudness;
pub mod true_peak;

use std::borrow::Cow;

use serde::Serialize;

pub use loudness::integrated_lufs;
pub use true_peak::{TruePeakMeter, frame_peaks, true_peak_db, true_peak_linear};

use limiter::{apply_one, limiter_gain, touched};
use true_peak::{frame_peaks_by, true_peak_db_by};

use crate::{Loudness, RenderError};

/// The warning for a buffer whose integrated loudness is undefined.
pub const SILENCE_WARNING: &str =
    "loudness: not measurable (shorter than 400 ms or below the -70 LUFS gate); no gain applied";

/// Integrated loudness and true peak of a buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measure {
    /// LUFS; `-inf` when undefined (silence).
    pub lufs: f64,
    /// dBTP; `-inf` for silence.
    pub true_peak_db: f64,
}

/// Measures `channels` at `sample_rate`.
pub fn measure(channels: &[Vec<f32>], sample_rate: u32) -> Measure {
    Measure {
        lufs: integrated_lufs(channels, sample_rate),
        true_peak_db: true_peak_db(channels),
    }
}

/// The loudness part of a render's summary. Non-finite floats serialise as `null`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoudnessSummary {
    /// `true` for [`Loudness::On`].
    pub enabled: bool,
    /// The target; `None` when off.
    pub target_lufs: Option<f64>,
    /// The ceiling; `None` when off.
    pub ceiling_dbtp: Option<f64>,
    /// The ceiling the limiter ran at in the final pass: the ceiling, lowered by the lossy
    /// correction loop when a decoded file overshot; `None` when off.
    pub limiter_ceiling_dbtp: Option<f64>,
    /// Before gain (`-inf`, serialised `null`, for silence).
    pub input_lufs: f64,
    pub input_true_peak_db: f64,
    /// The static gain applied (0 for silence and when off).
    pub gain_db: f64,
    /// Frames the limiter attenuated in its final pass.
    pub limiter_samples: u64,
    /// After gain and limiter.
    pub output_lufs: f64,
    pub output_true_peak_db: f64,
    /// The decoded file's true peak; lossy formats only, set by the encoder.
    pub decoded_true_peak_db: Option<f64>,
    /// Encode passes that ran, set by the encoder (0 until then).
    pub encodes: u32,
}

/// A leveled buffer: the input itself (borrowed) when nothing changed it.
#[derive(Debug, Clone, PartialEq)]
pub struct Leveled<'a> {
    pub channels: Cow<'a, [Vec<f32>]>,
    pub summary: LoudnessSummary,
}

/// A checked and measured input with its static gain: everything [`Prepared::level`] needs that
/// does not depend on the limiter ceiling.
#[derive(Debug, Clone)]
pub struct Prepared<'a> {
    input: &'a [Vec<f32>],
    sample_rate: u32,
    setting: Loudness,
    before: Measure,
    /// The static gain in dB (0 for silence and when off).
    gain_db: f64,
    /// The linear gain; `None` for exactly 0 dB (the samples are not touched).
    gain: Option<f64>,
    /// The frame peaks after the static gain (empty when off).
    peaks: Vec<f64>,
    /// Pushed by every [`Prepared::level`].
    warnings: Vec<String>,
}

/// Checks and measures `input` and works out the static gain for `setting`. Fails on channels of
/// different lengths or a non-finite sample.
pub fn prepare<'a>(
    input: &'a [Vec<f32>],
    sample_rate: u32,
    setting: &Loudness,
) -> Result<Prepared<'a>, RenderError> {
    check_input(input)?;
    let before = measure(input, sample_rate);
    let mut warnings = Vec::new();
    let (gain_db, peaks) = match *setting {
        Loudness::Off => {
            if before.true_peak_db > 0.0 {
                warnings.push(format!(
                    "loudness off: true peak is {:.1} dBTP (above 0 dBFS); the file will clip",
                    before.true_peak_db
                ));
            }
            (0.0, Vec::new())
        }
        Loudness::On { target_lufs, .. } => {
            let gain_db = if before.lufs.is_finite() {
                target_lufs - before.lufs
            } else {
                warnings.push(SILENCE_WARNING.to_string());
                0.0
            };
            let gain = linear_gain(gain_db);
            (gain_db, frame_peaks_by(input, |_, x| gained(x, gain)))
        }
    };
    Ok(Prepared {
        input,
        sample_rate,
        setting: *setting,
        before,
        gain_db,
        gain: linear_gain(gain_db),
        peaks,
        warnings,
    })
}

/// Levels `input`: [`prepare`], then [`Prepared::level`].
pub fn level<'a>(
    input: &'a [Vec<f32>],
    sample_rate: u32,
    setting: &Loudness,
    ceiling_trim_db: f64,
    warnings: &mut Vec<String>,
) -> Result<Leveled<'a>, RenderError> {
    prepare(input, sample_rate, setting)?.level(ceiling_trim_db, warnings)
}

impl<'a> Prepared<'a> {
    /// With [`Loudness::On`], the static gain, then the limiter at the ceiling lowered by
    /// `ceiling_trim_db` (a non-positive amount the encoder's correction loop passes; the static
    /// gain never changes); with [`Loudness::Off`], the input untouched (the trim is ignored).
    /// Warnings are pushed onto `warnings`.
    pub fn level(
        &self,
        ceiling_trim_db: f64,
        warnings: &mut Vec<String>,
    ) -> Result<Leveled<'a>, RenderError> {
        warnings.extend(self.warnings.iter().cloned());
        let before = self.before;
        let Loudness::On {
            target_lufs,
            true_peak_db: ceiling_db,
        } = self.setting
        else {
            return Ok(Leveled {
                channels: Cow::Borrowed(self.input),
                summary: LoudnessSummary {
                    enabled: false,
                    target_lufs: None,
                    ceiling_dbtp: None,
                    limiter_ceiling_dbtp: None,
                    input_lufs: before.lufs,
                    input_true_peak_db: before.true_peak_db,
                    gain_db: 0.0,
                    limiter_samples: 0,
                    output_lufs: before.lufs,
                    output_true_peak_db: before.true_peak_db,
                    decoded_true_peak_db: None,
                    encodes: 0,
                },
            });
        };
        let limiter_ceiling_db = ceiling_db + ceiling_trim_db;
        let (input, gain) = (self.input, self.gain);
        // The limiter judges the gained buffer without building it: frame `i` of channel `c` is
        // `gained(input[c][i])`, then times the limiter's gain `g[i]`.
        let (limiter, out_true_peak_db) =
            limiter_gain(&self.peaks, self.sample_rate, limiter_ceiling_db, |g| {
                true_peak_db_by(input, |i, x| limited(gained(x, gain), g, i))
            });
        let channels: Cow<'a, [Vec<f32>]> = match (&limiter, gain) {
            (None, None) => Cow::Borrowed(input),
            (None, Some(_)) => Cow::Owned(
                input
                    .iter()
                    .map(|ch| ch.iter().map(|&x| gained(x, gain)).collect())
                    .collect(),
            ),
            (Some(g), _) => Cow::Owned(
                input
                    .iter()
                    .map(|ch| {
                        ch.iter()
                            .zip(g)
                            .map(|(&x, &g)| apply_one(gained(x, gain), g))
                            .collect()
                    })
                    .collect(),
            ),
        };
        if let Cow::Owned(out) = &channels {
            check_output(out)?;
        }
        let output_lufs = integrated_lufs(&channels, self.sample_rate);
        if out_true_peak_db > limiter_ceiling_db {
            warnings.push(format!(
                "limiter: true peak is {:.2} dBTP, above the ceiling {:.2} dBTP",
                out_true_peak_db, limiter_ceiling_db
            ));
        }
        Ok(Leveled {
            channels,
            summary: LoudnessSummary {
                enabled: true,
                target_lufs: Some(target_lufs),
                ceiling_dbtp: Some(ceiling_db),
                limiter_ceiling_dbtp: Some(limiter_ceiling_db),
                input_lufs: before.lufs,
                input_true_peak_db: before.true_peak_db,
                gain_db: self.gain_db,
                limiter_samples: limiter.as_deref().map_or(0, touched),
                output_lufs,
                output_true_peak_db: out_true_peak_db,
                decoded_true_peak_db: None,
                encodes: 0,
            },
        })
    }
}

/// `10^(gain_db / 20)`; `None` (untouched) for exactly 0 dB.
fn linear_gain(gain_db: f64) -> Option<f64> {
    (gain_db != 0.0).then(|| 10f64.powf(gain_db / 20.0))
}

/// A sample times the static gain, one rounding to f32.
fn gained(x: f32, gain: Option<f64>) -> f32 {
    gain.map_or(x, |g| apply_one(x, g))
}

/// Frame `i` times the limiter gain `g[i]`, one rounding to f32.
fn limited(x: f32, g: &[f64], i: usize) -> f32 {
    g.get(i).map_or(x, |&g| apply_one(x, g))
}

/// Equal channel lengths and finite samples (the first non-finite sample in frame order is
/// named).
pub(crate) fn check_input(input: &[Vec<f32>]) -> Result<(), RenderError> {
    if let Some(first) = input.first()
        && input.iter().any(|c| c.len() != first.len())
    {
        let lens: Vec<String> = input.iter().map(|c| c.len().to_string()).collect();
        return Err(RenderError::Render(format!(
            "the render's channels differ in length ({})",
            lens.join(", ")
        )));
    }
    // Each channel scanned contiguously; a later channel only up to the frame found so far (a
    // tie in frame goes to the lower channel).
    let mut first: Option<(usize, usize)> = None;
    for (c, ch) in input.iter().enumerate() {
        let end = first.map_or(ch.len(), |(frame, _)| frame.min(ch.len()));
        if let Some(frame) = ch
            .get(..end)
            .and_then(|s| s.iter().position(|x| !x.is_finite()))
        {
            first = Some((frame, c));
        }
    }
    match first {
        Some((frame, c)) => Err(RenderError::Render(format!(
            "the render contains a non-finite sample (frame {frame}, channel {c})"
        ))),
        None => Ok(()),
    }
}

/// The gain cannot overflow a sample whose loudness was measurable, but a non-finite output is
/// never returned.
fn check_output(out: &[Vec<f32>]) -> Result<(), RenderError> {
    for (c, ch) in out.iter().enumerate() {
        if let Some(frame) = ch.iter().position(|x| !x.is_finite()) {
            return Err(RenderError::Render(format!(
                "leveling produced a non-finite sample (frame {frame}, channel {c})"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;

    fn sine(hz: f64, db: f64, frames: usize) -> Vec<f32> {
        let amp = 10f64.powf(db / 20.0);
        (0..frames)
            .map(|i| {
                (amp * (2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(SR)).sin()) as f32
            })
            .collect()
    }

    fn on(target: f64, ceiling: f64) -> Loudness {
        Loudness::On {
            target_lufs: target,
            true_peak_db: ceiling,
        }
    }

    fn bits(c: &[Vec<f32>]) -> Vec<Vec<u32>> {
        c.iter()
            .map(|ch| ch.iter().map(|x| x.to_bits()).collect())
            .collect()
    }

    #[test]
    fn level_gain_lands_on_the_target() {
        let x = vec![sine(1000.0, -30.0, 441_000); 2];
        let mut w = Vec::new();
        let out = level(&x, SR, &on(-14.0, -1.0), 0.0, &mut w).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(out.summary.limiter_samples, 0);
        assert!(
            (out.summary.output_lufs + 14.0).abs() <= 0.05,
            "{:?}",
            out.summary
        );
        assert!(
            (out.summary.input_lufs + 30.0).abs() <= 0.1,
            "{:?}",
            out.summary
        );
        assert_eq!(out.summary.gain_db, -14.0 - out.summary.input_lufs);
        // One rounding to f32 of the f64 product.
        let g = 10f64.powf(out.summary.gain_db / 20.0);
        assert_eq!(out.channels[0][1234], (f64::from(x[0][1234]) * g) as f32);
        assert!(out.summary.output_true_peak_db <= -1.0);
    }

    #[test]
    fn level_hot_transient_ends_at_or_below_the_target() {
        let mut l = sine(1000.0, -30.0, 441_000);
        let mut r = l.clone();
        for i in 200_000..200_050 {
            l[i] = 0.9;
            r[i] = -0.9;
        }
        let x = vec![l, r];
        let mut w = Vec::new();
        let out = level(&x, SR, &on(-14.0, -1.0), 0.0, &mut w).unwrap();
        assert!(out.summary.limiter_samples > 0, "{:?}", out.summary);
        assert!(out.summary.output_lufs <= -14.0 + 1e-9, "{:?}", out.summary);
        assert!(out.summary.output_true_peak_db <= -1.0, "{:?}", out.summary);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(out.channels[0].len(), 441_000);
        assert!(out.channels.iter().flatten().all(|v| v.is_finite()));
    }

    #[test]
    fn level_silence_gets_no_gain_and_one_warning() {
        let below = vec![sine(1000.0, -75.0, 220_500); 2];
        let short = vec![sine(1000.0, -20.0, 13_230); 2];
        for (name, x) in [
            ("zeros", vec![vec![0.0f32; 132_300]; 2]),
            ("below gate", below),
            ("short", short),
            ("empty", vec![vec![], vec![]]),
            ("no channels", vec![]),
        ] {
            for trim in [0.0, -0.7] {
                let mut w = Vec::new();
                let out = level(&x, SR, &on(-14.0, -1.0), trim, &mut w).unwrap();
                assert_eq!(out.summary.gain_db, 0.0, "{name}");
                assert_eq!(out.summary.gain_db, 0.0, "{name}");
                assert_eq!(w, vec![SILENCE_WARNING.to_string()], "{name}");
                assert_eq!(bits(&out.channels), bits(&x), "{name}");
                assert_eq!(out.summary.input_lufs, f64::NEG_INFINITY, "{name}");
                assert_eq!(out.summary.limiter_samples, 0, "{name}");
            }
        }
        assert_eq!(
            SILENCE_WARNING,
            "loudness: not measurable (shorter than 400 ms or below the -70 LUFS gate); no gain applied"
        );
    }

    #[test]
    fn level_silence_over_the_ceiling_is_still_limited() {
        // A short buffer has no loudness but its peaks still meet the ceiling.
        let x = vec![sine(1000.0, 3.0, 4410); 2];
        let mut w = Vec::new();
        let out = level(&x, SR, &on(-14.0, -1.0), 0.0, &mut w).unwrap();
        assert_eq!(out.summary.gain_db, 0.0);
        assert!(out.summary.limiter_samples > 0);
        assert!(out.summary.output_true_peak_db <= -1.0);
        assert_eq!(w, vec![SILENCE_WARNING.to_string()]);
    }

    #[test]
    fn level_ceiling_trim_lowers_the_limiter_ceiling_and_keeps_the_gain() {
        // Leveled to -14 LUFS, a sine peaks near -11 dBTP: a -16 dBTP ceiling limits it.
        let x = vec![sine(1000.0, -30.0, 441_000); 2];
        let mut w = Vec::new();
        let base = level(&x, SR, &on(-14.0, -16.0), 0.0, &mut w).unwrap();
        let trimmed = level(&x, SR, &on(-14.0, -16.0), -0.35, &mut w).unwrap();
        assert_eq!(trimmed.summary.gain_db, base.summary.gain_db);
        assert_eq!(trimmed.summary.ceiling_dbtp, Some(-16.0));
        assert_eq!(base.summary.limiter_ceiling_dbtp, Some(-16.0));
        assert_eq!(trimmed.summary.limiter_ceiling_dbtp, Some(-16.0 - 0.35));
        assert!(base.summary.output_true_peak_db <= -16.0);
        assert!(base.summary.limiter_samples > 0);
        assert!(trimmed.summary.output_true_peak_db < base.summary.output_true_peak_db);
        assert!(trimmed.summary.output_true_peak_db <= -16.35);
        assert!(w.is_empty(), "{w:?}");
        // Loudness off ignores the trim.
        let off = level(&x, SR, &Loudness::Off, -3.0, &mut w).unwrap();
        assert_eq!(bits(&off.channels), bits(&x));
        assert_eq!(off.summary.limiter_ceiling_dbtp, None);
    }

    #[test]
    fn level_zero_gain_leaves_samples_untouched() {
        let x = vec![sine(1000.0, -20.0, 88_200); 2];
        let lufs = integrated_lufs(&x, SR);
        let mut w = Vec::new();
        let out = level(&x, SR, &on(lufs, -1.0), 0.0, &mut w).unwrap();
        assert_eq!(out.summary.gain_db, 0.0);
        assert_eq!(bits(&out.channels), bits(&x));
    }

    #[test]
    fn level_off_returns_the_input_bit_for_bit() {
        let x = vec![sine(1000.0, -6.0, 44_100), sine(500.0, -12.0, 44_100)];
        let mut w = Vec::new();
        let out = level(&x, SR, &Loudness::Off, 0.0, &mut w).unwrap();
        assert_eq!(bits(&out.channels), bits(&x));
        assert!(w.is_empty(), "{w:?}");
        let s = &out.summary;
        assert!(!s.enabled);
        assert_eq!((s.target_lufs, s.ceiling_dbtp), (None, None));
        assert_eq!((s.gain_db, s.limiter_samples), (0.0, 0));
        assert_eq!(s.input_lufs, s.output_lufs);
        assert_eq!(s.input_true_peak_db, s.output_true_peak_db);
        assert!(s.input_lufs.is_finite());

        // Above 0 dBFS: reported, still untouched.
        let hot = vec![sine(1000.0, 2.0, 44_100); 2];
        let mut w = Vec::new();
        let out = level(&hot, SR, &Loudness::Off, 0.0, &mut w).unwrap();
        assert_eq!(bits(&out.channels), bits(&hot));
        assert_eq!(out.summary.limiter_samples, 0);
        let tp = true_peak_db(&hot);
        assert_eq!(
            w,
            vec![format!(
                "loudness off: true peak is {tp:.1} dBTP (above 0 dBFS); the file will clip"
            )]
        );
        assert!(
            w[0].starts_with("loudness off: true peak is 2.0 dBTP"),
            "{w:?}"
        );
    }

    #[test]
    fn level_rejects_non_finite_and_ragged_input() {
        for (bad, frame, ch) in [
            (f32::NAN, 7, 1),
            (f32::INFINITY, 3, 0),
            (f32::NEG_INFINITY, 0, 1),
        ] {
            let mut x = vec![vec![0.1f32; 20]; 2];
            x[ch][frame] = bad;
            x[1 - ch][frame + 5] = f32::NAN;
            for setting in [Loudness::Off, on(-14.0, -1.0)] {
                let err = level(&x, SR, &setting, 0.0, &mut Vec::new()).unwrap_err();
                assert_eq!(
                    err.to_string(),
                    format!(
                        "render failed: the render contains a non-finite sample (frame {frame}, channel {ch})"
                    )
                );
            }
        }
        let err = level(
            &[vec![0.0; 3], vec![0.0; 4]],
            SR,
            &on(-14.0, -1.0),
            0.0,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "render failed: the render's channels differ in length (3, 4)"
        );
    }

    #[test]
    fn level_extreme_values_never_panic_or_leak_non_finite() {
        let cases: Vec<(&str, Vec<Vec<f32>>)> = vec![
            ("max", vec![vec![f32::MAX, -f32::MAX, f32::MAX, 0.0]; 2]),
            (
                "max long",
                vec![
                    (0..44_100)
                        .map(|i| if i % 3 == 0 { f32::MAX } else { -f32::MAX })
                        .collect();
                    2
                ],
            ),
            ("subnormal", vec![vec![f32::MIN_POSITIVE / 4.0; 44_100]; 2]),
            ("one frame", vec![vec![0.5]; 2]),
        ];
        for (name, x) in cases {
            for setting in [
                Loudness::Off,
                on(-14.0, -1.0),
                on(0.0, 0.0),
                on(-60.0, -20.0),
            ] {
                match level(&x, SR, &setting, 0.0, &mut Vec::new()) {
                    Ok(out) => {
                        assert!(
                            out.channels.iter().flatten().all(|v| v.is_finite()),
                            "{name} {setting:?}"
                        );
                        assert_eq!(out.channels[0].len(), x[0].len(), "{name}");
                    }
                    Err(e) => assert!(matches!(e, RenderError::Render(_)), "{name}: {e}"),
                }
            }
        }
    }

    #[test]
    fn level_borrows_the_input_when_nothing_changes_it() {
        let x = vec![sine(1000.0, -20.0, 88_200); 2];
        let mut w = Vec::new();
        let off = level(&x, SR, &Loudness::Off, 0.0, &mut w).unwrap();
        assert!(matches!(off.channels, Cow::Borrowed(_)));
        let lufs = integrated_lufs(&x, SR);
        let same = level(&x, SR, &on(lufs, -1.0), 0.0, &mut w).unwrap();
        assert!(matches!(same.channels, Cow::Borrowed(_)));
        let louder = level(&x, SR, &on(lufs + 1.0, -1.0), 0.0, &mut w).unwrap();
        assert!(matches!(louder.channels, Cow::Owned(_)));
    }

    #[test]
    fn level_output_true_peak_is_the_measure_of_the_output() {
        let mut hot = sine(1000.0, -30.0, 441_000);
        for v in &mut hot[200_000..200_050] {
            *v = 0.9;
        }
        let quiet = vec![sine(1000.0, -30.0, 441_000); 2];
        let hot = vec![hot.clone(), hot];
        for (name, x, setting) in [
            ("limited", &hot, on(-14.0, -1.0)),
            ("gain only", &quiet, on(-14.0, -1.0)),
            ("tight ceiling", &quiet, on(-14.0, -16.0)),
            ("off", &quiet, Loudness::Off),
        ] {
            let prepared = prepare(x, SR, &setting).unwrap();
            for trim in [0.0, -0.4] {
                let out = prepared.level(trim, &mut Vec::new()).unwrap();
                let m = measure(&out.channels, SR);
                assert_eq!(
                    out.summary.output_true_peak_db.to_bits(),
                    m.true_peak_db.to_bits(),
                    "{name} {trim}"
                );
                assert_eq!(
                    out.summary.output_lufs.to_bits(),
                    m.lufs.to_bits(),
                    "{name} {trim}"
                );
                // Prepared once or every time: the same output.
                let again = level(x, SR, &setting, trim, &mut Vec::new()).unwrap();
                assert_eq!(bits(&again.channels), bits(&out.channels), "{name} {trim}");
                assert_eq!(again.summary, out.summary, "{name} {trim}");
            }
        }
    }

    #[test]
    fn level_names_the_lowest_channel_of_the_first_bad_frame() {
        let mut x = vec![vec![0.1f32; 20]; 3];
        x[2][4] = f32::NAN;
        x[1][4] = f32::INFINITY;
        x[0][9] = f32::NAN;
        let err = level(&x, SR, &Loudness::Off, 0.0, &mut Vec::new()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "render failed: the render contains a non-finite sample (frame 4, channel 1)"
        );
    }

    #[test]
    fn level_mono_works() {
        let x = vec![sine(1000.0, -30.0, 441_000)];
        let mut w = Vec::new();
        let out = level(&x, SR, &on(-16.0, -1.0), 0.0, &mut w).unwrap();
        assert!((out.summary.output_lufs + 16.0).abs() <= 0.05);
        assert_eq!(out.channels.len(), 1);
    }

    #[test]
    fn level_summary_serialises_camel_case_with_nulls() {
        let s = LoudnessSummary {
            enabled: true,
            target_lufs: Some(-14.0),
            ceiling_dbtp: Some(-1.0),
            limiter_ceiling_dbtp: Some(-1.5),
            input_lufs: f64::NEG_INFINITY,
            input_true_peak_db: f64::NEG_INFINITY,
            gain_db: 0.0,
            limiter_samples: 0,
            output_lufs: f64::NEG_INFINITY,
            output_true_peak_db: f64::NEG_INFINITY,
            decoded_true_peak_db: None,
            encodes: 0,
        };
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"enabled":true,"targetLufs":-14.0,"ceilingDbtp":-1.0,"limiterCeilingDbtp":-1.5,"inputLufs":null,"inputTruePeakDb":null,"gainDb":0.0,"limiterSamples":0,"outputLufs":null,"outputTruePeakDb":null,"decodedTruePeakDb":null,"encodes":0}"#
        );
    }

    #[test]
    fn level_measure() {
        assert_eq!(
            measure(&[], SR),
            Measure {
                lufs: f64::NEG_INFINITY,
                true_peak_db: f64::NEG_INFINITY
            }
        );
        let m = measure(&vec![sine(1000.0, -23.0, 882_000); 2], SR);
        assert!((m.lufs + 23.0).abs() < 0.1, "{m:?}");
        assert!((m.true_peak_db + 23.0).abs() < 0.1, "{m:?}");
    }
}
