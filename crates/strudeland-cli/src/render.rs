//! `strudeland render`: option resolution, the export adapter, progress and the summary output.
//! It runs offline and never opens the control socket.

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use strudeland_render::export::{
    self, Destination, Event, Finished, Kind, Prepared, Request, RunError,
};
use strudeland_render::{
    BitDepth, DEFAULT_TARGET_LUFS, DEFAULT_TRUE_PEAK_DB, EncoderOptions, ExportPreset, ExportRole,
    Format, LengthRequest, LoopPoints, Loudness, OutputOptions, Progress, RenderError, StemExport,
    StemGrouping, Summary, resolve_format,
};

use crate::cli::{OnOff, RenderArgs};

fn usage(flag: &'static str, message: impl Into<String>) -> RenderError {
    RenderError::Usage {
        flag,
        message: message.into(),
    }
}

/// Resolves the flags and the destination before any work, so a bad flag or a missing `--out`
/// fails fast. Creates nothing and never stats an explicit `--out`.
pub fn prepare(args: &RenderArgs) -> Result<Prepared, RenderError> {
    let length = if let Some(name) = &args.section {
        LengthRequest::Section { name: name.clone() }
    } else if let Some(bars) = args.length {
        LengthRequest::Length { bars }
    } else {
        LengthRequest::Clip { loops: args.loops }
    };
    let kind = if let Some(grouping) = args.stems {
        stem_options(args)?;
        Kind::Stems { grouping }
    } else {
        let preset = args.preset.map(ExportPreset::from);
        export::check_length_for_preset(&length, preset).map_err(|_| {
            usage(
                "--preset",
                "loop presets select their own bars; omit --section, --length and --loops",
            )
        })?;
        Kind::Mix {
            preset,
            output: mix_options(args)?,
        }
    };
    let destination = Destination::from_out(args.out.as_deref(), &args.song).map_err(cli_error)?;
    export::prepare(
        Request {
            song: args.song.clone(),
            length,
            kind,
            jobs: args.jobs,
            plan_timeout: Some(args.plan_timeout),
        },
        destination,
    )
    .map_err(cli_error)
}

/// The CLI's wording of an export error: the pipeline's, except a role mismatch keeps the text
/// the CLI printed before the module.
pub fn cli_error(e: export::Error) -> RenderError {
    match e {
        export::Error::RolesMismatch => RenderError::Render("unexpected intro file".into()),
        other => other.into(),
    }
}

/// Prints the partial-export line for a mix that wrote at least one file, and returns the error.
pub fn failure(lines: &ProgressLines<'_>, e: RunError) -> RenderError {
    match e {
        RunError::Failed(e) => cli_error(e),
        RunError::PartialWrite(p) => {
            if let (Some(failed), Some(last)) = (&p.failed, p.written.last()) {
                lines.partial_write(&last.absolute, failed);
            }
            cli_error(p.source)
        }
    }
}

/// The flags `--stems` rejects, checked in order. The stem output itself is fixed by the export.
fn stem_options(args: &RenderArgs) -> Result<(), RenderError> {
    let conflicts = [
        (
            args.preset.is_some(),
            "--preset",
            "omit --preset with --stems",
        ),
        (
            args.format.is_some_and(|f| f != Format::Wav),
            "--format",
            "--stems supports only WAV",
        ),
        (
            args.bit_depth.is_some_and(|b| b != BitDepth::F32),
            "--bit-depth",
            "--stems uses 32-bit float WAV (32f)",
        ),
        (
            args.loudness == Some(OnOff::On),
            "--loudness",
            "--stems exports pre-master audio with loudness off",
        ),
        (args.lufs.is_some(), "--lufs", "omit --lufs with --stems"),
        (
            args.true_peak.is_some(),
            "--true-peak",
            "omit --true-peak with --stems",
        ),
    ];
    if let Some((_, flag, message)) = conflicts.into_iter().find(|(conflict, ..)| *conflict) {
        return Err(usage(flag, message));
    }
    EncoderOptions::from_flags(
        Format::Wav,
        Some(BitDepth::F32),
        args.mp3_mode,
        args.quality,
    )?;
    Ok(())
}

fn mix_options(args: &RenderArgs) -> Result<OutputOptions, RenderError> {
    let format = resolve_format(args.format, args.out.as_deref())?;
    let encoder = EncoderOptions::from_flags(format, args.bit_depth, args.mp3_mode, args.quality)?;
    let loudness = if args.loudness == Some(OnOff::Off) {
        if args.lufs.is_some() {
            return Err(usage("--lufs", "--lufs does not apply with --loudness off"));
        }
        if args.true_peak.is_some() {
            return Err(usage(
                "--true-peak",
                "--true-peak does not apply with --loudness off",
            ));
        }
        Loudness::Off
    } else {
        Loudness::On {
            target_lufs: args.lufs.unwrap_or(DEFAULT_TARGET_LUFS),
            true_peak_db: args.true_peak.unwrap_or(DEFAULT_TRUE_PEAK_DB),
        }
    };
    Ok(OutputOptions { loudness, encoder })
}

/// A progress callback that writes one stderr line per step from the calling thread.
pub struct ProgressLines<'a> {
    stderr: RefCell<&'a mut dyn Write>,
    done: Cell<usize>,
}

impl<'a> ProgressLines<'a> {
    pub fn new(stderr: &'a mut dyn Write) -> Self {
        ProgressLines {
            stderr: RefCell::new(stderr),
            done: Cell::new(0),
        }
    }

