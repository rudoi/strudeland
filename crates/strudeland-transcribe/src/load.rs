//! Reading a span of a memo WAV, mixing it to mono and resampling it to the analysis rate.

use std::io;
use std::path::Path;

use hound::{SampleFormat, WavReader};
use rubato::audioadapter_buffers::direct::SequentialSlice;
use rubato::{Fft, FixedSync, Resampler};

use crate::consts::{ANALYSIS_RATE, MAX_WHOLE_SECONDS};
use crate::error::TranscribeError;

/// The lowest source rate the resampler accepts; below it one chunk would expand to millions of
/// samples.
pub const MIN_SOURCE_RATE: u32 = 1_000;
/// The highest source rate the resampler accepts; beyond it the FFT sizes grow without a use.
pub const MAX_SOURCE_RATE: u32 = 768_000;

/// A mono span of a memo.
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    /// The channel average of the span.
    pub samples: Vec<f32>,
    /// The file's rate, from the header.
    pub sample_rate: u32,
    /// The memo time of `samples[0]`, in seconds.
    pub start_seconds: f64,
    /// The whole file's length from the header, in seconds.
    pub memo_seconds: f64,
}

/// Which part of the file to read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Span {
    /// The whole file, at most [`MAX_WHOLE_SECONDS`] long.
    Whole,
    /// Seconds from the start of the file; an open end reads to the end of the file.
    Seconds { start: f64, end: Option<f64> },
}

fn wav_error(path: &Path, message: impl std::fmt::Display) -> TranscribeError {
    TranscribeError::Wav {
        path: path.display().to_string(),
        message: message.to_string(),
    }
}

/// Reads `span` of the WAV at `path` as mono, averaging channels. Only the span's frames are
/// read: the reader seeks to the start and stops at the end. An `end` past the file is clamped.
pub fn load_wav_span(path: &Path, span: Span) -> Result<Audio, TranscribeError> {
    let mut reader = WavReader::open(path).map_err(|e| wav_error(path, e))?;
    let spec = reader.spec();
    if spec.sample_rate == 0 || spec.channels == 0 {
        return Err(wav_error(path, "the header has no sample rate or channels"));
    }
    let rate = f64::from(spec.sample_rate);
    let channels = usize::from(spec.channels);
    let total = u64::from(reader.duration());
    let memo_seconds = total as f64 / rate;

    let (start_frame, end_frame) = match span {
        Span::Whole => {
            if memo_seconds > MAX_WHOLE_SECONDS {
                return Err(TranscribeError::TooLong {
                    seconds: memo_seconds,
                });
            }
            (0, total)
        }
        Span::Seconds { start, end } => span_frames(start, end, rate, total, memo_seconds)?,
    };

    let wanted = (end_frame - start_frame) as usize;
    // hound seeks in u32 samples
    let start_sample = start_frame.checked_mul(channels as u64);
    if start_sample.is_none_or(|s| s > u64::from(u32::MAX)) {
        return Err(wav_error(
            path,
            "the span starts beyond what the reader can seek to",
        ));
    }
    reader
        .seek(start_frame as u32)
        .map_err(|e| wav_error(path, e))?;
    let samples = read_mono(&mut reader, channels, wanted).map_err(|e| wav_error(path, e))?;
    if samples.is_empty() {
        return Err(wav_error(path, "the file ends before the span"));
    }

    Ok(Audio {
        samples,
        sample_rate: spec.sample_rate,
        start_seconds: start_frame as f64 / rate,
        memo_seconds,
    })
}

