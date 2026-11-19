//! Pure formatting of results into the CLI's human output.

use crate::docs::DocEntry;
use strudeland_ctl::{
    AppState, Audio, CodeSetResult, Conflict, EngineId, EvalOutcome, Event, EventsResult, Levels,
    PingResult, ProjectOpenResult, SaveResult, ScreenshotResult, SongDeleteResult,
    SongRenameResult, SongResult, TransportView,
};
use strudeland_project::{RecordingEntry, Sidecar, SidecarBacking};
use strudeland_transcribe::{KeySource, Transcription, consts::LOW_CONFIDENCE};

pub const SUSPENDED_WARNING: &str =
    "warning: audio is suspended: click in the strudeland window to enable it";
pub const CHANGED_NOTE: &str = "note: the file changed on disk; the conflict bar is still open";
pub const DELETED_NOTE: &str = "note: the file was deleted on disk";
pub const PLAN_TIMEOUT_HINT: &str = "(raise it with --plan-timeout)";
pub const TIMEOUT_HINT: &str = "(raise it with --timeout)";
pub const UNSAVED_HINT: &str =
    "hint: pass --discard to throw the unsaved edits away, or ask the user to save them first";
pub const BUSY_HINT: &str =
    "hint: the app is waiting on a dialog; ask the user to finish it, then try again";

/// What a command prints, and whether it counts as a failure (exit 1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    pub stdout: String,
    pub stderr: String,
    pub failed: bool,
}

/// The file stem of a song path: `songs/main.js` is `main`.
pub fn song_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map_or(name, |(stem, _)| stem)
}

fn number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn audio_str(a: Audio) -> &'static str {
    match a {
        Audio::Running => "running",
        Audio::Suspended => "suspended",
    }
}

