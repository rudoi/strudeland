//! Encoders: WAV, MP3, Ogg Vorbis and Opus, the verification decoders and the correction loop.

pub mod mp3;
pub mod ogg;
pub mod opus;
pub mod verify;
pub mod wav;

use std::time::Instant;

use crate::level::{LoudnessSummary, prepare};
use crate::{EncoderOptions, Format, Loudness, OutputOptions, Progress, RenderError};

/// Encodes `channels` at `sample_rate` once with `encoder`. No leveling, no verification.
pub fn encode_once(
    channels: &[Vec<f32>],
    sample_rate: u32,
    encoder: &EncoderOptions,
) -> Result<Vec<u8>, RenderError> {
    match *encoder {
        EncoderOptions::Wav { bit_depth } => wav::encode_wav(channels, sample_rate, bit_depth),
        EncoderOptions::Mp3 { mode, vbr_quality } => {
            mp3::encode_mp3(channels, sample_rate, mode, vbr_quality)
        }
        EncoderOptions::Ogg { quality } => ogg::encode_ogg(channels, sample_rate, quality),
        EncoderOptions::Opus { kbps } => opus::encode_opus(channels, sample_rate, kbps),
    }
}

/// Encode passes at most: the first encode and two corrections.
pub const MAX_ENCODES: u32 = 3;

/// The margin added to a lossy file's overshoot when the limiter ceiling is lowered, in dB.
pub const CORRECTION_MARGIN_DB: f64 = 0.1;

/// A leveled, encoded and (for lossy formats) verified buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct Encoded {
    pub bytes: Vec<u8>,
    /// The last pass's loudness, with `decoded_true_peak_db` and `encodes` set.
    pub loudness: LoudnessSummary,
    /// Time spent leveling, over every pass.
    pub level_ms: f64,
    /// Time spent encoding and decoding to verify, over every pass.
    pub encode_ms: f64,
}

/// The encoder and the verification decoder, behind a seam so tests can force an overshoot.
trait Codec {
    fn encode(
        &self,
        channels: &[Vec<f32>],
        sample_rate: u32,
        encoder: &EncoderOptions,
    ) -> Result<Vec<u8>, RenderError>;
    /// The decoded file's true peak in dBTP.
    fn decoded_true_peak_db(&self, format: Format, bytes: &[u8]) -> Result<f64, RenderError>;
}

/// The real encoders and decoders.
struct Real;

impl Codec for Real {
    fn encode(
        &self,
        channels: &[Vec<f32>],
        sample_rate: u32,
        encoder: &EncoderOptions,
    ) -> Result<Vec<u8>, RenderError> {
        encode_once(channels, sample_rate, encoder)
    }

    fn decoded_true_peak_db(&self, format: Format, bytes: &[u8]) -> Result<f64, RenderError> {
        verify::decoded_true_peak_db(format, bytes)
    }
}

/// Levels and encodes `input`. A lossy file is decoded again and its true peak measured over the
/// whole decoded stream; with loudness on and that peak over the ceiling, the limiter's ceiling is
/// lowered by the overshoot plus 0.1 dB and the original input leveled again (the same static
/// gain, so the loudness target holds, then the limiter at the lowered ceiling) and encoded again,
/// at most twice; a peak still over the ceiling after that is a warning. Lowering the gain instead
/// would not help a song the limiter already holds at the ceiling. With loudness off the decoded
/// peak is reported and never corrected. The input is checked, measured and its gain worked out
/// once; each pass only limits, measures and encodes. Every encode pass is reported as
/// `Progress::Encoded`.
pub fn encode(
    input: &[Vec<f32>],
    sample_rate: u32,
    out: &OutputOptions,
    progress: &dyn Fn(Progress),
    warnings: &mut Vec<String>,
) -> Result<Encoded, RenderError> {
    encode_with(&Real, input, sample_rate, out, progress, warnings)
}