/// The span as `(start frame, end frame)`, validated and clamped to the file.
fn span_frames(
    start: f64,
    end: Option<f64>,
    rate: f64,
    total: u64,
    memo_seconds: f64,
) -> Result<(u64, u64), TranscribeError> {
    let bad = |m: String| Err(TranscribeError::BadSpan(m));
    if !start.is_finite() || start < 0.0 {
        return bad(format!(
            "the span start must be a number of seconds, 0 or more (got {start})"
        ));
    }
    if let Some(e) = end
        && (!e.is_finite() || e < 0.0)
    {
        return bad(format!(
            "the span end must be a number of seconds, 0 or more (got {e})"
        ));
    }
    let start_frame = (start * rate).round() as u64;
    if start_frame >= total {
        return bad(format!(
            "the span start ({start:.3} s) is past the end of the memo ({memo_seconds:.3} s)"
        ));
    }
    let end_frame = match end {
        Some(e) if e <= start => {
            return bad(format!(
                "the span end ({e:.3} s) must be after its start ({start:.3} s)"
            ));
        }
        Some(e) => ((e * rate).round() as u64).min(total),
        None => total,
    };
    if end_frame <= start_frame {
        return bad(format!(
            "the span from {start:.3} s is shorter than one sample"
        ));
    }
    let seconds = (end_frame - start_frame) as f64 / rate;
    if seconds > MAX_WHOLE_SECONDS {
        return bad(format!(
            "the span is {seconds:.0} s long; pick up to 10 minutes"
        ));
    }
    Ok((start_frame, end_frame))
}

/// The message of the error hound 3.5.1 returns (`ErrorKind::Other`) when the reader ends before
/// a sample is complete (`ReadExt::read_into`).
const HOUND_SHORT_READ: &str = "Failed to read enough bytes.";

/// Whether a sample read error is the end of the data (a file shorter than its header claims):
/// hound's short-read error, or `UnexpectedEof` from the reader. Any other I/O error is a failed
/// read, not an end.
fn ends_data(e: &hound::Error) -> bool {
    match e {
        hound::Error::IoError(io) => match io.kind() {
            io::ErrorKind::UnexpectedEof => true,
            io::ErrorKind::Other => io
                .get_ref()
                .is_some_and(|inner| inner.to_string() == HOUND_SHORT_READ),
            _ => false,
        },
        _ => false,
    }
}

/// Reads up to `frames` frames from the reader's position and averages each frame's channels.
/// A file that ends early (a header that claims more data than the file holds) gives the frames
/// it has, which may be none; any other read failure is an error.
fn read_mono<R: io::Read>(
    reader: &mut WavReader<R>,
    channels: usize,
    frames: usize,
) -> Result<Vec<f32>, hound::Error> {
    let spec = reader.spec();
    let wanted = frames * channels;
    let mut raw: Vec<f32> = Vec::with_capacity(wanted.min(1 << 24));
    match spec.sample_format {
        SampleFormat::Float => {
            for s in reader.samples::<f32>().take(wanted) {
                match s {
                    Ok(v) => raw.push(v),
                    Err(e) if ends_data(&e) => break,
                    Err(e) => return Err(e),
                }
            }
        }
        SampleFormat::Int => {
            let scale = 1.0 / (2.0f64.powi(i32::from(spec.bits_per_sample) - 1));
            for s in reader.samples::<i32>().take(wanted) {
                match s {
                    Ok(v) => raw.push((f64::from(v) * scale) as f32),
                    Err(e) if ends_data(&e) => break,
                    Err(e) => return Err(e),
                }
            }
        }
    }
    if channels == 1 {
        return Ok(raw);
    }
    Ok(raw
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect())
}

/// The length of `len` samples at `rate` resampled to the analysis rate: `ceil(len * 16000 / rate)`.
pub fn resampled_len(len: usize, rate: u32) -> usize {
    let num = len as u128 * u128::from(ANALYSIS_RATE);
    num.div_ceil(u128::from(rate.max(1))) as usize
}

