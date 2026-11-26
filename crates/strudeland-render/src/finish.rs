//! Finishing: level, encode, verify and correct every file cut of a render. One loop, two
//! leveling strategies.

use std::borrow::Cow;
use std::time::Instant;

use crate::encode::{
    CORRECTION_MARGIN_DB, Codec, EncodeRequest, MAX_ENCODES, Mp3Flush, loop_points,
};
use crate::level::{Leveled, LoudnessSummary, Measure, Prepared, measure, prepare};
use crate::splice::FileCut;
use crate::{
    ExportFile, ExportRole, Format, Loudness, OutputOptions, Progress, RenderError, RenderOutput,
    RenderedAudio, Summary, ms_since,
};

/// How finishing sets a render's level.
pub(crate) enum Leveling {
    /// One cut: the static gain to the target, then the true-peak limiter. A lossy overshoot
    /// lowers the limiter's ceiling; the gain never changes.
    Limited,
    /// The cuts of a loop preset: one static gain shared by every cut, capped by `peak_db` and
    /// each cut's own peak, with no limiter. A lossy overshoot lowers the shared gain.
    RepeatSafeStatic { program: Measure, peak_db: f64 },
}

/// What a render hands to finishing.
pub(crate) struct Finishing<'a> {
    pub cuts: Vec<FileCut<'a>>,
    /// The render's summary: the base of every file's.
    pub summary: Summary,
    pub leveling: Leveling,
    pub mp3_flush: Mp3Flush,
}

impl<'a> FileCut<'a> {
    /// The whole joined window as one file: role `Single`, the window's bars, no loop.
    pub(crate) fn whole(audio: &'a RenderedAudio) -> FileCut<'a> {
        FileCut {
            role: ExportRole::Single,
            channels: Cow::Borrowed(&audio.channels),
            bars: audio.summary.window.from_bar..audio.summary.window.to_bar,
            loop_range: None,
        }
    }
}

const MP3_LOOP_WARNING: &str =
    "mp3 encoder delay and padding can cause gaps in loop playback; use WAV for sample-exact loops";

/// The strategy's state across passes.
enum Level<'a> {
    Limited {
        prepared: Prepared<'a>,
        /// The limiter ceiling's trim, lowered by each correction.
        trim: f64,
    },
    Static {
        /// Each cut's measure before the gain.
        before: Vec<Measure>,
        /// The shared gain, lowered by each correction.
        gain_db: f64,
        target: Option<f64>,
        ceiling: Option<f64>,
    },
}

impl<'a> Level<'a> {
    /// Checks and measures the cuts once and works out the gain; setup warnings go to `warnings`.
    fn setup(
        cuts: &'a [FileCut<'_>],
        leveling: Leveling,
        out: &OutputOptions,
        sample_rate: u32,
        warnings: &mut Vec<String>,
    ) -> Result<Self, RenderError> {
        let (program, program_peak) = match leveling {
            Leveling::Limited => {
                let [cut] = cuts else {
                    return Err(render_error("limited leveling finishes one file"));
                };
                return Ok(Level::Limited {
                    prepared: prepare(&cut.channels, sample_rate, &out.loudness)?,
                    trim: 0.0,
                });
            }
            Leveling::RepeatSafeStatic { program, peak_db } => (program, peak_db),
        };
        let before = if cuts.len() == 1 {
            vec![program]
        } else {
            cuts.iter()
                .map(|cut| measure(&cut.channels, sample_rate))
                .collect()
        };
        // Playing a split asset by itself pads its edges with silence during peak measurement.
        // Account for those edges as well as the continuous intro handoff and the loop wrap.
        let program_peak = before
            .iter()
            .fold(program_peak, |peak, file| peak.max(file.true_peak_db));
        let (target, ceiling, gain_db) = match out.loudness {
            Loudness::Off => {
                if program_peak > 0.0 {
                    warnings.push(format!("loudness off: true peak is {program_peak:.1} dBTP (above 0 dBFS); the file may clip"));
                }
                (None, None, 0.0)
            }
            Loudness::On {
                target_lufs,
                true_peak_db,
            } => {
                let ideal = if program.lufs.is_finite() {
                    target_lufs - program.lufs
                } else {
                    warnings.push(
                        "loudness: skipped normalization because integrated LUFS is unmeasurable; the true-peak ceiling still applies"
                            .to_owned(),
                    );
                    0.0
                };
                (
                    Some(target_lufs),
                    Some(true_peak_db),
                    ideal.min(true_peak_db - program_peak),
                )
            }
        };
        Ok(Level::Static {
            before,
            gain_db,
            target,
            ceiling,
        })
    }

    /// Levels cut `index`: its channels (borrowed when nothing changed them) and its loudness,
    /// with no decoded peak and no encodes yet. The limiter's warnings go to `pass_warnings`.
    fn level<'s>(
        &'s self,
        index: usize,
        cut: &'s FileCut<'_>,
        sample_rate: u32,
        pass_warnings: &mut Vec<String>,
    ) -> Result<Leveled<'s>, RenderError> {
        match self {
            Level::Limited { prepared, trim } => prepared.level(*trim, pass_warnings),
            Level::Static {
                before,
                gain_db,
                target,
                ceiling,
            } => {
                let before = before
                    .get(index)
                    .copied()
                    .ok_or_else(|| render_error("a file cut has no measure"))?;
                let gain_db = *gain_db;
                let (channels, after) = if gain_db == 0.0 {
                    (Cow::Borrowed(&*cut.channels), before)
                } else {
                    let gain = 10f64.powf(gain_db / 20.0);
                    let gained: Vec<Vec<f32>> = cut
                        .channels
                        .iter()
                        .map(|ch| ch.iter().map(|&x| (f64::from(x) * gain) as f32).collect())
                        .collect();
                    let after = measure(&gained, sample_rate);
                    (Cow::Owned(gained), after)
                };
                Ok(Leveled {
                    channels,
                    summary: LoudnessSummary {
                        enabled: target.is_some(),
                        target_lufs: *target,
                        ceiling_dbtp: *ceiling,
                        limiter_ceiling_dbtp: None,
                        input_lufs: before.lufs,
                        input_true_peak_db: before.true_peak_db,
                        gain_db,
                        limiter_samples: 0,
                        output_lufs: after.lufs,
                        output_true_peak_db: after.true_peak_db,
                        decoded_true_peak_db: None,
                        encodes: 0,
                    },
                })
            }
        }
    }

    /// Lowers the limiter ceiling (Limited) or the shared gain (Static) by the overshoot of
    /// `worst` over `ceiling` plus the margin.
    fn correct(&mut self, worst: f64, ceiling: f64) {
        match self {
            Level::Limited { trim, .. } => *trim -= worst - ceiling + CORRECTION_MARGIN_DB,
            Level::Static { gain_db, .. } => *gain_db -= worst - ceiling + CORRECTION_MARGIN_DB,
        }
    }
}

