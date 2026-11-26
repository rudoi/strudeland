//! The finishing characterization: every finishing path, format and loudness setting over seeded
//! synthetic PCM, pinned in `tests/golden/finish.json` (finished bytes as SHA-256, summaries with
//! timings zeroed, warnings, progress and loop points). It enters only through [`crate::finish`]
//! and [`finish_export`], the two seams every later refactor keeps.
//!
//! `STRUDELAND_WRITE_FINISH_GOLDEN=1 cargo test -p strudeland-render characterization` writes the
//! golden, on macOS aarch64 only (the reference platform, where it is also judged; the lossy
//! encoders' float paths differ by platform and architecture). Refresh it only after an encoder
//! crate bump.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::join::window_frames;
use crate::loop_export::finish_export;
use crate::{
    BitDepth, EncoderOptions, ExportFile, ExportPreset, ExportRole, LoopAudio, Loudness, Mp3Mode,
    OutputOptions, PipelineAudio, Progress, RenderError, Summary, Timings, testing,
};

const SR: u32 = 44_100;
/// The loop fixtures' tempo: 0.5 s per bar, so the 4-bar window is 2 s.
const LOOP_CPS: f64 = 2.0;
const LOOP_BARS: u32 = 4;
const WRITE_VAR: &str = "STRUDELAND_WRITE_FINISH_GOLDEN";
const REFERENCE: bool = cfg!(all(target_os = "macos", target_arch = "aarch64"));

fn secs(s: f64) -> usize {
    (s * f64::from(SR)).round() as usize
}

/// LCG noise, left channel then right, scaled to `amp` (the `encode` tests' generator).
fn lcg_noise(seed: u32, frames: usize, amp: f32) -> Vec<Vec<f32>> {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (f64::from(state) / f64::from(u32::MAX) * 2.0 - 1.0) as f32 * amp
    };
    let l: Vec<f32> = (0..frames).map(|_| next()).collect();
    let r: Vec<f32> = (0..frames).map(|_| next()).collect();
    vec![l, r]
}

fn sine(hz: f64, n: usize) -> f64 {
    (2.0 * std::f64::consts::PI * hz * n as f64 / f64::from(SR)).sin()
}

/// 2 s stereo: noise at 0.05 under 220 Hz (left) and 330 Hz (right) at 0.3.
fn bed() -> Vec<Vec<f32>> {
    let mut channels = lcg_noise(0x1234_5678, secs(2.0), 0.05);
    for (channel, hz) in channels.iter_mut().zip([220.0, 330.0]) {
        for (n, sample) in channel.iter_mut().enumerate() {
            *sample += (0.3 * sine(hz, n)) as f32;
        }
    }
    channels
}

/// `bed()` times 3: over 0 dBTP.
fn hot() -> Vec<Vec<f32>> {
    bed()
        .into_iter()
        .map(|ch| ch.into_iter().map(|x| x * 3.0).collect())
        .collect()
}

fn silence() -> Vec<Vec<f32>> {
    vec![vec![0.0; secs(1.0)]; 2]
}

/// Dense noise at 0.5 for 3 s (the `tests/encode.rs` `noise(3)` recipe): lossy encodes overshoot.
fn dense() -> Vec<Vec<f32>> {
    lcg_noise(0x5eed_0001, secs(3.0), 0.5)
}

/// `bed()` with a NaN in the first second.
fn nan_bed() -> Vec<Vec<f32>> {
    let mut channels = bed();
    channels[1][1000] = f32::NAN;
    channels
}

/// One-shot input: at most 1.5 s of `input` as the music, then a 0.3 s decaying sine tail (silent
/// for silent input) and 0.2 s of exact zeros, which the trim drops.
fn one_shot(input: &[Vec<f32>]) -> crate::RenderedAudio {
    let music = input[0].len().min(secs(1.5));
    let sounding = input.iter().any(|ch| ch.iter().any(|&x| x != 0.0));
    let amp = if sounding { 0.3 } else { 0.0 };
    let channels: Vec<Vec<f32>> = input
        .iter()
        .map(|ch| {
            let mut out = ch[..music].to_vec();
            out.extend((0..secs(0.3)).map(|k| {
                let decay = (-(k as f64) / f64::from(SR) / 0.05).exp();
                (amp * decay * sine(440.0, k)) as f32
            }));
            out.extend(std::iter::repeat_n(0.0, secs(0.2)));
            out
        })
        .collect();
    let mut audio = testing::audio(channels, SR);
    audio.music_frames = music as u64;
    audio
}

