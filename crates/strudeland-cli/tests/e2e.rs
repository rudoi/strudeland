//! Runs the real `strudeland` binary against a fake control server.

#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Output};
#[cfg(unix)]
use std::thread::JoinHandle;

use serde_json::Value;
#[cfg(unix)]
use serde_json::json;

/// Binds `path`, accepts one connection and answers each request line with the script entry
/// whose method matches: `{"error": {...}}` is sent as an error, anything else as the result.
#[cfg(unix)]
fn spawn_fake_server(path: &Path, script: Vec<(&'static str, Value)>) -> JoinHandle<()> {
    let listener = UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let Ok(line) = line else { return };
            let req: Value = serde_json::from_str(&line).unwrap();
            let method = req["method"].as_str().unwrap().to_string();
            let entry = script.iter().find(|(m, _)| *m == method).map(|(_, v)| v);
            let mut resp = json!({"jsonrpc": "2.0", "id": req["id"]});
            match entry {
                Some(v) if v.get("error").is_some() => resp["error"] = v["error"].clone(),
                Some(v) => resp["result"] = v.clone(),
                None => resp["error"] = json!({"code": -32601, "message": "unknown method"}),
            }
            writeln!(writer, "{resp}").unwrap();
        }
    })
}

fn strudeland(socket: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_strudeland"))
        .arg("--socket")
        .arg(socket)
        .args(args)
        .env_remove("STRUDELAND_SOCKET")
        .output()
        .unwrap()
}

#[cfg(unix)]
fn short_dir() -> tempfile::TempDir {
    // macOS limits socket paths to 103 bytes; $TMPDIR is long, /tmp is not.
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .prefix("e")
        .tempdir_in("/tmp")
        .unwrap()
}

