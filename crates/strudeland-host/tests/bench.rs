//! `strudeland-host bench`: the JSON document, the job hash, `--bars`/`--lengths`, a failing
//! song, bad flags, peak memory and `--profile`.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::repo_root;
use serde_json::Value;
use strudeland_host::bench::sha256_hex;

fn bench(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_strudeland-host"))
        .arg("bench")
        .args(args)
        .current_dir(repo_root())
        .output()
        .unwrap()
}

fn report(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}); stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

const SINE: &str = "tests/render/corpus/sine-basic.js";

fn write(dir: &Path, name: &str, text: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn bench_on_sine_basic_reports_one_planned_song() {
    let out = bench(&["--bars", "4", SINE]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let r = report(&out);
    assert_eq!(r["runtime"], "v8");
    assert!(r["version"].as_str().is_some_and(|v| !v.is_empty()));
    assert_eq!(r["bars"], 4);
    let song = &r["songs"][0];
    assert_eq!(song["name"], "sine-basic");
    assert_eq!(song["ok"], true);
    assert_eq!(song["bars"], 4);
    assert_eq!(song["fullSong"], false);
    assert!(song.get("error").is_none());
    assert!(song.get("profile").is_none());
    for key in ["loadMs", "queryMs", "planMs"] {
        let ms = song[key].as_f64().unwrap_or(-1.0);
        assert!(ms > 0.0, "{key}: {ms}");
    }
    assert!(song["loadMs"].as_f64() < song["planMs"].as_f64());
    assert!(r["maxRssBytes"].as_u64().unwrap() > 0);
    assert_eq!(
        r["budget"]["limits"]["fullSongMaxMs"].as_f64(),
        Some(2000.0)
    );
    // The table and the budget line are on stderr.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("sine-basic"), "{stderr}");
    assert!(stderr.contains("budget:"), "{stderr}");
}

#[test]
fn the_hash_and_size_are_those_of_the_plan_output() {
    let out = bench(&["--bars", "4", SINE]);
    let r = report(&out);
    let plan = Command::new(env!("CARGO_BIN_EXE_strudeland-host"))
        .args([
            "plan",
            "--song",
            SINE,
            "--request",
            r#"{"layout":"linear","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100}"#,
        ])
        .current_dir(repo_root())
        .output()
        .unwrap();
    assert_eq!(plan.status.code(), Some(0));
    let sha = sha256_hex(std::str::from_utf8(&plan.stdout).unwrap());
    assert_eq!(r["songs"][0]["jobSha256"], sha.as_str());
    assert_eq!(r["songs"][0]["jobBytes"], plan.stdout.len());
}

#[test]
fn bars_default_to_16_and_lengths_win() {
    let dir = tempfile::tempdir().unwrap();
    let out = bench(&[SINE]);
    assert_eq!(report(&out)["songs"][0]["bars"], 16);
    let lengths = write(
        dir.path(),
        "lengths.json",
        r#"{"sine-basic": 3, "other": 9}"#,
    );
    let out = bench(&["--bars", "5", "--lengths", &lengths, SINE]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let r = report(&out);
    assert_eq!(r["bars"], 5);
    assert_eq!(r["songs"][0]["bars"], 3);
    assert_eq!(r["songs"][0]["fullSong"], true);
    assert_eq!(r["budget"]["fullSongs"], 1);
    assert_eq!(r["budget"]["longest"]["name"], "sine-basic");
}

#[test]
fn a_failing_song_exits_1_and_the_others_still_run() {
    let dir = tempfile::tempdir().unwrap();
    let bad = write(dir.path(), "throws.js", "throw new Error('nope')\n");
    let out = bench(&["--bars", "2", &bad, SINE]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let r = report(&out);
    let songs = r["songs"].as_array().unwrap();
    assert_eq!(songs.len(), 2);
    assert_eq!(songs[0]["name"], "throws");
    assert_eq!(songs[0]["ok"], false);
    assert!(
        songs[0]["error"].as_str().unwrap().starts_with("eval: "),
        "{}",
        songs[0]["error"]
    );
    assert!(songs[0]["planMs"].is_null());
    assert!(songs[0]["jobSha256"].is_null());
    assert_eq!(songs[1]["ok"], true);
    // The budget is judged over the planned songs only.
    assert!(r["budget"]["medianPlanMs"].as_f64().is_some());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("failed: throws"), "{stderr}");
}

#[test]
fn bad_input_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let bad_lengths = write(dir.path(), "lengths.json", r#"{"sine-basic": 0}"#);
    let missing = dir.path().join("missing.js");
    for args in [
        vec![],
        vec!["--bars", "0", SINE],
        vec!["--bars", "x", SINE],
        vec!["--bars"],
        vec!["--runtime", "quickjs", SINE],
        vec!["--lengths", bad_lengths.as_str(), SINE],
        vec!["--lengths", "/nonexistent/lengths.json", SINE],
        vec![missing.to_str().unwrap()],
    ] {
        let out = bench(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("usage:"), "{args:?}: {stderr}");
    }
}

#[test]
fn profile_fields_are_present_and_leave_the_job_unchanged() {
    let plain = report(&bench(&["--bars", "4", SINE]));
    let out = bench(&["--bars", "4", "--profile", SINE]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let profiled = report(&out);
    let song = &profiled["songs"][0];
    let profile = &song["profile"];
    for key in ["loadMs", "evaluateMs", "queryMs", "jsonMs"] {
        let ms = profile[key].as_f64().unwrap_or(-1.0);
        assert!(ms >= 0.0, "{key}: {ms}");
    }
    assert!(profile["evaluateMs"].as_f64().unwrap() > 0.0);
    assert_eq!(song["jobSha256"], plain["songs"][0]["jobSha256"]);
    assert_eq!(song["jobBytes"], plain["songs"][0]["jobBytes"]);
}

#[test]
fn out_dir_receives_the_job() {
    let dir = tempfile::tempdir().unwrap();
    let jobs = dir.path().join("jobs");
    let out = bench(&["--bars", "4", "--out-dir", jobs.to_str().unwrap(), SINE]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let r = report(&out);
    let job = std::fs::read(jobs.join("sine-basic.job.json")).unwrap();
    assert_eq!(r["songs"][0]["jobBytes"], job.len());
}
