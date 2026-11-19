//! `strudeland render`: option resolution, the backend seam, progress and the summary output.
//! It runs offline and never opens the control socket.

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;
use strudeland_render::{
    DEFAULT_TARGET_LUFS, DEFAULT_TRUE_PEAK_DB, EncoderOptions, LengthRequest, Loudness,
    OutputOptions, OutputTarget, Progress, RenderError, RenderOptions, RenderOutput, Summary,
    resolve_format, resolve_output,
};

use crate::cli::{OnOff, RenderArgs};

/// What `render` needs from the render pipeline, so tests can inject a fake.
pub trait RenderBackend {
    /// Plans, renders, levels and encodes the song's code.
    fn render(
        &mut self,
        code: &str,
        opts: &RenderOptions,
        progress: &dyn Fn(Progress),
    ) -> Result<RenderOutput, RenderError>;

    /// Writes the encoded file and returns where it went.
    fn write(
        &mut self,
        out: &RenderOutput,
        target: &OutputTarget,
        progress: &dyn Fn(Progress),
    ) -> Result<PathBuf, RenderError>;
}

/// The real pipeline: `strudeland_render`.
pub struct StrudelandBackend;

impl RenderBackend for StrudelandBackend {
    fn render(
        &mut self,
        code: &str,
        opts: &RenderOptions,
        progress: &dyn Fn(Progress),
    ) -> Result<RenderOutput, RenderError> {
        strudeland_render::render(code, opts, progress)
    }

    fn write(
        &mut self,
        out: &RenderOutput,
        target: &OutputTarget,
        progress: &dyn Fn(Progress),
    ) -> Result<PathBuf, RenderError> {
        strudeland_render::write_output(out, target, progress)
    }
}

fn usage(flag: &'static str, message: impl Into<String>) -> RenderError {
    RenderError::Usage {
        flag,
        message: message.into(),
    }
}

/// Everything the flags resolve to: the render options and where the file goes.
pub struct Plan {
    pub opts: RenderOptions,
    pub target: OutputTarget,
}