#[test]
#[cfg(unix)]
fn state_against_a_fake_server() {
    let dir = short_dir();
    let sock = dir.path().join("s");
    let server = spawn_fake_server(
        &sock,
        vec![(
            "state",
            json!({
                "project": {"name": "demo", "root": "/tmp/demo"},
                "songs": [{"path": "songs/main.js", "title": "Main", "dirty": false, "conflict": null}],
                "active": "songs/main.js",
                "playing": null,
                "started": false, "cps": 0.5, "bpm": 120, "meter": "4/4", "audio": "suspended",
                "evalError": null, "metaError": null, "tempoOverride": false
            }),
        )],
    );
    let out = strudeland(&sock, &["state"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "project: demo (/tmp/demo)\nsongs:\n    main  \"Main\"   active\ntempo: 120 bpm 4/4\naudio: suspended\nplaying: stopped\n"
    );
    assert!(out.stderr.is_empty());
    server.join().unwrap();
}

#[test]
#[cfg(unix)]
fn rpc_error_exits_1() {
    let dir = short_dir();
    let sock = dir.path().join("s");
    let server = spawn_fake_server(
        &sock,
        vec![(
            "eval",
            json!({"error": {"code": -32004, "message": "no project open"}}),
        )],
    );
    let out = strudeland(&sock, &["eval"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        "error: no project open\n"
    );
    server.join().unwrap();
}

#[test]
#[cfg(unix)]
fn not_running_exits_2() {
    let dir = short_dir();
    let sock = dir.path().join("none");
    let out = strudeland(&sock, &["state"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(
        err.contains("strudeland is not running (no socket at "),
        "{err}"
    );
}

#[test]
fn usage_errors_exit_4_and_help_exits_0() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("none");
    assert_eq!(
        strudeland(&sock, &["--timeout", "0", "ping"]).status.code(),
        Some(4)
    );
    assert_eq!(strudeland(&sock, &["bogus"]).status.code(), Some(4));
    assert_eq!(strudeland(&sock, &["--help"]).status.code(), Some(0));
}

#[test]
#[cfg(unix)]
fn events_against_a_fake_server() {
    let dir = short_dir();
    let sock = dir.path().join("s");
    let result = json!({
        "song": {"path": "songs/main.js", "title": "Main"},
        "from": 1, "to": 2, "stale": false, "truncated": true, "meter": "4/4",
        "tempoOverride": false,
        "events": [{
            "bar": 1, "beat": 1.0, "begin": 0.0, "end": 0.25, "onset": true,
            "section": {"name": "intro", "bar": 1},
            "value": {"s": "bd"},
            "locations": [{"line": 2, "col": 5, "endLine": 2, "endCol": 7, "text": "s(\"bd\")"}]
        }]
    });
    let server = spawn_fake_server(&sock, vec![("events", result)]);
    let out = strudeland(&sock, &["events", "--bars", "1-2", "--all", "--limit", "5"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "song: main \"Main\"\nbars: 1-2 (4/4)\n\nbar 1  intro 1\n  1  s=bd @ 2:5\n"
    );
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        "warning: showing the first 1 events; narrow --bars or raise --limit\n"
    );
    server.join().unwrap();
}

#[test]
#[cfg(unix)]
fn events_sends_the_expected_params() {
    let dir = short_dir();
    let sock = dir.path().join("s");
    let listener = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let req: Value = serde_json::from_str(&line).unwrap();
        let resp = json!({"jsonrpc": "2.0", "id": req["id"],
            "error": {"code": -32007, "message": "nothing has been evaluated"}});
        writeln!(writer, "{resp}").unwrap();
        req
    });
    let out = strudeland(&sock, &["events", "--section", "intro", "--limit", "7"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        "error: nothing has been evaluated\n"
    );
    let req = server.join().unwrap();
    assert_eq!(req["method"], "events");
    assert_eq!(req["params"], json!({"section": "intro", "limit": 7}));
}

#[test]
fn events_usage_errors_exit_4() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("none");
    for args in [
        &["events", "--bars", "5-3"][..],
        &["events", "--bars", "1", "--section", "x"],
        &["events", "--limit", "0"],
    ] {
        assert_eq!(strudeland(&sock, args).status.code(), Some(4), "{args:?}");
    }
}

#[test]
#[cfg(unix)]
fn screenshot_against_a_fake_server() {
    let dir = short_dir();
    let sock = dir.path().join("s");
    let listener = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let req: Value = serde_json::from_str(&line).unwrap();
        let path = req["params"]["path"].clone();
        let resp = json!({"jsonrpc": "2.0", "id": req["id"],
            "result": {"path": path, "width": 1200, "height": 800}});
        writeln!(writer, "{resp}").unwrap();
        req
    });
    let out = Command::new(env!("CARGO_BIN_EXE_strudeland"))
        .current_dir(dir.path())
        .arg("--socket")
        .arg(&sock)
        .args(["screenshot", "shot.png"])
        .env_remove("STRUDELAND_SOCKET")
        .output()
        .unwrap();
    let req = server.join().unwrap();
    assert_eq!(req["method"], "screenshot");
    let sent = req["params"]["path"].as_str().unwrap().to_string();
    // Resolved against the CLI's directory (which may be reached through a symlink, like /tmp).
    assert!(Path::new(&sent).is_absolute(), "{sent}");
    assert!(sent.ends_with("/shot.png"), "{sent}");
    assert_eq!(
        std::fs::canonicalize(Path::new(&sent).parent().unwrap()).unwrap(),
        std::fs::canonicalize(dir.path()).unwrap()
    );
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("wrote {sent} (1200x800)\n")
    );
}

#[test]
fn skill_install_writes_under_home_without_a_socket() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_strudeland"))
        .args(["skill", "install"])
        .env("HOME", home.path())
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("STRUDELAND_SOCKET")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let path = home.path().join(".claude/skills/strudeland/SKILL.md");
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("installed "));
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("name: strudeland")
    );
}

fn corpus(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/render/corpus")
        .join(name)
}

