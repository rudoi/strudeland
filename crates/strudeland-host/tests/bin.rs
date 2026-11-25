//! The `strudeland-host` binary's stdout/stderr contract and exit codes.

mod common;

use std::process::{Command, Output};

use common::repo_root;
use serde_json::Value;

const REQUEST: &str = r#"{"layout":"linear","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100}"#;

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_strudeland-host"))
        .args(args)
        .current_dir(repo_root())
        .env_remove("STRUDELAND_HOST_LOG")
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The single line of stderr (panics if there is not exactly one).
fn one_line(out: &Output) -> String {
    let err = text(&out.stderr);
    assert!(err.ends_with('\n'), "{err:?}");
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 1, "{err:?}");
    lines[0].to_string()
}

fn temp_file(content: &str) -> tempfile::NamedTempFile {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), content).unwrap();
    file
}

const SONG: &str = "tests/render/corpus/sine-basic.js";

#[test]
fn plan_writes_the_job_to_stdout_and_one_info_line() {
    let out = run(&["plan", "--song", SONG, "--request", REQUEST]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let info: Value = serde_json::from_str(&one_line(&out)).unwrap();
    assert_eq!(info["jsonLength"].as_u64(), Some(out.stdout.len() as u64));
    let job: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(job["schema"], 2);
    assert!(!out.stdout.ends_with(b"\n"));
}

#[test]
fn plan_reads_the_request_from_a_file_and_writes_out() {
    let stdout_job = run(&["plan", "--song", SONG, "--request", REQUEST]).stdout;
    let request = temp_file(REQUEST);
    let dir = tempfile::tempdir().unwrap();
    let job_path = dir.path().join("job.json");
    let out = run(&[
        "plan",
        "--request",
        request.path().to_str().unwrap(),
        "--song",
        SONG,
        "--out",
        job_path.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stdout.is_empty());
    let info: Value = serde_json::from_str(&one_line(&out)).unwrap();
    let written = std::fs::read(&job_path).unwrap();
    assert_eq!(written, stdout_job);
    assert_eq!(info["jsonLength"].as_u64(), Some(written.len() as u64));
    // JSON text with leading spaces is still JSON text, not a path.
    let spaced = format!("  \n{REQUEST}");
    let out = run(&["plan", "--song", SONG, "--request", &spaced]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(out.stdout, stdout_job);
}

#[test]
fn plan_host_errors_exit_1_with_the_kind() {
    let syntax = temp_file("$: note(\"c\".s(\n");
    let out = run(&[
        "plan",
        "--song",
        syntax.path().to_str().unwrap(),
        "--request",
        REQUEST,
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty());
    let line = one_line(&out);
    assert!(
        line.starts_with("strudeland-host: eval: ") || line.starts_with("strudeland-host: meta: "),
        "{line}"
    );

    let bad_meta = temp_file("const meta = { bpm: 'x' };\n$: note(\"c\")\n");
    let out = run(&[
        "plan",
        "--song",
        bad_meta.path().to_str().unwrap(),
        "--request",
        REQUEST,
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(one_line(&out).starts_with("strudeland-host: meta: "));

    let waits = temp_file("await new Promise((r) => setTimeout(r, 5));\n$: note(\"c\")\n");
    let out = run(&[
        "plan",
        "--song",
        waits.path().to_str().unwrap(),
        "--request",
        REQUEST,
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(one_line(&out).starts_with("strudeland-host: unsettled: "));

    let multiline = temp_file("throw new Error('line one\\nline two');\n");
    let out = run(&[
        "plan",
        "--song",
        multiline.path().to_str().unwrap(),
        "--request",
        REQUEST,
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(one_line(&out).contains("line one\\nline two"));
}

fn assert_bad_input(out: &Output, needle: &str) {
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    let err = text(&out.stderr);
    let first = err.lines().next().unwrap_or_default();
    assert!(first.starts_with("strudeland-host: "), "{err}");
    assert!(first.contains(needle), "{needle}: {err}");
    assert!(err.contains("usage: strudeland-host plan"), "{err}");
}

#[test]
fn bad_input_exits_2_with_the_usage() {
    assert_bad_input(
        &run(&["plan", "--song", "no/such/song.js", "--request", REQUEST]),
        "no/such/song.js",
    );
    assert_bad_input(
        &run(&["plan", "--song", SONG, "--request", "no/such/request.json"]),
        "no/such/request.json",
    );
    assert_bad_input(
        &run(&["plan", "--song", SONG, "--request", "{"]),
        "invalid request",
    );
    assert_bad_input(
        &run(&[
            "plan",
            "--song",
            SONG,
            "--request",
            r#"{"layout":"linear","fromBar":4,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100}"#,
        ]),
        "fromBar",
    );
    assert_bad_input(
        &run(&[
            "plan",
            "--song",
            SONG,
            "--request",
            r#"{"layout":"linear","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100,"samples":{}}"#,
        ]),
        "samples",
    );
    assert_bad_input(&run(&["plan", "--song", SONG]), "missing --request");
    assert_bad_input(&run(&["plan", "--request", REQUEST]), "missing --song");
    assert_bad_input(
        &run(&["plan", "--song", SONG, "--request", REQUEST, "--bogus", "1"]),
        "unknown argument --bogus",
    );
    assert_bad_input(
        &run(&["plan", "--song", SONG, "--song", SONG, "--request", REQUEST]),
        "--song given twice",
    );
    assert_bad_input(&run(&["plan", "--song"]), "--song needs a value");
    assert_bad_input(&run(&["render"]), "unknown command render");
    assert_bad_input(&run(&[]), "missing command");
    let dir = tempfile::tempdir().unwrap();
    let unwritable = dir.path().join("missing-dir/job.json");
    assert_bad_input(
        &run(&[
            "plan",
            "--song",
            SONG,
            "--request",
            REQUEST,
            "--out",
            unwritable.to_str().unwrap(),
        ]),
        "writing",
    );
}

#[test]
fn query_prints_the_events() {
    let out = run(&["query", "--song", SONG, "--from", "0", "--to", "1"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    let events: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(!events.as_array().unwrap().is_empty());
    let clip = run(&[
        "query", "--layout", "clip", "--song", SONG, "--from", "0", "--to", "1",
    ]);
    assert_eq!(clip.status.code(), Some(0), "{clip:?}");
    let linear = run(&[
        "query", "--layout", "linear", "--song", SONG, "--from", "0", "--to", "1",
    ]);
    assert_eq!(linear.stdout, out.stdout);
}

#[test]
fn query_errors_follow_the_plan_exit_codes() {
    let out = run(&["query", "--song", SONG, "--from", "1", "--to", "1"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(one_line(&out), "strudeland-host: plan: bad window");
    let out = run(&["query", "--song", SONG, "--from", "NaN", "--to", "1"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_bad_input(
        &run(&["query", "--song", SONG, "--from", "zero", "--to", "1"]),
        "--from must be a number",
    );
    assert_bad_input(
        &run(&[
            "query", "--song", SONG, "--from", "0", "--to", "1", "--layout", "song",
        ]),
        "--layout must be clip or linear",
    );
    assert_bad_input(
        &run(&["query", "--song", SONG, "--from", "0"]),
        "missing --to",
    );
    assert_bad_input(
        &run(&["query", "--song", "nope.js", "--from", "0", "--to", "1"]),
        "nope.js",
    );
}

const RENDER_REQUEST: &str = r#"{"length":{"kind":"length","bars":4}}"#;
const SPIN: &str = "while (true) {}\n";

#[test]
fn plan_render_writes_one_json_line_and_nothing_on_stderr() {
    let out = run(&["plan-render", "--song", SONG, "--request", RENDER_REQUEST]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    assert!(
        out.stdout
            .starts_with(br#"{"window":{"fromBar":0,"toBar":4,"layout":"linear"},"#)
    );
    assert!(
        !out.stdout.contains(&b'\n'),
        "one line, no trailing newline"
    );
    let planned: Value = serde_json::from_slice(&out.stdout).unwrap();
    let chunks = planned["chunks"].as_array().unwrap();
    assert!(!chunks.is_empty());
    for chunk in chunks {
        let job: Value = serde_json::from_str(chunk["json"].as_str().unwrap()).unwrap();
        assert_eq!(job["schema"], 2);
    }
    // The single chunk equals `plan` of its request.
    if chunks.len() == 1 {
        let lookback = &chunks[0]["plan"]["lookbackSec"];
        let request = format!(
            r#"{{"layout":"linear","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":{lookback},"mode":"mix","sampleRate":44100}}"#
        );
        let single = run(&["plan", "--song", SONG, "--request", &request]);
        assert_eq!(single.status.code(), Some(0), "{single:?}");
        assert_eq!(text(&single.stdout), chunks[0]["json"].as_str().unwrap());
    }

    // From a request file, to --out.
    let request = temp_file(RENDER_REQUEST);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.json");
    let written = run(&[
        "plan-render",
        "--request",
        request.path().to_str().unwrap(),
        "--song",
        SONG,
        "--out",
        path.to_str().unwrap(),
    ]);
    assert_eq!(written.status.code(), Some(0), "{written:?}");
    assert!(written.stdout.is_empty() && written.stderr.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), out.stdout);
}

#[test]
fn plan_render_length_errors_exit_1_naming_the_flag() {
    let out = run(&[
        "plan-render",
        "--song",
        SONG,
        "--request",
        r#"{"length":{"kind":"clip"}}"#,
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty());
    assert_eq!(
        one_line(&out),
        "strudeland-host: meta: this song has no sections; pass --length N to say how many bars to render"
    );
    let spin = temp_file(SPIN);
    let out = run(&[
        "plan-render",
        "--song",
        spin.path().to_str().unwrap(),
        "--request",
        r#"{"length":{"kind":"length","bars":2},"timeoutSec":0.5}"#,
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty());
    assert_eq!(
        one_line(&out),
        "strudeland-host: timeout: planning did not finish within 0.5 s"
    );
}

#[test]
fn plan_render_bad_input_exits_2_with_the_usage() {
    let usage = |out: &Output| {
        assert!(
            text(&out.stderr).contains("strudeland-host plan-render --song <file>"),
            "{out:?}"
        );
    };
    for (request, needle) in [
        (
            r#"{"length":{"kind":"length","bars":4},"mode":"mix"}"#,
            "unknown field `mode`",
        ),
        (
            r#"{"length":{"kind":"length","bars":0}}"#,
            "bad request: length.bars",
        ),
        (
            r#"{"length":{"kind":"clip","loops":0}}"#,
            "bad request: --loops must be an integer from 1 to 1000",
        ),
        (
            r#"{"length":{"kind":"window","fromBar":4,"toBar":4,"layout":"linear"}}"#,
            "bad request: length window",
        ),
        (
            r#"{"length":{"kind":"length","bars":4},"timeoutSec":0}"#,
            "timeoutSec",
        ),
        (
            r#"{"length":{"kind":"length","bars":4,"loops":1}}"#,
            "invalid request",
        ),
        (
            r#"{"layout":"linear","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100}"#,
            "invalid request",
        ),
        ("{", "invalid request"),
        ("no/such/request.json", "no/such/request.json"),
    ] {
        let out = run(&["plan-render", "--song", SONG, "--request", request]);
        assert_bad_input(&out, needle);
        usage(&out);
    }
    assert_bad_input(
        &run(&[
            "plan-render",
            "--song",
            "no/such/song.js",
            "--request",
            RENDER_REQUEST,
        ]),
        "no/such/song.js",
    );
    assert_bad_input(&run(&["plan-render", "--song", SONG]), "missing --request");
    assert_bad_input(
        &run(&[
            "plan-render",
            "--song",
            SONG,
            "--request",
            RENDER_REQUEST,
            "--timeout",
            "5",
        ]),
        "unknown argument --timeout",
    );
    let dir = tempfile::tempdir().unwrap();
    let unwritable = dir.path().join("missing-dir/plan.json");
    assert_bad_input(
        &run(&[
            "plan-render",
            "--song",
            SONG,
            "--request",
            RENDER_REQUEST,
            "--out",
            unwritable.to_str().unwrap(),
        ]),
        "writing",
    );
}

#[test]
fn plan_timeout_bounds_the_planning() {
    let plain = run(&["plan", "--song", SONG, "--request", REQUEST]);
    let timed = run(&[
        "plan",
        "--song",
        SONG,
        "--request",
        REQUEST,
        "--timeout",
        "600",
    ]);
    assert_eq!(timed.status.code(), Some(0), "{timed:?}");
    assert_eq!(timed.stdout, plain.stdout);
    assert_eq!(timed.stderr, plain.stderr);

    let spin = temp_file(SPIN);
    let out = run(&[
        "plan",
        "--song",
        spin.path().to_str().unwrap(),
        "--request",
        REQUEST,
        "--timeout",
        "0.5",
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty());
    assert_eq!(
        one_line(&out),
        "strudeland-host: timeout: planning did not finish within 0.5 s"
    );

    for bad in ["0", "-1", "abc", "NaN", "inf", "86401", ""] {
        assert_bad_input(
            &run(&[
                "plan",
                "--song",
                SONG,
                "--request",
                REQUEST,
                "--timeout",
                bad,
            ]),
            "--timeout must be a number of seconds greater than 0 and at most 86400",
        );
    }
    assert_bad_input(
        &run(&["plan", "--song", SONG, "--request", REQUEST, "--timeout"]),
        "--timeout needs a value",
    );
}
