//! The `strudeland-engine` binary end to end: jobs from a file and stdin, the WAV it writes, the
//! summary, `signatures`, `controls --json` and the exit codes.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use strudeland_engine::wav::read_f32;

mod common;
use common::{PauseHap, base_job, read_json};

const BIN: &str = env!("CARGO_BIN_EXE_strudeland-engine");

fn job(channels: u32, values: Value) -> Value {
    base_job(
        22050,
        channels,
        values,
        [
            PauseHap {
                suspend_sec: 0.0,
                t: 0.0,
                duration: 0.2,
            },
            PauseHap {
                suspend_sec: 0.25,
                t: 0.3,
                duration: 0.1,
            },
        ],
    )
}

fn sine() -> Value {
    json!([{ "s": "sine", "note": 69 }])
}

/// A job with one pause per value: hap `j` plays value `j` at cycle `j + 0.5` (bar `j`), as
/// `table_job` in `src/controls.rs`.
fn table_job(values: &Value) -> Value {
    let n = values.as_array().unwrap().len();
    let pauses: Vec<Value> = (0..n)
        .map(|j| {
            let c = j as f64 + 0.5;
            json!({
                "suspendSec": if j == 0 { 0.0 } else { j as f64 - 0.05 },
                "haps": [{ "t": c, "duration": 0.25, "cycle": c, "v": j, "event": j }]
            })
        })
        .collect();
    json!({
        "schema": 2, "sampleRate": 44100, "frames": 44100 * n.max(1), "seed": 0, "cps": 1,
        "channels": { "count": 2, "multiChannelOrbits": false },
        "orbits": [{ "orbit": 1, "channels": [0, 1] }],
        "pauses": pauses,
        "values": values
    })
}

fn stdout_of(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

fn run(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(BIN)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    pipe.write_all(stdin.unwrap_or("").as_bytes()).unwrap();
    drop(pipe);
    child.wait_with_output().unwrap()
}

fn summary(out: &Output) -> Value {
    let text = String::from_utf8(out.stdout.clone()).unwrap();
    assert_eq!(text.lines().count(), 1, "one summary line: {text}");
    serde_json::from_str(&text).unwrap()
}

fn render_file(dir: &Path, job: &Value) -> (Output, std::path::PathBuf) {
    let job_path = dir.join("job.json");
    std::fs::write(&job_path, job.to_string()).unwrap();
    let out = dir.join("out.wav");
    let o = run(
        &[
            "render",
            "--job",
            job_path.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ],
        None,
    );
    (o, out)
}

#[test]
fn renders_a_job_file_to_a_stereo_wav() {
    let dir = tempfile::tempdir().unwrap();
    let (o, wav) = render_file(dir.path(), &job(2, sine()));
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let s = summary(&o);
    assert_eq!(s["ok"], true);
    assert_eq!(s["frames"], 22050);
    assert_eq!(s["channels"], 2);
    assert_eq!(s["errors"], json!([]));
    assert!(s["renderMs"].is_u64());
    let bytes = std::fs::read(&wav).unwrap();
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(
        u16::from_le_bytes([bytes[20], bytes[21]]),
        3,
        "plain float format"
    );
    assert_eq!(bytes.len(), 44 + 22050 * 2 * 4);
    let d = read_f32(&bytes).unwrap();
    assert_eq!(d.sample_rate, 44100);
    assert_eq!(d.channels.len(), 2);
    assert_eq!(d.channels[0].len(), 22050);
    let peak = d.channels[0].iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak > 0.05 && peak <= 0.24 + 1e-6, "peak {peak}");
    assert!(d.channels.iter().flatten().all(|x| x.is_finite()));
}

#[test]
fn renders_a_job_from_stdin_with_six_channels() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("six.wav");
    let o = run(
        &["render", "--out", out.to_str().unwrap(), "--job", "-"],
        Some(&job(6, sine()).to_string()),
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(summary(&o)["channels"], 6);
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(
        u16::from_le_bytes([bytes[20], bytes[21]]),
        0xfffe,
        "extensible above 2 channels"
    );
    assert_eq!(bytes.len(), 68 + 22050 * 6 * 4);
    let d = read_f32(&bytes).unwrap();
    assert_eq!(d.channels.len(), 6);
    assert!(d.channels.iter().all(|c| c.len() == 22050));
}

