//! The `strudeland-host` binary's stdout/stderr contract and exit codes.

mod common;

use std::process::{Command, Output};

use common::repo_root;
use serde_json::Value;

/// A single-chunk request: `[0, 4)` linear, no overlap or lookback, mix.
const REQUEST: &str = r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"},"chunk":{"overlapBars":0,"lookbackSec":0,"mode":"mix"}}"#;

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

/// The one chunk of a single-chunk `plan-render` output.
fn single_chunk(out: &Output) -> Value {
    let planned: Value = serde_json::from_slice(&out.stdout).unwrap();
    let chunks = planned["chunks"].as_array().unwrap();
    assert_eq!(chunks.len(), 1, "{planned}");
    chunks[0].clone()
}

#[test]
fn plan_render_with_a_chunk_plans_one_chunk_with_its_info() {
    let out = run(&["plan-render", "--song", SONG, "--request", REQUEST]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    let chunk = single_chunk(&out);
    let job: Value = serde_json::from_str(chunk["json"].as_str().unwrap()).unwrap();
    assert_eq!(job["schema"], 2);
    assert!(chunk["info"]["events"].is_array(), "{chunk}");
    assert!(chunk["info"]["stemPairs"].is_u64(), "{chunk}");
    // JSON text with leading spaces is still JSON text, not a path.
    let spaced = format!("  \n{REQUEST}");
    let again = run(&["plan-render", "--song", SONG, "--request", &spaced]);
    assert_eq!(again.status.code(), Some(0), "{again:?}");
    assert_eq!(again.stdout, out.stdout);
}

#[test]
fn plan_render_host_errors_exit_1_with_the_kind() {
    let syntax = temp_file("$: note(\"c\".s(\n");
    let out = run(&[
        "plan-render",
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
        "plan-render",
        "--song",
        bad_meta.path().to_str().unwrap(),
        "--request",
        REQUEST,
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(one_line(&out).starts_with("strudeland-host: meta: "));

    let waits = temp_file("await new Promise((r) => setTimeout(r, 5));\n$: note(\"c\")\n");
    let out = run(&[
        "plan-render",
        "--song",
        waits.path().to_str().unwrap(),
        "--request",
        REQUEST,
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(one_line(&out).starts_with("strudeland-host: unsettled: "));

    let multiline = temp_file("throw new Error('line one\\nline two');\n");
    let out = run(&[
        "plan-render",
        "--song",
        multiline.path().to_str().unwrap(),
        "--request",
        REQUEST,
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(one_line(&out).contains("line one\\nline two"));
}

#[test]
fn plan_is_not_a_command() {
    assert_bad_input(
        &run(&["plan", "--song", "x", "--request", "{}"]),
        "unknown command plan",
    );
}

fn assert_bad_input(out: &Output, needle: &str) {
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    let err = text(&out.stderr);
    let first = err.lines().next().unwrap_or_default();
    assert!(first.starts_with("strudeland-host: "), "{err}");
    assert!(first.contains(needle), "{needle}: {err}");
    assert!(err.contains("usage: strudeland-host plan-render"), "{err}");
}

#[test]
fn bad_input_exits_2_with_the_usage() {
    assert_bad_input(
        &run(&[
            "plan-render",
            "--song",
            SONG,
            "--request",
            REQUEST,
            "--bogus",
            "1",
        ]),
        "unknown argument --bogus",
    );
    assert_bad_input(
        &run(&[
            "plan-render",
            "--song",
            SONG,
            "--song",
            SONG,
            "--request",
            REQUEST,
        ]),
        "--song given twice",
    );
    assert_bad_input(&run(&["plan-render", "--song"]), "--song needs a value");
    assert_bad_input(
        &run(&["plan-render", "--request", REQUEST]),
        "missing --song",
    );
    assert_bad_input(&run(&["render"]), "unknown command render");
    assert_bad_input(&run(&[]), "missing command");
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
    // The single chunk equals the single-chunk plan of its request.
    if chunks.len() == 1 {
        let lookback = &chunks[0]["plan"]["lookbackSec"];
        let request = format!(
            r#"{{"length":{{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"}},"chunk":{{"overlapBars":0,"lookbackSec":{lookback},"mode":"mix"}}}}"#
        );
        let single = run(&["plan-render", "--song", SONG, "--request", &request]);
        assert_eq!(single.status.code(), Some(0), "{single:?}");
        assert_eq!(single_chunk(&single)["json"], chunks[0]["json"]);
        assert!(chunks[0].get("info").is_none(), "{planned}");
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
        // A chunk the bundle refuses.
        (
            r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"},"chunk":{"overlapBars":99997,"lookbackSec":0,"mode":"mix"}}"#,
            "bad request: chunk.overlapBars must be an integer from 0 to 99996",
        ),
        (
            r#"{"length":{"kind":"length","bars":4},"chunk":{"overlapBars":0,"lookbackSec":0,"mode":"mix"}}"#,
            "bad request: chunk needs a window length",
        ),
        (
            r#"{"length":{"kind":"clip"},"preset":"one-shot","chunk":{"overlapBars":0,"lookbackSec":0,"mode":"mix"}}"#,
            "bad request: chunk needs a window length",
        ),
        (
            r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"},"preset":"one-shot","chunk":{"overlapBars":0,"lookbackSec":0,"mode":"mix"}}"#,
            "bad request: chunk cannot be combined with a preset",
        ),
        // The chunk values a typed `SingleChunk` cannot hold: serde refuses them.
        (
            r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"},"chunk":{"overlapBars":-1,"lookbackSec":0,"mode":"mix"}}"#,
            "invalid request",
        ),
        (
            r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"},"chunk":{"overlapBars":0.5,"lookbackSec":0,"mode":"mix"}}"#,
            "invalid request",
        ),
        (
            r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"},"chunk":{"overlapBars":0,"lookbackSec":"x","mode":"mix"}}"#,
            "invalid request",
        ),
        (
            r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"},"chunk":{"overlapBars":0,"lookbackSec":null,"mode":"mix"}}"#,
            "invalid request",
        ),
        (
            r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"},"chunk":{"overlapBars":0,"lookbackSec":[],"mode":"mix"}}"#,
            "invalid request",
        ),
        (
            r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"},"chunk":{"overlapBars":0,"lookbackSec":0,"mode":"mono"}}"#,
            "invalid request",
        ),
        (
            r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"},"chunk":{"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100}}"#,
            "unknown field `sampleRate`",
        ),
        (
            r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"linear"},"chunk":3}"#,
            "invalid request",
        ),
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
fn plan_render_timeout_sec_bounds_the_planning() {
    let with_timeout =
        |secs: &str| format!(r#"{},"timeoutSec":{secs}}}"#, &REQUEST[..REQUEST.len() - 1]);
    let timed_request = with_timeout("600");
    assert!(
        timed_request.ends_with(r#""mode":"mix"},"timeoutSec":600}"#),
        "{timed_request}"
    );
    let plain = run(&["plan-render", "--song", SONG, "--request", REQUEST]);
    let timed = run(&["plan-render", "--song", SONG, "--request", &timed_request]);
    assert_eq!(timed.status.code(), Some(0), "{timed:?}");
    assert_eq!(timed.stdout, plain.stdout);
    assert_eq!(timed.stderr, plain.stderr);

    let spin = temp_file(SPIN);
    let spin_request = with_timeout("0.5");
    let out = run(&[
        "plan-render",
        "--song",
        spin.path().to_str().unwrap(),
        "--request",
        &spin_request,
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty());
    assert_eq!(
        one_line(&out),
        "strudeland-host: timeout: planning did not finish within 0.5 s"
    );

    for bad in ["0", "-1", "86401"] {
        assert_bad_input(
            &run(&[
                "plan-render",
                "--song",
                SONG,
                "--request",
                &with_timeout(bad),
            ]),
            "timeoutSec",
        );
    }
    assert_bad_input(
        &run(&[
            "plan-render",
            "--song",
            SONG,
            "--request",
            &with_timeout(r#""5""#),
        ]),
        "invalid request",
    );
}