    pub fn report(&self, p: Progress) {
        let line = match p {
            Progress::Planned { chunks } => format!("planned {chunks} chunk(s)"),
            Progress::ChunkRendered { of, .. } => {
                let done = self.done.get() + 1;
                self.done.set(done);
                format!("rendered {done}/{of}")
            }
            Progress::Encoded { pass: 1 } => "encoded".to_string(),
            Progress::Encoded { pass } => {
                format!("re-encoded (pass {pass}) to bring the decoded peak under the ceiling")
            }
            Progress::Written { path } => format!("wrote {}", path.display()),
        };
        let _ = writeln!(self.stderr.borrow_mut(), "{line}");
    }

    /// Prints the warnings after the run, for human and `--json` output alike.
    pub fn warnings<'w>(&self, warnings: impl IntoIterator<Item = &'w String>) {
        let mut err = self.stderr.borrow_mut();
        for w in warnings {
            let _ = writeln!(err, "warning: {w}");
        }
    }

    /// Reports one export event: pipeline steps and each written file with its warnings, the
    /// new ones only for a preset.
    pub fn event(&self, event: Event<'_>, preset: bool) {
        match event {
            Event::Pipeline(p) => self.report(p),
            Event::Publishing => {}
            Event::Written { file, new_warnings } => {
                self.report(Progress::Written {
                    path: file.absolute.clone(),
                });
                if preset {
                    self.warnings(new_warnings);
                } else {
                    self.warnings(&file.summary.warnings);
                }
            }
        }
    }

    pub(crate) fn partial_write(&self, completed: &Path, failed: &Path) {
        let _ = writeln!(
            self.stderr.borrow_mut(),
            "partial export: wrote {}, failed to write {}",
            completed.display(),
            failed.display(),
        );
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoopReport<'a> {
    #[serde(flatten)]
    points: &'a LoopPoints,
    godot_loop_offset: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WrittenExport<'a> {
    role: ExportRole,
    #[serde(serialize_with = "serialize_path")]
    path: PathBuf,
    #[serde(flatten)]
    summary: &'a Summary,
    loop_points: Option<LoopReport<'a>>,
}

fn serialize_path<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}

/// The stdout summary of a finished export.
pub fn summary(finished: &Finished, json: bool) -> String {
    match finished {
        Finished::Mix {
            preset: None,
            files,
            ..
        } => files
            .iter()
            .map(|f| {
                if json {
                    format!("{}\n", json_line(&f.summary, &f.absolute))
                } else {
                    human_summary(&f.summary, &f.absolute)
                }
            })
            .collect(),
        Finished::Mix {
            preset: Some(preset),
            files,
            ..
        } => {
            let written: Vec<WrittenExport<'_>> = files
                .iter()
                .map(|f| WrittenExport {
                    role: f.role,
                    path: f.absolute.clone(),
                    summary: &f.summary,
                    loop_points: f.loop_points.as_ref().map(|points| LoopReport {
                        points,
                        godot_loop_offset: points.godot_loop_offset(),
                    }),
                })
                .collect();
            if json {
                return format!(
                    "{}\n",
                    serde_json::json!({"preset": preset, "files": written})
                );
            }
            let mut output = String::new();
            for file in written {
                output.push_str(&human_summary(file.summary, &file.path));
                if let Some(loop_report) = file.loop_points {
                    let points = loop_report.points;
                    output.push_str(&format!(
                        "  loop starts at frame {}, spans {} frames at {} Hz (Godot loop_offset {} s)\n",
                        points.start_frame, points.length_frames, points.sample_rate, loop_report.godot_loop_offset,
                    ));
                }
            }
            output
        }
        Finished::Stems { grouping, export } => stems_summary(export, *grouping, json),
    }
}

/// The `--json` line: the summary's camelCase object with `path` added.
pub fn json_line(summary: &Summary, path: &Path) -> String {
    let mut value = serde_json::to_value(summary).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        map.insert(
            "path".to_string(),
            Value::String(path.to_string_lossy().into_owned()),
        );
    }
    value.to_string()
}

pub fn stems_summary(export: &StemExport, grouping: StemGrouping, json: bool) -> String {
    if json {
        let files: Vec<_> = export
            .files
            .iter()
            .map(|path| path.to_string_lossy())
            .collect();
        return format!(
            "{}\n",
            serde_json::json!({
                "grouping": grouping,
                "directory": export.directory.to_string_lossy(),
                "files": files,
                "reference": "pre-master",
                "summary": export.summary,
            })
        );
    }
    let summary = &export.summary;
    let mut output = format!(
        "rendered stem set with pre-master reference mix to {}\n  stereo 32-bit float WAV, {} Hz, {} frames, bars {}-{} ({} layout)\n",
        export.directory.display(),
        summary.sample_rate,
        summary.frames,
        summary.window.from_bar.saturating_add(1),
        summary.window.to_bar,
        summary.window.layout.as_str(),
    );
    for path in &export.files {
        output.push_str(&format!("  {}\n", path.display()));
    }
    output
}

/// One decimal; `-inf` prints as `-inf` (Rust's own text for the infinities).
fn f1(v: f64) -> String {
    if v.is_nan() {
        "nan".to_string()
    } else {
        format!("{v:.1}")
    }
}

