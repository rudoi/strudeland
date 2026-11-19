//! The dev binary's `bench` subcommand: renders songs through the whole pipeline at one worker
//! count, with the default loudness (-23 LUFS, -1 dBTP), and times every step and encoder.
//!
//! Per song: one [`render_audio`] over `{ kind: length, bars }` (`bars` from `--lengths`, else
//! `--bars`), then [`finish`] as Ogg Vorbis quality 6 (the headline: `wallMs`, `peakRssMb` and
//! the loudness numbers are those of render plus Ogg), then WAV 24-bit, MP3 CBR 320 and Opus
//! 160 kbps for their encode times. `peakRssMb` is read right after the Ogg finish. It is the
//! process peak (`getrusage`), so with several songs each song runs in its own child process
//! (`isolated: true` in the report); a single song runs in this process.
//!
//! The library's `limiterSamples` counts every frame whose limiter gain is below 1, and the
//! exponential release only returns to exactly 1 after about 1.8 s, so it overstates how much
//! audio the limiter really changed. The bench adds [`LimiterDepth`]: the share of frames
//! attenuated by more than 0.1, 1 and 3 dB, measured on a re-run of the final pass's leveling.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use strudeland_host::LengthRequest;
use strudeland_host::bench::{DEFAULT_BARS, max_rss_bytes, parse_bars, parse_lengths, song_name};

use crate::level::{LoudnessSummary, prepare};
use crate::{
    EncoderOptions, Format, Loudness, OutputOptions, RenderOptions, RenderedAudio, finish,
    render_audio,
};

/// `--jobs` default.
pub const DEFAULT_JOBS: usize = 4;
/// `--jobs` maximum.
pub const MAX_JOBS: usize = 64;
/// The planning timeout of every bench render (the CLI's default).
pub const PLAN_TIMEOUT: Duration = Duration::from_secs(120);
/// The encoders after the headline Ogg, in order.
pub const OTHER_FORMATS: [Format; 3] = [Format::Wav, Format::Mp3, Format::Opus];

/// The parsed `bench` command line.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchArgs {
    pub jobs: usize,
    pub bars: u32,
    pub lengths: Option<PathBuf>,
    pub out_dir: Option<PathBuf>,
    pub songs: Vec<PathBuf>,
}

/// Parses `bench`'s arguments (after the subcommand). Flags may appear anywhere; every other
/// argument is a song file.
pub fn parse_args(args: &[String]) -> Result<BenchArgs, String> {
    let mut jobs = None;
    let mut bars = None;
    let mut lengths = None;
    let mut out_dir = None;
    let mut songs = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--jobs" | "--bars" | "--lengths" | "--out-dir" => {
                let value = it.next().ok_or_else(|| format!("{arg} needs a value"))?;
                let twice = match arg.as_str() {
                    "--jobs" => jobs.replace(parse_jobs(value)?).is_some(),
                    "--bars" => bars.replace(parse_bars(value)?).is_some(),
                    "--lengths" => lengths.replace(PathBuf::from(value)).is_some(),
                    _ => out_dir.replace(PathBuf::from(value)).is_some(),
                };
                if twice {
                    return Err(format!("{arg} given twice"));
                }
            }
            flag if flag.starts_with("--") => return Err(format!("unknown argument {flag}")),
            song => songs.push(PathBuf::from(song)),
        }
    }
    if songs.is_empty() {
        return Err("no song given".into());
    }
    Ok(BenchArgs {
        jobs: jobs.unwrap_or(DEFAULT_JOBS),
        bars: bars.unwrap_or(DEFAULT_BARS),
        lengths,
        out_dir,
        songs,
    })
}

fn parse_jobs(text: &str) -> Result<usize, String> {
    match text.trim().parse::<usize>() {
        Ok(n) if (1..=MAX_JOBS).contains(&n) => Ok(n),
        _ => Err(format!(
            "--jobs must be an integer from 1 to {MAX_JOBS}, got {text}"
        )),
    }
}

/// One encoder's finish.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeReport {
    pub level_ms: f64,
    pub encode_ms: f64,
    pub bytes: u64,
    /// Encode passes (lossy: 1 to 3).
    pub encodes: u32,
    /// The decoded file's true peak (lossy only).
    pub decoded_true_peak_db: Option<f64>,
}

