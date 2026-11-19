//! `strudeland-host bench`: plans songs on fresh hosts and reports median load, query and plan
//! times, the job's size and hash, peak memory and the performance budget. See [`BUDGET`] and
//! the Performance section in the host README.
//!
//! Per song the request is `{ layout: linear, fromBar 0, toBar B, overlapBars 0, lookbackSec 0,
//! mode mix, sampleRate 44100 }`, `B` the song's `--lengths` entry (its full length: the song
//! enters budget item 1) or else `--bars`. Each of the [`RUNS`] runs uses a fresh [`Host`]: a
//! timed `plan_job` (`planMs`, with the worker's `loadMs`), then a timed `query(code, 0, B,
//! linear)` (`queryMs`). The reported numbers are the medians over the runs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::engine;
use crate::host::PlanTimings;
use crate::{Host, HostError, Layout, LookbackSec, Mode, RenderRequest, SAMPLE_RATE};

/// The performance budget, fixed before measuring (decision 25). Never loosened.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    /// Item 1: every full-song plan takes at most this long (ms).
    pub full_song_max_ms: f64,
    /// Item 1: the median full-song plan takes at most this long (ms).
    pub full_song_median_max_ms: f64,
    /// Item 2: the median per-request load (isolate, natives, bundle) takes at most this long (ms).
    pub load_median_max_ms: f64,
    /// The snapshot threshold: median load above this share of the median plan time.
    pub snapshot_load_ratio: f64,
}

/// The budget's constants.
pub const BUDGET: Budget = Budget {
    full_song_max_ms: 2000.0,
    full_song_median_max_ms: 500.0,
    load_median_max_ms: 100.0,
    snapshot_load_ratio: 0.25,
};

/// Runs per song, each on a fresh [`Host`].
pub const RUNS: usize = 3;

/// The default window when a song has no `--lengths` entry.
pub const DEFAULT_BARS: u32 = 16;

/// The widest window `bench` accepts: the bundle's widest `query` (`MAX_QUERY_CYCLES`).
pub const MAX_BENCH_BARS: u32 = 4096;

/// The parsed `bench` command line.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchArgs {
    pub bars: u32,
    pub lengths: Option<PathBuf>,
    pub profile: bool,
    pub out_dir: Option<PathBuf>,
    pub songs: Vec<PathBuf>,
}

/// Parses `bench`'s arguments (after the subcommand). Flags may appear anywhere; every other
/// argument is a song file.
pub fn parse_args(args: &[String]) -> Result<BenchArgs, String> {
    let mut bars = None;
    let mut lengths = None;
    let mut profile = false;
    let mut out_dir = None;
    let mut songs = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--profile" => {
                if profile {
                    return Err("--profile given twice".into());
                }
                profile = true;
            }
            "--bars" | "--lengths" | "--out-dir" => {
                let value = it.next().ok_or_else(|| format!("{arg} needs a value"))?;
                let twice = match arg.as_str() {
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
        bars: bars.unwrap_or(DEFAULT_BARS),
        lengths,
        profile,
        out_dir,
        songs,
    })
}

/// Parses a `--bars` value: a whole number of bars from 1 to [`MAX_BENCH_BARS`].
pub fn parse_bars(text: &str) -> Result<u32, String> {
    match text.trim().parse::<u32>() {
        Ok(n) if (1..=MAX_BENCH_BARS).contains(&n) => Ok(n),
        _ => Err(format!(
            "--bars must be an integer from 1 to {MAX_BENCH_BARS}, got {text}"
        )),
    }
}

/// Parses a lengths file: a JSON object `{ "<song file stem>": bars }`, each a whole number of
/// bars from 1 to [`MAX_BENCH_BARS`].
pub fn parse_lengths(text: &str) -> Result<BTreeMap<String, u32>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("invalid lengths: {e}"))?;
    let object = value
        .as_object()
        .ok_or("invalid lengths: not a JSON object")?;
    let mut lengths = BTreeMap::new();
    for (name, bars) in object {
        let bars = bars
            .as_u64()
            .and_then(|b| u32::try_from(b).ok())
            .filter(|b| (1..=MAX_BENCH_BARS).contains(b))
            .ok_or_else(|| {
                format!("invalid lengths: {name} must be an integer from 1 to {MAX_BENCH_BARS}")
            })?;
        lengths.insert(name.clone(), bars);
    }
    Ok(lengths)
}