/// Levels and encodes every cut of `finishing`. A lossy file is decoded again and its true peak
/// measured over the whole decoded stream; with loudness on and the worst file's peak over the
/// ceiling, the strategy corrects (Limited lowers the limiter ceiling, keeping the gain; Static
/// lowers the gain every file shares) and every cut is leveled and encoded again, at most twice;
/// a peak still over the ceiling after that is a warning. With loudness off the decoded peak is
/// reported and never corrected. Every pass is reported once as `Progress::Encoded`: Limited
/// right after its encode, Static after the whole pass's encodes and decodes.
pub(crate) fn finish_files(
    finishing: Finishing<'_>,
    out: &OutputOptions,
    codec: &dyn Codec,
    progress: &dyn Fn(Progress),
) -> Result<Vec<ExportFile>, RenderError> {
    let Finishing {
        cuts,
        summary: base,
        leveling,
        mp3_flush,
    } = finishing;
    let sample_rate = base.sample_rate;
    let format = out.encoder.format();
    let limited = matches!(leveling, Leveling::Limited);

    let mut setup_warnings = Vec::new();
    let started = Instant::now();
    let mut level = Level::setup(&cuts, leveling, out, sample_rate, &mut setup_warnings)?;
    let mut level_ms = ms_since(started);
    if format == Format::Mp3 && cuts.iter().any(|cut| cut.loop_range.is_some()) {
        setup_warnings.push(MP3_LOOP_WARNING.into());
    }

    let mut encode_ms = 0.0;
    // The limiter's and silence warnings of a corrected pass replace the previous pass's.
    let mut pass_warnings = Vec::new();
    for pass in 1..=MAX_ENCODES {
        pass_warnings.clear();
        let mut worst = f64::NEG_INFINITY;
        let mut files = Vec::with_capacity(cuts.len());
        for (index, cut) in cuts.iter().enumerate() {
            let started = Instant::now();
            let Leveled {
                channels,
                summary: mut loudness,
            } = level.level(index, cut, sample_rate, &mut pass_warnings)?;
            level_ms += ms_since(started);

            let started = Instant::now();
            let (range, loop_points) = cut
                .loop_range
                .as_ref()
                .map(|r| loop_points(format, r, sample_rate))
                .unzip();
            let bytes = codec.encode(&EncodeRequest {
                channels: &channels,
                sample_rate,
                encoder: &out.encoder,
                mp3_flush,
                loop_range: range,
            })?;
            if limited {
                progress(Progress::Encoded { pass });
            }
            let frames = channels.first().map_or(0, Vec::len) as u64;
            // The leveled buffer is no longer needed: free it before decoding.
            drop(channels);
            if format != Format::Wav {
                let peak = codec.decoded_true_peak_db(format, &bytes)?;
                worst = worst.max(peak);
                loudness.decoded_true_peak_db = Some(peak);
            }
            encode_ms += ms_since(started);

            let mut summary = base.clone();
            if !limited {
                summary.window.from_bar = cut.bars.start;
                summary.window.to_bar = cut.bars.end;
                summary.frames = frames;
                summary.seconds = frames as f64 / f64::from(sample_rate);
            }
            summary.format = Some(format);
            summary.bytes = Some(bytes.len() as u64);
            loudness.encodes = pass;
            summary.loudness = Some(loudness);
            files.push(ExportFile {
                role: cut.role,
                output: RenderOutput { bytes, summary },
                loop_points,
            });
        }
        if !limited {
            progress(Progress::Encoded { pass });
        }

        let mut exhausted = None;
        if let Loudness::On {
            true_peak_db: ceiling,
            ..
        } = out.loudness
            && worst > ceiling
        {
            if pass < MAX_ENCODES {
                level.correct(worst, ceiling);
                continue;
            }
            exhausted = Some(format!(
                "lossy encode: decoded true peak {worst:.2} dBTP is above the ceiling {ceiling:.1} dBTP after {} corrections",
                MAX_ENCODES - 1
            ));
        }
        for file in &mut files {
            let summary = &mut file.output.summary;
            summary.timings.level_ms = base.timings.level_ms + level_ms;
            summary.timings.encode_ms = encode_ms;
            summary.timings.total_ms += summary.timings.level_ms + encode_ms;
            summary.warnings.extend(setup_warnings.iter().cloned());
            summary.warnings.extend(pass_warnings.iter().cloned());
            summary.warnings.extend(exhausted.iter().cloned());
        }
        return Ok(files);
    }
    Err(render_error("no export encode pass was attempted"))
}