#[derive(Clone, Copy)]
enum Start {
    /// Loop from bar 2, with an isolated body.
    Bar2,
    /// Loop at bar 0, with the tail on the opening.
    Bar0,
}

/// `input` cycled, so any input fills the window and its tails.
fn cyc(input: &[Vec<f32>], ch: usize, n: usize) -> f32 {
    input[ch][n % input[ch].len()]
}

fn decayed(input: &[Vec<f32>], ch: usize, from: usize, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|k| {
            let decay = (-(k as f64) / f64::from(SR) / 0.25).exp();
            (f64::from(cyc(input, ch, from + k)) * decay) as f32
        })
        .collect()
}

/// A rendered loop export: a 4-bar window at cps 2.0 over `channels`, with `loop_audio`.
fn loop_audio(channels: Vec<Vec<f32>>, music: usize, loop_audio: LoopAudio) -> PipelineAudio {
    let mut audio = testing::audio(channels, SR);
    audio.music_frames = music as u64;
    audio.cps = LOOP_CPS;
    audio.summary.window.to_bar = LOOP_BARS;
    PipelineAudio {
        audio,
        loop_audio: Some(loop_audio),
        stems: None,
    }
}

fn loop_pipeline(input: &[Vec<f32>], start: Start) -> PipelineAudio {
    let music = window_frames(0, LOOP_BARS, LOOP_CPS) as usize;
    let window = |ch: usize| (0..music).map(|n| cyc(input, ch, n)).collect::<Vec<_>>();
    match start {
        Start::Bar2 => {
            let start_frame = window_frames(0, 2, LOOP_CPS) as usize;
            let channels = (0..input.len()).map(window).collect();
            let body = (0..input.len())
                .map(|ch| {
                    let mut body: Vec<f32> =
                        (start_frame..music).map(|n| cyc(input, ch, n)).collect();
                    body.extend(decayed(input, ch, music, secs(0.4)));
                    body
                })
                .collect();
            loop_audio(
                channels,
                music,
                LoopAudio {
                    start_bar: 2,
                    start_frame: start_frame as u64,
                    isolated_body: Some(body),
                },
            )
        }
        Start::Bar0 => {
            // 2.5 bodies of tail, so it folds twice.
            let channels = (0..input.len())
                .map(|ch| {
                    let mut channel = window(ch);
                    channel.extend(decayed(input, ch, music, secs(5.0)));
                    channel
                })
                .collect();
            loop_audio(
                channels,
                music,
                LoopAudio {
                    start_bar: 0,
                    start_frame: 0,
                    isolated_body: None,
                },
            )
        }
    }
}

/// A loop from bar 2 whose isolated body steps from +0.9 at its end to -0.9 at its start, so the
/// wrap peak sets the gain. The body's last 20 ms ramp up to +0.9; the opening's last 20 ms sit at
/// -0.9, against it, so the join's 10 ms blend leaves the ramp's last samples slightly lower and
/// the handoff peaks slightly below the wrap. (A flat +0.9 end loses: any blend residue there
/// raises the handoff's peak above the wrap's.)
fn wrap_pipeline() -> PipelineAudio {
    let music = window_frames(0, LOOP_BARS, LOOP_CPS) as usize;
    let start = window_frames(0, 2, LOOP_CPS) as usize;
    let edge = secs(0.02);
    let quiet = |n: usize, hz: f64| (0.05 * sine(hz, n)) as f32;
    let body_frames = music - start;
    let body_at = |n: usize, hz: f64| {
        if n < edge {
            -0.9
        } else if n >= body_frames - edge {
            (0.9 * (n + 1 + edge - body_frames) as f64 / edge as f64) as f32
        } else {
            quiet(start + n, hz)
        }
    };
    let hz = [220.0, 330.0];
    let channels = hz
        .iter()
        .map(|&hz| {
            (0..music)
                .map(|n| {
                    if n >= music - edge {
                        -0.9
                    } else if n >= start {
                        body_at(n - start, hz)
                    } else {
                        quiet(n, hz)
                    }
                })
                .collect()
        })
        .collect();
    let body = hz
        .iter()
        .map(|&hz| {
            let mut body: Vec<f32> = (0..body_frames).map(|n| body_at(n, hz)).collect();
            body.extend(std::iter::repeat_n(0.0, secs(0.4)));
            body
        })
        .collect();
    loop_audio(
        channels,
        music,
        LoopAudio {
            start_bar: 2,
            start_frame: start as u64,
            isolated_body: Some(body),
        },
    )
}