fn encode_with(
    codec: &dyn Codec,
    input: &[Vec<f32>],
    sample_rate: u32,
    out: &OutputOptions,
    progress: &dyn Fn(Progress),
    warnings: &mut Vec<String>,
) -> Result<Encoded, RenderError> {
    let format = out.encoder.format();
    let mut level_ms = 0.0;
    let mut encode_ms = 0.0;
    let mut trim = 0.0;
    let mut pass = 1;
    // The limiter's and silence warnings of the corrected passes replace the first pass's.
    let base = warnings.len();
    let started = Instant::now();
    let prepared = prepare(input, sample_rate, &out.loudness)?;
    level_ms += ms_since(started);
    loop {
        warnings.truncate(base);
        let started = Instant::now();
        let leveled = prepared.level(trim, warnings)?;
        level_ms += ms_since(started);

        let started = Instant::now();
        let bytes = codec.encode(&leveled.channels, sample_rate, &out.encoder)?;
        progress(Progress::Encoded { pass });
        let mut loudness = leveled.summary;
        loudness.encodes = pass;
        if format == Format::Wav {
            encode_ms += ms_since(started);
            return Ok(Encoded {
                bytes,
                loudness,
                level_ms,
                encode_ms,
            });
        }
        // The leveled buffer is no longer needed: free it before decoding.
        drop(leveled.channels);
        let dp = codec.decoded_true_peak_db(format, &bytes)?;
        encode_ms += ms_since(started);
        loudness.decoded_true_peak_db = Some(dp);

        let ceiling = match out.loudness {
            Loudness::On { true_peak_db, .. } if dp > true_peak_db => true_peak_db,
            _ => {
                return Ok(Encoded {
                    bytes,
                    loudness,
                    level_ms,
                    encode_ms,
                });
            }
        };
        if pass >= MAX_ENCODES {
            warnings.push(format!(
                "lossy encode: decoded true peak {dp:.2} dBTP is above the ceiling {ceiling:.1} dBTP after {} corrections",
                MAX_ENCODES - 1
            ));
            return Ok(Encoded {
                bytes,
                loudness,
                level_ms,
                encode_ms,
            });
        }
        // The next limiter ceiling: the previous one minus the overshoot and the margin.
        trim -= dp - ceiling + CORRECTION_MARGIN_DB;
        pass += 1;
    }
}