/// Resamples mono `samples` from `sample_rate` to 16 kHz with rubato's FFT resampler, the
/// startup delay trimmed so sample 0 lines up with the input. A 16 kHz source is copied. The
/// output has exactly `ceil(len * 16000 / rate)` samples; input shorter than one resampler chunk
/// is zero-padded.
pub fn resample_to_analysis(
    samples: &[f32],
    sample_rate: u32,
) -> Result<Vec<f32>, TranscribeError> {
    if !(MIN_SOURCE_RATE..=MAX_SOURCE_RATE).contains(&sample_rate) {
        return Err(TranscribeError::BadOption(format!(
            "the sample rate must be {MIN_SOURCE_RATE} to {MAX_SOURCE_RATE} Hz (got {sample_rate})"
        )));
    }
    if sample_rate == ANALYSIS_RATE || samples.is_empty() {
        return Ok(samples.to_vec());
    }
    let want = resampled_len(samples.len(), sample_rate);
    let fail = |e: &dyn std::fmt::Display| {
        TranscribeError::BadOption(format!("cannot resample from {sample_rate} Hz: {e}"))
    };
    let mut resampler = Fft::<f32>::new(
        sample_rate as usize,
        ANALYSIS_RATE as usize,
        1024,
        1,
        FixedSync::Input,
    )
    .map_err(|e| fail(&e))?;
    let input = SequentialSlice::new(samples, 1, samples.len()).map_err(|e| fail(&e))?;
    let mut out = resampler
        .process_all(&input, samples.len(), None)
        .map_err(|e| fail(&e))?
        .take_data();
    // rubato rounds the ratio in f64, which can give a frame more or fewer than the exact length
    out.resize(want, 0.0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;
    use std::fs;

    use hound::{WavSpec, WavWriter};

    use super::*;
    use crate::test_support::zero_crossing_hz;

    fn sine(hz: f64, rate: u32, seconds: f64) -> Vec<f32> {
        (0..(seconds * f64::from(rate)) as usize)
            .map(|i| (0.5 * (TAU * hz * i as f64 / f64::from(rate)).sin()) as f32)
            .collect()
    }

    fn write_int(path: &Path, bits: u16, rate: u32, channels: u16, frames: &[Vec<f32>]) {
        let spec = WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: bits,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(path, spec).unwrap();
        let max = 2.0f64.powi(i32::from(bits) - 1);
        for frame in frames {
            for s in frame {
                w.write_sample((f64::from(*s) * (max - 1.0)).round() as i32)
                    .unwrap();
            }
        }
        w.finalize().unwrap();
    }

    fn write_float(path: &Path, rate: u32, channels: u16, frames: &[Vec<f32>]) {
        let spec = WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let mut w = WavWriter::create(path, spec).unwrap();
        for frame in frames {
            for s in frame {
                w.write_sample(*s).unwrap();
            }
        }
        w.finalize().unwrap();
    }

    fn mono_frames(samples: &[f32]) -> Vec<Vec<f32>> {
        samples.iter().map(|s| vec![*s]).collect()
    }

    fn load(path: &Path, span: Span) -> Audio {
        load_wav_span(path, span).expect("loads")
    }

    /// Patches the `data` chunk size of a hound file so it claims `data_bytes` of data.
    fn claim_data_bytes(path: &Path, data_bytes: u32) {
        let mut bytes = fs::read(path).unwrap();
        let at = bytes
            .windows(4)
            .position(|w| w == b"data")
            .expect("a data chunk")
            + 4;
        bytes[at..at + 4].copy_from_slice(&data_bytes.to_le_bytes());
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn reads_float_and_int_pcm() {
        let dir = tempfile::tempdir().unwrap();
        let tone = sine(440.0, 16_000, 0.5);
        let f = dir.path().join("f.wav");
        write_float(&f, 16_000, 1, &mono_frames(&tone));
        let reference = load(&f, Span::Whole);
        assert_eq!(reference.samples, tone, "float PCM is read as it is");
        for bits in [8u16, 16, 24, 32] {
            let p = dir.path().join(format!("i{bits}.wav"));
            write_int(&p, bits, 16_000, 1, &mono_frames(&tone));
            let a = load(&p, Span::Whole);
            assert_eq!(a.samples.len(), tone.len(), "{bits}-bit length");
            let tolerance = 1.5 / 2.0f32.powi(i32::from(bits) - 1) + 1e-6;
            let worst = a
                .samples
                .iter()
                .zip(&tone)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f32::max);
            assert!(worst <= tolerance, "{bits}-bit differs by {worst}");
        }
    }

    #[test]
    fn mixes_channels_to_mono() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.wav");
        let frames: Vec<Vec<f32>> = (0..100)
            .map(|i| vec![i as f32 / 200.0, -0.25, 0.75])
            .collect();
        write_float(&p, 16_000, 3, &frames);
        let a = load(&p, Span::Whole);
        assert_eq!(a.samples.len(), 100);
        for (i, s) in a.samples.iter().enumerate() {
            let expected = (i as f32 / 200.0 - 0.25 + 0.75) / 3.0;
            assert!((s - expected).abs() < 1e-6, "frame {i}: {s} vs {expected}");
        }
        // stereo int16 too
        let q = dir.path().join("q.wav");
        write_int(&q, 16, 16_000, 2, &[vec![0.5, -0.5], vec![0.5, 0.25]]);
        let a = load(&q, Span::Whole);
        assert!(a.samples[0].abs() < 1e-4);
        assert!((a.samples[1] - 0.375).abs() < 1e-3);
    }

    #[test]
    fn reads_only_the_span() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("long.wav");
        // two seconds of ramp on disk; the header claims ten minutes
        let ramp: Vec<f32> = (0..32_000).map(|i| i as f32 / 32_000.0).collect();
        write_float(&p, 16_000, 1, &mono_frames(&ramp));
        claim_data_bytes(&p, 600 * 16_000 * 4);
        let a = load(
            &p,
            Span::Seconds {
                start: 0.5,
                end: Some(1.0),
            },
        );
        assert_eq!(a.samples.len(), 8_000, "exactly the span");
        assert_eq!(a.start_seconds, 0.5);
        assert_eq!(a.memo_seconds, 600.0);
        assert_eq!(a.sample_rate, 16_000);
        assert_eq!(a.samples[0], ramp[8_000], "the span starts at its frame");
        assert_eq!(a.samples[7_999], ramp[15_999]);
    }

    #[test]
    fn truncated_data_returns_the_frames_that_exist() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cut.wav");
        write_float(&p, 16_000, 1, &mono_frames(&vec![0.25; 16_000]));
        claim_data_bytes(&p, 60 * 16_000 * 4);
        let a = load(
            &p,
            Span::Seconds {
                start: 0.0,
                end: Some(5.0),
            },
        );
        assert_eq!(a.samples.len(), 16_000);
    }

    /// Reads `bytes`, then fails with `error` instead of ending.
    struct FailingReader {
        bytes: io::Cursor<Vec<u8>>,
        error: Option<io::Error>,
    }

    impl io::Read for FailingReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.bytes.read(buf)? {
                0 => Err(self
                    .error
                    .take()
                    .unwrap_or_else(|| io::Error::other("again"))),
                n => Ok(n),
            }
        }
    }

    /// A 16-bit mono WAV whose header claims `claimed` samples, holding only `held` of them.
    fn short_wav(claimed: u32, held: usize) -> Vec<u8> {
        let mut cursor = io::Cursor::new(Vec::new());
        let spec = WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::new(&mut cursor, spec).unwrap();
        for _ in 0..claimed {
            w.write_sample(1000i16).unwrap();
        }
        w.finalize().unwrap();
        let mut bytes = cursor.into_inner();
        bytes.truncate(bytes.len() - (claimed as usize - held) * 2);
        bytes
    }

    #[test]
    fn a_read_failure_is_an_error_not_the_end_of_the_data() {
        // the file holds 100 of its 1000 samples, then the reader fails
        let failing = |error: Option<io::Error>| {
            let mut reader = WavReader::new(FailingReader {
                bytes: io::Cursor::new(short_wav(1000, 100)),
                error,
            })
            .unwrap();
            read_mono(&mut reader, 1, 1000)
        };
        let err = failing(Some(io::Error::other("disk on fire"))).unwrap_err();
        assert!(err.to_string().contains("disk on fire"), "{err}");
        let err = failing(Some(io::Error::from(io::ErrorKind::PermissionDenied))).unwrap_err();
        assert!(matches!(err, hound::Error::IoError(_)), "{err}");
        // a reader that ends (read gives 0) is a file shorter than its header: the data it has
        let mut reader = WavReader::new(io::Cursor::new(short_wav(1000, 100))).unwrap();
        assert_eq!(read_mono(&mut reader, 1, 1000).unwrap().len(), 100);
        // so is an UnexpectedEof from the reader
        let samples = failing(Some(io::Error::from(io::ErrorKind::UnexpectedEof))).unwrap();
        assert_eq!(samples.len(), 100);
        // hound's own short-read error is the only Other error that ends the data
        assert!(ends_data(&hound::Error::IoError(io::Error::other(
            HOUND_SHORT_READ
        ))));
        assert!(!ends_data(&hound::Error::IoError(io::Error::other("nope"))));
        assert!(!ends_data(&hound::Error::FormatError("bad")));
    }

    #[test]
    fn whole_over_cap_is_too_long() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.wav");
        write_float(&p, 16_000, 1, &mono_frames(&[0.0; 100]));
        claim_data_bytes(&p, 1832 * 16_000 * 4);
        match load_wav_span(&p, Span::Whole) {
            Err(TranscribeError::TooLong { seconds }) => assert_eq!(seconds, 1832.0),
            other => panic!("expected TooLong, got {other:?}"),
        }
        // exactly at the cap is allowed
        claim_data_bytes(&p, 600 * 16_000 * 4);
        assert!(
            load_wav_span(&p, Span::Whole).is_ok(),
            "600 s is at the cap"
        );
        // a span of a long file works, a span past the cap does not
        claim_data_bytes(&p, 1832 * 16_000 * 4);
        let a = load(
            &p,
            Span::Seconds {
                start: 0.0,
                end: Some(0.003),
            },
        );
        assert_eq!(a.samples.len(), 48);
        let err = load_wav_span(
            &p,
            Span::Seconds {
                start: 0.0,
                end: Some(900.0),
            },
        )
        .unwrap_err();
        assert!(matches!(err, TranscribeError::BadSpan(_)), "{err:?}");
        assert_eq!(err.exit_code(), 4);
    }

    #[test]
    fn bad_span_cases() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.wav");
        write_float(&p, 16_000, 1, &mono_frames(&sine(220.0, 16_000, 2.0)));
        let cases: [(&str, f64, Option<f64>); 7] = [
            ("start equals end", 1.0, Some(1.0)),
            ("start after end", 1.5, Some(1.0)),
            ("negative start", -0.5, Some(1.0)),
            ("negative end", 0.0, Some(-1.0)),
            ("NaN start", f64::NAN, Some(1.0)),
            ("NaN end", 0.0, Some(f64::NAN)),
            ("start past the end", 2.5, None),
        ];
        for (name, start, end) in cases {
            match load_wav_span(&p, Span::Seconds { start, end }) {
                Err(TranscribeError::BadSpan(m)) => {
                    assert!(!m.is_empty(), "{name}: message");
                }
                other => panic!("{name}: expected BadSpan, got {other:?}"),
            }
        }
        for (name, start, end) in [
            ("start at the end", 2.0, None),
            ("infinite end", 0.0, Some(f64::INFINITY)),
        ] {
            assert!(
                matches!(
                    load_wav_span(&p, Span::Seconds { start, end }),
                    Err(TranscribeError::BadSpan(_))
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn end_past_file_is_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.wav");
        write_float(&p, 16_000, 1, &mono_frames(&sine(220.0, 16_000, 2.0)));
        let a = load(
            &p,
            Span::Seconds {
                start: 1.5,
                end: Some(99.0),
            },
        );
        assert_eq!(a.samples.len(), 8_000);
        assert_eq!(a.start_seconds, 1.5);
        let open = load(
            &p,
            Span::Seconds {
                start: 1.0,
                end: None,
            },
        );
        assert_eq!(open.samples.len(), 16_000);
        assert_eq!(open.memo_seconds, 2.0);
    }

    #[test]
    fn bad_file_is_wav_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.wav");
        let garbage = dir.path().join("garbage.wav");
        fs::write(&garbage, b"this is not a wav file at all").unwrap();
        let empty = dir.path().join("empty.wav");
        fs::write(&empty, b"").unwrap();
        for p in [&missing, &garbage, &empty] {
            match load_wav_span(p, Span::Whole) {
                Err(TranscribeError::Wav { path, message }) => {
                    assert_eq!(path, p.display().to_string());
                    assert!(!message.is_empty());
                }
                other => panic!("{}: expected Wav, got {other:?}", p.display()),
            }
            assert_eq!(load_wav_span(p, Span::Whole).unwrap_err().exit_code(), 1);
        }
    }

    #[test]
    fn sixteen_k_source_skips_resampler() {
        let tone = sine(300.0, 16_000, 0.3);
        assert_eq!(resample_to_analysis(&tone, 16_000).unwrap(), tone);
        assert!(resample_to_analysis(&[], 16_000).unwrap().is_empty());
    }

    #[test]
    fn resample_48k_keeps_a_tone_at_pitch() {
        for rate in [48_000, 44_100, 22_050] {
            let out = resample_to_analysis(&sine(440.0, rate, 1.0), rate).unwrap();
            let hz = zero_crossing_hz(&out, ANALYSIS_RATE, 0.1, 0.9);
            let cents = 1200.0 * (hz / 440.0).log2();
            assert!(cents.abs() < 1.0, "{rate} Hz: 440 Hz became {hz:.3} Hz");
        }
    }

    #[test]
    fn short_input_is_padded_not_panicking() {
        for len in [1usize, 2, 7, 100, 1023, 1025] {
            let input = vec![0.25f32; len];
            for rate in [8_000, 44_100, 48_000, 192_000] {
                let out = resample_to_analysis(&input, rate).unwrap();
                assert_eq!(out.len(), resampled_len(len, rate), "len {len} at {rate}");
                assert!(out.iter().all(|v| v.is_finite()));
            }
        }
        assert!(resample_to_analysis(&[], 48_000).unwrap().is_empty());
    }

    #[test]
    fn resampled_length_is_exact() {
        for rate in [44_100u32, 48_000, 8_000, 192_000] {
            for len in [rate as usize, rate as usize + 1, 12_345, 147, 294] {
                let out = resample_to_analysis(&vec![0.1; len], rate).unwrap();
                let expected = (len as u64 * 16_000).div_ceil(u64::from(rate)) as usize;
                assert_eq!(out.len(), expected, "{len} samples at {rate} Hz");
            }
        }
    }

    #[test]
    fn alignment_is_preserved() {
        for (rate, at) in [
            (48_000u32, 24_000usize),
            (44_100, 22_050),
            (8_000, 4_000),
            (192_000, 96_000),
        ] {
            let mut input = vec![0.0f32; rate as usize];
            input[at] = 1.0;
            let out = resample_to_analysis(&input, rate).unwrap();
            let peak = out
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .map(|(i, _)| i)
                .unwrap();
            let expected = at as f64 * 16_000.0 / f64::from(rate);
            assert!(
                (peak as f64 - expected).abs() <= 1.0,
                "{rate} Hz: impulse at {expected}, found at {peak}"
            );
        }
    }

    #[test]
    fn odd_rates_are_rejected_or_handled() {
        for rate in [0u32, 1, MIN_SOURCE_RATE - 1, MAX_SOURCE_RATE + 1, u32::MAX] {
            assert!(
                matches!(
                    resample_to_analysis(&[0.0; 10], rate),
                    Err(TranscribeError::BadOption(_))
                ),
                "rate {rate}"
            );
        }
        for rate in [MIN_SOURCE_RATE, 7_919, 11_025, 96_000, MAX_SOURCE_RATE] {
            let out = resample_to_analysis(&[0.5; 50], rate).unwrap();
            assert_eq!(out.len(), resampled_len(50, rate), "rate {rate}");
        }
    }

    #[test]
    fn loaded_synth_round_trips_through_the_resampler() {
        use crate::eval::synth::{Case, SynthSettings, write_wav};
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("synth.wav");
        let case = Case::generate(1000, &SynthSettings::default());
        let samples = case.render(48_000);
        write_wav(&p, &samples, 48_000).unwrap();
        let a = load(&p, Span::Whole);
        assert_eq!(a.samples, samples);
        assert!((a.memo_seconds - case.duration_seconds()).abs() < 1e-3);
        let out = resample_to_analysis(&a.samples, a.sample_rate).unwrap();
        assert_eq!(out.len(), resampled_len(samples.len(), 48_000));
    }
}