/// An intro-loop whose loop cut is 0.3 s (under the 400 ms gating block, so unmeasurable) while the
/// 2.3 s program is measurable. `splice` does not tie the start frame to the start bar.
fn short_body_pipeline() -> PipelineAudio {
    let input = bed();
    let music = window_frames(0, LOOP_BARS, LOOP_CPS) as usize;
    let start = music - secs(0.3);
    let channels = (0..2)
        .map(|ch| (0..music).map(|n| cyc(&input, ch, n)).collect())
        .collect();
    let body = (0..2)
        .map(|ch| (start..music).map(|n| cyc(&input, ch, n)).collect())
        .collect();
    loop_audio(
        channels,
        music,
        LoopAudio {
            start_bar: 3,
            start_frame: start as u64,
            isolated_body: Some(body),
        },
    )
}

#[derive(Clone, Copy)]
enum Path {
    Standard,
    OneShot,
    Loop(ExportPreset, Start),
}

impl Path {
    fn name(self) -> String {
        match self {
            Path::Standard => "standard".into(),
            Path::OneShot => "one-shot".into(),
            Path::Loop(preset, start) => {
                let preset = match preset {
                    ExportPreset::IntroLoop => "intro-loop",
                    ExportPreset::TaggedLoop => "tagged-loop",
                    ExportPreset::FoldedLoop => "folded-loop",
                    ExportPreset::OneShot => "one-shot",
                };
                let bar = match start {
                    Start::Bar2 => 2,
                    Start::Bar0 => 0,
                };
                format!("{preset}-bar-{bar}")
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Input {
    Bed,
    Hot,
    Silence,
    Dense,
    NanBed,
    /// Loop-only: [`wrap_pipeline`], from bar 2.
    Wrap,
    /// Loop-only: [`short_body_pipeline`], intro-loop from bar 3.
    ShortBody,
}

impl Input {
    fn name(self) -> &'static str {
        match self {
            Input::Bed => "bed",
            Input::Hot => "hot",
            Input::Silence => "silence",
            Input::Dense => "dense",
            Input::NanBed => "nan",
            Input::Wrap => "wrap",
            Input::ShortBody => "short-body",
        }
    }

    fn channels(self) -> Vec<Vec<f32>> {
        match self {
            Input::Bed => bed(),
            Input::Hot => hot(),
            Input::Silence => silence(),
            Input::Dense => dense(),
            Input::NanBed => nan_bed(),
            Input::Wrap | Input::ShortBody => unreachable!("a loop-only fixture"),
        }
    }
}

const FORMATS: [EncoderOptions; 7] = [
    EncoderOptions::Wav {
        bit_depth: BitDepth::B16,
    },
    WAV24,
    EncoderOptions::Wav {
        bit_depth: BitDepth::F32,
    },
    MP3_CBR,
    EncoderOptions::Mp3 {
        mode: Mp3Mode::Vbr,
        vbr_quality: 2,
    },
    OGG6,
    OPUS160,
];
const WAV24: EncoderOptions = EncoderOptions::Wav {
    bit_depth: BitDepth::B24,
};
const MP3_CBR: EncoderOptions = EncoderOptions::Mp3 {
    mode: Mp3Mode::Cbr,
    vbr_quality: 0,
};
const OGG6: EncoderOptions = EncoderOptions::Ogg { quality: 6 };
const OPUS160: EncoderOptions = EncoderOptions::Opus { kbps: 160 };

const fn on(target_lufs: f64, true_peak_db: f64) -> Loudness {
    Loudness::On {
        target_lufs,
        true_peak_db,
    }
}
const ON: Loudness = on(-14.0, -1.0);
/// The overshoot target: dense noise pushed to -3 LUFS makes every lossy codec overshoot -1 dBTP
/// after decoding, so both correction paths run (the overshoot guard checks it).
const OVERSHOOT: Loudness = on(-3.0, -1.0);
/// The cap cases' target, -1 LUFS: the ideal gain then exceeds the cap on the true peak (the cap
/// guards check it). For hot input folded from bar 2 (program -0.8 LUFS, +6.2 dBTP) the ideal is
/// -0.2 dB against a cap of -7.2 dB; at -14 LUFS the ideal (-13.2 dB) would sit under the cap. For
/// the wrap fixture (program -21.5 LUFS) the ideal is +20.5 dB against a cap of -2.08 dB.
const CAP: Loudness = on(-1.0, -1.0);

fn format_name(encoder: EncoderOptions) -> String {
    match encoder {
        EncoderOptions::Wav { bit_depth } => format!("wav{}", bit_depth.as_str()),
        EncoderOptions::Mp3 { mode, vbr_quality } => match mode {
            Mp3Mode::Cbr => "mp3-cbr".into(),
            Mp3Mode::Vbr => format!("mp3-vbr{vbr_quality}"),
        },
        EncoderOptions::Ogg { quality } => format!("ogg{quality}"),
        EncoderOptions::Opus { kbps } => format!("opus{kbps}"),
    }
}

fn loudness_name(loudness: Loudness) -> String {
    match loudness {
        Loudness::Off => "off".into(),
        Loudness::On {
            target_lufs,
            true_peak_db,
        } => format!("on:{target_lufs}:{true_peak_db}"),
    }
}

struct Case {
    name: String,
    path: Path,
    input: Input,
    out: OutputOptions,
    expect_error: bool,
}

fn case(path: Path, input: Input, encoder: EncoderOptions, loudness: Loudness) -> Case {
    Case {
        name: format!(
            "{}/{}/{}/{}",
            path.name(),
            input.name(),
            format_name(encoder),
            loudness_name(loudness)
        ),
        path,
        input,
        out: OutputOptions { loudness, encoder },
        expect_error: false,
    }
}

fn error_case(path: Path, input: Input, encoder: EncoderOptions) -> Case {
    Case {
        expect_error: true,
        ..case(path, input, encoder, ON)
    }
}

use ExportPreset::{FoldedLoop, IntroLoop, TaggedLoop};

const LOOPS: [Path; 6] = [
    Path::Loop(IntroLoop, Start::Bar2),
    Path::Loop(TaggedLoop, Start::Bar2),
    Path::Loop(FoldedLoop, Start::Bar2),
    Path::Loop(IntroLoop, Start::Bar0),
    Path::Loop(TaggedLoop, Start::Bar0),
    Path::Loop(FoldedLoop, Start::Bar0),
];
const INTRO2: Path = Path::Loop(IntroLoop, Start::Bar2);
const TAGGED2: Path = Path::Loop(TaggedLoop, Start::Bar2);
const FOLDED2: Path = Path::Loop(FoldedLoop, Start::Bar2);
const TAGGED0: Path = Path::Loop(TaggedLoop, Start::Bar0);

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    let paths = [Path::Standard, Path::OneShot].into_iter().chain(LOOPS);
    for path in paths {
        for encoder in FORMATS {
            for loudness in [ON, Loudness::Off] {
                cases.push(case(path, Input::Bed, encoder, loudness));
            }
        }
    }
    for path in [Path::Standard, Path::OneShot, TAGGED0] {
        for encoder in [WAV24, OGG6] {
            cases.push(case(path, Input::Silence, encoder, ON));
        }
    }
    for path in [Path::Standard, FOLDED2] {
        for encoder in [WAV24, MP3_CBR] {
            cases.push(case(path, Input::Hot, encoder, Loudness::Off));
        }
    }
    for path in [Path::Standard, Path::OneShot, INTRO2, FOLDED2] {
        for encoder in [MP3_CBR, OGG6, OPUS160] {
            cases.push(case(path, Input::Dense, encoder, OVERSHOOT));
        }
    }
    cases.push(case(FOLDED2, Input::Wrap, WAV24, CAP));
    cases.push(case(FOLDED2, Input::Hot, WAV24, CAP));
    for encoder in [WAV24, OGG6] {
        let mut short = case(INTRO2, Input::ShortBody, encoder, ON);
        short.name = short.name.replace("intro-loop-bar-2", "intro-loop-bar-3");
        cases.push(short);
    }
    for encoder in [WAV24, OGG6] {
        cases.push(case(INTRO2, Input::Silence, encoder, ON));
    }
    let bad_ogg = EncoderOptions::Ogg { quality: 99 };
    for path in [Path::Standard, Path::OneShot, INTRO2] {
        cases.push(error_case(path, Input::Bed, bad_ogg));
    }
    for path in [Path::Standard, TAGGED2] {
        cases.push(error_case(path, Input::NanBed, WAV24));
    }
    cases
}

fn run(case: &Case, progress: &dyn Fn(Progress)) -> Result<Vec<ExportFile>, RenderError> {
    match case.path {
        Path::Standard => {
            let audio = testing::audio(case.input.channels(), SR);
            crate::finish(&audio, &case.out, progress).map(|output| {
                vec![ExportFile {
                    role: ExportRole::Single,
                    output,
                    loop_points: None,
                }]
            })
        }
        Path::OneShot => {
            let pipeline = PipelineAudio {
                audio: one_shot(&case.input.channels()),
                loop_audio: None,
                stems: None,
            };
            finish_export(pipeline, ExportPreset::OneShot, &case.out, progress)
        }
        Path::Loop(preset, start) => {
            let pipeline = match case.input {
                Input::Wrap => wrap_pipeline(),
                Input::ShortBody => short_body_pipeline(),
                input => loop_pipeline(&input.channels(), start),
            };
            finish_export(pipeline, preset, &case.out, progress)
        }
    }
}

type Outcome = (Result<Vec<ExportFile>, RenderError>, Vec<String>);

/// Every case's result and progress, in case order. The cases are independent, so they run on a
/// few threads to keep the debug build's run time down.
fn run_all(cases: &[Case]) -> Vec<Outcome> {
    let next = AtomicUsize::new(0);
    let results: Vec<Mutex<Option<Outcome>>> = cases.iter().map(|_| Mutex::new(None)).collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(case) = cases.get(i) else { break };
                    let events = RefCell::new(Vec::new());
                    let result = run(case, &|p| events.borrow_mut().push(format!("{p:?}")));
                    *results[i].lock().unwrap() = Some((result, events.into_inner()));
                }
            });
        }
    });
    results
        .into_iter()
        .map(|slot| slot.into_inner().unwrap().unwrap())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn record(case: &Case, files: &[ExportFile], progress: &[String]) -> Value {
    let summaries: Vec<Value> = files
        .iter()
        .map(|file| {
            let mut summary = file.output.summary.clone();
            summary.timings = Timings::default();
            serde_json::to_value(summary).unwrap()
        })
        .collect();
    let files: Vec<Value> = files
        .iter()
        .map(|file| {
            json!({
                "role": file.role,
                "bytes": file.output.bytes.len(),
                "sha256": hex(&file.output.bytes),
                "loopPoints": file.loop_points,
            })
        })
        .collect();
    json!({
        "name": case.name,
        "files": files,
        "summaries": summaries,
        "progress": progress,
    })
}

