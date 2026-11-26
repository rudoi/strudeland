//! The clap definitions.

use std::path::PathBuf;
use std::time::Duration;

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use strudeland_ctl::BarRange;
use strudeland_render::{
    BitDepth, ExportPreset, Format, MAX_TIMEOUT_SEC, Mp3Mode, StemGrouping, timeout_from_secs,
};
use strudeland_transcribe::GridDivision;

#[derive(Parser, Debug, Clone, PartialEq)]
#[command(
    name = "strudeland",
    version,
    about = "Drive the running strudeland app over its local control endpoint, and render songs to audio files"
)]
pub struct Cli {
    /// Print the result as one line of JSON on stdout.
    #[arg(long, global = true)]
    pub json: bool,
    /// Seconds to wait for the app (1..=120).
    #[arg(
        long,
        global = true,
        default_value_t = 10,
        value_parser = clap::value_parser!(u64).range(1..=120)
    )]
    pub timeout: u64,
    /// The local control endpoint (overrides STRUDELAND_SOCKET).
    #[arg(long, global = true, value_name = "PATH")]
    pub socket: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum Command {
    /// Show the project, songs, tempo and playback state.
    State,
    /// Read or replace a song's code.
    Code {
        #[command(subcommand)]
        command: CodeCommand,
    },
    /// Evaluate a song.
    Eval {
        /// Song stem or path (default: the active song).
        #[arg(long)]
        song: Option<String>,
    },
    /// Evaluate a song and start playing.
    Play {
        /// Song stem or path (default: the active song).
        #[arg(long)]
        song: Option<String>,
    },
    /// Stop playback.
    Stop,
    /// Check that the app is running and ready.
    Ping,
    /// List the evaluated song's events with their beat, value and source position.
    Events {
        /// Bars to show: `N` or `N-M` (1-based, inclusive; default: the whole song up to the limit).
        #[arg(long, value_name = "N|N-M", value_parser = parse_bars)]
        bars: Option<BarRange>,
        /// Show one section of the song.
        #[arg(long, value_name = "NAME", conflicts_with = "bars")]
        section: Option<String>,
        /// Include continuations and events without a start.
        #[arg(long)]
        all: bool,
        /// Show at most this many events (1..=100000; default 2000).
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=i64::from(strudeland_ctl::EVENTS_LIMIT_MAX)))]
        limit: Option<u32>,
    },
    /// Look up Strudel function docs (offline, matching the pinned version).
    #[command(group(ArgGroup::new("what").required(true).multiple(false).args(["name", "search", "list"])))]
    Docs {
        /// Function name or synonym.
        name: Option<String>,
        /// Search names, synonyms and descriptions.
        #[arg(long, value_name = "TERM", value_parser = parse_term)]
        search: Option<String>,
        /// List every documented name.
        #[arg(long)]
        list: bool,
    },
    /// Write a PNG of the app's window content and print its path.
    Screenshot {
        /// Where to write it; relative to this directory (default: a new file in the temp dir).
        path: Option<PathBuf>,
    },
    /// Create, switch, rename or delete a song.
    Song {
        #[command(subcommand)]
        command: SongCommand,
    },
    /// Write song buffers to disk (only when the user asks).
    Save {
        /// Song stem or path (default: the active song).
        #[arg(long)]
        song: Option<String>,
        /// Save every song with unsaved edits.
        #[arg(long, conflicts_with = "song")]
        all: bool,
    },
    /// Open a project.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Render a song file to audio, offline (no app needed).
    Render(Box<RenderArgs>),
    /// Inspect the global instrument library and manage project pins.
    Instrument {
        #[command(subcommand)]
        command: InstrumentCommand,
    },
    /// Serve the instrument tools over MCP stdio.
    Mcp,
    /// List and inspect voice memos (offline, no app needed).
    Memo {
        #[command(subcommand)]
        command: MemoCommand,
    },
    /// Turn a voice memo into Strudel code (offline, no app needed).
    Transcribe(Box<TranscribeArgs>),
    /// Print or install the Claude Code skill for this CLI.
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
}

