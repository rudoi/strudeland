//! `strudeland-render`: the render pipeline's harness binary (not a `strudeland` subcommand).
//!
//! ```text
//! strudeland-render render --song <file>
//!     [ --from N --to N --layout clip|linear | --section NAME | --length N ] [--loops N]
//!     --out <file> [--info <file.json>] [--format wav|mp3|ogg|opus] [--bit-depth 16|24|32f]
//!     [--loudness on|off] [--jobs N] [--plan-timeout SECS]
//! strudeland-render bench [--jobs N] [--lengths FILE] [--bars N] [--out-dir DIR] <song…>
//! ```
//!
//! `render` runs the whole pipeline (plan, refuse, render, join, finish) and writes `--out`.
//! `--from`, `--to` and `--layout` together are an explicit window (the harness's); `--section`
//! and `--length` the other length requests; none of them is the song's clip (`--loops` sets its
//! loop count). The format is `--format`, else the `--out` extension, else WAV (a disagreement is
//! bad input). Defaults: loudness off (`on` is -23 LUFS with a -1 dBTP ceiling), a 32-bit float
//! WAV, a 120 s planning timeout. `--info`
//! writes `{ window, cps, structure, lookbackSec, errors, events, chunks, summary }` (camelCase).
//! Exit codes: 0 ok; 1 the render failed (stderr `strudeland-render: <message>`, nothing
//! written); 2 bad input (an unknown flag, an inconsistent length form, a bad option).
//!
//! `bench` renders each song over its `--lengths` bars (else `--bars`, default 16) at `--jobs`
//! workers (default 4) with the default loudness, finishes it as Ogg (the headline numbers) and
//! as WAV, MP3 and Opus, and prints one JSON document on stdout and a table on stderr
//! (`--out-dir` also gets `<song>.jobs<N>.json` per song). With several songs each runs in its
//! own process, so its peak RSS is its own. Exit 0 every song ran, 1 one failed (the others
//! still run), 2 bad input.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use serde::Serialize;
use strudeland_host::{
    ChunkWindow, Layout, LengthRequest, PlanError, PlannedWindow, Structure, timeout_from_secs,
};
use strudeland_render::{
    BitDepth, EncoderOptions, ExitClass, Format, Loudness, OnOff, OutputOptions, OutputTarget,
    RenderError, RenderOptions, Summary, bench, finish, render_audio, resolve_format, write_output,
};

#[derive(Debug, Parser)]
#[command(
    name = "strudeland-render",
    version,
    about = "Render a song with the headless host and the Rust engine (harness binary)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Render a song to a file.
    Render(Box<RenderArgs>),
    /// Time the whole pipeline and every encoder over songs.
    Bench {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Debug, clap::Args)]
struct RenderArgs {
    /// The song file.
    #[arg(long)]
    song: PathBuf,
    /// Explicit window: first layout bar (0-based), with --to and --layout.
    #[arg(long)]
    from: Option<u32>,
    /// Explicit window: end layout bar (exclusive), with --from and --layout.
    #[arg(long)]
    to: Option<u32>,
    /// Explicit window: clip or linear, with --from and --to.
    #[arg(long)]
    layout: Option<String>,
    /// Render one section.
    #[arg(long)]
    section: Option<String>,
    /// Render the first N bars.
    #[arg(long)]
    length: Option<u32>,
    /// Loop count of the clip (only without a length form).
    #[arg(long)]
    loops: Option<u32>,
    /// The output file.
    #[arg(long)]
    out: PathBuf,
    /// Also write the plan and the summary as JSON here.
    #[arg(long)]
    info: Option<PathBuf>,
    /// wav, mp3, ogg or opus (default: from the --out extension, else wav).
    #[arg(long, value_enum)]
    format: Option<Format>,
    /// 16, 24 or 32f (wav only; default 32f).
    #[arg(long, value_enum)]
    bit_depth: Option<BitDepth>,
    /// on (-23 LUFS, -1 dBTP) or off.
    #[arg(long, value_enum, default_value = "off")]
    loudness: OnOff,
    /// Worker threads (default min(cores, 4), clamped to the chunk count).
    #[arg(long)]
    jobs: Option<usize>,
    /// Planning timeout in seconds.
    #[arg(long, default_value_t = 120.0)]
    plan_timeout: f64,
}