/// The loudness of the headline (Ogg) finish. Non-finite values are `None` (`null`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchLoudness {
    pub target_lufs: Option<f64>,
    pub ceiling_dbtp: Option<f64>,
    /// The ceiling of the limiter's final pass (lowered when the decoded file overshot).
    pub limiter_ceiling_dbtp: Option<f64>,
    pub input_lufs: Option<f64>,
    pub input_true_peak_db: Option<f64>,
    pub gain_db: Option<f64>,
    /// `input_true_peak_db + gain_db`: the true peak the limiter received.
    pub pre_limiter_true_peak_db: Option<f64>,
    /// Frames the limiter attenuated.
    pub limiter_samples: u64,
    /// `limiter_samples / frames`.
    pub limiter_fraction: Option<f64>,
    /// How deep the limiter cut, per frame.
    pub limiter_depth: Option<LimiterDepth>,
    pub output_lufs: Option<f64>,
    pub output_true_peak_db: Option<f64>,
    pub decoded_true_peak_db: Option<f64>,
    pub encodes: u32,
}

fn finite(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}

impl BenchLoudness {
    /// The bench view of a finish's loudness over `frames` frames.
    pub fn from_summary(l: &LoudnessSummary, frames: u64) -> BenchLoudness {
        BenchLoudness {
            target_lufs: l.target_lufs,
            ceiling_dbtp: l.ceiling_dbtp,
            limiter_ceiling_dbtp: l.limiter_ceiling_dbtp,
            input_lufs: finite(l.input_lufs),
            input_true_peak_db: finite(l.input_true_peak_db),
            gain_db: finite(l.gain_db),
            pre_limiter_true_peak_db: finite(l.input_true_peak_db + l.gain_db),
            limiter_samples: l.limiter_samples,
            limiter_fraction: (frames > 0).then(|| l.limiter_samples as f64 / frames as f64),
            limiter_depth: None,
            output_lufs: finite(l.output_lufs),
            output_true_peak_db: finite(l.output_true_peak_db),
            decoded_true_peak_db: l.decoded_true_peak_db.and_then(finite),
            encodes: l.encodes,
        }
    }
}

/// Frames below this level (after the static gain) are not judged by [`limiter_depth`]: the f32
/// rounding of a smaller sample hides the gain (they count as not attenuated).
pub const DEPTH_FLOOR: f32 = 1e-3;

/// How deep the limiter cut: the fraction of all frames attenuated by more than 0.1, 1 and 3 dB,
/// and the deepest cut in dB.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimiterDepth {
    pub over_0_1_db: f64,
    pub over_1_db: f64,
    pub over_3_db: f64,
    pub max_db: f64,
}

/// The limiter's cut per frame, from the buffer before the limiter (`gained`, after the static
/// gain) and after it (`limited`): in each frame the channel with the largest `|gained|` gives the
/// gain `limited / gained`; frames under [`DEPTH_FLOOR`] are not judged. `None` without frames or
/// for channels of different shapes.
pub fn limiter_depth(gained: &[Vec<f32>], limited: &[Vec<f32>]) -> Option<LimiterDepth> {
    let frames = gained.first().map_or(0, Vec::len);
    let same_shape = gained.len() == limited.len()
        && gained
            .iter()
            .zip(limited)
            .all(|(a, b)| a.len() == frames && b.len() == frames);
    if frames == 0 || !same_shape {
        return None;
    }
    let (mut n01, mut n1, mut n3, mut max_db) = (0u64, 0u64, 0u64, 0.0f64);
    for i in 0..frames {
        let loudest = gained
            .iter()
            .zip(limited)
            .filter_map(|(a, b)| Some((*a.get(i)?, *b.get(i)?)))
            .max_by(|x, y| x.0.abs().total_cmp(&y.0.abs()));
        let Some((a, b)) = loudest else { continue };
        if a.abs() < DEPTH_FLOOR {
            continue;
        }
        let cut = -20.0 * (f64::from(b) / f64::from(a)).log10();
        n01 += u64::from(cut > 0.1);
        n1 += u64::from(cut > 1.0);
        n3 += u64::from(cut > 3.0);
        if cut > max_db {
            max_db = cut;
        }
    }
    let f = |n: u64| n as f64 / frames as f64;
    Some(LimiterDepth {
        over_0_1_db: f(n01),
        over_1_db: f(n1),
        over_3_db: f(n3),
        max_db,
    })
}