pub fn format_state(s: &AppState) -> String {
    let mut out = String::new();
    match &s.project {
        Some(p) => out.push_str(&format!("project: {} ({})\n", p.name, p.root)),
        None => out.push_str("project: none\n"),
    }
    if s.project.is_some() {
        out.push_str("songs:\n");
        let stems: Vec<&str> = s.songs.iter().map(|x| song_stem(&x.path)).collect();
        let titles: Vec<String> = s.songs.iter().map(|x| format!("\"{}\"", x.title)).collect();
        let name_w = stems.iter().map(|x| x.chars().count()).max().unwrap_or(0);
        let title_w = titles.iter().map(|x| x.chars().count()).max().unwrap_or(0);
        for (i, song) in s.songs.iter().enumerate() {
            let mut flags: Vec<String> = Vec::new();
            if s.active.as_deref() == Some(song.path.as_str()) {
                flags.push("active".into());
            }
            if s.playing.as_ref().is_some_and(|p| p.path == song.path) {
                flags.push("playing".into());
            }
            match song.conflict {
                Some(Conflict::Changed) => flags.push("conflict: changed".into()),
                Some(Conflict::Deleted) => flags.push("conflict: deleted".into()),
                None => {}
            }
            let mark = if song.dirty { '●' } else { ' ' };
            let mut line = format!(
                "  {mark} {:<name_w$}  {}",
                stems[i],
                titles[i],
                name_w = name_w
            );
            if !flags.is_empty() {
                let pad = title_w - titles[i].chars().count();
                line.push_str(&" ".repeat(pad + 3));
                line.push_str(&flags.join(", "));
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push_str(&format!("tempo: {} bpm {}", number(s.bpm), s.meter));
    if s.tempo_override {
        out.push_str(" (setcps in the code overrides meta.bpm)");
    }
    out.push('\n');
    if let Some(seed) = s.seed {
        out.push_str(&format!("seed: {}\n", number(seed)));
    }
    out.push_str(&format!("audio: {}\n", audio_str(s.audio)));
    if s.vim {
        out.push_str("keys: vim\n");
    }
    if s.engine == EngineId::Rust {
        out.push_str("engine: rust\n");
    }
    if let Some(l) = &s.live {
        out.push_str(&format!(
            "live: underruns {}, late {}, haps {}, batches {}, skipped {}, headroom {} ms, decode p50 {} us max {} us, invoke p50 {} ms p95 {} ms max {} ms\n",
            l.underruns,
            l.late_haps,
            l.haps,
            l.batches,
            l.skipped_haps,
            trim_decimals(l.render_headroom_ms, 1),
            l.batch_decode_us.p50,
            l.batch_decode_us.max,
            trim_decimals(l.invoke_rtt_ms.p50, 1),
            trim_decimals(l.invoke_rtt_ms.p95, 1),
            trim_decimals(l.invoke_rtt_ms.max, 1),
        ));
    }
    if let Some(l) = &s.levels {
        out.push_str(&levels_line(l));
    }
    match &s.playing {
        Some(p) => out.push_str(&format!("playing: {}\n", p.title)),
        None => out.push_str("playing: stopped\n"),
    }
    if let Some(p) = &s.playhead {
        match (p.song_bar, &p.section) {
            (Some(bar), Some(sec)) => {
                out.push_str(&format!("playhead: bar {bar} ({} {})\n", sec.name, sec.bar));
            }
            (Some(bar), None) => out.push_str(&format!("playhead: bar {bar}\n")),
            (None, _) => {
                let cycle = trim_decimals(p.cycle, 2);
                out.push_str(&format!("playhead: cycle {cycle}\n"));
            }
        }
    }
    let mut parts: Vec<String> = Vec::new();
    match s.transport.view {
        TransportView::Song => {}
        TransportView::Loop => parts.push(match &s.transport.loop_section {
            Some(name) => format!("loop {name}"),
            None => "loop".into(),
        }),
        TransportView::Release => parts.push("ending".into()),
    }
    if let Some(name) = &s.transport.armed_section {
        parts.push(format!("armed {name}"));
    }
    if !parts.is_empty() {
        out.push_str(&format!("transport: {}\n", parts.join(", ")));
    }
    if let Some(e) = &s.eval_error {
        out.push_str(&format!("eval error: {e}\n"));
    }
    if let Some(e) = &s.meta_error {
        out.push_str(&format!("meta error: {e}\n"));
    }
    if let Some(e) = &s.output_error {
        out.push_str(&format!("sound error: {e}\n"));
    }
    out
}

/// `M:SS.s` from seconds (total tenths rounded); `?` when unknown.
fn memo_duration(seconds: Option<f64>) -> String {
    seconds.map_or_else(|| "?".into(), clock_text)
}

fn backing_names(b: &SidecarBacking, sep: &str) -> String {
    let names: Vec<&str> = [
        (b.song, "song"),
        (b.metronome, "metronome"),
        (b.count_in, "count-in"),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .map(|(_, n)| *n)
    .collect();
    if names.is_empty() {
        "none".into()
    } else {
        names.join(sep)
    }
}

/// `memo list` human output: one aligned row per memo, or `no memos`; a bad sidecar's reason goes
/// to stderr.
pub fn format_memo_list(entries: &[RecordingEntry]) -> Report {
    if entries.is_empty() {
        return stdout_line("no memos".to_string());
    }
    let mut rows: Vec<[String; 6]> = Vec::new();
    let mut stderr = String::new();
    for e in entries {
        let dash = || "-".to_string();
        let mut row = [
            e.stem.clone(),
            memo_duration(e.duration),
            dash(),
            dash(),
            dash(),
            dash(),
        ];
        let mut cells = vec![];
        if let Some(sc) = &e.sidecar {
            row[2] = format!("{} bpm", number(sc.tempo.bpm));
            row[3] = sc.tempo.meter.clone();
            row[4] = backing_names(&sc.backing, "+");
            let mut bar = if sc.timing.bar1_frame.is_some() {
                "bar 1 known"
            } else {
                "bar 1 unknown"
            }
            .to_string();
            if sc.recovered {
                bar.push_str(" (recovered)");
            }
            cells.push(bar);
        } else if let Some(err) = &e.sidecar_error {
            stderr.push_str(&format!("warning: {err}\n"));
            cells.push("bad sidecar".to_string());
        }
        if let Some(c) = cells.pop() {
            row[5] = c;
        }
        rows.push(row);
    }
    let widths: Vec<usize> = (0..5)
        .map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
        .collect();
    let mut stdout = String::new();
    for r in &rows {
        for (i, w) in widths.iter().enumerate() {
            stdout.push_str(&format!("{:<w$}  ", r[i], w = w));
        }
        stdout.push_str(&r[5]);
        stdout.push('\n');
    }
    Report {
        stdout,
        stderr,
        failed: false,
    }
}

/// `bar1Frame / sampleRate`, when both are known.
fn bar1_seconds(e: &RecordingEntry) -> Option<f64> {
    let frame = e.sidecar.as_ref()?.timing.bar1_frame?;
    Some(frame / f64::from(e.sample_rate?))
}

/// `m:ss.s`, rounded to a tenth of a second.
fn clock_text(seconds: f64) -> String {
    let tenths = (seconds.max(0.0) * 10.0).round() as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths % 600 / 10, tenths % 10)
}

/// `bar:beat` for a note on the grid: the beat is `1 + step / stepsPerQuarter`, up to two
/// decimals (`1:1`, `1:1.25`, `2:1.33`); `-:-` when the grid is unresolved.
fn bar_beat(bar: Option<u32>, step: Option<u32>, steps_per_quarter: f64) -> String {
    match (bar, step) {
        (Some(bar), Some(step)) => {
            let beat = 1.0 + f64::from(step) / steps_per_quarter;
            format!("{bar}:{}", trim_decimals(beat, 2))
        }
        _ => "-:-".to_string(),
    }
}

/// `transcribe` human output: header, grid, tuning, key, the code and the note table.
pub fn format_transcription(t: &Transcription) -> Report {
    let mut out = format!(
        "memo: {}  {}",
        t.memo.as_deref().unwrap_or("?"),
        clock_text(t.duration)
    );
    if let Some(n) = &t.notation {
        if n.first_bar == n.last_bar {
            out.push_str(&format!("  (bar {})", n.first_bar));
        } else {
            out.push_str(&format!("  (bars {}-{})", n.first_bar, n.last_bar));
        }
    }
    out.push('\n');
    if t.notes.is_empty() {
        out.push_str("no notes\n");
        return stdout_only_report(out);
    }
    match &t.grid {
        Some(g) => {
            let mut source = format!(
                "tempo {}, phase {}",
                g.source.tempo.label(),
                g.source.phase.label()
            );
            if let Some(c) = g.confidence {
                source.push_str(&format!(", confidence {c:.2}"));
            }
            out.push_str(&format!(
                "grid: {} bpm, {}, {} steps per bar ({source})\n",
                trim_decimals(g.bpm, 1),
                g.meter,
                g.steps_per_bar
            ));
        }
        None => out.push_str("grid: none (not quantized)\n"),
    }
    out.push_str(&format!(
        "tuning: {} cents ({})\n",
        trim_decimals(f64::from(t.tuning.cents), 0),
        t.tuning.source.label()
    ));
    out.push_str(&match &t.key {
        Some(k) => match (k.source, k.confidence, &k.runner_up) {
            (KeySource::Detected, Some(c), Some(r)) => {
                format!("key: {} (confidence {c:.2}, runner-up {r})\n", k.name)
            }
            (KeySource::Detected, Some(c), None) => {
                format!("key: {} (confidence {c:.2})\n", k.name)
            }
            (KeySource::Detected, None, _) => format!("key: {} (detected)\n", k.name),
            (KeySource::Flag, _, _) => format!("key: {} (flag)\n", k.name),
        },
        None => match t.no_key {
            Some(reason) => format!("key: none ({reason})\n"),
            None => "key: none\n".to_string(),
        },
    });
    if let Some(n) = &t.notation {
        out.push_str(&format!("\nnote(\"{}\")\n", n.note));
        if let (Some(degrees), Some(scale)) = (&n.n, &n.scale) {
            out.push_str(&format!("n(\"{degrees}\").scale(\"{scale}\")\n"));
        }
    }
    let per_quarter = t.grid.as_ref().map_or(1.0, |g| {
        (f64::from(g.steps_per_bar) / g.beats_per_bar).max(f64::MIN_POSITIVE)
    });
    let rows: Vec<[String; 4]> = t
        .notes
        .iter()
        .map(|n| {
            let length = match n.steps {
                Some(steps) => steps.to_string(),
                None => format!("{:.2}s", (n.end - n.start).max(0.0)),
            };
            let flag = if n.confidence < LOW_CONFIDENCE {
                " ?"
            } else {
                ""
            };
            [
                bar_beat(n.bar, n.step, per_quarter),
                match &n.alternative {
                    Some(alt) => format!("{}/{}", n.name, alt.name),
                    None => n.name.clone(),
                },
                length,
                format!("{:.2}{flag}", n.confidence),
            ]
        })
        .collect();
    let head = ["bar:beat", "note", "length", "conf"];
    let width = |c: usize| {
        rows.iter()
            .map(|r| r[c].len())
            .chain([head[c].len()])
            .max()
            .unwrap_or(0)
            + 2
    };
    let widths = [width(0), width(1), width(2)];
    let line = |cells: &[String]| {
        format!(
            "{:<w0$}{:<w1$}{:<w2$}{}\n",
            cells[0],
            cells[1],
            cells[2],
            cells[3],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2]
        )
    };
    out.push('\n');
    out.push_str(&line(&head.map(String::from)));
    for r in &rows {
        out.push_str(&line(r));
    }
    stdout_only_report(out)
}

fn stdout_only_report(stdout: String) -> Report {
    Report {
        stdout,
        ..Report::default()
    }
}

/// `transcribe --json`: the transcription, camelCase, one line when printed.
pub fn transcription_json(t: &Transcription) -> serde_json::Value {
    serde_json::to_value(t).unwrap_or(serde_json::Value::Null)
}

/// `memo list --json`: one line.
pub fn memo_list_json(root: &str, entries: &[RecordingEntry]) -> serde_json::Value {
    let memos: Vec<serde_json::Value> = entries.iter().map(|e| memo_json(root, e)).collect();
    serde_json::json!({"project": root, "memos": memos})
}

fn memo_path(root: &str, e: &RecordingEntry) -> String {
    format!("{}/recordings/{}.wav", root.trim_end_matches('/'), e.stem)
}

fn memo_json(root: &str, e: &RecordingEntry) -> serde_json::Value {
    let sc = e.sidecar.as_ref();
    let part = |f: fn(&Sidecar) -> serde_json::Value| sc.map_or(serde_json::Value::Null, f);
    serde_json::json!({
        "name": e.stem,
        "path": memo_path(root, e),
        "duration": e.duration,
        "frames": e.frames,
        "sampleRate": e.sample_rate,
        "size": e.size,
        "modifiedMs": e.modified_ms,
        "hasSidecar": e.has_sidecar,
        "sidecarError": e.sidecar_error,
        "tempo": part(|s| serde_json::to_value(&s.tempo).unwrap_or_default()),
        "backing": part(|s| serde_json::to_value(&s.backing).unwrap_or_default()),
        "bar1Frame": part(|s| serde_json::json!(s.timing.bar1_frame)),
        "bar1Seconds": bar1_seconds(e),
        "recovered": part(|s| serde_json::json!(s.recovered)),
    })
}

/// `memo show --json`: the sidecar's keys plus the WAV facts, one line.
pub fn memo_show_json(root: &str, e: &RecordingEntry) -> serde_json::Value {
    let mut v = e
        .sidecar
        .as_ref()
        .and_then(|s| serde_json::to_value(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(m) = v.as_object_mut() {
        m.insert("name".into(), e.stem.clone().into());
        m.insert("path".into(), memo_path(root, e).into());
        m.insert("duration".into(), serde_json::json!(e.duration));
        m.insert("frames".into(), serde_json::json!(e.frames));
        m.insert("sampleRate".into(), serde_json::json!(e.sample_rate));
        m.insert("bar1Seconds".into(), serde_json::json!(bar1_seconds(e)));
        m.insert("hasSidecar".into(), e.has_sidecar.into());
    }
    v
}

/// `memo show` human output for a memo whose sidecar is valid or missing.
pub fn format_memo_show(root: &str, e: &RecordingEntry) -> String {
    let mut out = format!("{}\npath: {}\n", e.stem, memo_path(root, e));
    out.push_str(&match (e.duration, e.frames, e.sample_rate) {
        (Some(d), Some(f), Some(r)) => {
            format!(
                "duration: {} s ({f} frames at {r} Hz)\n",
                trim_decimals(d, 3)
            )
        }
        _ => "duration: unknown\n".to_string(),
    });
    let Some(sc) = &e.sidecar else {
        out.push_str("sidecar: none\n");
        return out;
    };
    out.push_str(&format!(
        "input: {} (channel {} of {})\n",
        sc.input.device_name,
        sc.input.channel + 1,
        sc.input.channels
    ));
    out.push_str(&format!(
        "tempo: {} bpm {}{}\n",
        number(sc.tempo.bpm),
        sc.tempo.meter,
        if sc.tempo.tempo_override {
            " (tempo override)"
        } else {
            ""
        }
    ));
    out.push_str(&format!("backing: {}\n", backing_names(&sc.backing, ", ")));
    if let Some(v) = &sc.backing.song_path {
        out.push_str(&format!("song: {v}\n"));
    }
    if let Some(v) = &sc.backing.start_section {
        out.push_str(&format!("start section: {v}\n"));
    }
    if let Some(v) = &sc.backing.loop_section {
        out.push_str(&format!("loop section: {v}\n"));
    }
    let bar = match (sc.timing.bar1_frame, bar1_seconds(e)) {
        (Some(f), Some(s)) => format!("frame {} ({s:.3} s)", trim_decimals(f, 3)),
        (Some(f), None) => format!("frame {}", trim_decimals(f, 3)),
        _ => "unknown".to_string(),
    };
    out.push_str(&format!("bar 1: {bar}\nxruns: {}\n", sc.xruns));
    if sc.recovered {
        out.push_str("recovered: yes\n");
    }
    out
}

/// The `levels` line of `state`: held peaks, clipped samples and the limiter.
fn levels_line(l: &Levels) -> String {
    let peak = |p: Option<f64>| match p {
        Some(db) => {
            let r = (db * 10.0).round() / 10.0;
            format!("{:+.1}", if r == 0.0 { 0.0 } else { r })
        }
        None => "-".to_string(),
    };
    let overs = l.overs;
    let mut out = format!(
        "levels  peak {} / {} dBFS  clipped {} {}  limiter {}",
        peak(l.peak_db[0]),
        peak(l.peak_db[1]),
        overs,
        if overs == 1 { "sample" } else { "samples" },
        if l.limiter { "on" } else { "off" },
    );
    if l.limiter && l.limit_db < -0.05 {
        out.push_str(&format!(" ({:.1} dB)", l.limit_db));
    }
    out.push('\n');
    out
}

/// `x` with `places` decimals, trailing zeros and a bare dot trimmed; `-0` reads as `0`.
fn trim_decimals(x: f64, places: usize) -> String {
    let t = format!("{x:.places$}");
    let t = if t.contains('.') {
        t.trim_end_matches('0').trim_end_matches('.')
    } else {
        &t
    };
    if t == "-0" { "0".into() } else { t.into() }
}

/// A beat with up to 3 decimals, trailing zeros trimmed.
fn beat_text(beat: f64) -> String {
    trim_decimals(beat, 3)
}

fn value_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) if !s.is_empty() && !s.contains(char::is_whitespace) => {
            s.clone()
        }
        serde_json::Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), value_number),
        other => other.to_string(),
    }
}