#[test]
fn a_stems_job_writes_each_orbit_to_its_pair() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("stems.wav");
    let mut j = job(
        6,
        json!([
            { "s": "sine", "note": 69, "orbit": 1 },
            { "s": "sine", "note": 57, "orbit": 3 }
        ]),
    );
    j["orbits"] = json!([
        { "orbit": 1, "channels": [0, 1] },
        { "orbit": 2, "channels": [2, 3] },
        { "orbit": 3, "channels": [4, 5] }
    ]);
    j["pauses"][1]["haps"][0]["v"] = json!(1);
    let o = run(
        &["render", "--out", out.to_str().unwrap(), "--job", "-"],
        Some(&j.to_string()),
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let d = read_f32(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(d.channels.len(), 6);
    let peak = |c: &[f32]| c.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak(&d.channels[0]) > 0.05);
    assert_eq!(
        d.channels[0], d.channels[1],
        "a mono voice on both sides of its pair"
    );
    assert!(
        d.channels[2]
            .iter()
            .chain(&d.channels[3])
            .all(|x| *x == 0.0),
        "orbit 2 has no haps"
    );
    assert!(peak(&d.channels[4]) > 0.05);
    assert_eq!(d.channels[4], d.channels[5]);
    // Orbit 1 plays only in the first half, orbit 3 only in the second.
    assert!(d.channels[0][13000..].iter().all(|x| *x == 0.0));
    assert!(d.channels[4][..13000].iter().all(|x| *x == 0.0));
}

#[test]
fn render_errors_exit_one_and_still_write_the_wav() {
    let dir = tempfile::tempdir().unwrap();
    let (o, wav) = render_file(dir.path(), &job(2, json!([{ "s": "bytebeat", "ir": "x" }])));
    assert_eq!(o.status.code(), Some(1));
    let s = summary(&o);
    assert_eq!(s["ok"], false);
    assert_eq!(
        s["errors"],
        json!([{ "kind": "unsupported", "message": "s=bytebeat keys=ir", "bar": 0 }])
    );
    let d = read_f32(&std::fs::read(wav).unwrap()).unwrap();
    assert!(d.channels.iter().flatten().all(|x| *x == 0.0));
    // A hap error (not a note) is exit 1 too.
    let (o, _) = render_file(dir.path(), &job(2, json!([{ "s": "sine", "note": "zz" }])));
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(summary(&o)["errors"][0]["kind"], "hap");
}

#[test]
fn bad_input_exits_two_without_a_wav() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("x.wav");
    let out_s = out.to_str().unwrap();
    let mut bad = job(2, sine());
    bad["schema"] = json!(1);
    let cases: Vec<(Vec<&str>, Option<String>)> = vec![
        (
            vec!["render", "--job", "-", "--out", out_s],
            Some(bad.to_string()),
        ),
        (
            vec!["render", "--job", "-", "--out", out_s],
            Some("{not json".into()),
        ),
        (
            vec!["render", "--job", "/nonexistent/job.json", "--out", out_s],
            None,
        ),
        (
            vec!["render", "--job", "-", "--out", "/nonexistent/dir/x.wav"],
            Some(job(2, sine()).to_string()),
        ),
        (vec!["render", "--job", "-"], None),
        (vec!["render", "--job", "-", "--out", out_s, "--fast"], None),
        (
            vec!["render", "--job", "-", "--job", "-", "--out", out_s],
            None,
        ),
        (vec!["render", "--job"], None),
        (vec!["signatures"], None),
        (vec!["signatures", "--job"], None),
        (vec!["signatures", "--job", "-", "--out", "x"], None),
        (vec!["signatures", "--job", "-"], Some(bad.to_string())),
        (vec!["signatures", "--job", "/nonexistent/job.json"], None),
        (vec!["controls"], None),
        (vec!["controls", "--yaml"], None),
        (vec!["play"], None),
        (vec![], None),
    ];
    for (args, stdin) in cases {
        let o = run(&args, stdin.as_deref());
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(o.stdout.is_empty(), "{args:?}");
        assert!(!o.stderr.is_empty(), "{args:?}");
        assert!(!out.exists(), "{args:?} wrote a WAV");
    }
    let o = run(
        &["render", "--job", "-", "--out", out_s],
        Some(&bad.to_string()),
    );
    assert!(String::from_utf8_lossy(&o.stderr).contains("schema 1 job: regenerate it"));
    let o = run(&["signatures", "--job", "-", "--out", "x"], None);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.starts_with("strudeland-engine: unknown argument --out\nusage:"),
        "{err}"
    );
    let o = run(&["signatures"], None);
    assert!(String::from_utf8_lossy(&o.stderr).contains("missing --job"));
    let o = run(&["signatures", "--job", "a", "--job", "b"], None);
    assert!(String::from_utf8_lossy(&o.stderr).contains("--job given twice"));
}