/// The flags of `render`.
#[derive(Args, Debug, Clone, PartialEq)]
pub struct RenderArgs {
    /// The song file; its contents on disk are rendered, not unsaved editor edits.
    pub song: PathBuf,
    /// Output path (default: the project's renders/ folder). With --stems, use a new directory.
    /// For intro-loop, this is the base path used to name the -intro and -loop files.
    #[arg(long, value_name = "PATH")]
    pub out: Option<PathBuf>,
    /// The mix format (default: from the --out extension, else wav).
    #[arg(long, value_name = "FORMAT", value_enum)]
    pub format: Option<Format>,
    /// Export loop assets or a one-shot with decay. Loop presets require a marked loop section
    /// and select their own bars.
    #[arg(long, value_name = "PRESET", value_enum)]
    pub preset: Option<Preset>,
    /// Export a pre-master reference mix and aligned stereo 32-bit float WAV stems.
    ///
    /// Orbit groups by orbit number. Voice groups by .label(), with a group for unlabeled parts.
    /// Instrument groups by resolved sound name. Voice and instrument include shared-effects
    /// stems by orbit. Soloing shared effects can include subtraction of direct audio when an
    /// orbit filter is active. The unity-gain sum reconstructs the pre-master reference mix.
    #[arg(long, value_name = "GROUPING", value_enum)]
    pub stems: Option<StemGrouping>,
    /// Render one section of the song, once.
    #[arg(long, value_name = "NAME", conflicts_with_all = ["length", "loops"])]
    pub section: Option<String>,
    /// Render the first N bars (needed for a song without sections).
    #[arg(
        long,
        value_name = "N",
        value_parser = clap::value_parser!(u32).range(1..=100000),
        conflicts_with = "loops"
    )]
    pub length: Option<u32>,
    /// Play the loop section N times (default: meta.loops).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=1000))]
    pub loops: Option<u32>,
    /// Target integrated loudness in LUFS (-60 to 0; default -23, EBU R128).
    #[arg(long, value_name = "DB", allow_negative_numbers = true, value_parser = parse_lufs)]
    pub lufs: Option<f64>,
    /// True-peak ceiling in dBTP (-20 to 0; default -1).
    #[arg(long, value_name = "DBTP", allow_negative_numbers = true, value_parser = parse_true_peak)]
    pub true_peak: Option<f64>,
    /// Level the output to the target loudness (default on).
    #[arg(long, value_name = "on|off", value_enum)]
    pub loudness: Option<OnOff>,
    /// wav sample format (default 24).
    #[arg(long, value_name = "BITS", value_enum)]
    pub bit_depth: Option<BitDepth>,
    /// mp3 rate control (default cbr, 320 kbps).
    #[arg(long, value_name = "MODE", value_enum)]
    pub mp3_mode: Option<Mp3Mode>,
    /// mp3 vbr quality 0-9, ogg quality -1 to 10 (default 6), opus kbps 6-510 (default 160).
    #[arg(
        long,
        value_name = "Q",
        allow_negative_numbers = true,
        value_parser = clap::value_parser!(i32).range(-1..=510)
    )]
    pub quality: Option<i32>,
    /// Worker threads (1-64; default min(cores, 4)).
    #[arg(long, value_name = "N", value_parser = parse_jobs)]
    pub jobs: Option<usize>,
    /// Seconds to wait for planning the song (default 120).
    #[arg(long, value_name = "SECS", default_value = "120", value_parser = parse_plan_timeout)]
    pub plan_timeout: Duration,
}

/// `--loudness on|off` (shared with the `strudeland-render` harness binary).
pub use strudeland_render::OnOff;

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Keep any intro and the first pass before the marked repeat region in one file.
    TaggedLoop,
    /// Export an opening with the first pass and a separate repeat file.
    IntroLoop,
    /// Start at the loop section and export the first pass and marked repeat in one file.
    FoldedLoop,
    /// Append the scheduled decay to the selected clip, section or length.
    OneShot,
}

impl From<Preset> for ExportPreset {
    fn from(preset: Preset) -> Self {
        match preset {
            Preset::TaggedLoop => Self::TaggedLoop,
            Preset::IntroLoop => Self::IntroLoop,
            Preset::FoldedLoop => Self::FoldedLoop,
            Preset::OneShot => Self::OneShot,
        }
    }
}

fn parse_finite(s: &str, what: &str) -> Result<f64, String> {
    let v: f64 = s
        .parse()
        .map_err(|_| format!("'{s}' is not a number of {what}"))?;
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("'{s}' is not a finite number of {what}"))
    }
}

fn parse_ranged(s: &str, what: &str, lo: f64, hi: f64) -> Result<f64, String> {
    let v = parse_finite(s, what)?;
    if (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(format!("{s} is outside {lo} to {hi} {what}"))
    }
}

fn parse_lufs(s: &str) -> Result<f64, String> {
    parse_ranged(s, "LUFS", -60.0, 0.0)
}

fn parse_true_peak(s: &str) -> Result<f64, String> {
    parse_ranged(s, "dBTP", -20.0, 0.0)
}

fn parse_jobs(s: &str) -> Result<usize, String> {
    match s.parse::<usize>() {
        Ok(n) if (1..=64).contains(&n) => Ok(n),
        _ => Err(format!("'{s}' is not a worker count from 1 to 64")),
    }
}

fn parse_plan_timeout(s: &str) -> Result<Duration, String> {
    let v = parse_finite(s, "seconds")?;
    timeout_from_secs(v)
        .ok_or_else(|| format!("{s} is not above 0 and at most {MAX_TIMEOUT_SEC} seconds"))
}

/// Parses `N` or `N-M` into a bar range; `N` means `N-N`.
/// A search term with something in it: an empty one would match every entry.
fn parse_term(s: &str) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() {
        Err("the search term is empty".into())
    } else {
        Ok(t.to_string())
    }
}