/// `strudeland render ...` through the real binary, real host and real engine.
fn render(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_strudeland"))
        .arg("render")
        .args(args)
        .env_remove("STRUDELAND_SOCKET")
        .output()
        .unwrap()
}

#[test]
fn render_writes_a_wav_through_the_real_pipeline() {
    use std::ffi::OsStr;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("sine.wav");
    let song = corpus("sine-basic.js");
    let o = render(&[
        song.as_os_str(),
        OsStr::new("--length"),
        OsStr::new("4"),
        OsStr::new("--out"),
        out.as_os_str(),
        OsStr::new("--json"),
    ]);
    let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
    assert_eq!(o.status.code(), Some(0), "{stderr}");
    let stdout = String::from_utf8(o.stdout).unwrap();
    assert_eq!(stdout.matches('\n').count(), 1, "{stdout}");
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["frames"], 352_800);
    assert_eq!(v["format"], "wav");
    assert_eq!(v["sampleRate"], 44_100);
    assert_eq!(v["window"]["fromBar"], 0);
    assert_eq!(v["window"]["toBar"], 4);
    let bytes = std::fs::read(&out).unwrap();
    // 24-bit stereo PCM: a 44-byte header and 6 bytes per frame.
    assert_eq!(bytes.len(), 44 + 352_800 * 6);
    assert_eq!(v["bytes"], bytes.len());
    assert_eq!(&bytes[..4], b"RIFF");
    assert!(stderr.contains("planned "), "{stderr}");
    assert!(stderr.contains("wrote "), "{stderr}");
    let tp = v["loudness"]["outputTruePeakDb"].as_f64().unwrap();
    assert!(tp <= -1.0 + 1e-6, "true peak {tp}");
}

