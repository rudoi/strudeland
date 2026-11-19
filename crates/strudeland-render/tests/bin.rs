//! The dev binary `strudeland-render`: exit codes, the float WAV, the info file, nothing written
//! on failure.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::corpus_path;
use strudeland_engine::wav::read_f32;

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_strudeland-render"))
        .args(args)
        .output()
        .unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn song(file: &str) -> String {
    corpus_path(file).to_string_lossy().into_owned()
}

fn path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn render_writes_the_float_wav_and_the_info() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("a.wav");
    let info = dir.path().join("a.json");
    let o = run(&[
        "render",
        "--song",
        &song("sine-basic.js"),
        "--from",
        "0",
        "--to",
        "2",
        "--layout",
        "linear",
        "--out",
        &path(&wav),
        "--info",
        &path(&info),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(stderr(&o), "");
    assert!(o.stdout.is_empty());
    let decoded = read_f32(&std::fs::read(&wav).unwrap()).unwrap();
    assert_eq!(decoded.channels.len(), 2);
    assert!(decoded.channels.iter().all(|c| c.len() == 176_400));
    let info: serde_json::Value = serde_json::from_slice(&std::fs::read(&info).unwrap()).unwrap();
    let keys: Vec<&str> = info
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let mut want = vec![
        "window",
        "cps",
        "structure",
        "lookbackSec",
        "errors",
        "events",
        "chunks",
        "summary",
    ];
    want.sort_unstable();
    let mut keys = keys;
    keys.sort_unstable();
    assert_eq!(keys, want);
    assert_eq!(
        info["window"],
        serde_json::json!({ "fromBar": 0, "toBar": 2, "layout": "linear" })
    );
    assert_eq!(info["cps"], 0.5);
    assert_eq!(info["errors"], serde_json::json!([]));
    assert!(!info["events"].as_array().unwrap().is_empty());
    assert_eq!(info["chunks"][0]["fromBar"], 0);
    assert!(info["chunks"][0]["windowStartFrame"].is_u64());
    assert_eq!(info["summary"]["frames"], 176_400);
    assert_eq!(info["summary"]["format"], "wav");
    assert_eq!(info["summary"]["sampleRate"], 44_100);
}

#[test]
fn render_levels_and_verifies_a_lossy_file() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["mp3", "ogg", "opus"] {
        let out = dir.path().join(format!("a.{ext}"));
        let info = dir.path().join(format!("{ext}.json"));
        let o = run(&[
            "render",
            "--song",
            &song("sine-basic.js"),
            "--length",
            "4",
            "--loudness",
            "on",
            "--out",
            &path(&out),
            "--info",
            &path(&info),
        ]);
        assert_eq!(o.status.code(), Some(0), "{ext}: {}", stderr(&o));
        let info: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&info).unwrap()).unwrap();
        let summary = &info["summary"];
        assert_eq!(summary["format"], ext);
        assert_eq!(
            summary["bytes"],
            std::fs::metadata(&out).unwrap().len(),
            "{ext}"
        );
        let l = &summary["loudness"];
        assert_eq!(l["enabled"], true, "{ext}");
        assert_eq!(l["ceilingDbtp"], -1.0, "{ext}");
        let lufs = l["outputLufs"].as_f64().unwrap();
        assert!((lufs + 23.0).abs() <= 0.1, "{ext}: {lufs}");
        let dp = l["decodedTruePeakDb"].as_f64().unwrap();
        assert!(dp <= -1.0, "{ext}: {dp}");
        assert!((1..=3).contains(&l["encodes"].as_u64().unwrap()), "{ext}");
    }
}

#[test]
fn length_forms_render() {
    let dir = tempfile::tempdir().unwrap();
    for (name, extra, frames) in [
        ("length", &["--length", "1"][..], 88_200usize),
        ("section", &["--section", "intro"][..], 176_400),
        ("clip loops", &["--loops", "1"][..], 8 * 88_200),
    ] {
        let file = if name == "length" {
            "sine-basic.js"
        } else {
            "song-linear.js"
        };
        let wav = dir.path().join(format!("{}.wav", name.replace(' ', "-")));
        let mut args = vec!["render", "--song"];
        let s = song(file);
        let w = path(&wav);
        args.push(&s);
        args.extend_from_slice(extra);
        args.extend_from_slice(&["--out", &w, "--jobs", "2"]);
        let o = run(&args);
        assert_eq!(o.status.code(), Some(0), "{name}: {}", stderr(&o));
        let decoded = read_f32(&std::fs::read(&wav).unwrap()).unwrap();
        assert_eq!(decoded.channels[0].len(), frames, "{name}");
    }
}