/// The median (the mean of the middle two for an even count); `None` for no values.
pub fn median(values: &[f64]) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let upper = sorted.get(n / 2).copied()?;
    if n % 2 == 1 {
        return Some(upper);
    }
    sorted.get(n / 2 - 1).map(|lower| (lower + upper) / 2.0)
}

/// `ru_maxrss` in bytes: macOS reports bytes, Linux KiB; a negative value is 0.
pub fn rss_bytes(raw: libc::c_long, macos: bool) -> u64 {
    let raw = u64::try_from(raw).unwrap_or(0);
    if macos { raw } else { raw.saturating_mul(1024) }
}

/// This process's peak resident set size in bytes (0 when unavailable).
pub fn max_rss_bytes() -> u64 {
    engine::max_rss_raw()
        .map(|raw| rss_bytes(raw, cfg!(target_os = "macos")))
        .unwrap_or(0)
}

/// One planned song's medians, the input of [`check_budget`].
#[derive(Debug, Clone, PartialEq)]
pub struct SongTiming {
    pub name: String,
    /// Planned over its full length (a `--lengths` entry): counts for budget item 1.
    pub full_song: bool,
    pub plan_ms: f64,
    pub load_ms: f64,
}

/// The slowest full-song plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Longest {
    pub name: String,
    pub plan_ms: f64,
}

/// The budget's verdict over one bench run. An item with no songs to judge is `null` and does
/// not count towards `passed`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetReport {
    pub limits: Budget,
    /// Songs planned over their full length.
    pub full_songs: usize,
    pub longest: Option<Longest>,
    /// Full songs over `full_song_max_ms`.
    pub over_full_song_max: Vec<String>,
    pub median_full_song_ms: Option<f64>,
    /// Over every planned song.
    pub median_load_ms: Option<f64>,
    /// Over every planned song.
    pub median_plan_ms: Option<f64>,
    /// Item 1: every full song within `full_song_max_ms`.
    pub full_song_max_passed: Option<bool>,
    /// Item 1: the full-song median within `full_song_median_max_ms`.
    pub full_song_median_passed: Option<bool>,
    /// Item 2: the load median within `load_median_max_ms`.
    pub load_median_passed: Option<bool>,
    /// The snapshot threshold: median load above `load_median_max_ms` or above
    /// `snapshot_load_ratio` of the median plan time.
    pub snapshot_triggered: bool,
    /// Every judged item passed.
    pub passed: bool,
}

/// Judges the planned songs' medians against [`BUDGET`].
pub fn check_budget(songs: &[SongTiming]) -> BudgetReport {
    let full: Vec<&SongTiming> = songs.iter().filter(|s| s.full_song).collect();
    let full_plan: Vec<f64> = full.iter().map(|s| s.plan_ms).collect();
    let longest = full
        .iter()
        .max_by(|a, b| a.plan_ms.total_cmp(&b.plan_ms))
        .map(|s| Longest {
            name: s.name.clone(),
            plan_ms: s.plan_ms,
        });
    let over_full_song_max: Vec<String> = full
        .iter()
        .filter(|s| s.plan_ms > BUDGET.full_song_max_ms)
        .map(|s| s.name.clone())
        .collect();
    let median_full_song_ms = median(&full_plan);
    let loads: Vec<f64> = songs.iter().map(|s| s.load_ms).collect();
    let plans: Vec<f64> = songs.iter().map(|s| s.plan_ms).collect();
    let median_load_ms = median(&loads);
    let median_plan_ms = median(&plans);
    let full_song_max_passed = (!full.is_empty()).then_some(over_full_song_max.is_empty());
    let full_song_median_passed = median_full_song_ms.map(|m| m <= BUDGET.full_song_median_max_ms);
    let load_median_passed = median_load_ms.map(|m| m <= BUDGET.load_median_max_ms);
    let snapshot_triggered = match (median_load_ms, median_plan_ms) {
        (Some(load), Some(plan)) => {
            load > BUDGET.load_median_max_ms || load > BUDGET.snapshot_load_ratio * plan
        }
        _ => false,
    };
    let passed = [
        full_song_max_passed,
        full_song_median_passed,
        load_median_passed,
    ]
    .iter()
    .all(|item| *item != Some(false));
    BudgetReport {
        limits: BUDGET,
        full_songs: full.len(),
        longest,
        over_full_song_max,
        median_full_song_ms,
        median_load_ms,
        median_plan_ms,
        full_song_max_passed,
        full_song_median_passed,
        load_median_passed,
        snapshot_triggered,
        passed,
    }
}