#[test]
fn signatures_prints_one_line_of_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job.json");
    let values = json!([{ "s": "sine" }, { "s": "sine", "ir": "x" }, { "s": "z_noise" }]);
    std::fs::write(&path, table_job(&values).to_string()).unwrap();
    let o = run(&["signatures", "--job", path.to_str().unwrap()], None);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(
        stdout_of(&o),
        "[{\"s\":\"sine\",\"keys\":[\"ir\"],\"firstBar\":1},{\"s\":\"z_noise\",\"keys\":[],\"firstBar\":2}]\n"
    );
    assert!(o.stderr.is_empty());
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "no files written"
    );
}

#[test]
fn signatures_of_a_supported_job_is_empty() {
    let o = run(
        &["signatures", "--job", "-"],
        Some(&job(2, sine()).to_string()),
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(stdout_of(&o), "[]\n");
    // Silence markers are never a signature.
    let o = run(
        &["signatures", "--job", "-"],
        Some(&table_job(&json!([{ "s": "~", "ir": "x" }])).to_string()),
    );
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(stdout_of(&o), "[]\n");
}

#[test]
fn signatures_reads_stdin_and_rejects_schema_1() {
    let mut bad = job(2, sine());
    bad["schema"] = json!(1);
    let o = run(&["signatures", "--job", "-"], Some(&bad.to_string()));
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(
        err,
        "strudeland-engine: invalid job:\nschema 1 job: regenerate it\n"
    );
    // A job read from stdin with an unsupported value.
    let o = run(
        &["signatures", "--job", "-"],
        Some(&job(2, json!([{ "s": "wt_flute" }])).to_string()),
    );
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(
        stdout_of(&o),
        "[{\"s\":\"wt_flute\",\"keys\":[],\"firstBar\":0}]\n"
    );
    // Not JSON at all.
    let o = run(&["signatures", "--job", "-"], Some("{"));
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("strudeland-engine: invalid job:"));
}

#[test]
fn signatures_match_the_shared_table() {
    let table = read_json(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/unsupported-signatures.json"),
    );
    let cases = table.as_array().unwrap();
    assert!(cases.len() >= 30, "{} cases", cases.len());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let o = run(
            &["signatures", "--job", "-"],
            Some(&table_job(&case["values"]).to_string()),
        );
        assert_eq!(
            o.status.code(),
            Some(0),
            "{name}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let text = stdout_of(&o);
        assert_eq!(text.lines().count(), 1, "{name}: {text}");
        let got: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(got, case["expected"], "case {name}");
    }
}

#[test]
fn an_existing_output_is_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("out.wav"), b"old").unwrap();
    let (o, wav) = render_file(dir.path(), &job(2, sine()));
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(std::fs::metadata(wav).unwrap().len(), 44 + 22050 * 8);
}

#[test]
fn controls_json_prints_the_allowlist() {
    let o = run(&["controls", "--json"], None);
    assert_eq!(o.status.code(), Some(0));
    let text = String::from_utf8(o.stdout).unwrap();
    assert_eq!(text.lines().count(), 1);
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["schema"], 2);
    let keys: Vec<&str> = v["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    assert_eq!(keys, strudeland_engine::ALLOWED_KEYS);
    let sounds: Vec<&str> = v["sounds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    assert_eq!(sounds, strudeland_engine::ALLOWED_SOUNDS);
    assert_eq!(v["unsupportedValues"], json!({}));
    assert_eq!(
        v["chainKeys"],
        json!(strudeland_engine::controls::CHAIN_KEYS)
    );
    assert!(v["chainKeys"].as_array().unwrap().contains(&json!("delay")));
    let mk = v["modulatorKeys"].as_object().unwrap();
    assert_eq!(mk.keys().collect::<Vec<_>>(), ["bmod", "env", "lfo"]);
    assert!(mk["lfo"].as_array().unwrap().contains(&json!("rate")));
    assert!(mk["env"].as_array().unwrap().contains(&json!("attack")));
    assert!(mk["bmod"].as_array().unwrap().contains(&json!("bus")));
    let targets = v["modulatorTargets"].as_array().unwrap();
    assert!(targets.contains(&json!("cutoff")) && !targets.contains(&json!("lprate")));
    assert_eq!(v.as_object().unwrap().len(), 7);
    assert_eq!(text.trim_end(), strudeland_engine::controls_json());
}