fn ms_since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BitDepth;
    use crate::level::{level, true_peak_db};

    /// Encodes as raw planar f32 and decodes with a gain of `boost_db[pass - 1]` dB (the last
    /// entry for later passes), so a test decides how far each pass overshoots.
    struct Stub {
        boost_db: Vec<f64>,
        decodes: std::cell::Cell<usize>,
    }

    impl Stub {
        fn new(boost_db: &[f64]) -> Self {
            Stub {
                boost_db: boost_db.to_vec(),
                decodes: std::cell::Cell::new(0),
            }
        }
    }

    impl Codec for Stub {
        fn encode(
            &self,
            channels: &[Vec<f32>],
            _sample_rate: u32,
            _encoder: &EncoderOptions,
        ) -> Result<Vec<u8>, RenderError> {
            Ok(channels
                .iter()
                .flatten()
                .flat_map(|x| x.to_le_bytes())
                .collect())
        }

        fn decoded_true_peak_db(&self, format: Format, bytes: &[u8]) -> Result<f64, RenderError> {
            Ok(true_peak_db(&self.decode(format, bytes)))
        }
    }

    impl Stub {
        fn decode(&self, _format: Format, bytes: &[u8]) -> Vec<Vec<f32>> {
            let n = self.decodes.get();
            self.decodes.set(n + 1);
            let db = self.boost_db[n.min(self.boost_db.len() - 1)];
            let g = 10f64.powf(db / 20.0);
            let all: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|b| (f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])) * g) as f32)
                .collect();
            let half = all.len() / 2;
            vec![all[..half].to_vec(), all[half..].to_vec()]
        }
    }

    /// Two seconds of dense seeded noise (an LCG), loud enough that leveling to -3 LUFS needs
    /// the limiter.
    fn noise() -> Vec<Vec<f32>> {
        let mut state: u32 = 0x1234_5678;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (f64::from(state) / f64::from(u32::MAX) * 2.0 - 1.0) as f32 * 0.5
        };
        let frames = 88_200;
        let l: Vec<f32> = (0..frames).map(|_| next()).collect();
        let r: Vec<f32> = (0..frames).map(|_| next()).collect();
        vec![l, r]
    }

    const LOUD: Loudness = Loudness::On {
        target_lufs: -3.0,
        true_peak_db: -1.0,
    };

    fn ogg_out(loudness: Loudness) -> OutputOptions {
        OutputOptions {
            loudness,
            encoder: EncoderOptions::Ogg { quality: 6 },
        }
    }

    fn run(
        stub: &Stub,
        input: &[Vec<f32>],
        out: &OutputOptions,
    ) -> (Encoded, Vec<Progress>, Vec<String>) {
        let events = std::cell::RefCell::new(Vec::new());
        let mut warnings = vec!["render: earlier".to_string()];
        let e = encode_with(
            stub,
            input,
            44_100,
            out,
            &|p| events.borrow_mut().push(p),
            &mut warnings,
        )
        .unwrap();
        (e, events.into_inner(), warnings)
    }

    fn passes(n: u32) -> Vec<Progress> {
        (1..=n).map(|pass| Progress::Encoded { pass }).collect()
    }

    #[test]
    fn encode_loop_one_pass_when_the_decoded_peak_is_under_the_ceiling() {
        let stub = Stub::new(&[0.0]);
        let (e, events, warnings) = run(&stub, &noise(), &ogg_out(LOUD));
        assert_eq!(e.loudness.encodes, 1);
        assert_eq!(events, passes(1));
        assert_eq!(warnings, vec!["render: earlier".to_string()]);
        assert!(e.loudness.limiter_samples > 0, "{:?}", e.loudness);
        let dp = e.loudness.decoded_true_peak_db.unwrap();
        assert_eq!(dp, e.loudness.output_true_peak_db);
        assert!(dp <= -1.0, "{dp}");
    }

    #[test]
    fn encode_loop_corrects_one_overshoot() {
        let input = noise();
        let first = run(&Stub::new(&[0.0]), &input, &ogg_out(LOUD)).0;
        // What the first pass decodes to: its leveled output plus 0.5 dB.
        let leveled = level(&input, 44_100, &LOUD, 0.0, &mut Vec::new()).unwrap();
        let probe = Stub::new(&[0.5]);
        let enc = EncoderOptions::Ogg { quality: 6 };
        let dp1 = true_peak_db(&probe.decode(
            Format::Ogg,
            &probe.encode(&leveled.channels, 44_100, &enc).unwrap(),
        ));
        let stub = Stub::new(&[0.5, 0.0]);
        let (e, events, warnings) = run(&stub, &input, &ogg_out(LOUD));
        assert_eq!(e.loudness.encodes, 2);
        assert_eq!(events, passes(2));
        assert_eq!(warnings, vec!["render: earlier".to_string()]);
        // The limiter ceiling fell by the first pass's overshoot plus 0.1 dB; the gain stayed.
        assert!(dp1 > -1.0, "{dp1}");
        assert_eq!(e.loudness.gain_db, first.loudness.gain_db);
        assert_eq!(e.loudness.ceiling_dbtp, Some(-1.0));
        assert_eq!(first.loudness.limiter_ceiling_dbtp, Some(-1.0));
        assert_eq!(
            e.loudness.limiter_ceiling_dbtp,
            Some(-1.0 + (0.0 - (dp1 - -1.0 + CORRECTION_MARGIN_DB)))
        );
        assert!(e.loudness.output_true_peak_db <= e.loudness.limiter_ceiling_dbtp.unwrap());
        assert!(e.loudness.decoded_true_peak_db.unwrap() <= -1.0);
    }

    #[test]
    fn encode_loop_converges_on_a_limited_signal_with_a_steady_overshoot() {
        // A codec that always adds 1 dB to a signal the limiter holds at the ceiling: lowering
        // the limiter ceiling (not the gain) brings it under in one correction.
        let first = run(&Stub::new(&[0.0]), &noise(), &ogg_out(LOUD)).0;
        let stub = Stub::new(&[1.0]);
        let (e, events, warnings) = run(&stub, &noise(), &ogg_out(LOUD));
        assert_eq!(e.loudness.encodes, 2);
        assert_eq!(events, passes(2));
        assert_eq!(warnings, vec!["render: earlier".to_string()]);
        assert!(e.loudness.decoded_true_peak_db.unwrap() <= -1.0);
        assert_eq!(e.loudness.gain_db, first.loudness.gain_db);
        assert!(
            e.loudness.limiter_ceiling_dbtp.unwrap() < -2.0,
            "{:?}",
            e.loudness
        );
    }

    #[test]
    fn encode_loop_stops_after_two_corrections_and_warns() {
        // An overshoot that grows faster than each correction.
        let first = run(&Stub::new(&[0.0]), &noise(), &ogg_out(LOUD)).0;
        let stub = Stub::new(&[1.0, 2.5, 4.0]);
        let (e, events, warnings) = run(&stub, &noise(), &ogg_out(LOUD));
        assert_eq!(e.loudness.encodes, MAX_ENCODES);
        assert_eq!(events, passes(3));
        assert_eq!(stub.decodes.get(), 3);
        assert_eq!(e.loudness.gain_db, first.loudness.gain_db);
        assert!(
            e.loudness.limiter_ceiling_dbtp.unwrap() < -3.0,
            "{:?}",
            e.loudness
        );
        let dp = e.loudness.decoded_true_peak_db.unwrap();
        assert!(dp > -1.0, "{dp}");
        assert_eq!(
            warnings,
            vec![
                "render: earlier".to_string(),
                format!(
                    "lossy encode: decoded true peak {dp:.2} dBTP is above the ceiling -1.0 dBTP after 2 corrections"
                )
            ]
        );
    }

    #[test]
    fn encode_loop_never_corrects_with_loudness_off() {
        let input: Vec<Vec<f32>> = noise()
            .into_iter()
            .map(|c| c.into_iter().map(|x| x * 2.5).collect())
            .collect();
        let stub = Stub::new(&[3.0]);
        let (e, events, warnings) = run(&stub, &input, &ogg_out(Loudness::Off));
        assert_eq!(e.loudness.encodes, 1);
        assert_eq!(events, passes(1));
        assert!(!e.loudness.enabled);
        // The bytes are the input, untouched.
        assert_eq!(
            e.bytes,
            stub.encode(&input, 44_100, &EncoderOptions::Ogg { quality: 6 })
                .unwrap()
        );
        let tp = e.loudness.input_true_peak_db;
        assert!(tp > 0.0, "{tp}");
        let dp = e.loudness.decoded_true_peak_db.unwrap();
        assert!((dp - (tp + 3.0)).abs() < 1e-3, "{dp} {tp}");
        // Only the clipping warning of loudness off; nothing about the decoded peak.
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(
            warnings[1].starts_with("loudness off: true peak is "),
            "{warnings:?}"
        );
    }

    #[test]
    fn encode_loop_does_not_decode_wav() {
        let stub = Stub::new(&[10.0]);
        let out = OutputOptions {
            loudness: LOUD,
            encoder: EncoderOptions::Wav {
                bit_depth: BitDepth::F32,
            },
        };
        let (e, events, _) = run(&stub, &noise(), &out);
        assert_eq!(stub.decodes.get(), 0);
        assert_eq!(e.loudness.encodes, 1);
        assert_eq!(e.loudness.decoded_true_peak_db, None);
        assert_eq!(events, passes(1));
    }

    #[test]
    fn encode_loop_silence_gets_no_gain_and_no_correction() {
        let stub = Stub::new(&[1.0]);
        let (e, events, warnings) = run(
            &stub,
            &[vec![0.0; 44_100], vec![0.0; 44_100]],
            &ogg_out(LOUD),
        );
        assert_eq!(e.loudness.encodes, 1);
        assert_eq!(events, passes(1));
        assert_eq!(e.loudness.gain_db, 0.0);
        assert_eq!(e.loudness.decoded_true_peak_db, Some(f64::NEG_INFINITY));
        assert_eq!(
            warnings,
            vec![
                "render: earlier".to_string(),
                crate::level::SILENCE_WARNING.to_string()
            ]
        );
    }

    #[test]
    fn encode_loop_errors_pass_through() {
        let events = std::cell::RefCell::new(Vec::new());
        let e = encode(
            &[vec![0.0; 10], vec![0.0; 11]],
            44_100,
            &ogg_out(LOUD),
            &|p| events.borrow_mut().push(p),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(matches!(e, RenderError::Render(_)), "{e:?}");
        assert!(events.into_inner().is_empty());
    }
}