/// Resolves the flags before any work, so a bad flag or a missing `--out` fails fast. Creates
/// nothing: a project's `renders/` is created only when the file is written.
pub fn plan(args: &RenderArgs) -> Result<Plan, RenderError> {
    let length = if let Some(name) = &args.section {
        LengthRequest::Section { name: name.clone() }
    } else if let Some(bars) = args.length {
        LengthRequest::Length { bars }
    } else {
        LengthRequest::Clip { loops: args.loops }
    };
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
    let target = resolve_output(args.out.as_deref(), &args.song, &length, format)?;
    Ok(Plan {
        opts: RenderOptions {
            length,
            jobs: args.jobs,
            plan_timeout: Some(args.plan_timeout),
            include_events: false,
            output: OutputOptions { loudness, encoder },
        },
        target,
    })
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
    pub fn warnings(&self, summary: &Summary) {
        let mut err = self.stderr.borrow_mut();
        for w in &summary.warnings {
            let _ = writeln!(err, "warning: {w}");
        }
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
    use strudeland_render::level::LoudnessSummary;
    use strudeland_render::{BitDepth, Format, Mp3Mode};
    use strudeland_render::{Timings, Unsupported};

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

    struct Fake {
        result: Option<Result<RenderOutput, RenderError>>,
        write_error: Option<RenderError>,
        seen: Vec<(String, RenderOptions)>,
        written: Vec<PathBuf>,
        events: Vec<Progress>,
    }

    impl Fake {
        fn ok() -> Fake {
            Fake::with(Ok(RenderOutput {
                bytes: vec![1, 2, 3],
                summary: summary(Format::Wav, false),
            }))
        }
        fn with(r: Result<RenderOutput, RenderError>) -> Fake {
            Fake {
                result: Some(r),
                write_error: None,
                seen: vec![],
                written: vec![],
                events: vec![],
            }
        }
    }

    impl RenderBackend for Fake {
        fn render(
            &mut self,
            code: &str,
            opts: &RenderOptions,
            progress: &dyn Fn(Progress),
        ) -> Result<RenderOutput, RenderError> {
            self.seen.push((code.to_string(), opts.clone()));
            for p in &self.events {
                progress(p.clone());
            }
            self.result.take().expect("one render")
        }
        fn write(
            &mut self,
            _: &RenderOutput,
            target: &OutputTarget,
            progress: &dyn Fn(Progress),
        ) -> Result<PathBuf, RenderError> {
            let path = target.path();
            if let Some(e) = self.write_error.take() {
                return Err(e);
            }
            self.written.push(path.to_path_buf());
            progress(Progress::Written {
                path: path.to_path_buf(),
            });
            Ok(path.to_path_buf())
        }
    }

    struct Out {
        code: i32,
        stdout: String,
        stderr: String,
    }

    fn run(args: &[&str], fake: &mut Fake) -> Out {
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

    #[test]
    fn success_passes_the_defaults_and_prints_the_summary() {
        let dir = tempfile::tempdir().unwrap();
        let (song, out) = (song(dir.path()), dir.path().join("o.wav"));
        let mut fake = Fake::ok();
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
        assert_eq!(fake.written, vec![out.clone()]);
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
        let mut fake = Fake::with(Ok(RenderOutput {
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
        let mut fake = Fake::with(Ok(RenderOutput {
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
        let mut fake = Fake::ok();
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
        let mut fake = Fake::ok();
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
        let mut fake = Fake::ok();
        run(&["render", &song, "--out", out, "--length", "4"], &mut fake);
        assert_eq!(fake.seen[0].1.length, LengthRequest::Length { bars: 4 });
        assert_eq!(
            fake.seen[0].1.output.encoder,
            EncoderOptions::Ogg { quality: 6 }
        );
        let mut fake = Fake::ok();
        run(&["render", &song, "--out", out, "--loops", "3"], &mut fake);
        assert_eq!(
            fake.seen[0].1.length,
            LengthRequest::Clip { loops: Some(3) }
        );
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
            let mut fake = Fake::ok();
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
    fn out_is_required_outside_a_project() {
        let dir = tempfile::tempdir().unwrap();
        let song = song(dir.path());
        let mut fake = Fake::ok();
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
        let mut fake = Fake::ok();
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
            fake.written,
            vec![root.join("renders").join("main-body.ogg")]
        );
        // --length and --loops name their own files, so they never replace the full render.
        for (flags, name) in [
            (&[][..], "main.wav"),
            (&["--length", "4"][..], "main-len4.wav"),
            (&["--loops", "2"][..], "main-loops2.wav"),
        ] {
            let mut fake = Fake::ok();
            let args: Vec<&str> = ["render", song.to_str().unwrap()]
                .into_iter()
                .chain(flags.iter().copied())
                .collect();
            let o = run(&args, &mut fake);
            assert_eq!(o.code, 0, "{flags:?}: {}", o.stderr);
            assert_eq!(
                fake.written,
                vec![root.join("renders").join(name)],
                "{flags:?}"
            );
        }
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
        let mut fake = Fake::with(Err(RenderError::Render("chunk 0: boom".into())));
        let o = run(&["render", song.to_str().unwrap()], &mut fake);
        assert_eq!(o.code, 1, "{}", o.stderr);
        assert!(fake.written.is_empty());
        assert!(!root.join("renders").exists());
    }

    #[test]
    fn an_unreadable_song_is_a_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.js");
        let mut fake = Fake::ok();
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
        let mut fake = Fake::with(Err(e));
        let o = run(
            &["render", &song, "--out", out.to_str().unwrap()],
            &mut fake,
        );
        assert!(fake.written.is_empty(), "nothing is written on failure");
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
        let mut fake = Fake::ok();
        fake.write_error = Some(RenderError::Io {
            path: out.clone(),
            source: std::io::Error::other("disk full"),
        });
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
        let mut fake = Fake::with(Err(RenderError::Render("x".into())));
        let o = run(&["--json", "render", &song, "--out", "x.wav"], &mut fake);
        assert_eq!(o.code, 1);
        assert!(o.stdout.is_empty());
        assert_eq!(o.stderr, "error: render failed: x\n");
    }

    #[test]
    fn global_timeout_and_socket_are_accepted_and_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let (song, out) = (song(dir.path()), dir.path().join("o.wav"));
        let mut fake = Fake::ok();
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