/// The first differing field between `a` and `b`, as a JSON path.
fn first_difference(a: &Value, b: &Value, path: String) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<_> = x.keys().chain(y.keys()).collect();
            keys.into_iter().find_map(|k| match (x.get(k), y.get(k)) {
                (Some(p), Some(q)) => first_difference(p, q, format!("{path}.{k}")),
                _ => Some(format!("{path}.{k}")),
            })
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path} (length {} vs {})", x.len(), y.len()));
            }
            x.iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (p, q))| first_difference(p, q, format!("{path}[{i}]")))
        }
        _ if a == b => None,
        _ => Some(format!("{path}: {a} vs {b}")),
    }
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/finish.json")
}

fn loudness(summary: &Summary) -> &crate::level::LoudnessSummary {
    summary.loudness.as_ref().unwrap()
}

#[test]
fn finished_output_is_frozen() {
    let write = std::env::var(WRITE_VAR).is_ok_and(|v| v == "1");
    assert!(
        !write || REFERENCE,
        "{WRITE_VAR}=1 writes the golden on macOS aarch64 only, the reference platform"
    );

    let cases = cases();
    let results = run_all(&cases);
    let mut records = Vec::with_capacity(cases.len());
    let mut summaries: BTreeMap<String, Vec<Summary>> = BTreeMap::new();
    let mut unexpected = Vec::new();
    for (case, (result, events)) in cases.iter().zip(results) {
        match (result, case.expect_error) {
            (Ok(files), false) => {
                records.push(record(case, &files, &events));
                summaries.insert(
                    case.name.clone(),
                    files.into_iter().map(|f| f.output.summary).collect(),
                );
            }
            (Err(e), true) => records.push(json!({
                "name": case.name,
                "error": e.to_string(),
                "progress": events,
            })),
            (Ok(_), true) => unexpected.push(format!("{}: expected an error", case.name)),
            (Err(e), false) => unexpected.push(format!("{}: {e}", case.name)),
        }
    }
    assert!(unexpected.is_empty(), "unexpected results: {unexpected:#?}");

    // The overshoot guard: the correction paths of both strategies are pinned.
    let corrected = |loops: bool| {
        summaries.iter().any(|(name, files)| {
            let is_loop = !(name.starts_with("standard/") || name.starts_with("one-shot/"));
            name.contains("/dense/")
                && is_loop == loops
                && files.iter().any(|s| loudness(s).encodes >= 2)
        })
    };
    assert!(corrected(false), "no limited overshoot case corrected");
    assert!(corrected(true), "no loop overshoot case corrected");

    // The cap guards (WAV, one pass, one cut, so the input is the program's measure).
    let single = |name: &str| {
        let files = &summaries[name];
        assert_eq!(files.len(), 1, "{name}");
        loudness(&files[0]).clone()
    };
    let Loudness::On {
        target_lufs,
        true_peak_db: ceiling,
    } = CAP
    else {
        unreachable!()
    };
    let program = single("folded-loop-bar-2/hot/wav24/on:-1:-1");
    assert_eq!(
        program.gain_db.to_bits(),
        (ceiling - program.input_true_peak_db).to_bits(),
        "the program peak binds the cap: {program:?}"
    );
    assert!(
        program.gain_db < target_lufs - program.input_lufs,
        "{program:?}"
    );
    let wrap = single("folded-loop-bar-2/wrap/wav24/on:-1:-1");
    assert!(
        wrap.gain_db < ceiling - wrap.input_true_peak_db,
        "the wrap peak binds the cap: {wrap:?}"
    );
    assert!(wrap.gain_db < target_lufs - wrap.input_lufs, "{wrap:?}");

    let actual = json!({ "schema": 1, "cases": records });
    let text = serde_json::to_string_pretty(&actual).unwrap() + "\n";
    if write {
        let path = golden_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        eprintln!("wrote {} ({} cases)", path.display(), cases.len());
        return;
    }
    if !REFERENCE {
        eprintln!(
            "finish golden: comparison skipped; it is judged on macOS aarch64 only ({} cases ran)",
            cases.len()
        );
        return;
    }
    let golden: Value = serde_json::from_str(
        &std::fs::read_to_string(golden_path())
            .unwrap_or_else(|e| panic!("read {}: {e}", golden_path().display())),
    )
    .unwrap();
    // Compare what the golden would hold, so float parsing treats both sides alike.
    let actual: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(golden["schema"], actual["schema"], "golden schema");
    let empty = Vec::new();
    let expected = golden["cases"].as_array().unwrap_or(&empty);
    let got = actual["cases"].as_array().unwrap_or(&empty);
    let mut differences = Vec::new();
    for (i, case) in got.iter().enumerate() {
        let name = case["name"].as_str().unwrap_or("?");
        match expected.get(i) {
            Some(old) => {
                if let Some(at) = first_difference(old, case, String::new()) {
                    differences.push(format!("{name}: {at}"));
                }
            }
            None => differences.push(format!("{name}: not in the golden")),
        }
    }
    for old in expected.iter().skip(got.len()) {
        differences.push(format!(
            "{}: missing from this run",
            old["name"].as_str().unwrap_or("?")
        ));
    }
    assert!(
        differences.is_empty(),
        "finished output differs from tests/golden/finish.json in {} case(s):\n{}",
        differences.len(),
        differences.join("\n")
    );
}