#[test]
fn unsupported_material_exits_1_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("a.wav");
    let info = dir.path().join("a.json");
    let o = run(&[
        "render",
        "--song",
        &song("samples.js"),
        "--from",
        "0",
        "--to",
        "2",
        "--layout",
        "linear",
        "--out",
        &path(&wav),
        "--info",
        &path(&info),
    ]);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert_eq!(
        stderr(&o),
        "strudeland-render: the song uses sounds or controls the engine cannot play yet; nothing was written:\n  s=hat first at bar 1\n  s=kick first at bar 1\n  s=tone first at bar 1\n"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn failures_exit_1() {
    let dir = tempfile::tempdir().unwrap();
    let wav = path(&dir.path().join("a.wav"));
    let sine = song("sine-basic.js");
    // An evaluation error, a missing output directory.
    let missing = path(&dir.path().join("no/a.wav"));
    let bad_song = dir.path().join("bad.js");
    std::fs::write(&bad_song, "$: note(").unwrap();
    let bad = path(&bad_song);
    for (name, args, prefix) in [
        (
            "eval",
            vec!["render", "--song", &bad, "--length", "1", "--out", &wav],
            "strudeland-render: eval: ",
        ),
        (
            "missing dir",
            vec![
                "render", "--song", &sine, "--length", "1", "--out", &missing,
            ],
            "strudeland-render: ",
        ),
    ] {
        let o = run(&args);
        assert_eq!(o.status.code(), Some(1), "{name}: {}", stderr(&o));
        assert!(stderr(&o).starts_with(prefix), "{name}: {}", stderr(&o));
    }
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "only bad.js is left"
    );
}

#[test]
fn bad_input_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let wav = path(&dir.path().join("a.wav"));
    let sine = song("sine-basic.js");
    let song_linear = song("song-linear.js");
    let missing = path(&dir.path().join("nope.js"));
    for (name, args, want) in [
        ("no subcommand", vec![], None),
        (
            "unknown flag",
            vec!["render", "--song", &sine, "--out", &wav, "--nope"],
            None,
        ),
        ("no out", vec!["render", "--song", &sine], None),
        (
            "partial window",
            vec![
                "render", "--song", &sine, "--from", "0", "--to", "2", "--out", &wav,
            ],
            Some("strudeland-render: --from, --to and --layout go together\n"),
        ),
        (
            "two forms",
            vec![
                "render",
                "--song",
                &sine,
                "--length",
                "2",
                "--section",
                "a",
                "--out",
                &wav,
            ],
            Some(
                "strudeland-render: give at most one of --from/--to/--layout, --section and --length\n",
            ),
        ),
        (
            "jobs 0",
            vec![
                "render", "--song", &sine, "--length", "1", "--jobs", "0", "--out", &wav,
            ],
            Some("strudeland-render: --jobs must be at least 1\n"),
        ),
        (
            "no length for a song without sections",
            vec!["render", "--song", &sine, "--out", &wav],
            Some(
                "strudeland-render: this song has no sections; pass --length N to say how many bars to render\n",
            ),
        ),
        (
            "unknown section",
            vec![
                "render",
                "--song",
                &song_linear,
                "--section",
                "drop",
                "--out",
                &wav,
            ],
            Some("strudeland-render: unknown section 'drop' (sections: intro, body, tail)\n"),
        ),
        (
            "missing song",
            vec!["render", "--song", &missing, "--length", "1", "--out", &wav],
            None,
        ),
        (
            "bad format",
            vec![
                "render", "--song", &sine, "--length", "1", "--format", "flac", "--out", &wav,
            ],
            None,
        ),
        (
            "format and extension disagree",
            vec![
                "render", "--song", &sine, "--length", "1", "--format", "mp3", "--out", &wav,
            ],
            Some(&*format!(
                "strudeland-render: --format mp3 disagrees with the extension of {wav}\n"
            )),
        ),
        (
            "bench without a song",
            vec!["bench", "--jobs", "2"],
            Some("strudeland-render: no song given\n"),
        ),
        (
            "bench jobs 0",
            vec!["bench", "--jobs", "0", &sine],
            Some("strudeland-render: --jobs must be an integer from 1 to 64, got 0\n"),
        ),
        ("bench missing song", vec!["bench", &missing], None),
    ] {
        let o = run(&args);
        assert_eq!(o.status.code(), Some(2), "{name}: {}", stderr(&o));
        let err = stderr(&o);
        assert!(err.starts_with("strudeland-render: "), "{name}: {err}");
        assert_eq!(err.lines().count(), 1, "{name}: {err}");
        if let Some(want) = want {
            assert_eq!(err, want, "{name}");
        }
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn help_exits_0() {
    let o = run(&["render", "--help"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stdout).contains("--plan-timeout"));
}

fn bench_doc(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout).unwrap()
}