fn render_error(message: &str) -> RenderError {
    RenderError::Render(message.into())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::ops::Range;

    use super::*;
    use crate::level::{SILENCE_WARNING, level, true_peak_db};
    use crate::{BitDepth, EncoderOptions, LoopPoints, Mp3Mode, testing};

    type Recorded = (usize, Option<Range<u64>>, Mp3Flush);

    /// Encodes as raw planar f32 and decodes with a gain of `boost_db[n]` dB on decode `n` (the
    /// last entry for later decodes), so a test decides how far each decode overshoots. Records
    /// every request's frames, loop range and flush.
    struct Stub {
        boost_db: Vec<f64>,
        decodes: Cell<usize>,
        requests: RefCell<Vec<Recorded>>,
        /// This decode (0-based over the whole run) fails.
        fail_decode_at: Option<usize>,
    }

    impl Stub {
        fn new(boost_db: &[f64]) -> Self {
            Stub {
                boost_db: boost_db.to_vec(),
                decodes: Cell::new(0),
                requests: RefCell::new(Vec::new()),
                fail_decode_at: None,
            }
        }

        fn failing_at(boost_db: &[f64], decode: usize) -> Self {
            Stub {
                fail_decode_at: Some(decode),
                ..Stub::new(boost_db)
            }
        }

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

    impl Codec for Stub {
        fn encode(&self, request: &EncodeRequest<'_>) -> Result<Vec<u8>, RenderError> {
            self.requests.borrow_mut().push((
                request.channels.first().map_or(0, Vec::len),
                request.loop_range.clone(),
                request.mp3_flush,
            ));
            Ok(request
                .channels
                .iter()
                .flatten()
                .flat_map(|x| x.to_le_bytes())
                .collect())
        }

        fn decoded_true_peak_db(&self, format: Format, bytes: &[u8]) -> Result<f64, RenderError> {
            if self.fail_decode_at == Some(self.decodes.get()) {
                self.decodes.set(self.decodes.get() + 1);
                return Err(RenderError::Encode("stub".into()));
            }
            Ok(true_peak_db(&self.decode(format, bytes)))
        }
    }

    const SR: u32 = 44_100;

    /// Dense seeded noise (an LCG) of `frames` frames at amplitude 0.5, loud enough that leveling
    /// to -3 LUFS needs the limiter.
    fn noise_frames(seed: u32, frames: usize) -> Vec<Vec<f32>> {
        let mut state = seed;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (f64::from(state) / f64::from(u32::MAX) * 2.0 - 1.0) as f32 * 0.5
        };
        let l: Vec<f32> = (0..frames).map(|_| next()).collect();
        let r: Vec<f32> = (0..frames).map(|_| next()).collect();
        vec![l, r]
    }

    /// Two seconds of [`noise_frames`].
    fn noise() -> Vec<Vec<f32>> {
        noise_frames(0x1234_5678, 88_200)
    }

    fn scaled(input: Vec<Vec<f32>>, by: f32) -> Vec<Vec<f32>> {
        input
            .into_iter()
            .map(|c| c.into_iter().map(|x| x * by).collect())
            .collect()
    }

    const LOUD: Loudness = Loudness::On {
        target_lufs: -3.0,
        true_peak_db: -1.0,
    };

    fn out(loudness: Loudness, encoder: EncoderOptions) -> OutputOptions {
        OutputOptions { loudness, encoder }
    }

    fn ogg_out(loudness: Loudness) -> OutputOptions {
        out(loudness, EncoderOptions::Ogg { quality: 6 })
    }

    fn wav_out(loudness: Loudness) -> OutputOptions {
        out(
            loudness,
            EncoderOptions::Wav {
                bit_depth: BitDepth::F32,
            },
        )
    }

    fn mp3_out(loudness: Loudness) -> OutputOptions {
        out(
            loudness,
            EncoderOptions::Mp3 {
                mode: Mp3Mode::Cbr,
                vbr_quality: 0,
            },
        )
    }

    /// The base summary of `input` with `warnings`.
    fn summary_of(input: &[Vec<f32>], warnings: &[&str]) -> Summary {
        let mut summary = testing::audio(input.to_vec(), SR).summary;
        summary.warnings = warnings.iter().map(|w| w.to_string()).collect();
        summary
    }

    fn cut(
        role: ExportRole,
        channels: &[Vec<f32>],
        bars: Range<u32>,
        loop_range: Option<Range<usize>>,
    ) -> FileCut<'_> {
        FileCut {
            role,
            channels: Cow::Borrowed(channels),
            bars,
            loop_range,
        }
    }

    fn single(channels: &[Vec<f32>]) -> FileCut<'_> {
        cut(ExportRole::Single, channels, 0..1, None)
    }

    fn static_leveling(program: &[Vec<f32>]) -> Leveling {
        let program = measure(program, SR);
        Leveling::RepeatSafeStatic {
            program,
            peak_db: program.true_peak_db,
        }
    }

    fn try_run(
        codec: &dyn Codec,
        cuts: Vec<FileCut<'_>>,
        summary: Summary,
        leveling: Leveling,
        mp3_flush: Mp3Flush,
        out: &OutputOptions,
    ) -> (Result<Vec<ExportFile>, RenderError>, Vec<Progress>) {
        let events = RefCell::new(Vec::new());
        let files = finish_files(
            Finishing {
                cuts,
                summary,
                leveling,
                mp3_flush,
            },
            out,
            codec,
            &|p| events.borrow_mut().push(p),
        );
        (files, events.into_inner())
    }

    fn run(
        codec: &dyn Codec,
        cuts: Vec<FileCut<'_>>,
        summary: Summary,
        leveling: Leveling,
        out: &OutputOptions,
    ) -> (Vec<ExportFile>, Vec<Progress>) {
        let (files, events) = try_run(codec, cuts, summary, leveling, Mp3Flush::Gap, out);
        (files.unwrap(), events)
    }

    /// Limited leveling of `input` as one cut with `NoGap`, after the warning "render: earlier".
    fn limited(
        stub: &Stub,
        input: &[Vec<f32>],
        out: &OutputOptions,
    ) -> (ExportFile, Vec<Progress>) {
        let (files, events) = try_run(
            stub,
            vec![single(input)],
            summary_of(input, &["render: earlier"]),
            Leveling::Limited,
            Mp3Flush::NoGap,
            out,
        );
        let mut files = files.unwrap();
        assert_eq!(files.len(), 1);
        (files.remove(0), events)
    }

    /// An intro-loop's two cuts: a 2 s intro and a 1 s loop body.
    fn intro_loop() -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
        (noise(), noise_frames(0x0bad_cafe, 44_100))
    }

    fn intro_loop_cuts<'a>(intro: &'a [Vec<f32>], body: &'a [Vec<f32>]) -> Vec<FileCut<'a>> {
        vec![
            cut(ExportRole::Intro, intro, 0..4, None),
            cut(ExportRole::Loop, body, 2..4, Some(0..body[0].len())),
        ]
    }

    fn intro_loop_leveling(intro: &[Vec<f32>], body: &[Vec<f32>]) -> Leveling {
        let program: Vec<Vec<f32>> = intro
            .iter()
            .zip(body)
            .map(|(a, b)| a.iter().chain(b).copied().collect())
            .collect();
        static_leveling(&program)
    }

    fn loudness(file: &ExportFile) -> &LoudnessSummary {
        file.output.summary.loudness.as_ref().unwrap()
    }

    fn passes(n: u32) -> Vec<Progress> {
        (1..=n).map(|pass| Progress::Encoded { pass }).collect()
    }

    /// A plain request at 44.1 kHz.
    fn request<'a>(channels: &'a [Vec<f32>], encoder: &'a EncoderOptions) -> EncodeRequest<'a> {
        EncodeRequest {
            channels,
            sample_rate: SR,
            encoder,
            mp3_flush: Mp3Flush::NoGap,
            loop_range: None,
        }
    }

    #[test]
    fn limited_one_pass_when_the_decoded_peak_is_under_the_ceiling() {
        let stub = Stub::new(&[0.0]);
        let (e, events) = limited(&stub, &noise(), &ogg_out(LOUD));
        let warnings = &e.output.summary.warnings;
        let e = loudness(&e);
        assert_eq!(e.encodes, 1);
        assert_eq!(events, passes(1));
        assert_eq!(warnings, &vec!["render: earlier".to_string()]);
        assert!(e.limiter_samples > 0, "{e:?}");
        let dp = e.decoded_true_peak_db.unwrap();
        assert_eq!(dp, e.output_true_peak_db);
        assert!(dp <= -1.0, "{dp}");
    }

    #[test]
    fn limited_corrects_one_overshoot() {
        let input = noise();
        let first = limited(&Stub::new(&[0.0]), &input, &ogg_out(LOUD)).0;
        let first = loudness(&first);
        // What the first pass decodes to: its leveled output plus 0.5 dB.
        let leveled = level(&input, SR, &LOUD, 0.0, &mut Vec::new()).unwrap();
        let probe = Stub::new(&[0.5]);
        let enc = EncoderOptions::Ogg { quality: 6 };
        let dp1 = true_peak_db(&probe.decode(
            Format::Ogg,
            &probe.encode(&request(&leveled.channels, &enc)).unwrap(),
        ));
        let stub = Stub::new(&[0.5, 0.0]);
        let (e, events) = limited(&stub, &input, &ogg_out(LOUD));
        let warnings = &e.output.summary.warnings;
        let e = loudness(&e);
        assert_eq!(e.encodes, 2);
        assert_eq!(events, passes(2));
        assert_eq!(warnings, &vec!["render: earlier".to_string()]);
        // The limiter ceiling fell by the first pass's overshoot plus 0.1 dB; the gain stayed.
        assert!(dp1 > -1.0, "{dp1}");
        assert_eq!(e.gain_db, first.gain_db);
        assert_eq!(e.ceiling_dbtp, Some(-1.0));
        assert_eq!(first.limiter_ceiling_dbtp, Some(-1.0));
        assert_eq!(
            e.limiter_ceiling_dbtp,
            Some(-1.0 + (0.0 - (dp1 - -1.0 + CORRECTION_MARGIN_DB)))
        );
        assert!(e.output_true_peak_db <= e.limiter_ceiling_dbtp.unwrap());
        assert!(e.decoded_true_peak_db.unwrap() <= -1.0);
    }

    #[test]
    fn limited_converges_on_a_limited_signal_with_a_steady_overshoot() {
        // A codec that always adds 1 dB to a signal the limiter holds at the ceiling: lowering
        // the limiter ceiling (not the gain) brings it under in one correction.
        let first = limited(&Stub::new(&[0.0]), &noise(), &ogg_out(LOUD)).0;
        let first = loudness(&first);
        let stub = Stub::new(&[1.0]);
        let (e, events) = limited(&stub, &noise(), &ogg_out(LOUD));
        let warnings = &e.output.summary.warnings;
        let e = loudness(&e);
        assert_eq!(e.encodes, 2);
        assert_eq!(events, passes(2));
        assert_eq!(warnings, &vec!["render: earlier".to_string()]);
        assert!(e.decoded_true_peak_db.unwrap() <= -1.0);
        assert_eq!(e.gain_db, first.gain_db);
        assert!(e.limiter_ceiling_dbtp.unwrap() < -2.0, "{e:?}");
    }

    #[test]
    fn limited_stops_after_two_corrections_and_warns() {
        // An overshoot that grows faster than each correction.
        let first = limited(&Stub::new(&[0.0]), &noise(), &ogg_out(LOUD)).0;
        let first = loudness(&first);
        let stub = Stub::new(&[1.0, 2.5, 4.0]);
        let (e, events) = limited(&stub, &noise(), &ogg_out(LOUD));
        let warnings = &e.output.summary.warnings;
        let e = loudness(&e);
        assert_eq!(e.encodes, MAX_ENCODES);
        assert_eq!(events, passes(3));
        assert_eq!(stub.decodes.get(), 3);
        assert_eq!(e.gain_db, first.gain_db);
        assert!(e.limiter_ceiling_dbtp.unwrap() < -3.0, "{e:?}");
        let dp = e.decoded_true_peak_db.unwrap();
        assert!(dp > -1.0, "{dp}");
        assert_eq!(
            warnings,
            &vec![
                "render: earlier".to_string(),
                format!(
                    "lossy encode: decoded true peak {dp:.2} dBTP is above the ceiling -1.0 dBTP after 2 corrections"
                )
            ]
        );
    }

    #[test]
    fn limited_never_corrects_with_loudness_off() {
        let input = scaled(noise(), 2.5);
        let stub = Stub::new(&[3.0]);
        let (file, events) = limited(&stub, &input, &ogg_out(Loudness::Off));
        let warnings = &file.output.summary.warnings;
        let e = loudness(&file);
        assert_eq!(e.encodes, 1);
        assert_eq!(events, passes(1));
        assert!(!e.enabled);
        // The bytes are the input, untouched.
        assert_eq!(
            file.output.bytes,
            stub.encode(&request(&input, &EncoderOptions::Ogg { quality: 6 }))
                .unwrap()
        );
        let tp = e.input_true_peak_db;
        assert!(tp > 0.0, "{tp}");
        let dp = e.decoded_true_peak_db.unwrap();
        assert!((dp - (tp + 3.0)).abs() < 1e-3, "{dp} {tp}");
        // Only the clipping warning of loudness off; nothing about the decoded peak.
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(
            warnings[1].starts_with("loudness off: true peak is "),
            "{warnings:?}"
        );
    }

    #[test]
    fn limited_does_not_decode_wav() {
        let stub = Stub::new(&[10.0]);
        let (e, events) = limited(&stub, &noise(), &wav_out(LOUD));
        let e = loudness(&e);
        assert_eq!(stub.decodes.get(), 0);
        assert_eq!(e.encodes, 1);
        assert_eq!(e.decoded_true_peak_db, None);
        assert_eq!(events, passes(1));
    }

    #[test]
    fn limited_silence_gets_no_gain_and_no_correction() {
        let stub = Stub::new(&[1.0]);
        let (e, events) = limited(
            &stub,
            &[vec![0.0; 44_100], vec![0.0; 44_100]],
            &ogg_out(LOUD),
        );
        let warnings = &e.output.summary.warnings;
        let e = loudness(&e);
        assert_eq!(e.encodes, 1);
        assert_eq!(events, passes(1));
        assert_eq!(e.gain_db, 0.0);
        assert_eq!(e.decoded_true_peak_db, Some(f64::NEG_INFINITY));
        assert_eq!(
            warnings,
            &vec!["render: earlier".to_string(), SILENCE_WARNING.to_string()]
        );
    }

    #[test]
    fn limited_errors_pass_through() {
        let input = [vec![0.0; 10], vec![0.0; 11]];
        let (e, events) = try_run(
            &Stub::new(&[0.0]),
            vec![single(&input)],
            summary_of(&input, &[]),
            Leveling::Limited,
            Mp3Flush::NoGap,
            &ogg_out(LOUD),
        );
        let e = e.unwrap_err();
        assert!(matches!(e, RenderError::Render(_)), "{e:?}");
        assert!(events.is_empty());
    }

    #[test]
    fn static_shared_gain_moves_both_files() {
        let (intro, body) = intro_loop();
        let run_with = |stub: &Stub| {
            run(
                stub,
                intro_loop_cuts(&intro, &body),
                summary_of(&intro, &[]),
                intro_loop_leveling(&intro, &body),
                &ogg_out(LOUD),
            )
        };
        let (first, first_events) = run_with(&Stub::new(&[0.0]));
        assert_eq!(first_events, passes(1));
        let first_gain = loudness(&first[0]).gain_db;
        assert_eq!(loudness(&first[1]).gain_db, first_gain);

        // What pass 1 decodes to: cut 1 plus 0.5 dB, cut 2 as it is.
        let probe = Stub::new(&[0.5, 0.0]);
        let enc = EncoderOptions::Ogg { quality: 6 };
        let g = 10f64.powf(first_gain / 20.0);
        let dp1 = [&intro, &body]
            .into_iter()
            .map(|cut| {
                let gained: Vec<Vec<f32>> = cut
                    .iter()
                    .map(|ch| ch.iter().map(|&x| (f64::from(x) * g) as f32).collect())
                    .collect();
                probe
                    .decoded_true_peak_db(
                        Format::Ogg,
                        &probe.encode(&request(&gained, &enc)).unwrap(),
                    )
                    .unwrap()
            })
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(dp1 > -1.0, "{dp1}");

        let stub = Stub::new(&[0.5, 0.0, 0.0, 0.0]);
        let (files, events) = run_with(&stub);
        assert_eq!(events, passes(2));
        assert_eq!(stub.decodes.get(), 4);
        let expected = first_gain - (dp1 - -1.0 + CORRECTION_MARGIN_DB);
        for file in &files {
            let l = loudness(file);
            assert_eq!(l.encodes, 2);
            assert_eq!(l.gain_db, expected);
            assert_eq!(l.limiter_ceiling_dbtp, None);
            assert!(l.decoded_true_peak_db.unwrap() <= -1.0, "{l:?}");
        }
    }

    #[test]
    fn static_stops_after_two_corrections_with_the_worst_peak() {
        let input = noise();
        let stub = Stub::new(&[1.0, 2.5, 4.0]);
        let (files, events) = run(
            &stub,
            vec![cut(ExportRole::Loop, &input, 0..2, Some(0..44_100))],
            summary_of(&input, &[]),
            static_leveling(&input),
            &ogg_out(LOUD),
        );
        assert_eq!(events, passes(3));
        assert_eq!(stub.decodes.get(), 3);
        let l = loudness(&files[0]);
        assert_eq!(l.encodes, 3);
        let worst = l.decoded_true_peak_db.unwrap();
        assert!(worst > -1.0, "{worst}");
        assert_eq!(
            files[0].output.summary.warnings.last(),
            Some(&format!(
                "lossy encode: decoded true peak {worst:.2} dBTP is above the ceiling -1.0 dBTP after 2 corrections"
            ))
        );
    }

    #[test]
    fn static_loudness_off_is_a_pcm_bypass() {
        let input = scaled(noise(), 2.5);
        let stub = Stub::new(&[3.0]);
        let (files, events) = run(
            &stub,
            vec![cut(ExportRole::Loop, &input, 0..2, Some(0..44_100))],
            summary_of(&input, &[]),
            static_leveling(&input),
            &ogg_out(Loudness::Off),
        );
        assert_eq!(events, passes(1));
        let file = &files[0];
        assert_eq!(
            file.output.bytes,
            Stub::new(&[0.0])
                .encode(&request(&input, &EncoderOptions::Ogg { quality: 6 }))
                .unwrap()
        );
        let l = loudness(file);
        assert_eq!(l.encodes, 1);
        assert_eq!(l.gain_db, 0.0);
        assert!(!l.enabled);
        let peak = l.input_true_peak_db;
        assert!(peak > 0.0, "{peak}");
        assert_eq!(
            file.output.summary.warnings,
            vec![format!(
                "loudness off: true peak is {peak:.1} dBTP (above 0 dBFS); the file may clip"
            )]
        );
    }

    #[test]
    fn neither_strategy_decodes_wav() {
        let input = noise();
        let stub = Stub::new(&[10.0]);
        let (file, _) = limited(&stub, &input, &wav_out(LOUD));
        assert_eq!(loudness(&file).decoded_true_peak_db, None);
        let (files, events) = run(
            &stub,
            vec![cut(ExportRole::Loop, &input, 0..2, Some(0..44_100))],
            summary_of(&input, &[]),
            static_leveling(&input),
            &wav_out(LOUD),
        );
        assert_eq!(events, passes(1));
        assert_eq!(loudness(&files[0]).decoded_true_peak_db, None);
        assert_eq!(stub.decodes.get(), 0);
    }

    #[test]
    fn the_wrap_peak_caps_the_shared_gain() {
        let input = noise();
        let own = measure(&input, SR);
        let peak_db = own.true_peak_db + 3.0;
        // The ideal gain to -3 LUFS is larger than the ceiling minus the wrap peak.
        assert!(-3.0 - own.lufs > -1.0 - peak_db, "{own:?}");
        let (files, _) = run(
            &Stub::new(&[0.0]),
            vec![cut(ExportRole::Loop, &input, 0..2, Some(0..44_100))],
            summary_of(&input, &[]),
            Leveling::RepeatSafeStatic {
                program: own,
                peak_db,
            },
            &ogg_out(LOUD),
        );
        let l = loudness(&files[0]);
        assert_eq!(l.encodes, 1);
        assert_eq!(l.gain_db, -1.0 - peak_db);
    }

    #[test]
    fn an_unmeasurable_loop_skips_normalization_but_keeps_the_ceiling() {
        let input = scaled(noise_frames(0x5eed_0002, 8_820), 2.5);
        let (files, _) = run(
            &Stub::new(&[0.0]),
            vec![cut(ExportRole::Loop, &input, 0..1, Some(0..4_410))],
            summary_of(&input, &[]),
            static_leveling(&input),
            &wav_out(LOUD),
        );
        let file = &files[0];
        assert_eq!(
            file.output.summary.warnings,
            vec![
                "loudness: skipped normalization because integrated LUFS is unmeasurable; the true-peak ceiling still applies"
                    .to_string()
            ]
        );
        assert!(
            !file
                .output
                .summary
                .warnings
                .iter()
                .any(|w| w == SILENCE_WARNING)
        );
        let l = loudness(file);
        assert!(l.gain_db < 0.0, "{l:?}");
        assert!(l.output_true_peak_db <= -1.0 + 1e-5, "{l:?}");
    }

    #[test]
    fn mp3_warns_about_loop_gaps_only_for_looped_cuts() {
        let gap_warning = MP3_LOOP_WARNING.to_string();
        let hot = scaled(noise(), 2.5);
        let (files, _) = run(
            &Stub::new(&[0.0]),
            vec![cut(ExportRole::Loop, &hot, 0..2, Some(0..44_100))],
            summary_of(&hot, &["render: earlier"]),
            static_leveling(&hot),
            &mp3_out(Loudness::Off),
        );
        let warnings = &files[0].output.summary.warnings;
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert_eq!(warnings[0], "render: earlier");
        assert!(warnings[1].starts_with("loudness off: "), "{warnings:?}");
        assert_eq!(warnings[2], gap_warning);

        let input = noise();
        let (files, _) = try_run(
            &Stub::new(&[0.0]),
            vec![single(&input)],
            summary_of(&input, &[]),
            Leveling::Limited,
            Mp3Flush::Gap,
            &mp3_out(LOUD),
        );
        let files = files.unwrap();
        assert!(
            !files[0].output.summary.warnings.contains(&gap_warning),
            "{:?}",
            files[0].output.summary.warnings
        );
    }

    #[test]
    fn each_caller_passes_its_flush_and_range() {
        let input = noise();
        let n = input[0].len();
        for flush in [Mp3Flush::NoGap, Mp3Flush::Gap] {
            let audio = testing::audio(input.clone(), SR);
            let stub = Stub::new(&[0.0]);
            let (files, _) = try_run(
                &stub,
                vec![FileCut::whole(&audio)],
                audio.summary.clone(),
                Leveling::Limited,
                flush,
                &ogg_out(LOUD),
            );
            assert_eq!(files.unwrap()[0].loop_points, None);
            assert_eq!(stub.requests.into_inner(), vec![(n, None, flush)]);
        }

        let (intro, body) = intro_loop();
        let body_frames = body[0].len();
        let stub = Stub::new(&[0.0]);
        let (files, _) = run(
            &stub,
            intro_loop_cuts(&intro, &body),
            summary_of(&intro, &[]),
            intro_loop_leveling(&intro, &body),
            &ogg_out(LOUD),
        );
        assert_eq!(
            stub.requests.into_inner(),
            vec![
                (intro[0].len(), None, Mp3Flush::Gap),
                (body_frames, Some(0..body_frames as u64), Mp3Flush::Gap),
            ]
        );
        assert_eq!(files[0].loop_points, None);
        assert_eq!(
            files[1].loop_points,
            Some(LoopPoints {
                start_frame: 0,
                length_frames: body_frames as u64,
                sample_rate: SR,
            })
        );

        let stub = Stub::new(&[0.0]);
        let (files, _) = run(
            &stub,
            vec![cut(ExportRole::Loop, &input, 0..2, Some(147..294))],
            summary_of(&input, &[]),
            static_leveling(&input),
            &out(Loudness::Off, EncoderOptions::Opus { kbps: 160 }),
        );
        assert_eq!(
            stub.requests.into_inner(),
            vec![(n, Some(160..320), Mp3Flush::Gap)]
        );
        assert_eq!(
            files[0].loop_points,
            Some(LoopPoints {
                start_frame: 160,
                length_frames: 160,
                sample_rate: 48_000,
            })
        );
    }

    #[test]
    fn limited_finishes_exactly_one_cut() {
        let (intro, body) = intro_loop();
        let (files, events) = try_run(
            &Stub::new(&[0.0]),
            intro_loop_cuts(&intro, &body),
            summary_of(&intro, &[]),
            Leveling::Limited,
            Mp3Flush::Gap,
            &ogg_out(LOUD),
        );
        match files {
            Err(RenderError::Render(m)) => assert_eq!(m, "limited leveling finishes one file"),
            other => panic!("{other:?}"),
        }
        assert!(events.is_empty());
    }

    #[test]
    fn progress_precedes_the_last_decode() {
        let input = noise();
        let (files, events) = try_run(
            &Stub::failing_at(&[0.0], 0),
            vec![single(&input)],
            summary_of(&input, &[]),
            Leveling::Limited,
            Mp3Flush::NoGap,
            &ogg_out(LOUD),
        );
        assert!(matches!(files, Err(RenderError::Encode(_))), "{files:?}");
        assert_eq!(events, passes(1));
    }

    #[test]
    fn static_progress_follows_the_whole_pass() {
        let input = noise();
        let one_cut = |stub: &Stub| {
            try_run(
                stub,
                vec![cut(ExportRole::Loop, &input, 0..2, Some(0..44_100))],
                summary_of(&input, &[]),
                static_leveling(&input),
                Mp3Flush::Gap,
                &ogg_out(LOUD),
            )
        };

        // The only cut of pass 1.
        let (files, events) = one_cut(&Stub::failing_at(&[0.0], 0));
        assert!(matches!(files, Err(RenderError::Encode(_))), "{files:?}");
        assert_eq!(events, vec![]);

        // Cut 2 of pass 1.
        let (intro, body) = intro_loop();
        let (files, events) = try_run(
            &Stub::failing_at(&[0.0], 1),
            intro_loop_cuts(&intro, &body),
            summary_of(&intro, &[]),
            intro_loop_leveling(&intro, &body),
            Mp3Flush::Gap,
            &ogg_out(LOUD),
        );
        assert!(matches!(files, Err(RenderError::Encode(_))), "{files:?}");
        assert_eq!(events, vec![]);

        // Pass 1 overshoots; pass 2 fails.
        let (files, events) = one_cut(&Stub::failing_at(&[1.0, 0.0], 1));
        assert!(matches!(files, Err(RenderError::Encode(_))), "{files:?}");
        assert_eq!(events, passes(1));
    }
}