fn wav_chunk<'a>(wav: &'a [u8], name: &[u8; 4]) -> Option<&'a [u8]> {
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    let mut cursor = 12;
    while cursor + 8 <= wav.len() {
        let size = u32::from_le_bytes(wav[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let start = cursor + 8;
        if &wav[cursor..cursor + 4] == name {
            return Some(&wav[start..start + size]);
        }
        cursor = start + size + (size % 2);
    }
    None
}

#[test]
fn render_exports_a_wav_intro_and_repeatable_loop_through_the_real_pipeline() {
    use std::ffi::OsStr;
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("score.js");
    std::fs::write(
        &song,
        r#"
const meta = {
  bpm: 240, meter: '4/4',
  sections: [
    { name: 'intro', bars: 1 },
    { name: 'body', bars: 1, loop: true },
    { name: 'ending', bars: 1 },
  ],
  loops: 3,
};
$: song(meta, {
  intro: note('a3').s('sine'),
  body: note('a4').s('sine'),
  ending: note('a5').s('sine'),
});
"#,
    )
    .unwrap();
    let out = dir.path().join("music.wav");
    let o = render(&[
        song.as_os_str(),
        OsStr::new("--preset"),
        OsStr::new("intro-loop"),
        OsStr::new("--out"),
        out.as_os_str(),
        OsStr::new("--bit-depth"),
        OsStr::new("16"),
        OsStr::new("--loudness"),
        OsStr::new("off"),
        OsStr::new("--json"),
    ]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let json: Value = serde_json::from_slice(&o.stdout).unwrap();
    let intro_path = dir.path().join("music-intro.wav");
    let loop_path = dir.path().join("music-loop.wav");
    assert_eq!(json["preset"], "intro-loop");
    let files = json["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["role"], "intro");
    assert_eq!(files[0]["path"], intro_path.to_str().unwrap());
    assert_eq!(files[0]["loopPoints"], Value::Null);
    assert_eq!(files[1]["role"], "loop");
    assert_eq!(files[1]["path"], loop_path.to_str().unwrap());
    assert_eq!(
        files[1]["loopPoints"],
        serde_json::json!({
            "startFrame": 0, "lengthFrames": 44_100, "sampleRate": 44_100, "godotLoopOffset": 0.0,
        })
    );
    assert!(!out.exists());

    let intro = std::fs::read(intro_path).unwrap();
    let body = std::fs::read(loop_path).unwrap();
    assert!(wav_chunk(&intro, b"smpl").is_none());
    let marker = wav_chunk(&body, b"smpl").unwrap();
    assert_eq!(u32::from_le_bytes(marker[44..48].try_into().unwrap()), 0);
    assert_eq!(
        u32::from_le_bytes(marker[48..52].try_into().unwrap()),
        44_099
    );
    let intro = wav_chunk(&intro, b"data").unwrap();
    let body = wav_chunk(&body, b"data").unwrap();
    assert_eq!(intro.len(), 44_100 * 4);
    assert_eq!(body.len(), 44_100 * 4);

    // Play the intro once, then the body twice. Count positive zero crossings in the
    // middle half second of each asset to check that only the body pitch repeats.
    let playback = [intro, body, body].concat();
    for (bar, crossings) in [110, 220, 220].into_iter().enumerate() {
        let start = bar * 44_100 + 11_025;
        let samples: Vec<_> = playback[start * 4..(start + 22_050) * 4]
            .chunks_exact(4)
            .map(|frame| i16::from_le_bytes([frame[0], frame[1]]))
            .collect();
        let actual = samples
            .windows(2)
            .filter(|pair| pair[0] <= 0 && pair[1] > 0)
            .count();
        assert!(actual.abs_diff(crossings) <= 1, "bar {bar}: {actual}");
    }
}

#[test]
fn render_loop_preset_requires_a_marked_loop() {
    use std::ffi::OsStr;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("missing-loop.wav");
    let song = corpus("sine-basic.js");
    let o = render(&[
        song.as_os_str(),
        OsStr::new("--preset"),
        OsStr::new("folded-loop"),
        OsStr::new("--out"),
        out.as_os_str(),
    ]);
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(4), "{stderr}");
    assert!(stderr.contains("loop: true"), "{stderr}");
    assert!(!out.exists());
}

#[test]
fn render_without_sections_needs_a_length() {
    use std::ffi::OsStr;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("x.wav");
    let song = corpus("sine-basic.js");
    let o = render(&[song.as_os_str(), OsStr::new("--out"), out.as_os_str()]);
    assert_eq!(
        o.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!out.exists());
}

#[test]
fn render_refuses_unsupported_sounds_and_writes_nothing() {
    use std::ffi::OsStr;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("x.wav");
    let song = corpus("samples.js");
    let o = render(&[
        song.as_os_str(),
        OsStr::new("--length"),
        OsStr::new("2"),
        OsStr::new("--out"),
        out.as_os_str(),
    ]);
    let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
    assert_eq!(o.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("first at bar"), "{stderr}");
    assert!(!out.exists());
}

#[cfg(windows)]
#[test]
fn cli_uses_a_verified_windows_control_pipe() {
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let socket = std::path::PathBuf::from(format!(
        r"\\.\pipe\strudeland-cli-test-{}",
        std::process::id()
    ));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let server = {
        let _entered = runtime.enter();
        strudeland_ctl::windows_pipe::listen(&socket, true).unwrap()
    };
    let server = std::thread::spawn(move || {
        runtime.block_on(async move {
            server.connect().await.unwrap();
            let mut server = BufReader::new(server);
            let mut line = String::new();
            server.read_line(&mut line).await.unwrap();
            let req: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(req["method"], "ping");
            let response = json!({"jsonrpc":"2.0", "id":req["id"], "result":{"version":"test","attached":true}});
            server.get_mut().write_all(format!("{response}\n").as_bytes()).await.unwrap();
            line.clear();
            assert_eq!(server.read_line(&mut line).await.unwrap(), 0);
        });
    });
    let output = strudeland(&socket, &["--timeout", "1", "ping"]);
    assert!(output.status.success(), "{output:?}");
    server.join().unwrap();
    let output = strudeland(&socket, &["--timeout", "1", "ping"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
}