#[test]
fn bench_one_song_in_process() {
    let dir = tempfile::tempdir().unwrap();
    let o = run(&[
        "bench",
        "--jobs",
        "2",
        "--bars",
        "2",
        "--out-dir",
        &path(dir.path()),
        &song("sine-basic.js"),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let doc = bench_doc(&o);
    assert_eq!(doc["jobs"], 2);
    assert_eq!(doc["isolated"], false);
    let s = &doc["songs"][0];
    assert_eq!(s["name"], "sine-basic");
    assert_eq!(s["ok"], true);
    assert_eq!(s["bars"], 2);
    assert_eq!(s["fullSong"], false);
    assert_eq!(s["frames"], 176_400);
    assert_eq!(s["audioSeconds"], 4.0);
    assert_eq!(s["chunks"], 1);
    assert_eq!(s["workers"], 1);
    assert!(s["peakRssMb"].as_f64().unwrap() > 0.0);
    assert!(s["wallMs"].as_f64().unwrap() > 0.0);
    let l = &s["loudness"];
    assert_eq!(l["targetLufs"], -23.0);
    assert_eq!(l["ceilingDbtp"], -1.0);
    assert!(
        (l["outputLufs"].as_f64().unwrap() + 23.0).abs() < 0.5,
        "{l}"
    );
    assert!(l["decodedTruePeakDb"].as_f64().unwrap() <= -1.0, "{l}");
    let enc = s["encoders"].as_object().unwrap();
    let names: Vec<&str> = enc.keys().map(String::as_str).collect();
    assert_eq!(names, ["mp3", "ogg", "opus", "wav"]);
    assert!(enc.values().all(|e| e["bytes"].as_u64().unwrap() > 0));
    assert_eq!(enc["wav"]["decodedTruePeakDb"], serde_json::Value::Null);
    let file = dir.path().join("sine-basic.jobs2.json");
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(&written, s);
    let err = stderr(&o);
    assert_eq!(err.lines().count(), 2, "{err}");
    assert!(err.starts_with("song "), "{err}");
}

#[test]
fn bench_isolates_songs_and_reports_failures() {
    let dir = tempfile::tempdir().unwrap();
    let lengths = dir.path().join("lengths.json");
    std::fs::write(&lengths, r#"{ "sine-basic": 1 }"#).unwrap();
    let o = run(&[
        "bench",
        "--jobs",
        "1",
        "--lengths",
        &path(&lengths),
        &song("sine-basic.js"),
        &song("samples.js"),
    ]);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    let doc = bench_doc(&o);
    assert_eq!(doc["isolated"], true);
    let songs = doc["songs"].as_array().unwrap();
    assert_eq!(songs.len(), 2);
    assert_eq!(songs[0]["ok"], true);
    assert_eq!(songs[0]["fullSong"], true);
    assert_eq!(songs[0]["frames"], 88_200);
    assert_eq!(songs[1]["name"], "samples");
    assert_eq!(songs[1]["ok"], false);
    assert_eq!(songs[1]["bars"], 16);
    assert!(
        songs[1]["error"].as_str().unwrap().contains("first at bar"),
        "{}",
        songs[1]["error"]
    );
    assert!(stderr(&o).ends_with("strudeland-render: failed: samples\n"));
}