/// A value number with up to 4 decimals, trailing zeros trimmed, so float noise from signals
/// (`0.5707106781186547`) reads as `0.5707`. `--json` keeps the exact value.
fn value_number(n: f64) -> String {
    if n.fract() == 0.0 {
        return number(n);
    }
    trim_decimals(n, 4)
}

/// `key=value` pairs: `s`, `note`, `n`, then the rest alphabetically.
fn event_value(e: &Event) -> String {
    let rank = |k: &str| match k {
        "s" => 0,
        "note" => 1,
        "n" => 2,
        _ => 3,
    };
    let mut keys: Vec<&String> = e.value.keys().collect();
    keys.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
    keys.iter()
        .map(|k| format!("{k}={}", value_text(&e.value[*k])))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `events`: header, then one block per bar with events; warnings on stderr.
pub fn format_events(r: &EventsResult) -> Report {
    let mut out = format!("song: {} \"{}\"\n", song_stem(&r.song.path), r.song.title);
    if r.from == r.to {
        out.push_str(&format!("bars: {} ({})\n", r.from, r.meter));
    } else {
        out.push_str(&format!("bars: {}-{} ({})\n", r.from, r.to, r.meter));
    }
    if r.events.is_empty() {
        out.push_str("\nno events\n");
    }
    let beats: Vec<String> = r.events.iter().map(|e| beat_text(e.beat)).collect();
    let width = beats.iter().map(|b| b.chars().count()).max().unwrap_or(0) + 2;
    let mut bar = None;
    for (e, beat) in r.events.iter().zip(&beats) {
        if bar != Some(e.bar) {
            bar = Some(e.bar);
            out.push_str(&format!("\nbar {}", e.bar));
            if let Some(sec) = &e.section {
                out.push_str(&format!("  {} {}", sec.name, sec.bar));
            }
            out.push('\n');
        }
        let locations = if e.locations.is_empty() {
            "-".to_string()
        } else {
            e.locations
                .iter()
                .map(|l| format!("{}:{}", l.line, l.col))
                .collect::<Vec<_>>()
                .join(",")
        };
        out.push_str(&format!(
            "{}{beat:<width$}{} @ {locations}\n",
            if e.onset { "  " } else { "~ " },
            event_value(e),
        ));
    }
    let mut stderr = String::new();
    if r.stale {
        stderr.push_str(
            "warning: the song changed since it was evaluated; positions refer to the evaluated text\n",
        );
    }
    if r.tempo_override {
        stderr.push_str(
            "warning: setcps in the code overrides meta.bpm; bar positions follow meta\n",
        );
    }
    if r.truncated {
        stderr.push_str(&format!(
            "warning: showing the first {} events; narrow --bars or raise --limit\n",
            r.events.len()
        ));
    }
    Report {
        stdout: out,
        stderr,
        failed: false,
    }
}

pub fn format_eval(o: &EvalOutcome) -> Report {
    let mut r = Report::default();
    append_eval(&mut r, o);
    r
}

fn append_eval(r: &mut Report, o: &EvalOutcome) {
    if !o.ok {
        let msg = o.error.as_deref().unwrap_or("evaluate failed");
        r.stderr.push_str(&format!("error: {msg}\n"));
        r.failed = true;
    } else if o.started {
        r.stdout.push_str(&format!("playing {}\n", o.title));
    } else {
        r.stdout
            .push_str(&format!("evaluated {} (not playing)\n", o.title));
        if o.audio == Audio::Suspended {
            r.stderr.push_str(SUSPENDED_WARNING);
            r.stderr.push('\n');
            r.failed = true;
        }
    }
}

pub fn format_set(s: &CodeSetResult) -> Report {
    let mut r = Report::default();
    let stem = song_stem(&s.song);
    let first = match (s.changed, s.dirty) {
        (true, true) => format!("{stem}: updated (unsaved)"),
        (true, false) => format!("{stem}: updated"),
        (false, _) => format!("{stem}: no change"),
    };
    r.stdout.push_str(&first);
    r.stdout.push('\n');
    match s.conflict {
        Some(Conflict::Changed) => {
            r.stdout.push_str(CHANGED_NOTE);
            r.stdout.push('\n');
        }
        Some(Conflict::Deleted) => {
            r.stdout.push_str(DELETED_NOTE);
            r.stdout.push('\n');
        }
        None => {}
    }
    if let Some(o) = &s.evaluated {
        append_eval(&mut r, o);
    }
    r
}

/// The `ping` report; an app that is not attached to a webview is running but not ready.
pub fn format_ping(p: &PingResult) -> Report {
    if p.attached {
        Report {
            stdout: format!("strudeland {} is running\n", p.version),
            ..Report::default()
        }
    } else {
        Report {
            stderr: format!("strudeland {} is running but not ready\n", p.version),
            ..Report::default()
        }
    }
}

/// `screenshot`: the file written and its pixel size.
pub fn format_screenshot(r: &ScreenshotResult) -> Report {
    Report {
        stdout: format!("wrote {} ({}x{})\n", r.path, r.width, r.height),
        ..Report::default()
    }
}

/// `song new`.
pub fn format_song_new(r: &SongResult) -> Report {
    stdout_line(format!("created {}", song_stem(&r.song.path)))
}

/// `song switch`.
pub fn format_song_switch(r: &SongResult) -> Report {
    stdout_line(format!("switched to {}", song_stem(&r.song.path)))
}

/// `song rename`: `main → intro`.
pub fn format_song_rename(r: &SongRenameResult) -> Report {
    stdout_line(format!(
        "renamed {} \u{2192} {}",
        song_stem(&r.from.path),
        song_stem(&r.to.path)
    ))
}

/// `song delete`, with a second line when it stopped playback.
pub fn format_song_delete(r: &SongDeleteResult) -> Report {
    let mut line = format!("deleted {}", song_stem(&r.song.path));
    if r.stopped {
        line.push_str("\nstopped playback");
    }
    stdout_line(line)
}

/// `save`: the stems saved, or that there was nothing to save.
pub fn format_save(r: &SaveResult) -> Report {
    if r.saved.is_empty() {
        return stdout_line("nothing to save".into());
    }
    let stems: Vec<_> = r.saved.iter().map(|s| song_stem(&s.path)).collect();
    stdout_line(format!("saved {}", stems.join(", ")))
}

/// `project open`.
pub fn format_project_open(r: &ProjectOpenResult) -> Report {
    if r.already_open {
        stdout_line(format!("{} is already open", r.project.name))
    } else {
        stdout_line(format!("opened {} ({})", r.project.name, r.project.root))
    }
}

fn stdout_line(line: String) -> Report {
    Report {
        stdout: format!("{line}\n"),
        ..Report::default()
    }
}

pub fn format_stop(was_playing: bool) -> String {
    if was_playing {
        "stopped\n"
    } else {
        "not playing\n"
    }
    .to_string()
}

/// `docs <name>`: header, description, params table and examples, blocks separated by a blank line.
pub fn format_docs_entry(e: &DocEntry) -> String {
    let mut header = e.name.clone();
    if !e.synonyms.is_empty() {
        header.push_str(&format!(" (synonyms: {})", e.synonyms.join(", ")));
    }
    if e.superdirt {
        header.push_str(" (superdirt only)");
    }
    let mut blocks = vec![header];
    if !e.description.is_empty() {
        blocks.push(e.description.clone());
    }
    if !e.params.is_empty() {
        let types: Vec<String> = e.params.iter().map(|p| p.types.join(" | ")).collect();
        let name_w = e
            .params
            .iter()
            .map(|p| p.name.chars().count())
            .max()
            .unwrap_or(0);
        let type_w = types.iter().map(|t| t.chars().count()).max().unwrap_or(0);
        let mut block = String::from("params:");
        for (p, t) in e.params.iter().zip(&types) {
            let desc = p
                .description
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let line = if desc.is_empty() {
                format!("  {:name_w$}  {t}", p.name)
            } else {
                format!("  {:name_w$}  {t:type_w$}  {desc}", p.name)
            };
            block.push('\n');
            block.push_str(line.trim_end());
        }
        blocks.push(block);
    }
    if !e.examples.is_empty() {
        let mut block = String::from("examples:");
        for ex in &e.examples {
            for line in ex.lines() {
                block.push_str("\n  ");
                block.push_str(line);
            }
        }
        blocks.push(block);
    }
    let mut out = blocks.join("\n\n");
    out.push('\n');
    out
}

const SEARCH_LINES: usize = 50;
const SEARCH_DESC_CHARS: usize = 80;

/// `docs --search`: one padded line per result, at most 50, then a `more` line.
pub fn format_docs_search(results: &[&DocEntry]) -> String {
    let shown = &results[..results.len().min(SEARCH_LINES)];
    let width = shown
        .iter()
        .map(|e| e.name.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for e in shown {
        let first: String = e
            .description
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(SEARCH_DESC_CHARS)
            .collect();
        let line = if first.is_empty() {
            e.name.clone()
        } else {
            format!("{:width$}  {first}", e.name)
        };
        out.push_str(line.trim_end());
        out.push('\n');
    }
    if results.len() > SEARCH_LINES {
        out.push_str(&format!(
            "({} more; narrow the search)\n",
            results.len() - SEARCH_LINES
        ));
    }
    out
}

/// `docs --list`: one name per line.
pub fn format_docs_list(names: &[&str]) -> String {
    names.iter().map(|n| format!("{n}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sref(path: &str) -> SongRef {
        SongRef {
            path: path.into(),
            title: "T".into(),
        }
    }

    #[test]
    fn song_lines() {
        let song = |p: &str| SongResult { song: sref(p) };
        assert_eq!(
            format_song_new(&song("songs/main2.js")).stdout,
            "created main2\n"
        );
        assert_eq!(
            format_song_switch(&song("songs/verse.js")).stdout,
            "switched to verse\n"
        );
        let renamed = format_song_rename(&SongRenameResult {
            from: sref("songs/main.js"),
            to: sref("songs/intro.js"),
        });
        assert_eq!(renamed.stdout, "renamed main \u{2192} intro\n");
        assert!(!renamed.failed && renamed.stderr.is_empty());
    }

    #[test]
    fn delete_line_mentions_a_stop() {
        let del = |stopped| {
            format_song_delete(&SongDeleteResult {
                song: sref("songs/verse.js"),
                stopped,
            })
            .stdout
        };
        assert_eq!(del(false), "deleted verse\n");
        assert_eq!(del(true), "deleted verse\nstopped playback\n");
    }

    #[test]
    fn save_line() {
        let save = |paths: &[&str]| {
            format_save(&SaveResult {
                saved: paths.iter().map(|p| sref(p)).collect(),
            })
            .stdout
        };
        assert_eq!(save(&[]), "nothing to save\n");
        assert_eq!(save(&["songs/main.js"]), "saved main\n");
        assert_eq!(
            save(&["songs/main.js", "songs/verse.js"]),
            "saved main, verse\n"
        );
    }

    #[test]
    fn project_open_line() {
        let open = |already_open| {
            format_project_open(&ProjectOpenResult {
                project: ProjectRef {
                    name: "demo".into(),
                    root: "/path".into(),
                },
                already_open,
            })
            .stdout
        };
        assert_eq!(open(false), "opened demo (/path)\n");
        assert_eq!(open(true), "demo is already open\n");
    }

    #[test]
    fn hints_name_their_remedy() {
        assert!(UNSAVED_HINT.starts_with("hint: ") && UNSAVED_HINT.contains("--discard"));
        assert!(BUSY_HINT.starts_with("hint: ") && BUSY_HINT.contains("dialog"));
    }

    #[test]
    fn screenshot_line() {
        let r = format_screenshot(&ScreenshotResult {
            path: "/tmp/strudeland-screenshot-1700000000000.png".into(),
            width: 2400,
            height: 1600,
        });
        assert_eq!(
            r,
            Report {
                stdout: "wrote /tmp/strudeland-screenshot-1700000000000.png (2400x1600)\n".into(),
                ..Report::default()
            }
        );
    }
    use strudeland_ctl::{
        Location, Playhead, Playing, ProjectRef, SectionPos, SongRef, SongSummary, TransportSummary,
    };

    fn song(path: &str, title: &str, dirty: bool, conflict: Option<Conflict>) -> SongSummary {
        SongSummary {
            path: path.into(),
            title: title.into(),
            dirty,
            conflict,
        }
    }

    fn base() -> AppState {
        AppState {
            project: Some(ProjectRef {
                name: "demo".into(),
                root: "/Users/me/demo".into(),
            }),
            songs: vec![
                song("songs/main.js", "Main Theme", true, None),
                song("songs/verse.js", "Verse", false, None),
                song("songs/bridge.js", "Bridge", false, Some(Conflict::Changed)),
            ],
            active: Some("songs/main.js".into()),
            playing: Some(Playing {
                path: "songs/main.js".into(),
                title: "Main Theme".into(),
            }),
            started: true,
            cps: 0.5,
            bpm: 120.0,
            meter: "4/4".into(),
            audio: Audio::Running,
            eval_error: None,
            meta_error: None,
            tempo_override: false,
            seed: None,
            playhead: None,
            transport: Default::default(),
            output_error: None,
            engine: Default::default(),
            live: None,
            levels: None,
            vim: false,
        }
    }

    fn outcome(ok: bool, started: bool, audio: Audio, error: Option<&str>) -> EvalOutcome {
        EvalOutcome {
            ok,
            song: "songs/main.js".into(),
            title: "Main Theme".into(),
            started,
            audio,
            error: error.map(Into::into),
        }
    }

    #[test]
    fn stems() {
        assert_eq!(song_stem("songs/main.js"), "main");
        assert_eq!(song_stem("main"), "main");
        assert_eq!(song_stem("songs/a.b.js"), "a.b");
        assert_eq!(song_stem("songs/.js"), ".js");
    }

    #[test]
    fn state_full_example() {
        let expected = "\
project: demo (/Users/me/demo)
songs:
  ● main    \"Main Theme\"   active, playing
    verse   \"Verse\"
    bridge  \"Bridge\"       conflict: changed
tempo: 120 bpm 4/4
audio: running
playing: Main Theme
";
        assert_eq!(format_state(&base()), expected);
    }

    #[test]
    fn state_override_errors_and_conflict_deleted() {
        let mut s = base();
        s.tempo_override = true;
        s.eval_error = Some("boom".into());
        s.meta_error = Some("bpm must be a number (1:20)".into());
        s.playing = None;
        s.songs[1].conflict = Some(Conflict::Deleted);
        let out = format_state(&s);
        assert!(out.contains("tempo: 120 bpm 4/4 (setcps in the code overrides meta.bpm)\n"));
        assert!(out.contains("playing: stopped\n"));
        assert!(out.contains("    verse   \"Verse\"        conflict: deleted\n"));
        assert!(out.ends_with("eval error: boom\nmeta error: bpm must be a number (1:20)\n"));
        assert!(!out.contains("playing, "));
    }

    #[test]
    fn state_shows_keys_vim_after_audio() {
        let mut s = base();
        assert!(!format_state(&s).contains("keys:"));
        s.vim = true;
        let out = format_state(&s);
        assert!(out.contains("audio: running\nkeys: vim\nplaying: "));
        assert_eq!(serde_json::to_value(&s).unwrap()["vim"], true);
    }

    #[test]
    fn state_shows_the_seed_after_the_tempo() {
        let mut s = base();
        assert!(!format_state(&s).contains("seed:"));
        s.seed = Some(7.0);
        let out = format_state(&s);
        assert!(out.contains("tempo: 120 bpm 4/4\nseed: 7\naudio: running\n"));
        s.seed = Some(0.0);
        assert!(format_state(&s).contains("seed: 0\n"));
    }

    #[test]
    fn state_engine_and_live_lines() {
        let s = base();
        assert!(!format_state(&s).contains("engine:"));
        assert!(!format_state(&s).contains("live:"));

        let mut r = base();
        r.engine = EngineId::Rust;
        let text = format_state(&r);
        assert!(text.contains("audio: running\nengine: rust\nplaying:"));
        assert!(!text.contains("live:"));

        r.live = Some(strudeland_ctl::LiveStats {
            underruns: 1,
            late_haps: 2,
            haps: 300,
            batches: 40,
            skipped_haps: 5,
            render_headroom_ms: 38.46,
            batch_decode_us: strudeland_ctl::BatchDecode { p50: 120, max: 900 },
            invoke_rtt_ms: strudeland_ctl::InvokeRtt {
                p50: 1.5,
                p95: 4.14,
                max: 9.0,
            },
        });
        assert!(format_state(&r).contains(
            "engine: rust\nlive: underruns 1, late 2, haps 300, batches 40, skipped 5, headroom 38.5 ms, decode p50 120 us max 900 us, invoke p50 1.5 ms p95 4.1 ms max 9 ms\nplaying:"
        ));

        r.live = Some(Default::default());
        assert!(format_state(&r).contains(
            "live: underruns 0, late 0, haps 0, batches 0, skipped 0, headroom 0 ms, decode p50 0 us max 0 us, invoke p50 0 ms p95 0 ms max 0 ms\n"
        ));
    }

    #[test]
    fn state_levels_line() {
        let mut s = base();
        assert!(!format_state(&s).contains("levels"));
        let mut l = Levels {
            peak_db: [Some(-0.4), Some(2.1)],
            overs: 1834,
            limit_db: 0.0,
            limiter: false,
        };
        s.levels = Some(l);
        assert!(format_state(&s).contains(
            "levels  peak -0.4 / +2.1 dBFS  clipped 1834 samples  limiter off\nplaying:"
        ));
        l = Levels {
            peak_db: [Some(-3.0), Some(-2.8)],
            overs: 0,
            limit_db: -2.3,
            limiter: true,
        };
        s.levels = Some(l);
        assert!(
            format_state(&s).contains(
                "levels  peak -3.0 / -2.8 dBFS  clipped 0 samples  limiter on (-2.3 dB)\n"
            )
        );
        l.limit_db = -0.04;
        s.levels = Some(l);
        assert!(format_state(&s).contains("limiter on\n"));
        s.levels = Some(Levels {
            peak_db: [None, None],
            overs: 1,
            limit_db: 0.0,
            limiter: false,
        });
        assert!(
            format_state(&s).contains("levels  peak - / - dBFS  clipped 1 sample  limiter off\n")
        );
        s.levels = Some(Levels {
            peak_db: [Some(0.0), Some(-0.0)],
            ..Default::default()
        });
        assert!(format_state(&s).contains("peak +0.0 / +0.0 dBFS"));
    }

    #[test]
    fn state_without_project() {
        let s = AppState {
            project: None,
            songs: vec![],
            active: None,
            playing: None,
            started: false,
            cps: 0.5,
            bpm: 120.0,
            meter: "4/4".into(),
            audio: Audio::Suspended,
            eval_error: None,
            meta_error: None,
            tempo_override: false,
            seed: None,
            playhead: None,
            transport: Default::default(),
            output_error: None,
            engine: Default::default(),
            live: None,
            levels: None,
            vim: false,
        };
        assert_eq!(
            format_state(&s),
            "project: none\ntempo: 120 bpm 4/4\naudio: suspended\nplaying: stopped\n"
        );
    }

    #[test]
    fn state_project_with_no_songs_and_fractional_bpm() {
        let mut s = base();
        s.songs.clear();
        s.active = None;
        s.bpm = 97.5;
        let out = format_state(&s);
        assert!(out.starts_with("project: demo (/Users/me/demo)\nsongs:\ntempo: 97.5 bpm"));
    }

    #[test]
    fn state_pads_names_to_longest_and_marks_dirty() {
        let mut s = base();
        s.songs = vec![
            song("songs/a.js", "A", true, None),
            song("songs/longname.js", "Long", false, None),
        ];
        s.active = None;
        s.playing = None;
        let out = format_state(&s);
        assert!(out.contains("\n  ● a         \"A\"\n"));
        assert!(out.contains("\n    longname  \"Long\"\n"));
    }

    #[test]
    fn eval_cases() {
        let r = format_eval(&outcome(true, true, Audio::Running, None));
        assert_eq!(
            r,
            Report {
                stdout: "playing Main Theme\n".into(),
                ..Report::default()
            }
        );
        let r = format_eval(&outcome(true, false, Audio::Suspended, None));
        assert_eq!(r.stdout, "evaluated Main Theme (not playing)\n");
        assert_eq!(
            r.stderr,
            "warning: audio is suspended: click in the strudeland window to enable it\n"
        );
        assert!(r.failed);
        let r = format_eval(&outcome(true, false, Audio::Running, None));
        assert_eq!(r.stdout, "evaluated Main Theme (not playing)\n");
        assert!(r.stderr.is_empty() && !r.failed);
        let r = format_eval(&outcome(
            false,
            false,
            Audio::Running,
            Some("meta: bpm must be a number (1:20)"),
        ));
        assert_eq!(r.stdout, "");
        assert_eq!(r.stderr, "error: meta: bpm must be a number (1:20)\n");
        assert!(r.failed);
        let r = format_eval(&outcome(false, false, Audio::Running, None));
        assert_eq!(r.stderr, "error: evaluate failed\n");
    }

    fn set(changed: bool, dirty: bool, conflict: Option<Conflict>) -> CodeSetResult {
        CodeSetResult {
            song: "songs/main.js".into(),
            changed,
            dirty,
            conflict,
            evaluated: None,
        }
    }

    #[test]
    fn set_first_lines() {
        assert_eq!(
            format_set(&set(true, true, None)).stdout,
            "main: updated (unsaved)\n"
        );
        assert_eq!(
            format_set(&set(true, false, None)).stdout,
            "main: updated\n"
        );
        assert_eq!(
            format_set(&set(false, false, None)).stdout,
            "main: no change\n"
        );
    }

    #[test]
    fn set_conflict_notes_then_eval() {
        let mut s = set(true, true, Some(Conflict::Changed));
        s.evaluated = Some(outcome(true, true, Audio::Running, None));
        let r = format_set(&s);
        assert_eq!(
            r.stdout,
            "main: updated (unsaved)\nnote: the file changed on disk; the conflict bar is still open\nplaying Main Theme\n"
        );
        assert!(!r.failed);
        let r = format_set(&set(false, false, Some(Conflict::Deleted)));
        assert_eq!(
            r.stdout,
            "main: no change\nnote: the file was deleted on disk\n"
        );
    }

    #[test]
    fn set_failed_eval() {
        let mut s = set(true, true, None);
        s.evaluated = Some(outcome(false, false, Audio::Running, Some("nope")));
        let r = format_set(&s);
        assert_eq!(r.stdout, "main: updated (unsaved)\n");
        assert_eq!(r.stderr, "error: nope\n");
        assert!(r.failed);
    }

    #[test]
    fn ping_and_stop() {
        let ok = format_ping(&PingResult {
            version: "0.1.0".into(),
            attached: true,
        });
        assert_eq!(ok.stdout, "strudeland 0.1.0 is running\n");
        let nr = format_ping(&PingResult {
            version: "0.1.0".into(),
            attached: false,
        });
        assert_eq!(nr.stderr, "strudeland 0.1.0 is running but not ready\n");
        assert_eq!(nr.stdout, "");
        assert_eq!(format_stop(true), "stopped\n");
        assert_eq!(format_stop(false), "not playing\n");
    }

    fn doc(name: &str) -> DocEntry {
        DocEntry {
            name: name.into(),
            synonyms: vec![],
            description: String::new(),
            params: vec![],
            examples: vec![],
            superdirt: false,
        }
    }

    fn dparam(name: &str, types: &[&str], description: &str) -> crate::docs::DocParam {
        crate::docs::DocParam {
            name: name.into(),
            types: types.iter().map(|t| (*t).into()).collect(),
            description: description.into(),
        }
    }

    #[test]
    fn docs_entry_fast() {
        let e = DocEntry {
            synonyms: vec!["density".into()],
            description: "Speed up a pattern by the given factor. Used by \"*\" in mini notation."
                .into(),
            params: vec![dparam("factor", &["number", "Pattern"], "speed up factor")],
            examples: vec!["s(\"bd hh sd hh\").fast(2) // s(\"[bd hh sd hh]*2\")".into()],
            ..doc("fast")
        };
        assert_eq!(
            format_docs_entry(&e),
            "fast (synonyms: density)\n\n\
             Speed up a pattern by the given factor. Used by \"*\" in mini notation.\n\n\
             params:\n  factor  number | Pattern  speed up factor\n\n\
             examples:\n  s(\"bd hh sd hh\").fast(2) // s(\"[bd hh sd hh]*2\")\n"
        );
    }

    #[test]
    fn docs_entry_without_synonyms_params_or_examples() {
        let e = DocEntry {
            description: "Does a thing.".into(),
            ..doc("thing")
        };
        assert_eq!(format_docs_entry(&e), "thing\n\nDoes a thing.\n");
        assert_eq!(format_docs_entry(&doc("bare")), "bare\n");
    }

    #[test]
    fn docs_entry_superdirt_and_both_flags() {
        let e = DocEntry {
            superdirt: true,
            ..doc("x")
        };
        assert_eq!(format_docs_entry(&e), "x (superdirt only)\n");
        let e = DocEntry {
            synonyms: vec!["a".into(), "b".into()],
            superdirt: true,
            ..doc("x")
        };
        assert_eq!(
            format_docs_entry(&e),
            "x (synonyms: a, b) (superdirt only)\n"
        );
    }

    #[test]
    fn docs_params_align_and_join_lines() {
        let e = DocEntry {
            params: vec![
                dparam("a", &["number"], "first\nsecond  line"),
                dparam("longer", &["string", "Pattern"], ""),
                dparam("c", &["x"], "d"),
            ],
            examples: vec!["one\ntwo".into()],
            ..doc("p")
        };
        assert_eq!(
            format_docs_entry(&e),
            "p\n\nparams:\n  a       number            first second line\n  \
             longer  string | Pattern\n  c       x                 d\n\nexamples:\n  one\n  two\n"
        );
    }

    #[test]
    fn docs_search_lines_pad_and_cut() {
        let long = DocEntry {
            description: format!("{}\nsecond line", "x".repeat(100)),
            ..doc("ab")
        };
        let short = DocEntry {
            description: "hi".into(),
            ..doc("abcd")
        };
        let empty = doc("z");
        let out = format_docs_search(&[&long, &short, &empty]);
        assert_eq!(out, format!("ab    {}\nabcd  hi\nz\n", "x".repeat(80)));
    }

    #[test]
    fn docs_search_truncates_at_fifty() {
        let entries: Vec<DocEntry> = (0..53).map(|i| doc(&format!("n{i:02}"))).collect();
        let refs: Vec<&DocEntry> = entries.iter().collect();
        let out = format_docs_search(&refs);
        assert_eq!(out.lines().count(), 51);
        assert!(out.ends_with("n49\n(3 more; narrow the search)\n"), "{out}");
        let exactly: Vec<&DocEntry> = entries.iter().take(50).collect();
        assert!(!format_docs_search(&exactly).contains("more"));
    }

    #[test]
    fn docs_list_one_per_line() {
        assert_eq!(format_docs_list(&["a", "b"]), "a\nb\n");
    }
    fn ev(bar: i64, beat: f64, value: serde_json::Value, locs: &[(u32, u32)]) -> Event {
        Event {
            bar,
            beat,
            begin: 0.0,
            end: 0.0,
            onset: true,
            section: Some(SectionPos {
                name: "intro".into(),
                bar: u32::try_from(bar).unwrap(),
            }),
            value: value.as_object().unwrap().clone(),
            locations: locs
                .iter()
                .map(|&(line, col)| Location {
                    line,
                    col,
                    end_line: line,
                    end_col: col + 1,
                    text: String::new(),
                })
                .collect(),
        }
    }

    fn events(evs: Vec<Event>, from: u32, to: u32) -> EventsResult {
        EventsResult {
            song: SongRef {
                path: "songs/main.js".into(),
                title: "Main Theme".into(),
            },
            from,
            to,
            stale: false,
            truncated: false,
            meter: "4/4".into(),
            tempo_override: false,
            events: evs,
        }
    }

    fn proposal_bar(bar: i64) -> Vec<Event> {
        let saw = |note: &str| json!({"s": "sawtooth", "note": note});
        vec![
            ev(bar, 1.0, json!({"s": "bd"}), &[(7, 15)]),
            ev(bar, 1.0, saw("c2"), &[(8, 17)]),
            ev(bar, 2.0, json!({"s": "hh"}), &[(7, 18)]),
            ev(bar, 3.0, json!({"s": "sd"}), &[(7, 21)]),
            ev(bar, 3.0, saw("eb2"), &[(8, 22)]),
            ev(bar, 4.0, json!({"s": "hh"}), &[(7, 24)]),
            ev(bar, 4.0, saw("g2"), &[(8, 26)]),
        ]
    }

    #[test]
    fn events_proposal_example() {
        let mut evs = proposal_bar(1);
        evs.extend(proposal_bar(2));
        let r = format_events(&events(evs, 1, 2));
        let expected = "\
song: main \"Main Theme\"
bars: 1-2 (4/4)

bar 1  intro 1
  1  s=bd @ 7:15
  1  s=sawtooth note=c2 @ 8:17
  2  s=hh @ 7:18
  3  s=sd @ 7:21
  3  s=sawtooth note=eb2 @ 8:22
  4  s=hh @ 7:24
  4  s=sawtooth note=g2 @ 8:26

bar 2  intro 2
  1  s=bd @ 7:15
  1  s=sawtooth note=c2 @ 8:17
  2  s=hh @ 7:18
  3  s=sd @ 7:21
  3  s=sawtooth note=eb2 @ 8:22
  4  s=hh @ 7:24
  4  s=sawtooth note=g2 @ 8:26
";
        assert_eq!(r.stdout, expected);
        assert_eq!(r.stderr, "");
        assert!(!r.failed);
    }

    #[test]
    fn events_alignment_continuations_and_values() {
        let mut cont = ev(3, 1.0, json!({"s": "pad"}), &[]);
        cont.onset = false;
        cont.section = None;
        let evs = vec![
            cont,
            ev(
                3,
                2.5,
                json!({"s": "a b", "n": 3.0, "gain": 0.5, "x": true}),
                &[(1, 2), (3, 4)],
            ),
            ev(3, 1.3333, json!({"s": ""}), &[]),
        ];
        let r = format_events(&events(evs, 3, 3));
        assert_eq!(
            r.stdout,
            "song: main \"Main Theme\"\nbars: 3 (4/4)\n\nbar 3\n~ 1      s=pad @ -\n  2.5    s=\"a b\" n=3 gain=0.5 x=true @ 1:2,3:4\n  1.333  s=\"\" @ -\n"
        );
    }

    #[test]
    fn events_none_and_warnings() {
        let r = format_events(&events(vec![], 1, 4));
        assert_eq!(
            r.stdout,
            "song: main \"Main Theme\"\nbars: 1-4 (4/4)\n\nno events\n"
        );
        let one = vec![ev(1, 1.0, json!({"s": "bd"}), &[])];
        let with = |f: fn(&mut EventsResult)| {
            let mut e = events(one.clone(), 1, 1);
            f(&mut e);
            format_events(&e).stderr
        };
        let stale = "warning: the song changed since it was evaluated; positions refer to the evaluated text\n";
        let tempo = "warning: setcps in the code overrides meta.bpm; bar positions follow meta\n";
        let trunc = "warning: showing the first 1 events; narrow --bars or raise --limit\n";
        assert_eq!(with(|e| e.stale = true), stale);
        assert_eq!(with(|e| e.tempo_override = true), tempo);
        assert_eq!(with(|e| e.truncated = true), trunc);
        assert_eq!(
            with(|e| {
                e.stale = true;
                e.tempo_override = true;
                e.truncated = true;
            }),
            format!("{stale}{tempo}{trunc}")
        );
    }

    #[test]
    fn value_numbers_round_float_noise() {
        assert_eq!(value_number(0.5707106781186547), "0.5707");
        assert_eq!(value_number(0.49999999999999994), "0.5");
        assert_eq!(value_number(0.9), "0.9");
        assert_eq!(value_number(-0.00001), "0");
        assert_eq!(value_number(7000.0), "7000");
        assert_eq!(value_number(-2.5), "-2.5");
    }

    /// The proposal's example: a 4-bar memo at 100 bpm from the sidecar, four notes.
    fn transcription_example() -> Transcription {
        let note = |bar: u32, step: u32, steps: u32, name: &str, midi: i32, conf: f64| {
            json!({"start": 2.43, "end": 2.97, "bar": bar, "step": step, "steps": steps,
                "midi": midi, "name": name, "degree": "0", "cents": 3.1, "confidence": conf,
                "components": {"voiced": 0.97, "stability": 0.93, "fit": 0.94, "onset": null},
                "snapErrorMs": -14.0, "tiedToNext": false})
        };
        serde_json::from_value(json!({
            "memo": "main-01", "duration": 12.4,
            "grid": {"bpm": 100.0, "meter": "4/4", "beatsPerBar": 4.0, "origin": 2.4,
                "source": {"tempo": "sidecar", "phase": "sidecar"}, "confidence": null,
                "stepsPerBar": 16, "division": "16"},
            "tuning": {"cents": -12.0, "source": "estimated"},
            "key": {"name": "A minor", "tonicPc": 9, "scale": "minor", "confidence": 0.18,
                "runnerUp": "C major", "correlation": 0.82, "source": "detected"},
            "notation": {"note": "a3 c4 e4@2 ~ g4 a4 e4@2 ~@4", "n": "0 2 4@2 ~ 6 7 4@2 ~@4",
                "scale": "A3:minor", "firstBar": 1, "lastBar": 4},
            "notes": [note(1, 0, 1, "a3", 57, 0.91), note(1, 4, 1, "c4", 60, 0.88),
                note(1, 8, 2, "e4", 64, 0.83), note(4, 4, 1, "g4", 67, 0.41)],
            "dropped": [], "timingSpread": 18.5, "warnings": [],
            "backing": {"song": "songs/main.js", "startSection": "verse"}
        }))
        .unwrap()
    }

    #[test]
    fn format_transcription_table_and_flags() {
        let r = format_transcription(&transcription_example());
        assert!(!r.failed && r.stderr.is_empty());
        assert_eq!(
            r.stdout,
            "memo: main-01  0:12.4  (bars 1-4)\n\
             grid: 100 bpm, 4/4, 16 steps per bar (tempo sidecar, phase sidecar)\n\
             tuning: -12 cents (estimated)\n\
             key: A minor (confidence 0.18, runner-up C major)\n\
             \n\
             note(\"a3 c4 e4@2 ~ g4 a4 e4@2 ~@4\")\n\
             n(\"0 2 4@2 ~ 6 7 4@2 ~@4\").scale(\"A3:minor\")\n\
             \n\
             bar:beat  note  length  conf\n\
             1:1       a3    1       0.91\n\
             1:2       c4    1       0.88\n\
             1:3       e4    2       0.83\n\
             4:2       g4    1       0.41 ?\n"
        );
        let json = transcription_json(&transcription_example());
        assert_eq!(json["notation"]["firstBar"], 1);
        assert_eq!(json["notation"]["lastBar"], 4);
        assert_eq!(json["memo"], "main-01");
    }

    #[test]
    fn an_ambiguous_note_names_its_alternative() {
        let mut t = transcription_example();
        t.notes[1].cents = 38.0;
        t.notes[1].alternative = strudeland_transcribe::report::Alternative::of(60, 38.0);
        let out = format_transcription(&t).stdout;
        assert!(out.contains("1:2       c4/c#4  1       0.88\n"), "{out}");
        let json = transcription_json(&t);
        assert_eq!(json["notes"][1]["alternative"]["name"], "c#4");
        assert!(json["notes"][0]["alternative"].is_null());
    }

    #[test]
    fn no_notes_prints_only_no_notes() {
        let mut t = transcription_example();
        t.notes.clear();
        t.notation = None;
        t.key = None;
        assert_eq!(
            format_transcription(&t).stdout,
            "memo: main-01  0:12.4\nno notes\n"
        );
    }

    #[test]
    fn unresolved_grid_layout() {
        let mut t = transcription_example();
        t.grid = None;
        t.notation = None;
        for n in &mut t.notes {
            n.bar = None;
            n.step = None;
            n.steps = None;
        }
        t.notes[0].end = t.notes[0].start + 0.43;
        let out = format_transcription(&t).stdout;
        assert_eq!(
            out,
            "memo: main-01  0:12.4\n\
             grid: none (not quantized)\n\
             tuning: -12 cents (estimated)\n\
             key: A minor (confidence 0.18, runner-up C major)\n\
             \n\
             bar:beat  note  length  conf\n\
             -:-       a3    0.43s   0.91\n\
             -:-       c4    0.54s   0.88\n\
             -:-       e4    0.54s   0.83\n\
             -:-       g4    0.54s   0.41 ?\n"
        );
        assert!(!out.contains("note(\""));
    }

    #[test]
    fn key_line_forms() {
        let key_line = |t: &Transcription| {
            format_transcription(t)
                .stdout
                .lines()
                .find(|l| l.starts_with("key: "))
                .unwrap()
                .to_string()
        };
        let mut t = transcription_example();
        assert_eq!(
            key_line(&t),
            "key: A minor (confidence 0.18, runner-up C major)"
        );
        let mut k = t.key.clone().unwrap();
        k.name = "C major".into();
        k.source = strudeland_transcribe::KeySource::Flag;
        k.confidence = None;
        k.runner_up = None;
        t.key = Some(k);
        assert_eq!(key_line(&t), "key: C major (flag)");
        t.key = None;
        t.notation.as_mut().unwrap().n = None;
        assert_eq!(key_line(&t), "key: none");
        t.no_key = Some(strudeland_transcribe::NoKey::TooFewNotes);
        assert_eq!(key_line(&t), "key: none (fewer than 6 notes)");
        assert!(!format_transcription(&t).stdout.contains("\nn(\""));
    }

    #[test]
    fn bar_beat_formatting() {
        assert_eq!(bar_beat(Some(1), Some(0), 4.0), "1:1");
        assert_eq!(bar_beat(Some(1), Some(4), 4.0), "1:2");
        // A sixteenth after the beat is beat 1.25 of the bar; a triplet step is 1.33.
        assert_eq!(bar_beat(Some(1), Some(1), 4.0), "1:1.25");
        assert_eq!(bar_beat(Some(2), Some(2), 6.0), "2:1.33");
        assert_eq!(bar_beat(Some(0), Some(15), 4.0), "0:4.75");
        assert_eq!(bar_beat(None, None, 4.0), "-:-");
        assert_eq!(clock_text(12.4), "0:12.4");
        assert_eq!(clock_text(59.96), "1:00.0");
        assert_eq!(clock_text(125.0), "2:05.0");
    }

    #[test]
    fn trim_decimals_trims_zeros_and_negative_zero() {
        assert_eq!(trim_decimals(2.5, 3), "2.5");
        assert_eq!(trim_decimals(2.0, 3), "2");
        assert_eq!(trim_decimals(10.0, 2), "10");
        assert_eq!(trim_decimals(10.0, 0), "10");
        assert_eq!(trim_decimals(1.0 / 3.0, 2), "0.33");
        assert_eq!(trim_decimals(0.999, 2), "1");
        assert_eq!(trim_decimals(-0.0001, 2), "0");
        assert_eq!(trim_decimals(-0.0, 2), "0");
        assert_eq!(trim_decimals(-1.25, 1), "-1.2");
        assert_eq!(trim_decimals(0.0, 4), "0");
    }

    #[test]
    fn beat_text_trims() {
        assert_eq!(beat_text(1.0), "1");
        assert_eq!(beat_text(2.5), "2.5");
        assert_eq!(beat_text(1.0 + 1.0 / 3.0), "1.333");
        assert_eq!(beat_text(10.0), "10");
    }

    fn with_state(f: impl FnOnce(&mut AppState)) -> String {
        let mut s = base();
        f(&mut s);
        format_state(&s)
    }

    #[test]
    fn state_proposal_example() {
        let out = with_state(|s| {
            s.songs.truncate(1);
            s.songs[0].dirty = false;
            s.playhead = Some(Playhead {
                cycle: 2.0,
                song_bar: Some(3),
                section: Some(SectionPos {
                    name: "body".into(),
                    bar: 1,
                }),
            });
            s.transport = TransportSummary {
                view: TransportView::Loop,
                loop_section: Some("body".into()),
                armed_section: None,
            };
            s.output_error = Some("sound nope not found! Is it loaded?".into());
        });
        assert_eq!(
            out,
            "project: demo (/Users/me/demo)\nsongs:\n    main  \"Main Theme\"   active, playing\ntempo: 120 bpm 4/4\naudio: running\nplaying: Main Theme\nplayhead: bar 3 (body 1)\ntransport: loop body\nsound error: sound nope not found! Is it loaded?\n"
        );
    }

    #[test]
    fn state_playhead_variants() {
        let head = |cycle, song_bar, section| {
            with_state(|s| {
                s.playhead = Some(Playhead {
                    cycle,
                    song_bar,
                    section,
                })
            })
        };
        assert!(head(1.0, Some(5), None).ends_with("playing: Main Theme\nplayhead: bar 5\n"));
        assert!(head(3.5, None, None).ends_with("playhead: cycle 3.5\n"));
        assert!(head(3.0, None, None).ends_with("playhead: cycle 3\n"));
        assert!(head(1.0 / 3.0, None, None).ends_with("playhead: cycle 0.33\n"));
        assert!(head(0.0, None, None).ends_with("playhead: cycle 0\n"));
        assert!(head(10.0, None, None).ends_with("playhead: cycle 10\n"));
    }

    #[test]
    fn state_transport_parts_and_order() {
        let t = |view, loop_section: Option<&str>, armed: Option<&str>| {
            with_state(|s| {
                s.transport = TransportSummary {
                    view,
                    loop_section: loop_section.map(Into::into),
                    armed_section: armed.map(Into::into),
                };
                s.eval_error = Some("boom".into());
            })
        };
        assert!(t(TransportView::Release, None, None).contains("transport: ending\neval error"));
        assert!(t(TransportView::Song, None, Some("intro")).contains("transport: armed intro\n"));
        assert!(
            t(TransportView::Loop, Some("body"), Some("outro"))
                .contains("transport: loop body, armed outro\n")
        );
        assert!(!t(TransportView::Song, None, None).contains("transport:"));
        assert!(!with_state(|_| {}).contains("playhead:"));
        assert!(!with_state(|_| {}).contains("sound error"));
        let both = with_state(|s| {
            s.meta_error = Some("m".into());
            s.output_error = Some("o".into());
        });
        assert!(both.ends_with("meta error: m\nsound error: o\n"));
    }

    #[test]
    fn memo_duration_rounds_to_tenths() {
        assert_eq!(memo_duration(None), "?");
        assert_eq!(memo_duration(Some(0.0)), "0:00.0");
        assert_eq!(memo_duration(Some(12.384)), "0:12.4");
        assert_eq!(memo_duration(Some(59.96)), "1:00.0");
        assert_eq!(memo_duration(Some(723.5)), "12:03.5");
        assert_eq!(memo_duration(Some(63.0)), "1:03.0");
    }

    #[test]
    fn memo_list_of_nothing_says_so() {
        let r = format_memo_list(&[]);
        assert_eq!(
            (r.stdout.as_str(), r.stderr.as_str(), r.failed),
            ("no memos\n", "", false)
        );
    }
}