/// The length request the flags name.
fn length_request(a: &RenderArgs) -> Result<LengthRequest, String> {
    let window = [a.from.is_some(), a.to.is_some(), a.layout.is_some()];
    let forms = usize::from(window.contains(&true))
        + usize::from(a.section.is_some())
        + usize::from(a.length.is_some());
    if forms > 1 {
        return Err("give at most one of --from/--to/--layout, --section and --length".to_string());
    }
    if a.loops.is_some() && forms > 0 {
        return Err("--loops applies only to the clip (no --from, --section or --length)".into());
    }
    if let (Some(from_bar), Some(to_bar), Some(layout)) = (a.from, a.to, a.layout.as_deref()) {
        let layout = layout
            .parse::<Layout>()
            .map_err(|_| format!("--layout must be clip or linear, not {layout:?}"))?;
        return Ok(LengthRequest::Window {
            from_bar,
            to_bar,
            layout,
        });
    }
    if window.contains(&true) {
        return Err("--from, --to and --layout go together".into());
    }
    if let Some(name) = &a.section {
        return Ok(LengthRequest::Section { name: name.clone() });
    }
    if let Some(bars) = a.length {
        return Ok(LengthRequest::Length { bars });
    }
    Ok(LengthRequest::Clip { loops: a.loops })
}

/// The render options the flags name.
fn options(a: &RenderArgs) -> Result<RenderOptions, String> {
    let length = length_request(a)?;
    let format = resolve_format(a.format, Some(&a.out)).map_err(|e| e.to_string())?;
    // The harness writes 32-bit float WAV unless told otherwise.
    let bit_depth = a
        .bit_depth
        .or((format == Format::Wav).then_some(BitDepth::F32));
    let encoder =
        EncoderOptions::from_flags(format, bit_depth, None, None).map_err(|e| e.to_string())?;
    let loudness = match a.loudness {
        OnOff::Off => Loudness::Off,
        OnOff::On => Loudness::default(),
    };
    let plan_timeout = timeout_from_secs(a.plan_timeout).ok_or_else(|| {
        format!(
            "--plan-timeout must be a number of seconds greater than 0 and at most {}",
            strudeland_host::MAX_TIMEOUT_SEC
        )
    })?;
    Ok(RenderOptions {
        instruments: Default::default(),
        length,
        jobs: a.jobs,
        plan_timeout: Some(plan_timeout),
        include_events: a.info.is_some(),
        output: OutputOptions { loudness, encoder },
    })
}

/// The `--info` document.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Info<'a> {
    window: &'a PlannedWindow,
    cps: f64,
    structure: &'a Structure,
    lookback_sec: f64,
    errors: &'a [PlanError],
    events: Option<&'a serde_json::Value>,
    chunks: &'a [ChunkWindow],
    summary: &'a Summary,
}

/// How `render` failed: bad input (exit 2) or a failed render (exit 1).
enum Failure {
    BadInput(String),
    Failed(String),
}

impl From<RenderError> for Failure {
    fn from(e: RenderError) -> Self {
        match e.exit_class() {
            ExitClass::Usage => Failure::BadInput(e.to_string()),
            ExitClass::Failed => Failure::Failed(e.to_string()),
        }
    }
}

fn run_render(a: &RenderArgs) -> Result<(), Failure> {
    let opts = options(a).map_err(Failure::BadInput)?;
    let code = std::fs::read_to_string(&a.song)
        .map_err(|e| Failure::BadInput(format!("{}: {e}", a.song.display())))?;
    let audio = render_audio(&code, &opts, &|_| {})?;
    let out = finish(&audio, &opts.output, &|_| {})?;
    write_output(&out, &OutputTarget::file(&a.out), &|_| {})?;
    if let Some(path) = &a.info {
        let info = Info {
            window: &audio.summary.window,
            cps: audio.cps,
            structure: &audio.structure,
            lookback_sec: audio.lookback_sec,
            errors: &audio.plan_errors,
            events: audio.events.as_ref(),
            chunks: &audio.chunks,
            summary: &out.summary,
        };
        let bytes = serde_json::to_vec(&info).map_err(|e| Failure::Failed(e.to_string()))?;
        write_file(path, &bytes)?;
    }
    Ok(())
}