/// Where a plan's time went (`--profile`), medians over the runs, in ms.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// New isolate and context, natives, bundle evaluation.
    pub load_ms: f64,
    /// The `evaluating` mark to the `querying` mark: `meta`, transpile and evaluate.
    pub evaluate_ms: f64,
    /// The `querying` mark to the result slot being set: window queries, selection, job plan
    /// and the job's JSON text.
    pub query_ms: f64,
    /// The result read-out.
    pub json_ms: f64,
}

/// One song's line in the report. Timings are medians in ms; `null` when the song failed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongReport {
    pub name: String,
    pub bars: u32,
    /// Planned over its full length (`--lengths`).
    pub full_song: bool,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub load_ms: Option<f64>,
    pub query_ms: Option<f64>,
    pub plan_ms: Option<f64>,
    pub job_bytes: Option<usize>,
    pub job_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
}

/// The bench's JSON document.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchReport {
    pub runtime: &'static str,
    /// The embedded V8's version.
    pub version: String,
    /// `--bars` (the window of songs without a `--lengths` entry).
    pub bars: u32,
    pub songs: Vec<SongReport>,
    pub max_rss_bytes: u64,
    pub budget: BudgetReport,
}

impl BenchReport {
    /// True when every song planned (exit 0; else 1).
    pub fn all_ok(&self) -> bool {
        self.songs.iter().all(|s| s.ok)
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// The linear mix request over `[0, bars)` with no lookback.
pub fn request(bars: u32) -> RenderRequest {
    RenderRequest {
        layout: Layout::Linear,
        from_bar: 0,
        to_bar: bars,
        overlap_bars: 0,
        lookback_sec: LookbackSec::Seconds(0.0),
        mode: Mode::Mix,
        sample_rate: SAMPLE_RATE,
    }
}

/// The hex sha256 of `text`.
pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The profile of one run from the worker's timings; `None` when a mark is missing.
fn run_profile(timings: &PlanTimings) -> Option<Profile> {
    let at = |label: &str| {
        timings
            .marks
            .iter()
            .find(|(l, _)| l == label)
            .map(|(_, t)| *t)
    };
    let evaluating = at("evaluating")?;
    let querying = at("querying")?;
    let settled = at("settled")?;
    Some(Profile {
        load_ms: ms(timings.load),
        evaluate_ms: ms(querying.saturating_duration_since(evaluating)),
        query_ms: ms(settled.saturating_duration_since(querying)),
        json_ms: ms(timings.json),
    })
}

struct Run {
    job: String,
    load_ms: f64,
    plan_ms: f64,
    query_ms: f64,
    profile: Option<Profile>,
}

fn one_run(code: &str, bars: u32, profile: bool) -> Result<Run, String> {
    let fail = |e: HostError| e.to_string();
    let host = Host::new().map_err(fail)?;
    let start = Instant::now();
    let (job, timings) = host
        .plan_job_timed(code, &request(bars), profile, None)
        .map_err(fail)?;
    let plan_ms = ms(start.elapsed());
    let start = Instant::now();
    host.query(code, 0.0, f64::from(bars), Layout::Linear)
        .map_err(fail)?;
    let query_ms = ms(start.elapsed());
    let profile = if profile {
        Some(run_profile(&timings).ok_or("the bundle's phase marks are missing")?)
    } else {
        None
    };
    Ok(Run {
        job: job.json,
        load_ms: ms(timings.load),
        plan_ms,
        query_ms,
        profile,
    })
}

fn median_of(runs: &[Run], f: impl Fn(&Run) -> f64) -> Option<f64> {
    median(&runs.iter().map(f).collect::<Vec<_>>())
}

/// Benches one song: [`RUNS`] runs on fresh hosts. A failure (or a job that differs between
/// runs) is reported in the song's `error`. `out_dir` receives the job as `<name>.job.json`.
pub fn bench_song(
    name: &str,
    code: &str,
    bars: u32,
    full_song: bool,
    profile: bool,
    out_dir: Option<&Path>,
) -> SongReport {
    let failed = |error: String| SongReport {
        name: name.to_string(),
        bars,
        full_song,
        ok: false,
        error: Some(error),
        load_ms: None,
        query_ms: None,
        plan_ms: None,
        job_bytes: None,
        job_sha256: None,
        profile: None,
    };
    let mut runs = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        match one_run(code, bars, profile) {
            Ok(run) => runs.push(run),
            Err(e) => return failed(e),
        }
    }
    let Some(first) = runs.first() else {
        return failed("no run".into());
    };
    let sha = sha256_hex(&first.job);
    if runs.iter().any(|r| r.job != first.job) {
        return failed("the job differs between runs".into());
    }
    if let Some(dir) = out_dir {
        let path = dir.join(format!("{name}.job.json"));
        if let Err(e) = std::fs::write(&path, &first.job) {
            return failed(format!("writing {}: {e}", path.display()));
        }
    }
    let profile = if profile {
        let p = |f: fn(&Profile) -> f64| {
            median(
                &runs
                    .iter()
                    .filter_map(|r| r.profile.as_ref().map(f))
                    .collect::<Vec<_>>(),
            )
            .unwrap_or(f64::NAN)
        };
        Some(Profile {
            load_ms: p(|x| x.load_ms),
            evaluate_ms: p(|x| x.evaluate_ms),
            query_ms: p(|x| x.query_ms),
            json_ms: p(|x| x.json_ms),
        })
    } else {
        None
    };
    SongReport {
        name: name.to_string(),
        bars,
        full_song,
        ok: true,
        error: None,
        load_ms: median_of(&runs, |r| r.load_ms),
        query_ms: median_of(&runs, |r| r.query_ms),
        plan_ms: median_of(&runs, |r| r.plan_ms),
        job_bytes: Some(first.job.len()),
        job_sha256: Some(sha),
        profile,
    }
}

/// A song's name: its file stem.
pub fn song_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Runs the bench. `Err` is bad input (an unreadable song or lengths file, an out-dir that cannot
/// be created); a song that fails to plan is reported in the result. `progress` sees each song as
/// it finishes.
pub fn run(args: &BenchArgs, mut progress: impl FnMut(&SongReport)) -> Result<BenchReport, String> {
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
        songs.push((song_name(path), code));
    }
    if let Some(dir) = &args.out_dir {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let mut reports = Vec::with_capacity(songs.len());
    for (name, code) in &songs {
        let (bars, full_song) = match lengths.get(name) {
            Some(&bars) => (bars, true),
            None => (args.bars, false),
        };
        let report = bench_song(
            name,
            code,
            bars,
            full_song,
            args.profile,
            args.out_dir.as_deref(),
        );
        progress(&report);
        reports.push(report);
    }
    let timings: Vec<SongTiming> = reports
        .iter()
        .filter_map(|s| {
            Some(SongTiming {
                name: s.name.clone(),
                full_song: s.full_song,
                plan_ms: s.plan_ms?,
                load_ms: s.load_ms?,
            })
        })
        .collect();
    Ok(BenchReport {
        runtime: "v8",
        version: engine::v8_version().to_string(),
        bars: args.bars,
        budget: check_budget(&timings),
        songs: reports,
        max_rss_bytes: max_rss_bytes(),
    })
}

fn cell(value: Option<f64>) -> String {
    value.map_or_else(|| "-".into(), |v| format!("{v:.1}"))
}

/// One table row for a song (stderr).
pub fn format_row(s: &SongReport) -> String {
    let mut row = format!(
        "{:<24} {:>5} {:>4} {:>9} {:>9} {:>9} {:>10}",
        s.name,
        s.bars,
        if s.full_song { "full" } else { "" },
        cell(s.load_ms),
        cell(s.query_ms),
        cell(s.plan_ms),
        s.job_bytes.map_or_else(|| "-".into(), |b| b.to_string()),
    );
    if let Some(p) = &s.profile {
        row.push_str(&format!(
            "  load {:.1} evaluate {:.1} query {:.1} json {:.1}",
            p.load_ms, p.evaluate_ms, p.query_ms, p.json_ms
        ));
    }
    if let Some(e) = &s.error {
        row.push_str(&format!("  FAILED {}", e.replace('\n', "\\n")));
    }
    row
}

/// The table header (stderr).
pub const TABLE_HEADER: &str =
    "song                      bars      load ms  query ms   plan ms  job bytes";

fn verdict(item: Option<bool>) -> &'static str {
    match item {
        Some(true) => "pass",
        Some(false) => "FAIL",
        None => "not judged",
    }
}

/// The budget summary (stderr).
pub fn format_budget(b: &BudgetReport) -> String {
    let longest = b
        .longest
        .as_ref()
        .map_or_else(|| "-".into(), |l| format!("{} {:.1} ms", l.name, l.plan_ms));
    format!(
        "budget: every full song <= {:.0} ms: {} (longest {longest}{}); full-song median <= {:.0} ms: {} ({}); median load <= {:.0} ms: {} ({}); median plan {}; snapshot {}; {}",
        b.limits.full_song_max_ms,
        verdict(b.full_song_max_passed),
        if b.over_full_song_max.is_empty() {
            String::new()
        } else {
            format!("; over: {}", b.over_full_song_max.join(", "))
        },
        b.limits.full_song_median_max_ms,
        verdict(b.full_song_median_passed),
        cell(b.median_full_song_ms),
        b.limits.load_median_max_ms,
        verdict(b.load_median_passed),
        cell(b.median_load_ms),
        cell(b.median_plan_ms),
        if b.snapshot_triggered {
            "triggered"
        } else {
            "not triggered"
        },
        if b.passed { "PASSED" } else { "MISSED" },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn song(name: &str, full_song: bool, plan_ms: f64, load_ms: f64) -> SongTiming {
        SongTiming {
            name: name.into(),
            full_song,
            plan_ms,
            load_ms,
        }
    }

    #[test]
    fn the_budget_constants_are_the_fixed_ones() {
        assert_eq!(BUDGET.full_song_max_ms, 2000.0);
        assert_eq!(BUDGET.full_song_median_max_ms, 500.0);
        assert_eq!(BUDGET.load_median_max_ms, 100.0);
        assert_eq!(BUDGET.snapshot_load_ratio, 0.25);
    }

    #[test]
    fn median_of_odd_even_and_empty_counts() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[7.0]), Some(7.0));
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
        assert_eq!(median(&[5.0, 5.0]), Some(5.0));
        assert_eq!(median(&[9.0, -1.0, 0.5, 100.0, 2.0]), Some(2.0));
    }

    #[test]
    fn rss_is_bytes_on_macos_and_kib_on_linux() {
        assert_eq!(rss_bytes(123_456, true), 123_456);
        assert_eq!(rss_bytes(123_456, false), 123_456 * 1024);
        assert_eq!(rss_bytes(-1, true), 0);
        assert_eq!(rss_bytes(-1, false), 0);
        assert_eq!(rss_bytes(libc::c_long::MAX, false), u64::MAX);
        assert!(max_rss_bytes() > 0);
    }

    #[test]
    fn a_budget_with_everything_within_passes() {
        let r = check_budget(&[
            song("a", true, 300.0, 20.0),
            song("b", true, 1900.0, 30.0),
            song("c", true, 400.0, 25.0),
            song("spot", false, 5000.0, 30.0),
        ]);
        assert_eq!(r.full_songs, 3);
        assert_eq!(
            r.longest,
            Some(Longest {
                name: "b".into(),
                plan_ms: 1900.0
            })
        );
        assert_eq!(r.median_full_song_ms, Some(400.0));
        assert_eq!(r.median_load_ms, Some(27.5));
        assert_eq!(r.median_plan_ms, Some(1150.0));
        assert_eq!(r.full_song_max_passed, Some(true));
        assert_eq!(r.full_song_median_passed, Some(true));
        assert_eq!(r.load_median_passed, Some(true));
        assert!(r.over_full_song_max.is_empty());
        assert!(!r.snapshot_triggered);
        assert!(r.passed);
    }

    #[test]
    fn one_song_over_two_seconds_fails_item_one() {
        let r = check_budget(&[
            song("a", true, 300.0, 20.0),
            song("slow", true, 2000.5, 20.0),
            song("c", true, 200.0, 20.0),
        ]);
        assert_eq!(r.over_full_song_max, vec!["slow".to_string()]);
        assert_eq!(r.full_song_max_passed, Some(false));
        assert_eq!(r.full_song_median_passed, Some(true));
        assert!(!r.passed);
        // Exactly at the limit is within.
        let r = check_budget(&[song("edge", true, 2000.0, 20.0)]);
        assert_eq!(r.full_song_max_passed, Some(true));
    }

    #[test]
    fn a_median_over_half_a_second_fails_item_one() {
        let r = check_budget(&[
            song("a", true, 600.0, 20.0),
            song("b", true, 700.0, 20.0),
            song("c", true, 100.0, 20.0),
        ]);
        assert_eq!(r.median_full_song_ms, Some(600.0));
        assert_eq!(r.full_song_max_passed, Some(true));
        assert_eq!(r.full_song_median_passed, Some(false));
        assert!(!r.passed);
    }

    #[test]
    fn a_load_median_over_100_ms_fails_item_two_and_triggers_the_snapshot() {
        let r = check_budget(&[
            song("a", true, 1000.0, 120.0),
            song("b", true, 400.0, 110.0),
            song("c", true, 450.0, 90.0),
        ]);
        assert_eq!(r.median_load_ms, Some(110.0));
        assert_eq!(r.load_median_passed, Some(false));
        assert!(r.snapshot_triggered);
        assert!(!r.passed);
    }

    #[test]
    fn a_load_over_a_quarter_of_the_plan_triggers_the_snapshot_only() {
        let r = check_budget(&[
            song("a", true, 200.0, 60.0),
            song("b", true, 236.0, 61.0),
            song("c", true, 300.0, 59.0),
        ]);
        // 60 > 0.25 * 236.
        assert_eq!(r.median_load_ms, Some(60.0));
        assert!(r.snapshot_triggered);
        assert_eq!(r.load_median_passed, Some(true));
        assert!(r.passed);
        // Exactly a quarter does not trigger.
        let r = check_budget(&[song("a", true, 240.0, 60.0)]);
        assert!(!r.snapshot_triggered);
    }

    #[test]
    fn items_without_songs_are_not_judged() {
        let r = check_budget(&[]);
        assert_eq!(r.full_song_max_passed, None);
        assert_eq!(r.full_song_median_passed, None);
        assert_eq!(r.load_median_passed, None);
        assert_eq!(r.longest, None);
        assert!(!r.snapshot_triggered);
        assert!(r.passed);
        let r = check_budget(&[song("spot", false, 9000.0, 20.0)]);
        assert_eq!(r.full_songs, 0);
        assert_eq!(r.full_song_max_passed, None);
        assert_eq!(r.load_median_passed, Some(true));
        assert!(r.passed);
    }

    #[test]
    fn the_budget_report_serialises_in_camel_case() {
        let r = check_budget(&[song("a", true, 300.0, 20.0)]);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["limits"]["fullSongMaxMs"], 2000.0);
        assert_eq!(v["limits"]["snapshotLoadRatio"], 0.25);
        assert_eq!(v["longest"]["planMs"], 300.0);
        assert_eq!(v["passed"], true);
        assert_eq!(v["snapshotTriggered"], false);
    }

    #[test]
    fn bench_arguments_parse() {
        let a = parse_args(&args(&["a.js", "--bars", "8", "b.js", "--profile"])).unwrap();
        assert_eq!(a.bars, 8);
        assert!(a.profile);
        assert_eq!(a.songs, vec![PathBuf::from("a.js"), PathBuf::from("b.js")]);
        assert_eq!(a.lengths, None);
        assert_eq!(a.out_dir, None);
        let a = parse_args(&args(&["--lengths", "l.json", "--out-dir", "o", "s.js"])).unwrap();
        assert_eq!(a.bars, DEFAULT_BARS);
        assert!(!a.profile);
        assert_eq!(a.lengths, Some(PathBuf::from("l.json")));
        assert_eq!(a.out_dir, Some(PathBuf::from("o")));
    }

    #[test]
    fn bad_bench_arguments_are_rejected() {
        for (list, message) in [
            (&[][..], "no song given"),
            (&["--bars", "8"][..], "no song given"),
            (&["s.js", "--bars"][..], "--bars needs a value"),
            (&["s.js", "--bars", "0"][..], "--bars must be"),
            (&["s.js", "--bars", "4097"][..], "--bars must be"),
            (&["s.js", "--bars", "1.5"][..], "--bars must be"),
            (&["s.js", "--bars", "x"][..], "--bars must be"),
            (&["s.js", "--bars", "2", "--bars", "3"][..], "given twice"),
            (&["s.js", "--profile", "--profile"][..], "given twice"),
            (
                &["s.js", "--runtime", "v8"][..],
                "unknown argument --runtime",
            ),
            (&["s.js", "--lengths"][..], "--lengths needs a value"),
        ] {
            let e = parse_args(&args(list)).unwrap_err();
            assert!(e.contains(message), "{list:?}: {e}");
        }
    }

    #[test]
    fn lengths_parse_and_reject_bad_entries() {
        let l = parse_lengths(r#"{"pulse": 120, "short-intro": 1}"#).unwrap();
        assert_eq!(l.get("pulse"), Some(&120));
        assert_eq!(l.get("short-intro"), Some(&1));
        assert!(parse_lengths("{}").unwrap().is_empty());
        for bad in [
            "[]",
            "3",
            "{",
            r#"{"a": 0}"#,
            r#"{"a": 4097}"#,
            r#"{"a": 1.5}"#,
            r#"{"a": -1}"#,
            r#"{"a": "8"}"#,
        ] {
            assert!(parse_lengths(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_request_is_the_linear_full_window() {
        let r = request(42);
        assert_eq!(
            r.to_json().unwrap(),
            r#"{"layout":"linear","fromBar":0,"toBar":42,"overlapBars":0,"lookbackSec":0.0,"mode":"mix","sampleRate":44100}"#
        );
    }

    #[test]
    fn sha256_is_lowercase_hex() {
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_profile_needs_all_three_marks() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let timings = PlanTimings {
            load: Duration::from_millis(30),
            marks: vec![
                ("evaluating".into(), at(1)),
                ("querying".into(), at(11)),
                ("settled".into(), at(111)),
            ],
            json: Duration::from_millis(2),
        };
        assert_eq!(
            run_profile(&timings),
            Some(Profile {
                load_ms: 30.0,
                evaluate_ms: 10.0,
                query_ms: 100.0,
                json_ms: 2.0,
            })
        );
        let mut missing = timings.clone();
        missing.marks.pop();
        assert_eq!(run_profile(&missing), None);
    }

    #[test]
    fn the_song_name_is_the_file_stem() {
        assert_eq!(song_name(Path::new("songs/pulse.js")), "pulse");
        assert_eq!(song_name(Path::new("x")), "x");
    }
}