/// The human summary, printed on stdout after a successful write.
pub fn human_summary(summary: &Summary, path: &Path) -> String {
    let mut out = format!("rendered {}\n", path.display());
    let format = summary.format.map_or("?".to_string(), |f| f.to_string());
    let mib = summary.bytes.unwrap_or(0) as f64 / (1024.0 * 1024.0);
    let layout = summary.window.layout.as_str();
    out.push_str(&format!(
        "  {format} {} MiB, {} s, bars {}-{} ({layout} layout, {} chunk(s))\n",
        f1(mib),
        f1(summary.seconds),
        summary.window.from_bar.saturating_add(1),
        summary.window.to_bar,
        summary.chunks,
    ));
    if let Some(l) = &summary.loudness {
        if l.enabled {
            out.push_str(&format!(
                "  loudness {} -> {} LUFS (gain {:+.1} dB), true peak {} dBTP, limiter touched {} samples\n",
                f1(l.input_lufs),
                f1(l.output_lufs),
                l.gain_db,
                f1(l.output_true_peak_db),
                l.limiter_samples,
            ));
        } else {
            out.push_str(&format!(
                "  loudness off: {} LUFS, true peak {} dBTP\n",
                f1(l.input_lufs),
                f1(l.input_true_peak_db),
            ));
        }
        if let Some(dp) = l.decoded_true_peak_db {
            out.push_str(&format!(
                "  decoded true peak {} dBTP after {} encode(s)\n",
                f1(dp),
                l.encodes
            ));
        }
    }
    let t = &summary.timings;
    out.push_str(&format!(
        "  time: plan {} s, render {} s, encode {} s\n",
        f1(t.plan_ms / 1000.0),
        f1(t.render_ms / 1000.0),
        f1((t.level_ms + t.encode_ms) / 1000.0),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use crate::run::{Env, Transport, run_with};
    use clap::Parser;
    use std::time::Duration;
    use strudeland_ctl::{CallError, Method};
    use strudeland_render::HostError;
    use strudeland_render::Mp3Mode;
    use strudeland_render::level::LoudnessSummary;
    use strudeland_render::testing::FakePipeline;
    use strudeland_render::{ExportFile, RenderOutput, Timings, Unsupported};

    struct NoTransport;
    impl Transport for NoTransport {
        fn call(&mut self, method: Method, _: Value) -> Result<Value, CallError> {
            panic!("render must not open the transport: {method:?}");
        }
    }

    pub(super) fn summary(format: Format, lossy: bool) -> Summary {
        let window = serde_json::from_value(
            serde_json::json!({"fromBar": 0, "toBar": 4, "layout": "linear"}),
        )
        .unwrap();
        Summary {
            format: Some(format),
            bytes: Some(1_572_864),
            window,
            frames: 352_800,
            seconds: 8.0,
            sample_rate: 44_100,
            chunks: 2,
            jobs: 2,
            loudness: Some(LoudnessSummary {
                enabled: true,
                target_lufs: Some(-14.0),
                ceiling_dbtp: Some(-1.0),
                limiter_ceiling_dbtp: Some(-1.0),
                input_lufs: -20.04,
                input_true_peak_db: -3.0,
                gain_db: 6.04,
                limiter_samples: 12,
                output_lufs: -14.0,
                output_true_peak_db: -1.0,
                decoded_true_peak_db: lossy.then_some(-1.2),
                encodes: if lossy { 2 } else { 1 },
            }),
            timings: Timings {
                plan_ms: 1500.0,
                render_ms: 2500.0,
                level_ms: 100.0,
                encode_ms: 400.0,
                total_ms: 4500.0,
            },
            warnings: vec!["w one".into()],
        }
    }

    fn ok() -> FakePipeline {
        with(Ok(RenderOutput {
            bytes: vec![1, 2, 3],
            summary: summary(Format::Wav, false),
        }))
    }

    fn with(r: Result<RenderOutput, RenderError>) -> FakePipeline {
        FakePipeline {
            render: Some(r),
            ..FakePipeline::default()
        }
    }

    fn with_export(files: Vec<ExportFile>) -> FakePipeline {
        FakePipeline {
            render: None,
            export: Some(Ok(files)),
            ..ok()
        }
    }

    struct Out {
        code: i32,
        stdout: String,
        stderr: String,
    }

    fn run(args: &[&str], fake: &mut FakePipeline) -> Out {
        let cli =
            Cli::try_parse_from(std::iter::once("strudeland").chain(args.iter().copied())).unwrap();
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let code = {
            let mut env = Env {
                stdin: &mut std::io::empty(),
                stdin_is_tty: false,
                stdout: &mut stdout,
                stdout_is_tty: false,
                stderr: &mut stderr,
            };
            run_with(&cli, &mut NoTransport, fake, &mut env)
        };
        Out {
            code,
            stdout: String::from_utf8(stdout).unwrap(),
            stderr: String::from_utf8(stderr).unwrap(),
        }
    }

    fn song(dir: &Path) -> String {
        let p = dir.join("song.js");
        std::fs::write(&p, "note('c3')").unwrap();
        p.to_string_lossy().into_owned()
    }

    fn export_file(role: ExportRole, loop_points: Option<LoopPoints>) -> ExportFile {
        ExportFile {
            role,
            output: RenderOutput {
                bytes: vec![1, 2, 3],
                summary: summary(Format::Wav, false),
            },
            loop_points,
        }
    }

    #[test]
    fn tagged_export_reports_decoded_loop_points() {
        let dir = tempfile::tempdir().unwrap();
        let (song, out) = (song(dir.path()), dir.path().join("tag.wav"));
        let files = || {
            vec![export_file(
                ExportRole::Single,
                Some(LoopPoints {
                    start_frame: 24_000,
                    length_frames: 96_000,
                    sample_rate: 48_000,
                }),
            )]
        };
        let args = [
            "render",
            &song,
            "--out",
            out.to_str().unwrap(),
            "--preset",
            "tagged-loop",
        ];
        let mut fake = with_export(files());
        let o = run(&args, &mut fake);
        assert_eq!(o.code, 0, "{}", o.stderr);
        assert_eq!(fake.preset, Some(ExportPreset::TaggedLoop));
        assert!(o.stdout.contains(
            "loop starts at frame 24000, spans 96000 frames at 48000 Hz (Godot loop_offset 0.5 s)"
        ));
        let mut json_args = args.to_vec();
        json_args.push("--json");
        let mut fake = with_export(files());
        let o = run(&json_args, &mut fake);
        assert_eq!(o.code, 0, "{}", o.stderr);
        assert_eq!(o.stdout.lines().count(), 1);
        let v: Value = serde_json::from_str(&o.stdout).unwrap();
        assert_eq!(v["preset"], "tagged-loop");
        assert_eq!(v["files"][0]["role"], "single");
        assert_eq!(v["files"][0]["path"], out.to_str().unwrap());
        assert_eq!(v["files"][0]["format"], "wav");
        assert_eq!(
            v["files"][0]["loopPoints"],
            serde_json::json!({
                "startFrame": 24_000, "lengthFrames": 96_000, "sampleRate": 48_000, "godotLoopOffset": 0.5,
            })
        );
        assert!(o.stderr.contains("warning: w one"));
    }

    #[test]
    fn split_write_failure_reports_both_destinations() {
        let dir = tempfile::tempdir().unwrap();
        let (song, out) = (song(dir.path()), dir.path().join("pair.wav"));
        let intro = dir.path().join("pair-intro.wav");
        let body = dir.path().join("pair-loop.wav");
        let pair = || {
            vec![
                export_file(ExportRole::Intro, None),
                export_file(
                    ExportRole::Loop,
                    Some(LoopPoints {
                        start_frame: 0,
                        length_frames: 44_100,
                        sample_rate: 44_100,
                    }),
                ),
            ]
        };
        let args = [
            "--json",
            "render",
            &song,
            "--out",
            out.to_str().unwrap(),
            "--preset",
            "intro-loop",
        ];
        let mut fake = with_export(pair());
        let success = run(&args, &mut fake);
        assert_eq!(success.code, 0, "{}", success.stderr);
        assert_eq!(success.stderr.matches("warning: w one").count(), 1);
        let json: Value = serde_json::from_str(&success.stdout).unwrap();
        assert_eq!(json["files"].as_array().unwrap().len(), 2);
        assert_eq!(json["files"][0]["path"], intro.to_str().unwrap());
        assert_eq!(json["files"][0]["role"], "intro");
        assert_eq!(json["files"][0]["loopPoints"], Value::Null);
        assert_eq!(json["files"][1]["path"], body.to_str().unwrap());
        assert_eq!(json["files"][1]["role"], "loop");
        assert_eq!(json["files"][1]["loopPoints"]["startFrame"], 0);
        assert_eq!(json["files"][0]["warnings"][0], "w one");
        assert_eq!(json["files"][1]["warnings"][0], "w one");
        let mut fake = with_export(pair());
        fake.publishes_before_error = 1;
        fake.publish_error = Some(export::Error::Render(RenderError::Io {
            path: body.clone(),
            source: std::io::Error::other("disk full"),
        }));
        let o = run(&args, &mut fake);
        assert_eq!(o.code, 1, "{}", o.stderr);
        assert_eq!(fake.published, vec![intro.clone()]);
        assert!(o.stdout.is_empty());
        assert!(o.stderr.contains(&format!(
            "partial export: wrote {}, failed to write {}",
            intro.display(),
            body.display()
        )));
        assert!(o.stderr.contains("disk full"));
    }

    #[test]
    fn only_one_shot_accepts_length_flags() {
        let dir = tempfile::tempdir().unwrap();
        let song = song(dir.path());
        for preset in ["tagged-loop", "intro-loop", "folded-loop"] {
            for (flag, value) in [("--section", "body"), ("--length", "2"), ("--loops", "3")] {
                let mut fake = ok();
                let o = run(
                    &[
                        "render", &song, "--out", "x.wav", "--preset", preset, flag, value,
                    ],
                    &mut fake,
                );
                assert_eq!(o.code, 4, "{}", o.stderr);
                assert!(o.stderr.contains("loop presets select their own bars"));
                assert!(fake.seen.is_empty());
                assert!(fake.published.is_empty());
            }
        }
        for (flag, value, expected) in [
            (
                "--section",
                "body",
                LengthRequest::Section {
                    name: "body".into(),
                },
            ),
            ("--length", "2", LengthRequest::Length { bars: 2 }),
            ("--loops", "3", LengthRequest::Clip { loops: Some(3) }),
        ] {
            let mut fake = with_export(vec![export_file(ExportRole::Single, None)]);
            let o = run(
                &[
                    "--json", "render", &song, "--out", "x.wav", "--preset", "one-shot", flag,
                    value,
                ],
                &mut fake,
            );
            assert_eq!(o.code, 0, "{}", o.stderr);
            assert_eq!(fake.preset, Some(ExportPreset::OneShot));
            assert_eq!(fake.seen[0].1.length, expected);
            let v: Value = serde_json::from_str(&o.stdout).unwrap();
            assert_eq!(v["files"][0]["loopPoints"], Value::Null);
        }
    }

    #[test]
    fn success_passes_the_defaults_and_prints_the_summary() {
        let dir = tempfile::tempdir().unwrap();
        let (song, out) = (song(dir.path()), dir.path().join("o.wav"));
        let mut fake = ok();
        fake.events = vec![
            Progress::Planned { chunks: 2 },
            Progress::ChunkRendered { index: 1, of: 2 },
            Progress::ChunkRendered { index: 0, of: 2 },
            Progress::Encoded { pass: 1 },
        ];
        let o = run(
            &["render", &song, "--out", out.to_str().unwrap()],
            &mut fake,
        );
        assert_eq!(o.code, 0, "{}", o.stderr);
        let (code, opts) = &fake.seen[0];
        assert_eq!(code, "note('c3')");
        assert_eq!(opts.length, LengthRequest::Clip { loops: None });
        assert_eq!(opts.jobs, None);
        assert_eq!(opts.plan_timeout, Some(Duration::from_secs(120)));
        assert!(!opts.include_events);
        assert_eq!(
            opts.output,
            OutputOptions {
                loudness: Loudness::On {
                    target_lufs: -23.0,
                    true_peak_db: -1.0
                },
                encoder: EncoderOptions::Wav {
                    bit_depth: BitDepth::B24
                },
            }
        );
        assert_eq!(fake.published, vec![out.clone()]);
        assert_eq!(
            o.stderr,
            format!(
                "planned 2 chunk(s)\nrendered 1/2\nrendered 2/2\nencoded\nwrote {}\nwarning: w one\n",
                out.display()
            )
        );
        assert_eq!(
            o.stdout,
            format!(
                "rendered {}\n  wav 1.5 MiB, 8.0 s, bars 1-4 (linear layout, 2 chunk(s))\n  loudness -20.0 -> -14.0 LUFS (gain +6.0 dB), true peak -1.0 dBTP, limiter touched 12 samples\n  time: plan 1.5 s, render 2.5 s, encode 0.5 s\n",
                out.display()
            )
        );
    }

    #[test]
    fn lossy_summary_has_the_decoded_peak_and_reencode_progress() {
        let dir = tempfile::tempdir().unwrap();
        let (song, out) = (song(dir.path()), dir.path().join("o.mp3"));
        let mut fake = with(Ok(RenderOutput {
            bytes: vec![0],
            summary: summary(Format::Mp3, true),
        }));
        fake.events = vec![Progress::Encoded { pass: 1 }, Progress::Encoded { pass: 2 }];
        let o = run(
            &["render", &song, "--out", out.to_str().unwrap()],
            &mut fake,
        );
        assert_eq!(o.code, 0);
        assert!(o.stdout.contains("  mp3 1.5 MiB"), "{}", o.stdout);
        assert!(
            o.stdout
                .contains("  decoded true peak -1.2 dBTP after 2 encode(s)\n"),
            "{}",
            o.stdout
        );
        assert!(
            o.stderr
                .contains("re-encoded (pass 2) to bring the decoded peak under the ceiling\n"),
            "{}",
            o.stderr
        );
    }

    #[test]
    fn loudness_off_summary_and_silence_print_as_documented() {
        let dir = tempfile::tempdir().unwrap();
        let (song, out) = (song(dir.path()), dir.path().join("o.wav"));
        let mut s = summary(Format::Wav, false);
        if let Some(l) = s.loudness.as_mut() {
            l.enabled = false;
            l.input_lufs = f64::NEG_INFINITY;
            l.input_true_peak_db = f64::NEG_INFINITY;
        }
        let mut fake = with(Ok(RenderOutput {
            bytes: vec![0],
            summary: s,
        }));
        let o = run(
            &[
                "render",
                &song,
                "--out",
                out.to_str().unwrap(),
                "--loudness",
                "off",
            ],
            &mut fake,
        );
        assert_eq!(o.code, 0);
        assert!(
            o.stdout
                .contains("  loudness off: -inf LUFS, true peak -inf dBTP\n"),
            "{}",
            o.stdout
        );
        assert_eq!(fake.seen[0].1.output.loudness, Loudness::Off);
    }

    #[test]
    fn json_is_one_camel_case_line_with_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let (song, out) = (song(dir.path()), dir.path().join("o.wav"));
        let mut fake = ok();
        let o = run(
            &["--json", "render", &song, "--out", out.to_str().unwrap()],
            &mut fake,
        );
        assert_eq!(o.code, 0);
        assert_eq!(o.stdout.matches('\n').count(), 1);
        let v: Value = serde_json::from_str(&o.stdout).unwrap();
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "bytes",
                "chunks",
                "format",
                "frames",
                "jobs",
                "loudness",
                "path",
                "sampleRate",
                "seconds",
                "timings",
                "warnings",
                "window"
            ]
        );
        assert_eq!(v["path"], out.to_string_lossy().as_ref());
        assert_eq!(v["format"], "wav");
        assert_eq!(v["frames"], 352_800);
        assert_eq!(v["window"]["toBar"], 4);
        assert_eq!(v["loudness"]["limiterCeilingDbtp"], -1.0);
        assert_eq!(v["timings"]["totalMs"], 4500.0);
        assert!(
            o.stderr.contains("warning: w one"),
            "warnings stay on stderr"
        );
    }

    #[test]
    fn f1_prints_one_decimal_and_the_non_finite_names() {
        assert_eq!(f1(-14.04), "-14.0");
        assert_eq!(f1(f64::NEG_INFINITY), "-inf");
        assert_eq!(f1(f64::INFINITY), "inf");
        assert_eq!(f1(f64::NAN), "nan");
    }

    #[test]
    fn json_turns_silence_into_null() {
        let mut s = summary(Format::Wav, false);
        if let Some(l) = s.loudness.as_mut() {
            l.input_lufs = f64::NEG_INFINITY;
        }
        let v: Value = serde_json::from_str(&json_line(&s, Path::new("/x.wav"))).unwrap();
        assert_eq!(v["loudness"]["inputLufs"], Value::Null);
    }

    #[test]
    fn every_flag_reaches_the_options() {
        let dir = tempfile::tempdir().unwrap();
        let song = song(dir.path());
        let out = dir.path().join("o.mp3");
        let mut fake = ok();
        let o = run(
            &[
                "render",
                &song,
                "--out",
                out.to_str().unwrap(),
                "--section",
                "body",
                "--lufs",
                "-16.5",
                "--true-peak",
                "-2",
                "--mp3-mode",
                "vbr",
                "--quality",
                "3",
                "--jobs",
                "3",
                "--plan-timeout",
                "7.5",
            ],
            &mut fake,
        );
        assert_eq!(o.code, 0, "{}", o.stderr);
        let opts = &fake.seen[0].1;
        assert_eq!(
            opts.length,
            LengthRequest::Section {
                name: "body".into()
            }
        );
        assert_eq!(opts.jobs, Some(3));
        assert_eq!(opts.plan_timeout, Some(Duration::from_millis(7500)));
        assert_eq!(
            opts.output,
            OutputOptions {
                loudness: Loudness::On {
                    target_lufs: -16.5,
                    true_peak_db: -2.0
                },
                encoder: EncoderOptions::Mp3 {
                    mode: Mp3Mode::Vbr,
                    vbr_quality: 3
                },
            }
        );
    }

    #[test]
    fn length_and_loops_flags_pick_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let song = song(dir.path());
        let out = dir.path().join("o.ogg");
        let out = out.to_str().unwrap();
        let mut fake = ok();
        run(&["render", &song, "--out", out, "--length", "4"], &mut fake);
        assert_eq!(fake.seen[0].1.length, LengthRequest::Length { bars: 4 });
        assert_eq!(
            fake.seen[0].1.output.encoder,
            EncoderOptions::Ogg { quality: 6 }
        );
        let mut fake = ok();
        run(&["render", &song, "--out", out, "--loops", "3"], &mut fake);
        assert_eq!(
            fake.seen[0].1.length,
            LengthRequest::Clip { loops: Some(3) }
        );
    }

    #[test]
    fn malformed_project_pin_fails_before_rendering_even_with_explicit_output() {
        let dir = tempfile::tempdir().unwrap();
        let project = strudeland_project::Project::create(&dir.path().join("p"), "test").unwrap();
        std::fs::write(project.root().join("instruments/lead.json"), "{}").unwrap();
        let song = project.root().join("songs/main.js");
        let out = dir.path().join("mix.wav");
        let mut fake = ok();
        let result = run(
            &[
                "render",
                song.to_str().unwrap(),
                "--out",
                out.to_str().unwrap(),
            ],
            &mut fake,
        );
        assert_eq!(result.code, 1);
        assert!(result.stderr.contains("lead"), "{}", result.stderr);
        assert!(fake.seen.is_empty());
        assert!(!out.exists());
    }

    #[test]
    fn usage_errors_exit_4_before_any_render() {
        let dir = tempfile::tempdir().unwrap();
        let song = song(dir.path());
        let out = dir.path().join("o.wav");
        let out = out.to_str().unwrap();
        let cases: [(&[&str], &str); 7] = [
            (
                &["--loudness", "off", "--lufs", "-10"],
                "--lufs does not apply with --loudness off",
            ),
            (
                &["--loudness", "off", "--true-peak", "-3"],
                "--true-peak does not apply",
            ),
            (&["--format", "mp3"], "--format mp3 disagrees"),
            (&["--mp3-mode", "vbr"], "--mp3-mode only applies to mp3"),
            (
                &["--bit-depth", "16", "--format", "wav", "--quality", "2"],
                "--quality does not apply to wav",
            ),
            (&["--format", "opus", "--bit-depth", "16"], "disagrees"),
            (&["--quality", "5"], "--quality does not apply to wav"),
        ];
        for (extra, want) in cases {
            let mut fake = ok();
            let mut args = vec!["render", song.as_str(), "--out", out];
            args.extend(extra);
            let o = run(&args, &mut fake);
            assert_eq!(o.code, 4, "{extra:?}: {}", o.stderr);
            assert!(o.stderr.starts_with("error: "), "{extra:?}: {}", o.stderr);
            assert!(o.stderr.contains(want), "{extra:?}: {}", o.stderr);
            assert!(fake.seen.is_empty(), "{extra:?} must not render");
            assert!(o.stdout.is_empty());
        }
    }

    #[test]
    fn incompatible_stem_flags_fail_before_rendering() {
        let dir = tempfile::tempdir().unwrap();
        let song = song(dir.path());
        let out = dir.path().join("set");
        for (flag, value, message) in [
            ("--preset", "one-shot", "omit --preset"),
            ("--format", "opus", "supports only WAV"),
            ("--bit-depth", "24", "32-bit float"),
            ("--loudness", "on", "loudness off"),
            ("--lufs", "-16", "omit --lufs"),
            ("--true-peak", "-2", "omit --true-peak"),
            ("--mp3-mode", "vbr", "--mp3-mode only applies"),
            ("--quality", "2", "--quality does not apply"),
        ] {
            let o = run(
                &[
                    "render",
                    &song,
                    "--stems",
                    "voice",
                    "--out",
                    out.to_str().unwrap(),
                    flag,
                    value,
                ],
                &mut ok(),
            );
            assert_eq!(o.code, 4, "{flag}: {}", o.stderr);
            assert!(o.stderr.contains(message), "{flag}: {}", o.stderr);
            assert!(o.stdout.is_empty());
            assert!(!out.exists());
        }
    }

    #[test]
    fn stems_use_float_wav_and_resolve_the_project_range_directory() {
        let dir = tempfile::tempdir().unwrap();
        let project = strudeland_project::Project::create(&dir.path().join("p"), "t").unwrap();
        let song = project.root().join("songs/main.js");
        std::fs::write(&song, "note('c3')").unwrap();
        let directory = project.root().join("renders/main-body-stems");
        let mut fake = FakePipeline {
            stems: Some(Ok(StemExport {
                directory: directory.clone(),
                files: vec![],
                summary: summary(Format::Wav, false),
            })),
            ..ok()
        };
        let o = run(
            &[
                "render",
                song.to_str().unwrap(),
                "--stems",
                "instrument",
                "--section",
                "body",
                "--format",
                "wav",
                "--bit-depth",
                "32f",
                "--loudness",
                "off",
            ],
            &mut fake,
        );
        assert_eq!(o.code, 0, "{}", o.stderr);
        assert_eq!(
            fake.seen[0].1.length,
            LengthRequest::Section {
                name: "body".into()
            }
        );
        assert_eq!(
            fake.seen[0].1.output,
            OutputOptions {
                loudness: Loudness::Off,
                encoder: EncoderOptions::Wav {
                    bit_depth: BitDepth::F32
                },
            },
        );
        assert_eq!(fake.grouping, Some(StemGrouping::Instrument));
        assert_eq!(fake.stem_targets, vec![directory.clone()]);
        assert!(!directory.exists());
    }

    #[test]
    fn stem_output_identifies_the_reference_and_reports_partial_writes() {
        let dir = tempfile::tempdir().unwrap();
        let song = song(dir.path());
        let out = dir.path().join("stems");
        let mix = out.join("mix.wav");
        let manifest = out.join("stems.json");
        let args = [
            "render",
            &song,
            "--out",
            out.to_str().unwrap(),
            "--stems",
            "voice",
        ];
        let mut success = FakePipeline {
            stems: Some(Ok(StemExport {
                directory: out.clone(),
                files: vec![mix.clone(), manifest.clone()],
                summary: summary(Format::Wav, false),
            })),
            ..ok()
        };
        let o = run(&args, &mut success);
        assert_eq!(o.code, 0, "{}", o.stderr);
        assert!(o.stdout.contains("pre-master reference mix"));
        assert!(o.stdout.contains(mix.to_str().unwrap()));
        assert!(o.stdout.contains(manifest.to_str().unwrap()));
        assert!(o.stderr.contains("warning: w one"));

        let mut failure = FakePipeline {
            stems: Some(Err(RenderError::Io {
                path: manifest.clone(),
                source: std::io::Error::other("disk full"),
            })),
            events: vec![Progress::Written { path: mix.clone() }],
            ..ok()
        };
        let args: Vec<_> = ["--json"].into_iter().chain(args).collect();
        let o = run(&args, &mut failure);
        assert_eq!(o.code, 1);
        assert!(o.stdout.is_empty());
        assert!(o.stderr.contains(&format!("wrote {}", mix.display())));
        assert!(
            o.stderr
                .contains(&format!("{}: disk full", manifest.display()))
        );
    }

    #[test]
    fn out_is_required_outside_a_project() {
        let dir = tempfile::tempdir().unwrap();
        let song = song(dir.path());
        let mut fake = ok();
        let o = run(&["render", &song], &mut fake);
        assert_eq!(o.code, 4);
        assert!(
            o.stderr.contains("--out is required:")
                && o.stderr.contains("is not in a strudeland project"),
            "{}",
            o.stderr
        );
        assert!(fake.seen.is_empty());
    }

    #[test]
    fn a_project_song_defaults_to_renders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let root = strudeland_project::Project::create(&root.join("p"), "t")
            .unwrap()
            .root()
            .to_path_buf();
        let song = root.join("songs").join("main.js");
        std::fs::write(&song, "note('c3')").unwrap();
        let mut fake = ok();
        let o = run(
            &[
                "render",
                song.to_str().unwrap(),
                "--section",
                "body",
                "--format",
                "ogg",
            ],
            &mut fake,
        );
        assert_eq!(o.code, 0, "{}", o.stderr);
        assert_eq!(
            fake.published,
            vec![root.join("renders").join("main-body.ogg")]
        );
        // --length and --loops name their own files, so they never replace the full render.
        for (flags, name) in [
            (&[][..], "main.wav"),
            (&["--length", "4"][..], "main-len4.wav"),
            (&["--loops", "2"][..], "main-loops2.wav"),
        ] {
            let mut fake = ok();
            let args: Vec<&str> = ["render", song.to_str().unwrap()]
                .into_iter()
                .chain(flags.iter().copied())
                .collect();
            let o = run(&args, &mut fake);
            assert_eq!(o.code, 0, "{flags:?}: {}", o.stderr);
            assert_eq!(
                fake.published,
                vec![root.join("renders").join(name)],
                "{flags:?}"
            );
        }
    }

    #[test]
    fn preset_outputs_use_distinct_project_paths() {
        let dir = tempfile::tempdir().unwrap();
        let project = strudeland_project::Project::create(&dir.path().join("p"), "t").unwrap();
        let song = project.root().join("songs/main.js");
        std::fs::write(&song, "note('c3')").unwrap();
        let renders = project.root().join("renders");
        for (preset, role, name) in [
            ("tagged-loop", ExportRole::Single, "main-tagged-loop.wav"),
            ("folded-loop", ExportRole::Loop, "main-folded-loop.wav"),
            ("one-shot", ExportRole::Single, "main-one-shot.wav"),
        ] {
            let mut fake = with_export(vec![export_file(role, None)]);
            let out = run(
                &["render", song.to_str().unwrap(), "--preset", preset],
                &mut fake,
            );
            assert_eq!(out.code, 0, "{preset}: {}", out.stderr);
            assert_eq!(fake.published, vec![renders.join(name)]);
        }
        let mut fake = with_export(vec![
            export_file(ExportRole::Intro, None),
            export_file(ExportRole::Loop, None),
        ]);
        let out = run(
            &["render", song.to_str().unwrap(), "--preset", "intro-loop"],
            &mut fake,
        );
        assert_eq!(out.code, 0, "{}", out.stderr);
        assert_eq!(
            fake.published,
            vec![
                renders.join("main-intro.wav"),
                renders.join("main-loop.wav")
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn split_validates_the_intro_path_before_rendering() {
        let dir = tempfile::tempdir().unwrap();
        let project = strudeland_project::Project::create(&dir.path().join("p"), "t").unwrap();
        let song = project.root().join("songs/main.js");
        std::fs::write(&song, "note('c3')").unwrap();
        let renders = project.root().join("renders");
        std::os::unix::fs::symlink(
            dir.path().join("outside.wav"),
            renders.join("main-intro.wav"),
        )
        .unwrap();
        let args = ["render", song.to_str().unwrap(), "--preset", "intro-loop"];

        let mut fake = ok();
        let out = run(&args, &mut fake);
        assert_eq!(out.code, 1, "{}", out.stderr);
        assert!(out.stderr.contains("main-intro.wav"), "{}", out.stderr);
        assert!(fake.seen.is_empty());
        assert!(fake.published.is_empty());
    }

    #[test]
    fn a_failed_render_leaves_no_renders_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let root = strudeland_project::Project::create(&root.join("p"), "t")
            .unwrap()
            .root()
            .to_path_buf();
        std::fs::remove_dir(root.join("renders")).unwrap();
        let song = root.join("songs").join("main.js");
        std::fs::write(&song, "note('c3')").unwrap();
        let mut fake = with(Err(RenderError::Render("chunk 0: boom".into())));
        let o = run(&["render", song.to_str().unwrap()], &mut fake);
        assert_eq!(o.code, 1, "{}", o.stderr);
        assert!(fake.published.is_empty());
        assert!(!root.join("renders").exists());
    }

    #[test]
    fn an_unreadable_song_is_a_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.js");
        let mut fake = ok();
        let o = run(
            &["render", missing.to_str().unwrap(), "--out", "x.wav"],
            &mut fake,
        );
        assert_eq!(o.code, 4);
        assert!(o.stderr.starts_with("error: cannot read "), "{}", o.stderr);
        let bad = dir.path().join("bad.js");
        std::fs::write(&bad, [0xff, 0xfe]).unwrap();
        let o = run(
            &["render", bad.to_str().unwrap(), "--out", "x.wav"],
            &mut fake,
        );
        assert_eq!(o.code, 4);
        assert!(o.stderr.contains("not valid UTF-8"), "{}", o.stderr);
        assert!(fake.seen.is_empty());
    }

    fn fail(e: RenderError) -> Out {
        let dir = tempfile::tempdir().unwrap();
        let song = song(dir.path());
        let out = dir.path().join("o.wav");
        let mut fake = with(Err(e));
        let o = run(
            &["render", &song, "--out", out.to_str().unwrap()],
            &mut fake,
        );
        assert!(fake.published.is_empty(), "nothing is written on failure");
        assert!(o.stdout.is_empty(), "no stdout on failure");
        o
    }

    #[test]
    fn host_length_errors_are_usage_other_plan_errors_fail() {
        let o = fail(RenderError::Plan(HostError::MetaFlag {
            flag: "--length".into(),
            message: "this song has no sections; pass --length N".into(),
        }));
        assert_eq!(o.code, 4);
        assert_eq!(
            o.stderr,
            "error: this song has no sections; pass --length N\n"
        );
        for (e, text) in [
            (HostError::Eval("boom".into()), "error: eval: boom\n"),
            (HostError::Meta("bad".into()), "error: meta: bad\n"),
        ] {
            let o = fail(RenderError::Plan(e));
            assert_eq!(o.code, 1);
            assert_eq!(o.stderr, text);
        }
    }

    #[test]
    fn a_planning_timeout_exits_1_with_the_hint() {
        let o = fail(RenderError::Plan(HostError::Timeout { millis: 1500 }));
        assert_eq!(o.code, 1);
        assert!(o.stderr.starts_with("error: "), "{}", o.stderr);
        assert!(
            o.stderr.ends_with("(raise it with --plan-timeout)\n"),
            "{}",
            o.stderr
        );
    }

    #[test]
    fn unsupported_material_exits_1_with_the_list() {
        let o = fail(RenderError::Unsupported(vec![Unsupported {
            s: "zzfx".into(),
            keys: vec![],
            layout_bar: 2,
            section: None,
        }]));
        assert_eq!(o.code, 1);
        assert!(o.stderr.contains("s=zzfx first at bar 3"), "{}", o.stderr);
    }

    #[test]
    fn render_encode_and_write_failures_exit_1() {
        let o = fail(RenderError::Render("chunk 2: x".into()));
        assert_eq!(
            (o.code, o.stderr.as_str()),
            (1, "error: render failed: chunk 2: x\n")
        );
        let o = fail(RenderError::Encode("lame".into()));
        assert_eq!(
            (o.code, o.stderr.as_str()),
            (1, "error: encode failed: lame\n")
        );

        let dir = tempfile::tempdir().unwrap();
        let (song, out) = (song(dir.path()), dir.path().join("o.wav"));
        let mut fake = ok();
        fake.publish_error = Some(export::Error::Render(RenderError::Io {
            path: out.clone(),
            source: std::io::Error::other("disk full"),
        }));
        let o = run(
            &["render", &song, "--out", out.to_str().unwrap()],
            &mut fake,
        );
        assert_eq!(o.code, 1);
        assert!(o.stderr.contains("disk full"), "{}", o.stderr);
        assert!(o.stdout.is_empty());
    }

    #[test]
    fn json_does_not_change_errors() {
        let dir = tempfile::tempdir().unwrap();
        let song = song(dir.path());
        let mut fake = with(Err(RenderError::Render("x".into())));
        let o = run(&["--json", "render", &song, "--out", "x.wav"], &mut fake);
        assert_eq!(o.code, 1);
        assert!(o.stdout.is_empty());
        assert_eq!(o.stderr, "error: render failed: x\n");
    }

    #[test]
    fn global_timeout_and_socket_are_accepted_and_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let (song, out) = (song(dir.path()), dir.path().join("o.wav"));
        let mut fake = ok();
        let o = run(
            &[
                "--timeout",
                "5",
                "--socket",
                "/nonexistent.sock",
                "render",
                &song,
                "--out",
                out.to_str().unwrap(),
            ],
            &mut fake,
        );
        assert_eq!(o.code, 0, "{}", o.stderr);
    }
}