/// `bench`: the JSON document on stdout, a table on stderr; exit 1 when a song failed.
fn run_bench(raw: &[String]) -> Result<(), Failure> {
    let args = bench::parse_args(raw).map_err(Failure::BadInput)?;
    let exe = std::env::current_exe().ok();
    let mut header = true;
    let report = bench::run(&args, exe.as_deref(), |song| {
        if std::mem::take(&mut header) {
            eprintln!("{}", bench::TABLE_HEADER);
        }
        eprintln!("{}", bench::format_row(song));
    })
    .map_err(Failure::BadInput)?;
    let text = serde_json::to_string_pretty(&report).map_err(|e| Failure::Failed(e.to_string()))?;
    println!("{text}");
    if report.all_ok() {
        Ok(())
    } else {
        let failed: Vec<&str> = report
            .songs
            .iter()
            .filter(|s| !s.ok)
            .map(|s| s.name.as_str())
            .collect();
        Err(Failure::Failed(format!("failed: {}", failed.join(", "))))
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    strudeland_project::atomic_write_bytes(path, bytes).map_err(|e| Failure::Failed(e.to_string()))
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if !e.use_stderr() => {
            let _ = e.print();
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            let text = e.to_string();
            let line = text
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("bad arguments");
            eprintln!(
                "strudeland-render: {}",
                line.strip_prefix("error: ").unwrap_or(line)
            );
            return ExitCode::from(2);
        }
    };
    let result = match &cli.command {
        Command::Render(args) => run_render(args),
        Command::Bench { args } => run_bench(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::BadInput(m)) => {
            eprintln!("strudeland-render: {m}");
            ExitCode::from(2)
        }
        Err(Failure::Failed(m)) => {
            eprintln!("strudeland-render: {m}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(extra: &[&str]) -> Result<RenderArgs, String> {
        let mut argv = vec!["strudeland-render", "render", "--song", "s.js"];
        if !extra.contains(&"--out") {
            argv.extend_from_slice(&["--out", "o.wav"]);
        }
        argv.extend_from_slice(extra);
        match Cli::try_parse_from(argv)
            .map_err(|e| e.to_string())?
            .command
        {
            Command::Render(a) => Ok(*a),
            Command::Bench { .. } => Err("bench".into()),
        }
    }

    fn length(extra: &[&str]) -> Result<LengthRequest, String> {
        length_request(&args(extra)?)
    }

    #[test]
    fn length_forms() {
        assert_eq!(length(&[]), Ok(LengthRequest::Clip { loops: None }));
        assert_eq!(
            length(&["--loops", "3"]),
            Ok(LengthRequest::Clip { loops: Some(3) })
        );
        assert_eq!(
            length(&["--from", "2", "--to", "6", "--layout", "linear"]),
            Ok(LengthRequest::Window {
                from_bar: 2,
                to_bar: 6,
                layout: Layout::Linear
            })
        );
        assert_eq!(
            length(&["--from", "0", "--to", "12", "--layout", "clip"]),
            Ok(LengthRequest::Window {
                from_bar: 0,
                to_bar: 12,
                layout: Layout::Clip
            })
        );
        assert_eq!(
            length(&["--section", "body"]),
            Ok(LengthRequest::Section {
                name: "body".into()
            })
        );
        assert_eq!(
            length(&["--length", "5"]),
            Ok(LengthRequest::Length { bars: 5 })
        );
    }

    #[test]
    fn bad_length_forms() {
        for (extra, want) in [
            (
                &["--from", "0", "--to", "2"][..],
                "--from, --to and --layout go together",
            ),
            (
                &["--layout", "clip"][..],
                "--from, --to and --layout go together",
            ),
            (
                &["--from", "0", "--to", "2", "--layout", "song"][..],
                "--layout must be clip or linear, not \"song\"",
            ),
            (
                &["--section", "a", "--length", "2"][..],
                "give at most one of --from/--to/--layout, --section and --length",
            ),
            (
                &["--from", "0", "--length", "2"][..],
                "give at most one of --from/--to/--layout, --section and --length",
            ),
            (
                &["--length", "2", "--loops", "2"][..],
                "--loops applies only to the clip (no --from, --section or --length)",
            ),
            (
                &["--section", "a", "--loops", "2"][..],
                "--loops applies only to the clip (no --from, --section or --length)",
            ),
        ] {
            assert_eq!(length(extra), Err(want.to_string()), "{extra:?}");
        }
    }

    #[test]
    fn option_defaults_and_errors() {
        let o = options(&args(&[]).unwrap()).unwrap();
        assert_eq!(o.output.loudness, Loudness::Off);
        assert_eq!(
            o.output.encoder,
            EncoderOptions::Wav {
                bit_depth: BitDepth::F32
            }
        );
        assert_eq!(o.plan_timeout, Some(std::time::Duration::from_secs(120)));
        assert_eq!(o.jobs, None);
        assert!(!o.include_events);
        let o = options(&args(&["--info", "i.json", "--jobs", "2", "--loudness", "on"]).unwrap())
            .unwrap();
        assert!(o.include_events);
        assert_eq!(o.jobs, Some(2));
        assert_eq!(o.output.loudness, Loudness::default());
        for extra in [
            &["--format", "ogg", "--out", "o.ogg"][..],
            &["--format", "ogg", "--out", "o"][..],
            &["--out", "o.OGG"][..],
        ] {
            let o = options(&args(extra).unwrap()).unwrap();
            assert_eq!(
                o.output.encoder,
                EncoderOptions::Ogg { quality: 6 },
                "{extra:?}"
            );
        }
        let o = options(&args(&["--out", "o.opus", "--loudness", "on"]).unwrap()).unwrap();
        assert_eq!(o.output.encoder, EncoderOptions::Opus { kbps: 160 });
        assert_eq!(o.output.loudness, Loudness::default());
        let o = options(&args(&["--out", "o.mp3"]).unwrap()).unwrap();
        assert_eq!(o.output.encoder, EncoderOptions::default_for(Format::Mp3));
        let o = options(&args(&["--bit-depth", "24"]).unwrap()).unwrap();
        assert_eq!(
            o.output.encoder,
            EncoderOptions::Wav {
                bit_depth: BitDepth::B24
            }
        );
        for (extra, want) in [
            (
                &["--format", "ogg"][..],
                "--format ogg disagrees with the extension of o.wav",
            ),
            (
                &["--out", "o.flac"][..],
                "cannot tell the format from the extension \"flac\"",
            ),
            (
                &["--out", "o.mp3", "--bit-depth", "16"][..],
                "--bit-depth only applies to wav",
            ),
            (&["--plan-timeout", "0"][..], "--plan-timeout must be"),
            (&["--plan-timeout", "1e9"][..], "--plan-timeout must be"),
        ] {
            let e = options(&args(extra).unwrap()).unwrap_err();
            assert!(e.starts_with(want), "{extra:?}: {e}");
        }
        // Values outside the enums are clap's errors (exit 2, like every bad flag).
        for (extra, value) in [
            (&["--format", "flac"][..], "'flac'"),
            (&["--bit-depth", "8"][..], "'8'"),
            (&["--loudness", "loud"][..], "'loud'"),
        ] {
            let e = args(extra).unwrap_err();
            assert!(
                e.contains(&format!("invalid value {value}")),
                "{extra:?}: {e}"
            );
        }
        // The flags `scripts/render/batch-render.ts` passes.
        let a = args(&["--loudness", "off", "--format", "wav", "--bit-depth", "32f"]).unwrap();
        assert_eq!(
            (a.loudness, a.format, a.bit_depth),
            (OnOff::Off, Some(Format::Wav), Some(BitDepth::F32))
        );
    }
}