/// [`limiter_depth`] of the leveling `loudness` gives `channels` with the ceiling trimmed to
/// `limiter_ceiling_dbtp` (the final pass of a finish); `None` when leveling fails.
fn depth_of(channels: &[Vec<f32>], sample_rate: u32, l: &LoudnessSummary) -> Option<LimiterDepth> {
    let (Some(ceiling), Some(limiter_ceiling), Some(target)) =
        (l.ceiling_dbtp, l.limiter_ceiling_dbtp, l.target_lufs)
    else {
        return None;
    };
    let setting = Loudness::On {
        target_lufs: target,
        true_peak_db: ceiling,
    };
    let prepared = prepare(channels, sample_rate, &setting).ok()?;
    let limited = prepared
        .level(limiter_ceiling - ceiling, &mut Vec::new())
        .ok()?;
    // A ceiling far above any sample: the limiter is a no-op, so this is the static gain.
    let gained = prepared.level(1000.0 - ceiling, &mut Vec::new()).ok()?;
    limiter_depth(&gained.channels, &limited.channels)
}

/// One song's line in the report; the measurements are `None` when the song failed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SongReport {
    pub name: String,
    pub bars: u32,
    /// Rendered over its full length (a `--lengths` entry).
    pub full_song: bool,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `--jobs`.
    pub jobs: usize,
    /// Worker threads used (`--jobs` clamped to the chunk count).
    pub workers: Option<usize>,
    pub chunks: Option<usize>,
    pub frames: Option<u64>,
    pub audio_seconds: Option<f64>,
    pub plan_ms: Option<f64>,
    /// Chunk renders and the join.
    pub render_ms: Option<f64>,
    /// The Ogg finish.
    pub level_ms: Option<f64>,
    pub encode_ms: Option<f64>,
    /// `render_audio` plus the Ogg finish, as the CLI's `render --format ogg` spends it.
    pub wall_ms: Option<f64>,
    /// `audio_seconds / wall`: times faster than real time.
    pub realtime: Option<f64>,
    /// The process's peak resident set after the Ogg finish, in MiB.
    pub peak_rss_mb: Option<f64>,
    pub loudness: Option<BenchLoudness>,
    /// Every encoder's finish, by format name.
    pub encoders: BTreeMap<String, EncodeReport>,
    pub warnings: Vec<String>,
}

impl SongReport {
    /// A song that could not be benched.
    pub fn failed(name: &str, bars: u32, full_song: bool, jobs: usize, error: String) -> Self {
        SongReport {
            name: name.to_string(),
            bars,
            full_song,
            ok: false,
            error: Some(error),
            jobs,
            workers: None,
            chunks: None,
            frames: None,
            audio_seconds: None,
            plan_ms: None,
            render_ms: None,
            level_ms: None,
            encode_ms: None,
            wall_ms: None,
            realtime: None,
            peak_rss_mb: None,
            loudness: None,
            encoders: BTreeMap::new(),
            warnings: Vec::new(),
        }
    }
}

/// The bench's JSON document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchReport {
    pub jobs: usize,
    /// `--bars` (the length of songs without a `--lengths` entry).
    pub bars: u32,
    /// Each song ran in its own process, so `peakRssMb` is that song's own peak.
    pub isolated: bool,
    pub songs: Vec<SongReport>,
}