fn parse_bars(s: &str) -> Result<BarRange, String> {
    let bar = |part: &str| -> Result<u32, String> {
        let n: u32 = part
            .parse()
            .map_err(|_| format!("'{s}' is not a bar or a bar range like 3-5"))?;
        if n == 0 {
            return Err("bars start at 1".into());
        }
        Ok(n)
    };
    let (from, to) = match s.split_once('-') {
        Some((a, b)) => (bar(a)?, bar(b)?),
        None => {
            let n = bar(s)?;
            (n, n)
        }
    };
    if from > to {
        return Err(format!("the range '{s}' runs backwards"));
    }
    Ok(BarRange { from, to })
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum SkillCommand {
    /// Print the skill (SKILL.md) to stdout.
    Print,
    /// Write the skill where Claude Code finds it (~/.claude/skills/strudeland/SKILL.md).
    Install {
        /// Install into DIR/.claude/skills (default: the current directory) instead of ~/.claude.
        /// Write `--project=DIR`: a bare `--project` followed by a word would take it as DIR.
        #[arg(long, num_args = 0..=1, default_missing_value = ".", value_name = "DIR")]
        project: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum InstrumentCommand {
    /// List the global library's definitions and fingerprints.
    Library,
    /// Create a global library instrument from a schema-1 JSON definition.
    Create {
        id: String,
        #[arg(long, value_name = "PATH")]
        file: PathBuf,
    },
    /// List the project's pinned definitions and fingerprints.
    List {
        /// The project folder (default: the current directory's project, else the app's).
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
    },
    /// Copy a library instrument into the project for the first time.
    Pin {
        id: String,
        /// The project folder (default: the current directory's project, else the app's).
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
    },
    /// Apply a pinned instrument to a complete expression in the active editor.
    Apply {
        id: String,
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
        /// Assert the active song's project-relative path.
        #[arg(long)]
        song: Option<String>,
        /// Zero-based UTF-16 start offset.
        #[arg(long, value_name = "N")]
        from: u32,
        /// Exclusive UTF-16 end offset.
        #[arg(long, value_name = "N")]
        to: u32,
        /// File containing the entire expected editor buffer, including unsaved edits.
        #[arg(long, value_name = "PATH")]
        expect_file: PathBuf,
    },
    /// Replace a pin from the library if its fingerprint matches.
    Update {
        id: String,
        /// The current project pin fingerprint from instrument list.
        #[arg(long, value_name = "HASH")]
        expect: String,
        /// The project folder (default: the current directory's project, else the app's).
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum MemoCommand {
    /// List the project's memos.
    List {
        /// The project folder (default: the current directory's project, else the app's).
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
    },
    /// Show one memo's timing and settings.
    Show {
        /// The memo's name; a trailing `.wav` is ignored.
        #[arg(value_parser = strip_wav)]
        name: String,
        /// The project folder (default: the current directory's project, else the app's).
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
    },
}

/// The flags of `transcribe`.
#[derive(Args, Debug, Clone, PartialEq)]
pub struct TranscribeArgs {
    /// The memo's name; a trailing `.wav` is ignored.
    #[arg(value_parser = strip_wav)]
    pub name: String,
    /// The project folder (default: the current directory's project, else the app's).
    #[arg(long, value_name = "DIR")]
    pub project: Option<PathBuf>,
    /// Steps per quarter note: 4, 8, 16 (default), 8t or 16t.
    #[arg(long, value_name = "GRID", default_value = "16", value_parser = parse_grid)]
    pub grid: GridDivision,
    /// Tempo in quarter notes per minute (overrides the sidecar).
    #[arg(long, value_name = "BPM", value_parser = parse_bpm)]
    pub bpm: Option<f64>,
    /// Meter N/D (overrides the sidecar).
    #[arg(long, value_name = "N/D")]
    pub meter: Option<String>,
    /// Bar 1's downbeat in seconds from the start of the file (overrides the sidecar).
    #[arg(long, value_name = "SECONDS", value_parser = parse_nonneg)]
    pub bar1: Option<f64>,
    /// Move the grid origin later (positive) or earlier (negative), in milliseconds.
    #[arg(
        long,
        value_name = "MS",
        allow_negative_numbers = true,
        default_value_t = 0.0
    )]
    pub shift: f64,
    /// Key, ROOT:SCALE, e.g. C:major or A:minor.
    #[arg(long, value_name = "KEY")]
    pub key: Option<String>,
    /// Tuning offset in cents (positive: the singer is sharp); 0 turns it off.
    #[arg(long, value_name = "CENTS", allow_negative_numbers = true)]
    pub tuning: Option<f32>,
    /// Analyse from this many seconds into the file.
    #[arg(long, value_name = "S", requires = "end", value_parser = parse_nonneg)]
    pub start: Option<f64>,
    /// Analyse up to this many seconds into the file.
    #[arg(long, value_name = "S", requires = "start", value_parser = parse_nonneg)]
    pub end: Option<f64>,
    /// Analyse these bars of the memo, A-B (1-based, inclusive).
    #[arg(long, value_name = "A-B", conflicts_with_all = ["start", "end"], value_parser = parse_memo_bars)]
    pub bars: Option<(u32, u32)>,
    /// Add pitch frames and onset strengths to the JSON (needs --json).
    #[arg(long)]
    pub debug: bool,
}

/// A memo name without a trailing `.wav` (stems cannot contain a dot).
fn strip_wav(s: &str) -> Result<String, String> {
    Ok(s.strip_suffix(".wav").unwrap_or(s).to_string())
}

/// `--grid`: 4, 8, 16, 8t or 16t.
fn parse_grid(s: &str) -> Result<GridDivision, String> {
    GridDivision::parse(s).ok_or_else(|| format!("bad grid \"{s}\"; use 4, 8, 16, 8t or 16t"))
}

/// `--bpm`: a finite tempo from 20 to 400.
fn parse_bpm(s: &str) -> Result<f64, String> {
    parse_ranged(s, "bpm", 20.0, 400.0)
}

/// A finite number of seconds, 0 or more.
fn parse_nonneg(s: &str) -> Result<f64, String> {
    let v = parse_finite(s, "seconds")?;
    if v >= 0.0 {
        Ok(v)
    } else {
        Err(format!("{s} is below 0 seconds"))
    }
}

/// `--bars A-B`: 1-based and inclusive, with `1 <= A <= B`.
fn parse_memo_bars(s: &str) -> Result<(u32, u32), String> {
    let bad = || format!("bad bars \"{s}\"; use A-B with 1 <= A <= B, such as 2-5");
    let (a, b) = s.split_once('-').ok_or_else(bad)?;
    let a: u32 = a.parse().map_err(|_| bad())?;
    let b: u32 = b.parse().map_err(|_| bad())?;
    if a < 1 || a > b {
        return Err(bad());
    }
    Ok((a, b))
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum SongCommand {
    /// Create songs/NAME.js from the template and switch to it.
    New { name: String },
    /// Show a song in the editor.
    Switch { song: String },
    /// Rename a song.
    Rename {
        /// Song stem or path (default: the active song).
        #[arg(long)]
        song: Option<String>,
        /// The new stem.
        #[arg(value_name = "NEW")]
        name: String,
    },
    /// Delete a song's file.
    Delete {
        /// Song stem or path (default: the active song).
        #[arg(long)]
        song: Option<String>,
        /// Delete even with unsaved edits.
        #[arg(long)]
        discard: bool,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ProjectCommand {
    /// Open the project in PATH (relative to the current directory).
    Open {
        path: PathBuf,
        /// Open even with unsaved edits.
        #[arg(long)]
        discard: bool,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum CodeCommand {
    /// Print a song's code.
    Get {
        /// Song stem or path (default: the active song).
        #[arg(long)]
        song: Option<String>,
    },
    /// Replace a song's code, from a file or stdin.
    Set {
        /// Song stem or path (default: the active song).
        #[arg(long)]
        song: Option<String>,
        /// Read the code from this file; `-` or absent reads stdin.
        #[arg(long, value_name = "FILE")]
        file: Option<String>,
        /// Evaluate after setting, even if the song is not playing.
        #[arg(long, conflicts_with = "no_eval")]
        eval: bool,
        /// Do not evaluate after setting.
        #[arg(long)]
        no_eval: bool,
        /// Fail unless the song still equals this file's contents.
        #[arg(long, value_name = "FILE")]
        expect_file: Option<PathBuf>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use strudeland_ctl::{EVENTS_LIMIT_DEFAULT, EVENTS_LIMIT_MAX};

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("strudeland").chain(args.iter().copied()))
    }

    fn render(args: &[&str]) -> Result<RenderArgs, clap::Error> {
        let mut all = vec!["render", "song.js"];
        all.extend(args);
        match parse(&all)?.command {
            Command::Render(a) => Ok(*a),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn render_defaults() {
        let a = render(&[]).unwrap();
        assert_eq!(a.song, PathBuf::from("song.js"));
        assert_eq!(a.preset, None);
        assert_eq!(a.stems, None);
        assert_eq!(
            (a.out, a.format, a.section, a.length, a.loops),
            (None, None, None, None, None)
        );
        assert_eq!((a.lufs, a.true_peak, a.loudness), (None, None, None));
        assert_eq!(
            (a.bit_depth, a.mp3_mode, a.quality, a.jobs),
            (None, None, None, None)
        );
        assert_eq!(a.plan_timeout, Duration::from_secs(120));
        assert!(parse(&["render"]).is_err(), "the song is required");
    }

    #[test]
    fn render_every_flag_parses() {
        let a = render(&[
            "--out",
            "o.mp3",
            "--format",
            "mp3",
            "--length",
            "8",
            "--lufs",
            "-16",
            "--true-peak",
            "-1.5",
            "--loudness",
            "on",
            "--bit-depth",
            "32f",
            "--mp3-mode",
            "vbr",
            "--quality",
            "-1",
            "--jobs",
            "4",
            "--plan-timeout",
            "30",
        ])
        .unwrap();
        assert_eq!(a.out, Some(PathBuf::from("o.mp3")));
        assert_eq!(a.format, Some(Format::Mp3));
        assert_eq!(a.length, Some(8));
        assert_eq!((a.lufs, a.true_peak), (Some(-16.0), Some(-1.5)));
        assert_eq!(a.loudness, Some(OnOff::On));
        assert_eq!(a.bit_depth, Some(BitDepth::F32));
        assert_eq!(a.mp3_mode, Some(Mp3Mode::Vbr));
        assert_eq!(
            (a.quality, a.jobs, a.plan_timeout),
            (Some(-1), Some(4), Duration::from_secs(30))
        );
        let a = render(&["--section", "body", "--lufs=-9"]).unwrap();
        assert_eq!((a.section.as_deref(), a.lufs), (Some("body"), Some(-9.0)));
        assert_eq!(render(&["--loops", "3"]).unwrap().loops, Some(3));
    }

    #[test]
    fn render_stem_groupings_parse() {
        for (flag, grouping) in [
            ("orbit", StemGrouping::Orbit),
            ("voice", StemGrouping::Voice),
            ("instrument", StemGrouping::Instrument),
        ] {
            assert_eq!(render(&["--stems", flag]).unwrap().stems, Some(grouping));
        }
    }

    #[test]
    fn render_flag_conflicts_are_usage_errors() {
        for args in [
            &["--section", "a", "--length", "4"][..],
            &["--section", "a", "--loops", "2"],
            &["--length", "4", "--loops", "2"],
        ] {
            let e = render(args).unwrap_err();
            assert_eq!(e.kind(), ErrorKind::ArgumentConflict, "{args:?}");
            assert!(e.use_stderr(), "{args:?}");
        }
    }

    #[test]
    fn render_bad_values_are_usage_errors() {
        for args in [
            &["--format", "flac"][..],
            &["--preset", "loop"],
            &["--stems", "label"],
            &["--length", "0"],
            &["--length", "100001"],
            &["--length", "x"],
            &["--loops", "0"],
            &["--loops", "1001"],
            &["--lufs", "1"],
            &["--lufs", "-61"],
            &["--lufs", "NaN"],
            &["--lufs", "inf"],
            &["--lufs", "loud"],
            &["--true-peak", "-21"],
            &["--true-peak", "0.5"],
            &["--loudness", "maybe"],
            &["--bit-depth", "8"],
            &["--mp3-mode", "abr"],
            &["--quality", "-2"],
            &["--quality", "511"],
            &["--quality", "1.5"],
            &["--jobs", "0"],
            &["--jobs", "65"],
            &["--jobs", "-1"],
            &["--plan-timeout", "0"],
            &["--plan-timeout", "-5"],
            &["--plan-timeout", "86401"],
            &["--plan-timeout", "NaN"],
            &["--plan-timeout", "inf"],
        ] {
            let e = render(args).unwrap_err();
            assert!(e.use_stderr(), "{args:?} should exit 4");
        }
        for (flag, ok) in [
            ("--lufs", "0"),
            ("--lufs", "-60"),
            ("--true-peak", "-20"),
            ("--plan-timeout", "86400"),
            ("--quality", "510"),
            ("--jobs", "64"),
            ("--length", "100000"),
            ("--loops", "1000"),
        ] {
            assert!(render(&[flag, ok]).is_ok(), "{flag} {ok}");
        }
    }

    #[test]
    fn defaults() {
        let cli = parse(&["state"]).unwrap();
        assert_eq!(cli.command, Command::State);
        assert!(!cli.json);
        assert_eq!(cli.timeout, 10);
        assert_eq!(cli.socket, None);
    }

    #[test]
    fn timeout_range() {
        assert_eq!(parse(&["--timeout", "1", "ping"]).unwrap().timeout, 1);
        assert_eq!(parse(&["--timeout", "120", "ping"]).unwrap().timeout, 120);
        for bad in ["0", "121", "-1", "abc", "1.5"] {
            let e = parse(&["--timeout", bad, "ping"]).unwrap_err();
            assert!(e.use_stderr(), "{bad}");
        }
    }

    #[test]
    fn eval_conflicts_with_no_eval() {
        let e = parse(&["code", "set", "--eval", "--no-eval"]).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn global_flags_before_and_after() {
        let before = parse(&["--json", "--timeout", "5", "--socket", "/s", "state"]).unwrap();
        let after = parse(&["state", "--json", "--timeout", "5", "--socket", "/s"]).unwrap();
        assert_eq!(before, after);
        assert!(before.json);
        assert_eq!(before.timeout, 5);
        assert_eq!(before.socket, Some(PathBuf::from("/s")));
        let nested = parse(&["code", "get", "--json", "--timeout", "7"]).unwrap();
        assert!(nested.json);
        assert_eq!(nested.timeout, 7);
    }

    #[test]
    fn code_get_shape() {
        let cli = parse(&["code", "get", "--song", "verse"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Code {
                command: CodeCommand::Get {
                    song: Some("verse".into())
                }
            }
        );
    }

    #[test]
    fn code_set_shape() {
        let cli = parse(&[
            "code",
            "set",
            "--song",
            "main",
            "--file",
            "-",
            "--no-eval",
            "--expect-file",
            "old.js",
        ])
        .unwrap();
        assert_eq!(
            cli.command,
            Command::Code {
                command: CodeCommand::Set {
                    song: Some("main".into()),
                    file: Some("-".into()),
                    eval: false,
                    no_eval: true,
                    expect_file: Some("old.js".into()),
                }
            }
        );
        let bare = parse(&["code", "set", "--eval"]).unwrap();
        assert!(matches!(
            bare.command,
            Command::Code {
                command: CodeCommand::Set {
                    file: None,
                    eval: true,
                    ..
                }
            }
        ));
    }

    #[test]
    fn other_commands() {
        assert!(matches!(
            parse(&["eval", "--song", "a"]).unwrap().command,
            Command::Eval { song: Some(_) }
        ));
        assert!(matches!(
            parse(&["play"]).unwrap().command,
            Command::Play { song: None }
        ));
        assert_eq!(parse(&["stop"]).unwrap().command, Command::Stop);
        assert_eq!(parse(&["ping"]).unwrap().command, Command::Ping);
    }

    fn transcribe(args: &[&str]) -> Result<TranscribeArgs, clap::Error> {
        let mut all = vec!["transcribe", "take"];
        all.extend(args);
        match parse(&all)?.command {
            Command::Transcribe(a) => Ok(*a),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn transcribe_parses_every_flag() {
        let a = transcribe(&[
            "--project",
            "/p",
            "--grid",
            "8t",
            "--bpm",
            "98.5",
            "--meter",
            "6/8",
            "--bar1",
            "2.4",
            "--shift",
            "-15",
            "--key",
            "A:minor",
            "--tuning",
            "-12.5",
            "--start",
            "1",
            "--end",
            "9",
            "--debug",
        ])
        .unwrap();
        assert_eq!(
            a,
            TranscribeArgs {
                name: "take".into(),
                project: Some(PathBuf::from("/p")),
                grid: GridDivision::EighthTriplet,
                bpm: Some(98.5),
                meter: Some("6/8".into()),
                bar1: Some(2.4),
                shift: -15.0,
                key: Some("A:minor".into()),
                tuning: Some(-12.5),
                start: Some(1.0),
                end: Some(9.0),
                bars: None,
                debug: true,
            }
        );
        let d = transcribe(&["--bars", "2-5"]).unwrap();
        assert_eq!(d.bars, Some((2, 5)));
        assert_eq!(d.grid, GridDivision::Sixteenth);
        assert_eq!((d.shift, d.bpm, d.debug), (0.0, None, false));
    }

    #[test]
    fn transcribe_bars_conflicts_with_start() {
        let e = transcribe(&["--bars", "1-2", "--start", "1", "--end", "2"]).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn transcribe_start_needs_end() {
        assert_eq!(
            transcribe(&["--start", "1"]).unwrap_err().kind(),
            ErrorKind::MissingRequiredArgument
        );
        assert_eq!(
            transcribe(&["--end", "1"]).unwrap_err().kind(),
            ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn transcribe_bad_grid_rejected() {
        for g in ["3", "16T", "32", "", "8 t"] {
            assert!(transcribe(&["--grid", g]).is_err(), "{g:?}");
        }
        for g in ["4", "8", "16", "8t", "16t"] {
            assert!(transcribe(&["--grid", g]).is_ok(), "{g:?}");
        }
    }

    #[test]
    fn transcribe_strips_wav_suffix() {
        let cli = parse(&["transcribe", "memo-01.wav"]).unwrap();
        let Command::Transcribe(a) = cli.command else {
            panic!("not transcribe")
        };
        assert_eq!(a.name, "memo-01");
    }

    #[test]
    fn transcribe_bad_bpm_rejected() {
        for b in ["19.9", "400.1", "NaN", "inf", "-inf", "x"] {
            assert!(transcribe(&["--bpm", b]).is_err(), "{b:?}");
        }
        assert_eq!(transcribe(&["--bpm", "20"]).unwrap().bpm, Some(20.0));
        assert_eq!(transcribe(&["--bpm", "400"]).unwrap().bpm, Some(400.0));
        let msg = |args: &[&str]| transcribe(args).unwrap_err().to_string();
        assert!(msg(&["--bpm", "19.9"]).contains("19.9 is outside 20 to 400 bpm"));
        assert!(msg(&["--bpm", "x"]).contains("'x' is not a number of bpm"));
        assert!(msg(&["--bpm", "inf"]).contains("'inf' is not a finite number of bpm"));
    }

    #[test]
    fn transcribe_bad_bars_rejected() {
        for b in ["0-3", "4-2", "a-b", "3", "1-", "-3", "1-2-3"] {
            assert!(transcribe(&["--bars", b]).is_err(), "{b:?}");
        }
        assert_eq!(transcribe(&["--bars", "3-3"]).unwrap().bars, Some((3, 3)));
    }

    #[test]
    fn transcribe_negative_shift_and_tuning_parse() {
        let a = transcribe(&["--shift", "-30", "--tuning", "-8"]).unwrap();
        assert_eq!((a.shift, a.tuning), (-30.0, Some(-8.0)));
        let a = transcribe(&["--tuning", "0"]).unwrap();
        assert_eq!(a.tuning, Some(0.0));
        assert!(transcribe(&["--bar1", "-1"]).is_err());
        let err = transcribe(&["--bar1=-1"]).unwrap_err().to_string();
        assert!(err.contains("-1 is below 0 seconds"), "{err}");
        let err = transcribe(&["--bar1", "nan"]).unwrap_err().to_string();
        assert!(
            err.contains("'nan' is not a finite number of seconds"),
            "{err}"
        );
    }

    #[test]
    fn memo_shapes() {
        let memo = |c| Command::Memo { command: c };
        assert_eq!(
            parse(&["memo", "list"]).unwrap().command,
            memo(MemoCommand::List { project: None })
        );
        assert_eq!(
            parse(&["memo", "list", "--project", "/p"]).unwrap().command,
            memo(MemoCommand::List {
                project: Some(PathBuf::from("/p"))
            })
        );
        assert_eq!(
            parse(&["memo", "show", "memo-01.wav", "--project", "d"])
                .unwrap()
                .command,
            memo(MemoCommand::Show {
                name: "memo-01".into(),
                project: Some(PathBuf::from("d"))
            })
        );
        assert_eq!(
            parse(&["memo", "show", "a b"]).unwrap().command,
            memo(MemoCommand::Show {
                name: "a b".into(),
                project: None
            })
        );
        assert!(parse(&["memo", "show"]).is_err());
        assert!(parse(&["memo", "list", "--project"]).is_err());
        assert!(parse(&["memo"]).is_err());
    }

    #[test]
    fn song_shapes() {
        let song = |c| Command::Song { command: c };
        assert_eq!(
            parse(&["song", "new", "intro"]).unwrap().command,
            song(SongCommand::New {
                name: "intro".into()
            })
        );
        assert_eq!(
            parse(&["song", "switch", "verse"]).unwrap().command,
            song(SongCommand::Switch {
                song: "verse".into()
            })
        );
        assert_eq!(
            parse(&["song", "rename", "intro"]).unwrap().command,
            song(SongCommand::Rename {
                song: None,
                name: "intro".into()
            })
        );
        assert_eq!(
            parse(&["song", "rename", "--song", "main", "intro"])
                .unwrap()
                .command,
            song(SongCommand::Rename {
                song: Some("main".into()),
                name: "intro".into()
            })
        );
        assert_eq!(
            parse(&["song", "delete"]).unwrap().command,
            song(SongCommand::Delete {
                song: None,
                discard: false
            })
        );
        assert_eq!(
            parse(&["song", "delete", "--song", "verse", "--discard"])
                .unwrap()
                .command,
            song(SongCommand::Delete {
                song: Some("verse".into()),
                discard: true
            })
        );
    }

    #[test]
    fn save_shapes() {
        assert_eq!(
            parse(&["save"]).unwrap().command,
            Command::Save {
                song: None,
                all: false
            }
        );
        assert_eq!(
            parse(&["save", "--song", "a"]).unwrap().command,
            Command::Save {
                song: Some("a".into()),
                all: false
            }
        );
        assert_eq!(
            parse(&["save", "--all"]).unwrap().command,
            Command::Save {
                song: None,
                all: true
            }
        );
        let e = parse(&["save", "--song", "a", "--all"]).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn project_open_shape() {
        let open = |discard| Command::Project {
            command: ProjectCommand::Open {
                path: "demo".into(),
                discard,
            },
        };
        assert_eq!(
            parse(&["project", "open", "demo"]).unwrap().command,
            open(false)
        );
        assert_eq!(
            parse(&["project", "open", "demo", "--discard"])
                .unwrap()
                .command,
            open(true)
        );
    }

    #[test]
    fn song_and_project_usage_errors() {
        for args in [
            &["song"][..],
            &["song", "new"],
            &["song", "switch"],
            &["song", "rename"],
            &["song", "bogus"],
            &["project"],
            &["project", "open"],
        ] {
            let e = parse(args).unwrap_err();
            assert!(e.use_stderr(), "{args:?}");
        }
    }

    #[test]
    fn skill_shapes() {
        let skill = |c| Command::Skill { command: c };
        assert_eq!(
            parse(&["skill", "print"]).unwrap().command,
            skill(SkillCommand::Print)
        );
        let install = |project| skill(SkillCommand::Install { project });
        assert_eq!(parse(&["skill", "install"]).unwrap().command, install(None));
        assert_eq!(
            parse(&["skill", "install", "--project"]).unwrap().command,
            install(Some(".".into()))
        );
        assert_eq!(
            parse(&["skill", "install", "--project=music"])
                .unwrap()
                .command,
            install(Some("music".into()))
        );
        assert!(parse(&["skill"]).is_err());
        assert!(parse(&["skill", "print", "--project"]).is_err());
    }

    #[test]
    fn docs_shapes() {
        assert_eq!(
            parse(&["docs", "fast"]).unwrap().command,
            Command::Docs {
                name: Some("fast".into()),
                search: None,
                list: false
            }
        );
        assert_eq!(
            parse(&["docs", "--search", "x"]).unwrap().command,
            Command::Docs {
                name: None,
                search: Some("x".into()),
                list: false
            }
        );
        assert_eq!(
            parse(&["docs", "--list"]).unwrap().command,
            Command::Docs {
                name: None,
                search: None,
                list: true
            }
        );
    }

    #[test]
    fn docs_needs_exactly_one_argument() {
        for args in [
            &["docs"][..],
            &["docs", "fast", "--list"],
            &["docs", "fast", "--search", "x"],
            &["docs", "--search", "x", "--list"],
            &["docs", "--search", ""],
            &["docs", "--search", "  "],
        ] {
            assert!(parse(args).unwrap_err().use_stderr(), "{args:?}");
        }
    }

    #[test]
    fn events_bars_parsing() {
        let bars = |a: &str| match parse(&["events", "--bars", a]).unwrap().command {
            Command::Events { bars, .. } => bars,
            c => panic!("{c:?}"),
        };
        assert_eq!(bars("3"), Some(BarRange { from: 3, to: 3 }));
        assert_eq!(bars("3-5"), Some(BarRange { from: 3, to: 5 }));
        assert_eq!(bars("4-4"), Some(BarRange { from: 4, to: 4 }));
        for bad in ["0", "5-3", "a", "3-", "-3", "1-2-3", "", "0-2", "1.5"] {
            let e = parse(&["events", "--bars", bad]).unwrap_err();
            assert!(e.use_stderr(), "{bad}");
        }
    }

    #[test]
    fn events_flags() {
        assert_eq!(
            parse(&["events"]).unwrap().command,
            Command::Events {
                bars: None,
                section: None,
                all: false,
                limit: None
            }
        );
        assert_eq!(
            parse(&["events", "--section", "intro", "--all", "--limit", "5"])
                .unwrap()
                .command,
            Command::Events {
                bars: None,
                section: Some("intro".into()),
                all: true,
                limit: Some(5)
            }
        );
        let e = parse(&["events", "--bars", "1", "--section", "x"]).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::ArgumentConflict);
        let over = (EVENTS_LIMIT_MAX + 1).to_string();
        for bad in ["0", over.as_str(), "-1", "x"] {
            assert!(parse(&["events", "--limit", bad]).unwrap_err().use_stderr());
        }
        assert!(parse(&["events", "--limit", &EVENTS_LIMIT_MAX.to_string()]).is_ok());
        assert!(parse(&["events", "--song", "a"]).unwrap_err().use_stderr());
    }

    #[test]
    fn events_limit_docs_name_the_ctl_limits() {
        let command = <Cli as clap::CommandFactory>::command();
        let events = command
            .find_subcommand("events")
            .expect("an events subcommand");
        let limit = events
            .get_arguments()
            .find(|a| a.get_id() == "limit")
            .expect("an events --limit argument");
        let help = limit.get_help().expect("--limit has help").to_string();
        assert!(help.contains(&format!("1..={EVENTS_LIMIT_MAX}")), "{help}");
        assert!(
            help.contains(&format!("default {EVENTS_LIMIT_DEFAULT}")),
            "{help}"
        );

        let line = crate::skill::SKILL
            .lines()
            .find(|l| l.contains("`--limit N`"))
            .expect("SKILL.md documents --limit N");
        assert!(line.contains(&format!("1 to {EVENTS_LIMIT_MAX}")), "{line}");
        assert!(
            line.contains(&format!("default {EVENTS_LIMIT_DEFAULT}")),
            "{line}"
        );
    }

    #[test]
    fn screenshot_parses() {
        assert_eq!(
            parse(&["screenshot"]).unwrap().command,
            Command::Screenshot { path: None }
        );
        assert_eq!(
            parse(&["screenshot", "out.png"]).unwrap().command,
            Command::Screenshot {
                path: Some(PathBuf::from("out.png"))
            }
        );
        assert!(
            parse(&["screenshot", "a.png", "b.png"])
                .unwrap_err()
                .use_stderr()
        );
    }

    #[test]
    fn usage_errors() {
        assert!(parse(&[]).unwrap_err().use_stderr());
        assert!(parse(&["bogus"]).unwrap_err().use_stderr());
        assert!(parse(&["code"]).unwrap_err().use_stderr());
    }

    #[test]
    fn help_and_version_go_to_stdout() {
        assert!(!parse(&["--help"]).unwrap_err().use_stderr());
        assert!(!parse(&["--version"]).unwrap_err().use_stderr());
    }
}