impl BenchReport {
    /// True when every song ran (exit 0; else 1).
    pub fn all_ok(&self) -> bool {
        self.songs.iter().all(|s| s.ok)
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

const MIB: f64 = 1024.0 * 1024.0;

/// The options of a bench render: `bars` from the start, `jobs` workers, default loudness, Ogg.
pub fn render_options(bars: u32, jobs: usize) -> RenderOptions {
    RenderOptions {
        length: LengthRequest::Length { bars },
        jobs: Some(jobs),
        plan_timeout: Some(PLAN_TIMEOUT),
        include_events: false,
        output: OutputOptions {
            loudness: Loudness::default(),
            encoder: EncoderOptions::default_for(Format::Ogg),
        },
    }
}

fn encode_report(audio: &RenderedAudio, format: Format) -> Result<EncodeReport, String> {
    let out = OutputOptions {
        loudness: Loudness::default(),
        encoder: EncoderOptions::default_for(format),
    };
    let done = finish(audio, &out, &|_| {}).map_err(|e| format!("{format}: {e}"))?;
    let s = &done.summary;
    let loudness = s.loudness.as_ref();
    Ok(EncodeReport {
        level_ms: s.timings.level_ms,
        encode_ms: s.timings.encode_ms,
        bytes: s.bytes.unwrap_or(0),
        encodes: loudness.map_or(0, |l| l.encodes),
        decoded_true_peak_db: loudness.and_then(|l| l.decoded_true_peak_db),
    })
}

/// Benches one song in this process.
pub fn bench_song(name: &str, code: &str, bars: u32, full_song: bool, jobs: usize) -> SongReport {
    let failed = |e: String| SongReport::failed(name, bars, full_song, jobs, e);
    let opts = render_options(bars, jobs);
    let started = Instant::now();
    let audio = match render_audio(code, &opts, &|_| {}) {
        Ok(a) => a,
        Err(e) => return failed(e.to_string()),
    };
    let ogg = match finish(&audio, &opts.output, &|_| {}) {
        Ok(o) => o,
        Err(e) => return failed(format!("ogg: {e}")),
    };
    let wall = started.elapsed();
    let peak_rss_mb = max_rss_bytes() as f64 / MIB;
    let s = &ogg.summary;
    let mut encoders = BTreeMap::new();
    let ogg_loudness = s.loudness.as_ref();
    encoders.insert(
        Format::Ogg.ext().to_string(),
        EncodeReport {
            level_ms: s.timings.level_ms,
            encode_ms: s.timings.encode_ms,
            bytes: s.bytes.unwrap_or(0),
            encodes: ogg_loudness.map_or(0, |l| l.encodes),
            decoded_true_peak_db: ogg_loudness.and_then(|l| l.decoded_true_peak_db),
        },
    );
    for format in OTHER_FORMATS {
        match encode_report(&audio, format) {
            Ok(r) => {
                encoders.insert(format.ext().to_string(), r);
            }
            Err(e) => return failed(e),
        }
    }
    let wall_ms = ms(wall);
    SongReport {
        name: name.to_string(),
        bars,
        full_song,
        ok: true,
        error: None,
        jobs,
        workers: Some(s.jobs),
        chunks: Some(s.chunks),
        frames: Some(s.frames),
        audio_seconds: Some(s.seconds),
        plan_ms: Some(s.timings.plan_ms),
        render_ms: Some(s.timings.render_ms),
        level_ms: Some(s.timings.level_ms),
        encode_ms: Some(s.timings.encode_ms),
        wall_ms: Some(wall_ms),
        realtime: realtime(s.seconds, wall_ms),
        peak_rss_mb: Some(peak_rss_mb),
        loudness: ogg_loudness.map(|l| BenchLoudness {
            limiter_depth: depth_of(&audio.channels, audio.summary.sample_rate, l),
            ..BenchLoudness::from_summary(l, s.frames)
        }),
        encoders,
        warnings: s.warnings.clone(),
    }
}

/// `audio_seconds` per wall second; `None` for a zero wall time.
pub fn realtime(audio_seconds: f64, wall_ms: f64) -> Option<f64> {
    (wall_ms > 0.0).then(|| audio_seconds / (wall_ms / 1000.0))
}

/// The arguments of the child process that benches one song.
pub fn child_args(song: &Path, bars: u32, jobs: usize) -> Vec<String> {
    vec![
        "bench".into(),
        "--jobs".into(),
        jobs.to_string(),
        "--bars".into(),
        bars.to_string(),
        song.to_string_lossy().into_owned(),
    ]
}

/// The song report of a finished child: its stdout's only song, or a failure naming how the
/// child ended and its last stderr line.
pub fn child_report(
    name: &str,
    bars: u32,
    full_song: bool,
    jobs: usize,
    status: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
) -> SongReport {
    let failed = |e: String| SongReport::failed(name, bars, full_song, jobs, e);
    let parsed: Result<BenchReport, _> = serde_json::from_slice(stdout);
    match (status, parsed) {
        (Some(0 | 1), Ok(report)) => match <[SongReport; 1]>::try_from(report.songs) {
            Ok([mut song]) => {
                song.full_song = full_song;
                song
            }
            Err(songs) => failed(format!(
                "the bench process reported {} songs, not 1",
                songs.len()
            )),
        },
        (status, _) => {
            let text = String::from_utf8_lossy(stderr);
            let last = text
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            let how = status.map_or_else(
                || "was killed by a signal".to_string(),
                |c| format!("exited {c}"),
            );
            failed(format!("the bench process {how}: {last}"))
        }
    }
}

/// Runs the bench. `Err` is bad input (an unreadable song or lengths file, an out-dir that cannot
/// be created). With `exe` and more than one song, each song runs as `exe bench …` in its own
/// process; otherwise in this one. `progress` sees each song as it finishes.
pub fn run(
    args: &BenchArgs,
    exe: Option<&Path>,
    mut progress: impl FnMut(&SongReport),
) -> Result<BenchReport, String> {
    let lengths = match &args.lengths {
        Some(path) => parse_lengths(
            &std::fs::read_to_string(path)
                .map_err(|e| format!("reading the lengths {}: {e}", path.display()))?,
        )?,
        None => BTreeMap::new(),
    };
    let mut songs = Vec::with_capacity(args.songs.len());
    for path in &args.songs {
        let code = std::fs::read_to_string(path)
            .map_err(|e| format!("reading the song {}: {e}", path.display()))?;
        songs.push((path, song_name(path), code));
    }
    if let Some(dir) = &args.out_dir {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let isolate = exe.filter(|_| songs.len() > 1);
    let mut reports = Vec::with_capacity(songs.len());
    for (path, name, code) in &songs {
        let (bars, full_song) = match lengths.get(name) {
            Some(&bars) => (bars, true),
            None => (args.bars, false),
        };
        let report = match isolate {
            Some(exe) => match Command::new(exe)
                .args(child_args(path, bars, args.jobs))
                .output()
            {
                Ok(o) => child_report(
                    name,
                    bars,
                    full_song,
                    args.jobs,
                    o.status.code(),
                    &o.stdout,
                    &o.stderr,
                ),
                Err(e) => SongReport::failed(
                    name,
                    bars,
                    full_song,
                    args.jobs,
                    format!("cannot run {}: {e}", exe.display()),
                ),
            },
            None => bench_song(name, code, bars, full_song, args.jobs),
        };
        if let Some(dir) = &args.out_dir {
            let file = dir.join(format!("{name}.jobs{}.json", args.jobs));
            let text = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
            std::fs::write(&file, text).map_err(|e| format!("writing {}: {e}", file.display()))?;
        }
        progress(&report);
        reports.push(report);
    }
    Ok(BenchReport {
        jobs: args.jobs,
        bars: args.bars,
        isolated: isolate.is_some(),
        songs: reports,
    })
}

fn cell(value: Option<f64>, scale: f64, digits: usize) -> String {
    value.map_or_else(|| "-".into(), |v| format!("{:.*}", digits, v * scale))
}

/// The table header (stderr).
pub const TABLE_HEADER: &str = "song                      bars chunks jobs  audio s  plan s render s  level s  enc s   wall s     xRT  rss MiB  in LUFS  gain dB  limiter %  >1 dB %  max cut  out LUFS  out dBTP  ogg dBTP";

/// One table row for a song (stderr).
pub fn format_row(s: &SongReport) -> String {
    let l = s.loudness.as_ref();
    let mut row = format!(
        "{:<24} {:>5} {:>6} {:>4} {:>8} {:>7} {:>8} {:>8} {:>6} {:>8} {:>7} {:>8} {:>8} {:>8} {:>10} {:>8} {:>8} {:>9} {:>9} {:>9}",
        s.name,
        s.bars,
        s.chunks.map_or_else(|| "-".into(), |c| c.to_string()),
        s.workers.unwrap_or(s.jobs),
        cell(s.audio_seconds, 1.0, 1),
        cell(s.plan_ms, 1e-3, 2),
        cell(s.render_ms, 1e-3, 2),
        cell(s.level_ms, 1e-3, 2),
        cell(s.encode_ms, 1e-3, 2),
        cell(s.wall_ms, 1e-3, 2),
        cell(s.realtime, 1.0, 1),
        cell(s.peak_rss_mb, 1.0, 0),
        cell(l.and_then(|l| l.input_lufs), 1.0, 1),
        cell(l.and_then(|l| l.gain_db), 1.0, 1),
        cell(l.and_then(|l| l.limiter_fraction), 100.0, 3),
        cell(
            l.and_then(|l| l.limiter_depth).map(|d| d.over_1_db),
            100.0,
            3
        ),
        cell(l.and_then(|l| l.limiter_depth).map(|d| d.max_db), 1.0, 1),
        cell(l.and_then(|l| l.output_lufs), 1.0, 1),
        cell(l.and_then(|l| l.output_true_peak_db), 1.0, 2),
        cell(l.and_then(|l| l.decoded_true_peak_db), 1.0, 2),
    );
    if let Some(e) = &s.error {
        row.push_str(&format!("  FAILED {}", e.replace('\n', "\\n")));
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::level;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_flags_anywhere_with_defaults() {
        assert_eq!(
            parse_args(&args(&["a.js"])),
            Ok(BenchArgs {
                jobs: 4,
                bars: 16,
                lengths: None,
                out_dir: None,
                songs: vec!["a.js".into()],
            })
        );
        assert_eq!(
            parse_args(&args(&[
                "a.js",
                "--jobs",
                "1",
                "b.js",
                "--bars",
                "8",
                "--lengths",
                "l.json",
                "--out-dir",
                "out",
            ])),
            Ok(BenchArgs {
                jobs: 1,
                bars: 8,
                lengths: Some("l.json".into()),
                out_dir: Some("out".into()),
                songs: vec!["a.js".into(), "b.js".into()],
            })
        );
        assert_eq!(
            parse_args(&args(&["--jobs", "64", "--bars", "4096", "a.js"]))
                .map(|a| (a.jobs, a.bars)),
            Ok((64, 4096))
        );
    }

    #[test]
    fn rejects_bad_arguments() {
        for (list, want) in [
            (&[][..], "no song given"),
            (&["--jobs", "1"][..], "no song given"),
            (&["a.js", "--jobs"][..], "--jobs needs a value"),
            (&["a.js", "--out-dir"][..], "--out-dir needs a value"),
            (
                &["a.js", "--jobs", "0"][..],
                "--jobs must be an integer from 1 to 64, got 0",
            ),
            (
                &["a.js", "--jobs", "65"][..],
                "--jobs must be an integer from 1 to 64, got 65",
            ),
            (
                &["a.js", "--jobs", "-1"][..],
                "--jobs must be an integer from 1 to 64, got -1",
            ),
            (
                &["a.js", "--bars", "0"][..],
                "--bars must be an integer from 1 to 4096, got 0",
            ),
            (
                &["a.js", "--bars", "4097"][..],
                "--bars must be an integer from 1 to 4096, got 4097",
            ),
            (
                &["a.js", "--bars", "1.5"][..],
                "--bars must be an integer from 1 to 4096, got 1.5",
            ),
            (
                &["a.js", "--jobs", "1", "--jobs", "2"][..],
                "--jobs given twice",
            ),
            (
                &["a.js", "--lengths", "a", "--lengths", "b"][..],
                "--lengths given twice",
            ),
            (&["a.js", "--profile"][..], "unknown argument --profile"),
        ] {
            assert_eq!(parse_args(&args(list)), Err(want.to_string()), "{list:?}");
        }
    }

    fn summary(input_lufs: f64, gain_db: f64, touched: u64) -> LoudnessSummary {
        LoudnessSummary {
            enabled: true,
            target_lufs: Some(-14.0),
            ceiling_dbtp: Some(-1.0),
            limiter_ceiling_dbtp: Some(-1.2),
            input_lufs,
            input_true_peak_db: -3.0,
            gain_db,
            limiter_samples: touched,
            output_lufs: -14.0,
            output_true_peak_db: -1.25,
            decoded_true_peak_db: Some(-1.05),
            encodes: 2,
        }
    }

    #[test]
    fn bench_loudness_derives_the_fraction_and_the_limiter_input() {
        let l = BenchLoudness::from_summary(&summary(-20.0, 6.0, 441), 44_100);
        assert_eq!(l.limiter_fraction, Some(0.01));
        assert_eq!(l.pre_limiter_true_peak_db, Some(3.0));
        assert_eq!(l.input_lufs, Some(-20.0));
        assert_eq!(l.limiter_ceiling_dbtp, Some(-1.2));
        assert_eq!(l.decoded_true_peak_db, Some(-1.05));
        assert_eq!(l.encodes, 2);
        // Silence: the undefined loudness is None, and no frames gives no fraction.
        let mut s = summary(f64::NEG_INFINITY, 0.0, 0);
        s.input_true_peak_db = f64::NEG_INFINITY;
        s.output_lufs = f64::NEG_INFINITY;
        let l = BenchLoudness::from_summary(&s, 0);
        assert_eq!(l.input_lufs, None);
        assert_eq!(l.input_true_peak_db, None);
        assert_eq!(l.pre_limiter_true_peak_db, None);
        assert_eq!(l.output_lufs, None);
        assert_eq!(l.limiter_fraction, None);
        assert_eq!(l.gain_db, Some(0.0));
    }

    #[test]
    fn limiter_depth_counts_cuts_from_the_loudest_channel() {
        let db = |d: f64| 10f64.powf(-d / 20.0) as f32;
        // Frame 0 untouched, 1 cut 0.5 dB, 2 cut 2 dB, 3 cut 6 dB (judged on the louder right
        // channel, the left one is near silence), 4 below the floor.
        let gained = vec![
            vec![0.5, 0.5, 0.5, 1e-5, 1e-4],
            vec![0.1, 0.1, 0.1, 0.9, 1e-4],
        ];
        let limited = vec![
            vec![0.5, 0.5 * db(0.5), 0.5 * db(2.0), 1e-5, 0.0],
            vec![0.1, 0.1 * db(0.5), 0.1 * db(2.0), 0.9 * db(6.0), 0.0],
        ];
        let d = limiter_depth(&gained, &limited).unwrap();
        assert_eq!(d.over_0_1_db, 3.0 / 5.0);
        assert_eq!(d.over_1_db, 2.0 / 5.0);
        assert_eq!(d.over_3_db, 1.0 / 5.0);
        assert!((d.max_db - 6.0).abs() < 1e-5, "{}", d.max_db);
        // An untouched buffer has no cut.
        let d = limiter_depth(&gained, &gained).unwrap();
        assert_eq!(
            d,
            LimiterDepth {
                over_0_1_db: 0.0,
                over_1_db: 0.0,
                over_3_db: 0.0,
                max_db: 0.0
            }
        );
        assert_eq!(limiter_depth(&[], &[]), None);
        assert_eq!(limiter_depth(&[vec![]], &[vec![]]), None);
        assert_eq!(limiter_depth(&gained, &gained[..1]), None);
        assert_eq!(limiter_depth(&[vec![0.5]], &[vec![0.5, 0.5]]), None);
    }

    #[test]
    fn depth_of_a_loud_burst_matches_the_summary_ceiling() {
        // A -20 dBFS sine with one 10 ms full-scale burst: the static gain (positive, the burst
        // is too short to move the integrated loudness much) lifts the burst far over the
        // ceiling, so the limiter cuts deep around it and nowhere else.
        let sr = 44_100;
        let n = 3 * sr as usize;
        let ch: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(sr);
                let a = if (sr as usize..sr as usize + 441).contains(&i) {
                    1.0
                } else {
                    0.1
                };
                (a * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as f32
            })
            .collect();
        let channels = vec![ch.clone(), ch];
        let mut w = Vec::new();
        let loud = Loudness::On {
            target_lufs: -14.0,
            true_peak_db: -1.0,
        };
        let l = level(&channels, sr, &loud, 0.0, &mut w).unwrap();
        assert!(l.summary.gain_db > 3.0, "{:?}", l.summary);
        let d = depth_of(&channels, sr, &l.summary).unwrap();
        assert!(d.max_db > 3.0, "{d:?}");
        assert!(d.over_3_db > 0.0 && d.over_3_db < 0.1, "{d:?}");
        assert!(
            d.over_0_1_db < l.summary.limiter_samples as f64 / n as f64,
            "{d:?}"
        );
        let mut off = l.summary.clone();
        off.ceiling_dbtp = None;
        assert_eq!(depth_of(&channels, sr, &off), None);
    }

    #[test]
    fn realtime_is_audio_per_wall_second() {
        assert_eq!(realtime(60.0, 3000.0), Some(20.0));
        assert_eq!(realtime(60.0, 0.0), None);
    }

    #[test]
    fn child_args_name_the_song_bars_and_jobs() {
        assert_eq!(
            child_args(Path::new("/s/a.js"), 90, 1),
            args(&["bench", "--jobs", "1", "--bars", "90", "/s/a.js"])
        );
        // The child parses them back to the same request.
        let parsed = parse_args(child_args(Path::new("/s/a.js"), 90, 1).get(1..).unwrap());
        assert_eq!(parsed.map(|a| (a.jobs, a.bars)), Ok((1, 90)));
    }

    fn ok_song(name: &str) -> SongReport {
        let mut s = SongReport::failed(name, 4, false, 4, String::new());
        s.ok = true;
        s.error = None;
        s.wall_ms = Some(1.0);
        s.loudness = Some(BenchLoudness::from_summary(&summary(-20.0, 6.0, 1), 10));
        s
    }

    #[test]
    fn child_report_takes_the_only_song_and_the_parents_full_song() {
        let doc = BenchReport {
            jobs: 4,
            bars: 4,
            isolated: false,
            songs: vec![ok_song("a")],
        };
        let out = serde_json::to_vec(&doc).unwrap();
        let mut want = ok_song("a");
        want.full_song = true;
        assert_eq!(child_report("a", 4, true, 4, Some(0), &out, b""), want);
        // Exit 1 is a song that failed in the child: its report stands.
        let failed = BenchReport {
            songs: vec![SongReport::failed("a", 4, false, 4, "boom".into())],
            ..doc.clone()
        };
        let out = serde_json::to_vec(&failed).unwrap();
        assert_eq!(
            child_report("a", 4, false, 4, Some(1), &out, b"").error,
            Some("boom".into())
        );
    }

    #[test]
    fn child_report_failures_name_how_the_child_ended() {
        let r = child_report("a", 4, true, 1, None, b"", b"x\nlast line\n\n");
        assert!(!r.ok);
        assert!(r.full_song);
        assert_eq!(r.jobs, 1);
        assert_eq!(
            r.error.as_deref(),
            Some("the bench process was killed by a signal: last line")
        );
        let r = child_report("a", 4, false, 4, Some(2), b"", b"strudeland-render: bad\n");
        assert_eq!(
            r.error.as_deref(),
            Some("the bench process exited 2: strudeland-render: bad")
        );
        let r = child_report("a", 4, false, 4, Some(0), b"not json", b"");
        assert_eq!(r.error.as_deref(), Some("the bench process exited 0: "));
        let two = BenchReport {
            jobs: 4,
            bars: 4,
            isolated: false,
            songs: vec![ok_song("a"), ok_song("b")],
        };
        let r = child_report(
            "a",
            4,
            false,
            4,
            Some(0),
            &serde_json::to_vec(&two).unwrap(),
            b"",
        );
        assert_eq!(
            r.error.as_deref(),
            Some("the bench process reported 2 songs, not 1")
        );
    }

    #[test]
    fn a_report_round_trips_through_json_with_nulls() {
        let doc = BenchReport {
            jobs: 1,
            bars: 16,
            isolated: true,
            songs: vec![
                ok_song("a"),
                SongReport::failed("b", 3, true, 1, "e".into()),
            ],
        };
        let text = serde_json::to_string(&doc).unwrap();
        assert!(text.contains(r#""peakRssMb":null"#), "{text}");
        assert!(text.contains(r#""limiterFraction":0.1"#), "{text}");
        assert!(text.contains(r#""error":"e""#), "{text}");
        assert_eq!(serde_json::from_str::<BenchReport>(&text).unwrap(), doc);
        assert!(!doc.all_ok());
    }

    #[test]
    fn rows_show_dashes_for_a_failed_song() {
        let row = format_row(&SongReport::failed("b", 3, true, 1, "x\ny".into()));
        assert!(row.starts_with("b "), "{row}");
        assert!(row.ends_with("  FAILED x\\ny"), "{row}");
        let row = format_row(&ok_song("a"));
        assert!(row.contains(" 10.000 "), "limiter %: {row}");
        assert!(!row.contains("FAILED"), "{row}");
    }

    #[test]
    fn run_rejects_unreadable_inputs() {
        let a = |songs: Vec<PathBuf>, lengths: Option<PathBuf>| BenchArgs {
            jobs: 1,
            bars: 1,
            lengths,
            out_dir: None,
            songs,
        };
        let e = run(&a(vec!["/nonexistent/a.js".into()], None), None, |_| {}).unwrap_err();
        assert!(e.starts_with("reading the song /nonexistent/a.js: "), "{e}");
        let e = run(
            &a(
                vec!["/nonexistent/a.js".into()],
                Some("/nonexistent/l.json".into()),
            ),
            None,
            |_| {},
        )
        .unwrap_err();
        assert!(
            e.starts_with("reading the lengths /nonexistent/l.json: "),
            "{e}"
        );
    }
}
