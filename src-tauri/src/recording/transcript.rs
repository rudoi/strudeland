//! The characterization transcript of every take flow: each step's command result, the state
//! events it emitted, the `recordings/` listing and, for stops and sidecar reads, the key
//! sidecar fields, asserted against one inline JSON value.
//!
//! The adapters below call the functions under test. The frozen region (`normalize`, the flows
//! and the test) and the expected block never change to absorb a difference: a difference is a
//! change of behaviour.
//!
//! `STRUDELAND_PRINT_TRANSCRIPT=1` prints the actual transcript before the assertion.

use super::rig::*;
use super::*;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};
use strudeland_record::FaultKind;
use strudeland_record::testing::{FakeHandle, Limits};

// --- adapters ---

/// One flow's rig and the steps recorded so far.
struct Script {
    rig: Rig,
    steps: Vec<Value>,
    /// The number of state events already recorded.
    seen: usize,
    /// The newest stream's fake source.
    handle: Option<FakeHandle>,
    /// The project the rig opened, kept so its listing reads after another project opens.
    alpha: Arc<Project>,
    /// Every project root the flow opened, with its placeholder.
    roots: Vec<(String, &'static str)>,
}

impl Script {
    fn new(rig: Rig) -> Script {
        let alpha = rig.project();
        let roots = vec![(alpha.root().to_string_lossy().into_owned(), "<root>")];
        Script {
            rig,
            steps: Vec::new(),
            seen: 0,
            handle: None,
            alpha,
            roots,
        }
    }

    fn fresh() -> Script {
        Script::new(rig())
    }

    fn with_limits(limits: Limits) -> Script {
        Script::new(rig_with(limits))
    }

    /// The state events since the previous step.
    fn new_events(&mut self) -> Vec<Value> {
        let states = self.rig.sink.states();
        let events = states[self.seen..]
            .iter()
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
        self.seen = states.len();
        events
    }

    /// The names in alpha's `recordings/` but `.gitkeep`, sorted; null when it is not a folder.
    fn listing(&self) -> Value {
        let dir = self.alpha.root().join("recordings");
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Value::Null;
        };
        let mut names: Vec<String> = entries
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != ".gitkeep")
            .collect();
        names.sort();
        json!(names)
    }

    fn record(&mut self, label: &str, result: Value) {
        let events = self.new_events();
        let files = self.listing();
        self.steps.push(json!({
            "do": label,
            "result": result,
            "events": events,
            "files": files,
        }));
    }

    /// The step's `{"ok": …}` or `{"err": {kind, message}}`.
    fn call<T: Serialize>(&mut self, label: &str, result: Result<T, CommandError>) {
        let value = match result {
            Ok(v) => json!({ "ok": serde_json::to_value(v).unwrap() }),
            Err(e) => json!({ "err": serde_json::to_value(e).unwrap() }),
        };
        self.record(label, value);
    }

    /// A step on the rig and its fakes with no result; `f`'s value goes back to the flow.
    fn act<R>(&mut self, label: &str, f: impl FnOnce(&Rig) -> R) -> R {
        let out = f(&self.rig);
        self.record(label, Value::Null);
        out
    }

    /// A step whose result is a value read from the rig.
    fn observe(&mut self, label: &str, f: impl FnOnce(&Rig) -> Value) {
        let value = f(&self.rig);
        self.record(label, value);
    }

    /// Keeps the newest stream the recorder opened.
    fn take_handles(&mut self) {
        while let Ok(handle) = self.rig.handles.lock().unwrap().try_recv() {
            self.handle = Some(handle);
        }
    }

    fn handle(&self) -> &FakeHandle {
        self.handle.as_ref().expect("a stream was opened")
    }

    fn arm(&mut self, device: Option<&str>, channel: Option<u16>) {
        let label = match (device, channel) {
            (None, None) => "arm".to_string(),
            (Some(d), None) => format!("arm {d}"),
            (None, Some(c)) => format!("arm channel {c}"),
            (Some(d), Some(c)) => format!("arm {d} channel {c}"),
        };
        let result = arm(
            &self.rig.core,
            &self.rig.prefs(),
            device.map(str::to_string),
            channel,
        );
        self.take_handles();
        self.call(&label, result);
    }

    /// `arm()` on another thread; it blocks in a gated permission request.
    fn spawn_arm(&self) -> JoinHandle<Result<RecordingStatus, CommandError>> {
        let core = Arc::clone(&self.rig.core);
        let prefs = self.rig.prefs();
        std::thread::spawn(move || arm(&core, &prefs, None, None))
    }

    /// Waits until the platform saw `n` permission requests. Records nothing.
    fn wait_for_requests(&self, n: usize) {
        let deadline = Instant::now() + WAIT;
        while self.rig.platform.requests.load(Ordering::SeqCst) < n {
            assert!(Instant::now() < deadline, "no permission request");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Releases the gated request and joins the spawned arm; its result is the step's.
    fn release_and_join(
        &mut self,
        label: &str,
        gate: mpsc::Sender<()>,
        joined: JoinHandle<Result<RecordingStatus, CommandError>>,
    ) {
        gate.send(()).unwrap();
        let result = joined.join().unwrap();
        self.take_handles();
        self.call(label, result);
    }

    fn disarm(&mut self) {
        let result = disarm(&self.rig.core, &self.rig.prefs());
        self.call("disarm", result);
    }

    fn status(&mut self) {
        let result = status(&self.rig.core, &self.rig.prefs());
        self.call("status", Ok::<_, CommandError>(result));
    }

    fn devices(&mut self) {
        let result = devices(&self.rig.core, &self.rig.prefs());
        self.call("devices", result);
    }

    fn write_prefs(&mut self, device: &str, channel: u16) {
        let prefs = Prefs {
            device_id: Some(device.to_string()),
            channel,
        };
        let result = write_prefs(&self.rig.prefs(), &prefs);
        self.observe(&format!("write prefs {device} channel {channel}"), |_| {
            json!(result.is_ok())
        });
    }

    fn start_label(song: Option<&str>) -> String {
        format!("start {}", song.unwrap_or("none"))
    }

    /// Starts a take and delivers its first block (10 frames at `T0`) once it runs.
    fn start(&mut self, song: Option<&str>) {
        let result = match &self.handle {
            Some(handle) => self.rig.start(handle, song),
            None => start(
                &self.rig.core,
                &self.rig.projects,
                &self.rig.root,
                song,
                start_fields(),
            ),
        };
        self.call(&Script::start_label(song), result);
    }

    /// [`Script::start`] with these start fields.
    fn start_with(&mut self, label: &str, song: Option<&str>, fields: StartFields) {
        let result = self.rig.start_with(self.handle(), song, fields);
        self.call(label, result);
    }

    /// Starts a take without delivering any audio.
    fn start_silent(&mut self, song: Option<&str>) {
        let result = start(
            &self.rig.core,
            &self.rig.projects,
            &self.rig.root,
            song,
            start_fields(),
        );
        self.call(
            &format!("{} without audio", Script::start_label(song)),
            result,
        );
    }

    /// Starts against the open project's root with `suffix` appended.
    fn start_at_root(&mut self, suffix: &str) {
        let root = format!("{}{suffix}", self.rig.root);
        let result = start(
            &self.rig.core,
            &self.rig.projects,
            &root,
            None,
            start_fields(),
        );
        self.call(&format!("start at root + {suffix}"), result);
    }

    /// Starts with no project open.
    fn start_no_project(&mut self) {
        let result = start(
            &self.rig.core,
            &ProjectState::default(),
            &self.rig.root,
            None,
            start_fields(),
        );
        self.call("start with no project open", result);
    }

    /// Delivers frames `from..to` and waits for the writer.
    fn push(&mut self, from: u64, to: u64) {
        self.rig.push(self.handle(), from, to);
        self.record(&format!("push frames {from}..{to}"), Value::Null);
    }

    /// Arms, starts `main` and records frames 10 to 1000, as three steps.
    fn take(&mut self) {
        self.arm(None, None);
        self.start(Some("main"));
        self.push(10, 1_000);
    }

    fn stop(&mut self, label: &str, fields: Option<StopFields>) {
        let result = stop(&self.rig.core, fields)
            .map(|entry| summarize(serde_json::to_value(entry).unwrap()));
        self.call(label, result);
    }

    fn sidecar(&mut self, stem: &str) {
        let sidecar = self.alpha.read_sidecar(stem);
        let value = match sidecar {
            Ok(Some(sidecar)) => summarize_sidecar(&serde_json::to_value(sidecar).unwrap()),
            Ok(None) => Value::Null,
            Err(e) => json!({ "error": e.to_string() }),
        };
        self.observe(&format!("sidecar {stem}"), |_| value);
    }

    fn reload(&mut self) {
        on_reload(&self.rig.core);
        self.record("reload", Value::Null);
    }

    fn quit(&mut self) {
        shutdown(&self.rig.core);
        self.record("quit", Value::Null);
    }

    fn repair(&mut self) {
        let outcome = repair_on_open(&self.rig.core, &self.alpha);
        let value = match outcome {
            RepairOutcome::Skipped => json!("skipped"),
            RepairOutcome::Repaired(report) => json!({ "repaired": report }),
            RepairOutcome::Failed(message) => json!({ "failed": message }),
        };
        self.call("repair on open", Ok::<_, CommandError>(value));
    }

    /// Opens another project, `beta`, in the rig's project state.
    fn open_beta(&mut self) {
        let project = Project::create(&self.rig.tmp.path().join("beta"), "beta").unwrap();
        let root = self.rig.projects.open_unwatched(project);
        self.roots.push((root, "<root:beta>"));
        self.record("open beta", Value::Null);
    }

    /// The open project's listing (`recording_list`).
    fn list_open(&mut self, label: &str) {
        let root = self.roots.last().unwrap().0.clone();
        let result = list(&self.rig.projects, &root);
        self.call(label, result);
    }

    fn inject_failed(&mut self, arm_id: u64, error: RecordError) {
        let label = format!("failed arm {arm_id}: {error}");
        failed(&self.rig.core, arm_id, error);
        self.record(&label, Value::Null);
    }

    /// `ended_by_itself` for a 1000-frame take at `stem`'s partial path (as `start` names it).
    fn inject_ended(&mut self, stem: &str, ended: Ended) {
        let label = format!("take ended {ended:?} at {stem}'s partial");
        let result = TakeResult {
            path: self
                .alpha
                .root()
                .join("recordings")
                .join(format!(".{stem}.wav.partial")),
            frames: 1_000,
            sample_rate: RATE,
            channel: 0,
            anchors: Vec::new(),
            xruns: 0,
            ended,
        };
        ended_by_itself(&self.rig.core, result);
        self.record(&label, Value::Null);
    }

    /// Waits for the first state event with an error after the recorded ones.
    fn wait_error(&self) {
        let deadline = Instant::now() + WAIT;
        while !self.rig.sink.states()[self.seen..]
            .iter()
            .any(|e| e.error.is_some())
        {
            assert!(Instant::now() < deadline, "no state event with an error");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A stream fault, and the wait for the error event it brings.
    fn fault_and_wait(&mut self, kind: FaultKind, message: &str) {
        assert!(self.handle().fault(kind, message));
        self.wait_error();
        self.record(&format!("fault {kind:?} {message}"), Value::Null);
    }

    /// Delivers frames `from..to` past the cap, and waits for the error event it brings.
    fn push_and_wait_error(&mut self, from: u64, to: u64) {
        let block = vec![0.5f32; (to - from) as usize];
        assert!(self.handle().push(&block, T0 + from * 1_000_000));
        self.wait_error();
        self.record(
            &format!("push frames {from}..{to} past the cap"),
            Value::Null,
        );
    }
}

// --- frozen: begin ---

#[cfg(target_os = "macos")]
const RESTRICTED_TEXT: &str = "microphone access is restricted on this mac";
#[cfg(not(target_os = "macos"))]
const RESTRICTED_TEXT: &str = "microphone access is restricted by the system";

/// The key fields of a listed entry.
fn summarize(entry: Value) -> Value {
    let keys = [
        "stem",
        "frames",
        "sampleRate",
        "duration",
        "hasSidecar",
        "sidecarError",
        "size",
    ];
    let mut out = serde_json::Map::new();
    for key in keys {
        out.insert(key.to_string(), entry[key].clone());
    }
    let sidecar = match &entry["sidecar"] {
        Value::Null => Value::Null,
        sidecar => summarize_sidecar(sidecar),
    };
    out.insert("sidecar".to_string(), sidecar);
    Value::Object(out)
}

/// The key fields of a sidecar.
fn summarize_sidecar(sidecar: &Value) -> Value {
    let timing = &sidecar["timing"];
    let anchors = timing["anchors"].as_array().cloned().unwrap_or_default();
    json!({
        "schema": sidecar["schema"],
        "app": { "version": sidecar["app"]["version"] },
        "input": sidecar["input"],
        "output": sidecar["output"],
        "tempo": sidecar["tempo"],
        "backing": sidecar["backing"],
        "xruns": sidecar["xruns"],
        "recovered": sidecar["recovered"],
        "timing": {
            "bar1Frame": timing["bar1Frame"],
            "bar1HostNs": timing["bar1HostNs"],
            "clockOffsetNs": timing["clockOffsetNs"],
            "outputStamps": timing["outputStamps"],
            "anchorCount": anchors.len(),
            "firstAnchor": anchors.first().cloned().unwrap_or(Value::Null),
        },
    })
}

/// The path replacements of a flow, in order: its project roots, then the temp dir canonical
/// and raw.
fn paths(s: &Script) -> Vec<(String, &'static str)> {
    let mut paths = s.roots.clone();
    let tmp = s.rig.tmp.path();
    if let Ok(canonical) = tmp.canonicalize() {
        paths.push((canonical.to_string_lossy().into_owned(), "<tmp>"));
    }
    paths.push((tmp.to_string_lossy().into_owned(), "<tmp>"));
    paths
}

/// Replaces the volatile values: paths, times, the app version and the restricted text.
fn normalize(value: &mut Value, paths: &[(String, &'static str)]) {
    match value {
        Value::String(text) => {
            let mut matched = false;
            for (path, token) in paths {
                if text.contains(path.as_str()) {
                    *text = text.replace(path.as_str(), token);
                    matched = true;
                }
            }
            if matched {
                *text = text.replace('\\', "/");
            }
            if text == RESTRICTED_TEXT {
                *text = "<restricted>".to_string();
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize(item, paths);
            }
        }
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                match key.as_str() {
                    "elapsedSeconds" if !item.is_null() => *item = json!("<elapsed>"),
                    "modifiedMs" => *item = json!("<mtime>"),
                    "createdAt" => *item = json!("<created>"),
                    "app" if item["version"] == env!("CARGO_PKG_VERSION") => {
                        item["version"] = json!("<app version>")
                    }
                    _ => normalize(item, paths),
                }
            }
        }
        _ => {}
    }
}

/// The values of a take still being written (the `write-error` flow).
fn normalize_live(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(normalize_live),
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                match key.as_str() {
                    "frames" | "duration" | "size" => *item = json!("<live>"),
                    _ => normalize_live(item),
                }
            }
        }
        _ => {}
    }
}

fn bar1(ns: &str) -> Option<StopFields> {
    Some(stop_fields(Some(ns)))
}

fn meter_four() -> Option<StopFields> {
    let mut fields = stop_fields(Some("1500000000"));
    fields.tempo.meter = "four".into();
    Some(fields)
}

fn set_permission(s: &mut Script, permission: Permission) {
    s.act(&format!("permission {permission:?}"), |r| {
        *r.platform.permission.lock().unwrap() = permission
    });
}

fn set_answer(s: &mut Script, permission: Permission) {
    s.act(&format!("answer {permission:?}"), |r| {
        *r.platform.answer.lock().unwrap() = permission
    });
}

fn set_arm_failure(s: &mut Script, error: RecordError) {
    s.act(&format!("next arm fails: {error}"), |r| {
        *r.arm_failure.lock().unwrap() = Some(error)
    });
}

fn block(s: &mut Script) -> PathBuf {
    s.act("block publication", block_publication)
}

fn unblock(s: &mut Script, conflict: PathBuf) {
    s.act("remove the conflict", |_| {
        std::fs::remove_file(conflict).unwrap()
    });
}

fn prefs_text(s: &mut Script) {
    s.observe("prefs file", |r| {
        json!(std::fs::read_to_string(r.prefs()).ok())
    });
}

fn cap_rig() -> Script {
    Script::with_limits(Limits {
        max_frames: Some(100),
        start_timeout: Duration::from_secs(2),
    })
}

/// `cap` up to the push past the cap.
fn capped() -> Script {
    let mut s = cap_rig();
    s.arm(None, None);
    s.start(Some("main"));
    s.push_and_wait_error(10, 210);
    s
}

fn arm_prompt_granted() -> Script {
    let mut s = Script::fresh();
    set_permission(&mut s, Permission::NotDetermined);
    set_answer(&mut s, Permission::Authorized);
    s.arm(None, None);
    s.status();
    s
}

fn arm_prompt_denied() -> Script {
    let mut s = Script::fresh();
    set_permission(&mut s, Permission::NotDetermined);
    set_answer(&mut s, Permission::Denied);
    s.arm(None, None);
    s.status();
    s
}

fn arm_denied() -> Script {
    let mut s = Script::fresh();
    set_permission(&mut s, Permission::Denied);
    s.arm(None, None);
    s
}

fn arm_restricted() -> Script {
    let mut s = Script::fresh();
    set_permission(&mut s, Permission::Restricted);
    s.arm(None, None);
    s
}

fn arm_refused_while_armed() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    set_permission(&mut s, Permission::Denied);
    s.arm(None, None);
    s.status();
    s
}

fn arm_while_request_pending() -> Script {
    let mut s = Script::fresh();
    set_permission(&mut s, Permission::NotDetermined);
    let release = s.act("gate the permission request", |r| {
        let (release, gate) = mpsc::channel();
        *r.platform.gate.lock().unwrap() = Some(gate);
        release
    });
    let first = s.spawn_arm();
    s.wait_for_requests(1);
    s.arm(None, None);
    s.release_and_join("release the request and join the first arm", release, first);
    s
}

fn no_prompt_stream_decides() -> Script {
    let mut s = Script::fresh();
    s.act("no permission prompt", |r| {
        r.platform.has_prompt.store(false, Ordering::Release)
    });
    set_permission(&mut s, Permission::NotDetermined);
    s.status();
    set_arm_failure(&mut s, RecordError::PermissionDenied);
    s.arm(None, None);
    s.status();
    s.arm(None, None);
    s.status();
    s
}

fn arm_no_device() -> Script {
    let mut s = Script::fresh();
    s.act("no devices", |r| r.platform.devices.lock().unwrap().clear());
    s.arm(None, None);
    s.arm(Some("fake:gone"), None);
    s
}

fn arm_absent_device() -> Script {
    let mut s = Script::fresh();
    s.arm(Some("fake:gone"), None);
    s
}

fn arm_channel_out_of_range() -> Script {
    let mut s = Script::fresh();
    s.arm(Some("fake:usb"), Some(4));
    s
}

fn rearm() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    s.arm(None, None);
    s.arm(Some("fake:usb"), Some(2));
    s.status();
    s
}

fn rearm_fails_to_idle() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    set_arm_failure(&mut s, RecordError::Backend("refused".into()));
    s.arm(Some("fake:usb"), None);
    s.status();
    s
}

fn exclusive_input() -> Script {
    let mut s = Script::fresh();
    s.act("usb lists without channels", |r| {
        let mut busy = usb();
        busy.channels = 0;
        busy.sample_rate = 0;
        *r.platform.devices.lock().unwrap() = vec![mic(), busy];
    });
    s.write_prefs("fake:usb", 2);
    set_arm_failure(&mut s, RecordError::PermissionDenied);
    s.arm(None, None);
    s.arm(None, None);
    s.devices();
    s.arm(Some("fake:usb"), Some(3));
    s
}

fn prefs() -> Script {
    let mut s = Script::fresh();
    s.write_prefs("fake:gone", 2);
    prefs_text(&mut s);
    s.arm(None, None);
    prefs_text(&mut s);
    s.arm(None, Some(0));
    prefs_text(&mut s);
    s.arm(Some("fake:usb"), None);
    prefs_text(&mut s);
    s
}

fn disarm_flow() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    s.disarm();
    s.disarm();
    s
}

fn busy_during_take() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.arm(None, None);
    s.disarm();
    s.start(None);
    s
}

fn start_names() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    s.start(Some("main"));
    s.status();
    s.stop("stop", None);
    s.start(Some("main"));
    s.stop("stop", None);
    s.start(None);
    s.stop("stop", None);
    s.start(Some("a/b"));
    s.stop("stop", None);
    s
}

fn start_not_armed() -> Script {
    let mut s = Script::fresh();
    s.start(None);
    s
}

fn start_stale_root() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    s.start_at_root("-old");
    s.status();
    s
}

fn start_no_project() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    s.start_no_project();
    s
}

fn start_memo_error() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    s.act("replace recordings/ with a file", |r| {
        let dir = r.recordings_dir();
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::write(&dir, "not a folder").unwrap();
    });
    s.start(None);
    s
}

fn start_timeout() -> Script {
    let mut s = Script::with_limits(Limits {
        max_frames: None,
        start_timeout: Duration::from_millis(100),
    });
    s.arm(None, None);
    s.start_silent(None);
    s.status();
    s
}

fn stop_with_fields() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.stop("stop bar1 1500000000", bar1("1500000000"));
    s.status();
    s
}

fn stop_without_fields() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.stop("stop", None);
    s
}

fn stop_invalid_fields() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.stop("stop meter four", meter_four());
    s
}

fn stop_bar1_outside() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.stop("stop bar1 500000000", bar1("500000000"));
    s
}

fn stop_nothing() -> Script {
    let mut s = Script::fresh();
    s.stop("stop", None);
    s.take();
    s.stop("stop", None);
    s.stop("stop", None);
    s
}

fn stop_other_project() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.open_beta();
    s.stop("stop", None);
    s.list_open("beta recordings");
    s
}

fn invalid_start_tempo() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    let mut fields = start_fields();
    fields.tempo.cps = 0.0;
    s.start_with("start main cps 0", Some("main"), fields);
    s.push(10, 1_000);
    s.stop("stop", None);
    let mut fields = start_fields();
    fields.tempo.cps = 0.0;
    s.start_with("start main cps 0", Some("main"), fields);
    s.push(10, 1_000);
    let mut stop_with = stop_fields(Some("1500000000"));
    stop_with.tempo.cps = 0.0;
    s.stop("stop bar1 1500000000 cps 0", Some(stop_with));
    s
}

fn publish_retry() -> Script {
    let mut s = Script::fresh();
    s.take();
    let conflict = block(&mut s);
    s.stop("stop bar1 1500000000", bar1("1500000000"));
    s.stop("stop", None);
    unblock(&mut s, conflict);
    s.stop("stop bar1 1500000000", bar1("1500000000"));
    s.stop("stop", None);
    s
}

/// A take whose stop failed to publish, then the conflict removed.
fn unpublished() -> Script {
    let mut s = Script::fresh();
    s.take();
    let conflict = block(&mut s);
    s.stop("stop", None);
    unblock(&mut s, conflict);
    s
}

fn settle_by_arm() -> Script {
    let mut s = unpublished();
    s.arm(None, None);
    s.sidecar("main-01");
    s
}

fn settle_by_start() -> Script {
    let mut s = unpublished();
    s.start(Some("main"));
    s.sidecar("main-01");
    s.stop("stop", None);
    s
}

fn settle_by_disarm() -> Script {
    let mut s = unpublished();
    s.disarm();
    s.sidecar("main-01");
    s
}

fn settle_by_reload() -> Script {
    let mut s = unpublished();
    s.reload();
    s.sidecar("main-01");
    s.status();
    s
}

fn settle_by_quit() -> Script {
    let mut s = unpublished();
    s.quit();
    s.sidecar("main-01");
    s
}

fn cap() -> Script {
    let mut s = capped();
    s.sidecar("main-01");
    s.status();
    s.stop("stop bar1 1050000000", bar1("1050000000"));
    s.stop("stop", None);
    s
}

fn cap_then_invalid_fields() -> Script {
    let mut s = capped();
    s.stop("stop meter four", meter_four());
    s.sidecar("main-01");
    s
}

fn cap_then_loss() -> Script {
    let mut s = capped();
    s.fault_and_wait(FaultKind::DeviceLost, "unplugged");
    s.status();
    s.stop("stop", None);
    s
}

fn cap_then_start() -> Script {
    let mut s = capped();
    s.start(Some("main"));
    s.sidecar("main-01");
    s.stop("stop", None);
    s.sidecar("main-02");
    s
}

fn write_error() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.inject_ended("main-01", Ended::WriteError("disk full".into()));
    s.status();
    s.stop("stop", None);
    s.quit();
    s
}

fn ended_after_stop() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.stop("stop", None);
    s.inject_ended("main-01", Ended::Cap);
    s.status();
    s
}

fn ended_wrong_path() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.inject_ended("other-01", Ended::Cap);
    s.status();
    s.stop("stop", None);
    s
}

fn device_lost_take() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.fault_and_wait(FaultKind::DeviceLost, "unplugged");
    s.sidecar("main-01");
    s.status();
    s.stop("stop bar1 1250000000", bar1("1250000000"));
    s.stop("stop", None);
    s.arm(None, None);
    s
}

fn device_lost_take_blocked() -> Script {
    let mut s = Script::fresh();
    s.take();
    let conflict = block(&mut s);
    s.fault_and_wait(FaultKind::DeviceLost, "unplugged");
    s.stop("stop", None);
    unblock(&mut s, conflict);
    s.stop("stop", None);
    s.stop("stop", None);
    s
}

fn device_lost_armed() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    s.fault_and_wait(FaultKind::DeviceLost, "unplugged");
    s.status();
    s
}

fn backend_fault_take() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.fault_and_wait(FaultKind::Other, "x");
    s.stop("stop", None);
    s
}

fn backend_fault_armed() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    s.fault_and_wait(FaultKind::Other, "x");
    s
}

fn failed_after_disarm() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    s.disarm();
    s.inject_failed(1, RecordError::DeviceLost("late".into()));
    s
}

fn stale_failed() -> Script {
    let mut s = Script::fresh();
    s.arm(None, None);
    s.arm(Some("fake:usb"), None);
    s.inject_failed(1, RecordError::DeviceLost("old".into()));
    s.status();
    s.inject_failed(2, RecordError::DeviceLost("now".into()));
    s.status();
    s
}

fn arm_after_loss() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.fault_and_wait(FaultKind::DeviceLost, "unplugged");
    s.arm(None, None);
    s.stop("stop", None);
    s
}

fn start_after_loss() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.fault_and_wait(FaultKind::DeviceLost, "unplugged");
    s.start(None);
    s.status();
    s.stop("stop", None);
    s
}

fn reload_idle() -> Script {
    let mut s = Script::fresh();
    s.reload();
    s.arm(None, None);
    s.reload();
    s.status();
    s
}

fn reload_take() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.reload();
    s.status();
    s.stop("stop", None);
    s.sidecar("main-01");
    s.arm(None, None);
    s
}

fn quit_take() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.quit();
    s.sidecar("main-01");
    s.quit();
    s
}

fn repair() -> Script {
    let mut s = Script::fresh();
    s.take();
    s.act("stale partial memo-09", |r| {
        let path = r.recordings_dir().join(".memo-09.wav.partial");
        let file = std::fs::File::create(&path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(60))
            .unwrap();
    });
    s.repair();
    s.fault_and_wait(FaultKind::DeviceLost, "unplugged");
    s.repair();
    s.stop("stop", None);
    s.repair();
    s
}

type Flow = fn() -> Script;

const FLOWS: &[(&str, Flow)] = &[
    ("arm-prompt-granted", arm_prompt_granted),
    ("arm-prompt-denied", arm_prompt_denied),
    ("arm-denied", arm_denied),
    ("arm-restricted", arm_restricted),
    ("arm-refused-while-armed", arm_refused_while_armed),
    ("arm-while-request-pending", arm_while_request_pending),
    ("no-prompt-stream-decides", no_prompt_stream_decides),
    ("arm-no-device", arm_no_device),
    ("arm-absent-device", arm_absent_device),
    ("arm-channel-out-of-range", arm_channel_out_of_range),
    ("rearm", rearm),
    ("rearm-fails-to-idle", rearm_fails_to_idle),
    ("exclusive-input", exclusive_input),
    ("prefs", prefs),
    ("disarm", disarm_flow),
    ("busy-during-take", busy_during_take),
    ("start-names", start_names),
    ("start-not-armed", start_not_armed),
    ("start-stale-root", start_stale_root),
    ("start-no-project", start_no_project),
    ("start-memo-error", start_memo_error),
    ("start-timeout", start_timeout),
    ("stop-with-fields", stop_with_fields),
    ("stop-without-fields", stop_without_fields),
    ("stop-invalid-fields", stop_invalid_fields),
    ("stop-bar1-outside", stop_bar1_outside),
    ("stop-nothing", stop_nothing),
    ("stop-other-project", stop_other_project),
    ("invalid-start-tempo", invalid_start_tempo),
    ("publish-retry", publish_retry),
    ("settle-by-arm", settle_by_arm),
    ("settle-by-start", settle_by_start),
    ("settle-by-disarm", settle_by_disarm),
    ("settle-by-reload", settle_by_reload),
    ("settle-by-quit", settle_by_quit),
    ("cap", cap),
    ("cap-then-invalid-fields", cap_then_invalid_fields),
    ("cap-then-loss", cap_then_loss),
    ("cap-then-start", cap_then_start),
    ("write-error", write_error),
    ("ended-after-stop", ended_after_stop),
    ("ended-wrong-path", ended_wrong_path),
    ("device-lost-take", device_lost_take),
    ("device-lost-take-blocked", device_lost_take_blocked),
    ("device-lost-armed", device_lost_armed),
    ("backend-fault-take", backend_fault_take),
    ("backend-fault-armed", backend_fault_armed),
    ("failed-after-disarm", failed_after_disarm),
    ("stale-failed", stale_failed),
    ("arm-after-loss", arm_after_loss),
    ("start-after-loss", start_after_loss),
    ("reload-idle", reload_idle),
    ("reload-take", reload_take),
    ("quit-take", quit_take),
    ("repair", repair),
];

#[test]
fn transcript_of_every_take_flow() {
    let mut actual = serde_json::Map::new();
    for (name, flow) in FLOWS {
        let script = flow();
        let paths = paths(&script);
        let mut steps = Value::Array(script.steps);
        normalize(&mut steps, &paths);
        if *name == "write-error" {
            normalize_live(&mut steps);
        }
        actual.insert(name.to_string(), steps);
    }
    let actual = Value::Object(actual);
    if std::env::var_os("STRUDELAND_PRINT_TRANSCRIPT").is_some_and(|v| v == "1") {
        println!("{}", serde_json::to_string_pretty(&actual).unwrap());
    }
    assert_eq!(actual, serde_json::from_str::<Value>(EXPECTED).unwrap());
}

// --- frozen: end ---

// --- expected: begin ---
const EXPECTED: &str = r#"{
  "arm-absent-device": [
    {
      "do": "arm fake:gone",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "noDevice",
          "message": "no input device"
        }
      }
    }
  ],
  "arm-after-loss": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "fault DeviceLost unplugged",
      "events": [
        {
          "error": {
            "kind": "deviceLost",
            "message": "the input device was lost: unplugged"
          },
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "stop",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "err": {
          "kind": "other",
          "message": "no take in progress"
        }
      }
    }
  ],
  "arm-channel-out-of-range": [
    {
      "do": "arm fake:usb channel 4",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "other",
          "message": "channel 4 is out of range for 4 channels"
        }
      }
    }
  ],
  "arm-denied": [
    {
      "do": "permission Denied",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "arm",
      "events": [
        {
          "error": {
            "kind": "permissionDenied",
            "message": "microphone access denied"
          },
          "state": "idle"
        }
      ],
      "files": [],
      "result": {
        "err": {
          "kind": "permissionDenied",
          "message": "microphone access denied"
        }
      }
    }
  ],
  "arm-no-device": [
    {
      "do": "no devices",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "arm",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "noDevice",
          "message": "no input device"
        }
      }
    },
    {
      "do": "arm fake:gone",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "noDevice",
          "message": "no input device"
        }
      }
    }
  ],
  "arm-prompt-denied": [
    {
      "do": "permission NotDetermined",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "answer Denied",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "arm",
      "events": [
        {
          "error": {
            "kind": "permissionDenied",
            "message": "microphone access denied"
          },
          "state": "idle"
        }
      ],
      "files": [],
      "result": {
        "err": {
          "kind": "permissionDenied",
          "message": "microphone access denied"
        }
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "denied",
          "state": "idle",
          "stem": null
        }
      }
    }
  ],
  "arm-prompt-granted": [
    {
      "do": "permission NotDetermined",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "answer Authorized",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    }
  ],
  "arm-refused-while-armed": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "permission Denied",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "arm",
      "events": [
        {
          "error": {
            "kind": "permissionDenied",
            "message": "microphone access denied"
          },
          "state": "idle"
        }
      ],
      "files": [],
      "result": {
        "err": {
          "kind": "permissionDenied",
          "message": "microphone access denied"
        }
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "denied",
          "state": "armed",
          "stem": null
        }
      }
    }
  ],
  "arm-restricted": [
    {
      "do": "permission Restricted",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "arm",
      "events": [
        {
          "error": {
            "kind": "permissionDenied",
            "message": "<restricted>"
          },
          "state": "idle"
        }
      ],
      "files": [],
      "result": {
        "err": {
          "kind": "permissionDenied",
          "message": "<restricted>"
        }
      }
    }
  ],
  "arm-while-request-pending": [
    {
      "do": "permission NotDetermined",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "gate the permission request",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "arm",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "busy",
          "message": "a microphone access request is pending"
        }
      }
    },
    {
      "do": "release the request and join the first arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    }
  ],
  "backend-fault-armed": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "fault Other x",
      "events": [
        {
          "error": {
            "kind": "other",
            "message": "audio backend: x"
          },
          "state": "idle"
        }
      ],
      "files": [],
      "result": null
    }
  ],
  "backend-fault-take": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "fault Other x",
      "events": [
        {
          "error": {
            "kind": "other",
            "message": "audio backend: x"
          },
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    }
  ],
  "busy-during-take": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "arm",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "err": {
          "kind": "busy",
          "message": "a take is in progress"
        }
      }
    },
    {
      "do": "disarm",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "err": {
          "kind": "busy",
          "message": "a take is in progress"
        }
      }
    },
    {
      "do": "start none",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "err": {
          "kind": "busy",
          "message": "a take is in progress"
        }
      }
    }
  ],
  "cap": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..210 past the cap",
      "events": [
        {
          "error": {
            "kind": "limitReached",
            "message": "recording stopped at the length limit"
          },
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "sidecar main-01",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "app": {
          "version": "<app version>"
        },
        "backing": {
          "countIn": true,
          "loopSection": null,
          "metronome": false,
          "song": true,
          "songPath": "songs/main.js",
          "startSection": null
        },
        "input": {
          "channel": 0,
          "channels": 1,
          "deviceId": "fake:mic",
          "deviceName": "fake mic",
          "sampleRate": 1000
        },
        "output": {
          "baseLatency": 0.0,
          "outputLatency": null
        },
        "recovered": false,
        "schema": 1,
        "tempo": {
          "beatsPerBar": 4.0,
          "bpm": 120.0,
          "cps": 0.5,
          "meter": "4/4",
          "tempoOverride": false
        },
        "timing": {
          "anchorCount": 1,
          "bar1Frame": null,
          "bar1HostNs": null,
          "clockOffsetNs": {
            "end": null,
            "start": null
          },
          "firstAnchor": [
            0,
            "1000000000"
          ],
          "outputStamps": []
        },
        "xruns": 0
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "stop bar1 1050000000",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 0.1,
          "frames": 100,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": false,
              "loopSection": null,
              "metronome": true,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": "verse"
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.005,
              "outputLatency": 0.02
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 3.0,
              "bpm": 90.0,
              "cps": 0.5,
              "meter": "6/8",
              "tempoOverride": true
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": 50.0,
              "bar1HostNs": "1050000000",
              "clockOffsetNs": {
                "end": null,
                "start": {
                  "offsetNs": "-12",
                  "rttNs": "40"
                }
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": [
                [
                  0.5,
                  "1000000123"
                ]
              ]
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 468,
          "stem": "main-01"
        }
      }
    },
    {
      "do": "stop",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "err": {
          "kind": "other",
          "message": "no take in progress"
        }
      }
    }
  ],
  "cap-then-invalid-fields": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..210 past the cap",
      "events": [
        {
          "error": {
            "kind": "limitReached",
            "message": "recording stopped at the length limit"
          },
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "stop meter four",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 0.1,
          "frames": 100,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 468,
          "stem": "main-01"
        }
      }
    },
    {
      "do": "sidecar main-01",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "app": {
          "version": "<app version>"
        },
        "backing": {
          "countIn": true,
          "loopSection": null,
          "metronome": false,
          "song": true,
          "songPath": "songs/main.js",
          "startSection": null
        },
        "input": {
          "channel": 0,
          "channels": 1,
          "deviceId": "fake:mic",
          "deviceName": "fake mic",
          "sampleRate": 1000
        },
        "output": {
          "baseLatency": 0.0,
          "outputLatency": null
        },
        "recovered": false,
        "schema": 1,
        "tempo": {
          "beatsPerBar": 4.0,
          "bpm": 120.0,
          "cps": 0.5,
          "meter": "4/4",
          "tempoOverride": false
        },
        "timing": {
          "anchorCount": 1,
          "bar1Frame": null,
          "bar1HostNs": null,
          "clockOffsetNs": {
            "end": null,
            "start": null
          },
          "firstAnchor": [
            0,
            "1000000000"
          ],
          "outputStamps": []
        },
        "xruns": 0
      }
    }
  ],
  "cap-then-loss": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..210 past the cap",
      "events": [
        {
          "error": {
            "kind": "limitReached",
            "message": "recording stopped at the length limit"
          },
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "fault DeviceLost unplugged",
      "events": [
        {
          "error": {
            "kind": "deviceLost",
            "message": "the input device was lost: unplugged"
          },
          "state": "idle"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "status",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 0.1,
          "frames": 100,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 468,
          "stem": "main-01"
        }
      }
    }
  ],
  "cap-then-start": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..210 past the cap",
      "events": [
        {
          "error": {
            "kind": "limitReached",
            "message": "recording stopped at the length limit"
          },
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-02"
        }
      ],
      "files": [
        ".main-02.wav.partial",
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "stem": "main-02"
        }
      }
    },
    {
      "do": "sidecar main-01",
      "events": [],
      "files": [
        ".main-02.wav.partial",
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "app": {
          "version": "<app version>"
        },
        "backing": {
          "countIn": true,
          "loopSection": null,
          "metronome": false,
          "song": true,
          "songPath": "songs/main.js",
          "startSection": null
        },
        "input": {
          "channel": 0,
          "channels": 1,
          "deviceId": "fake:mic",
          "deviceName": "fake mic",
          "sampleRate": 1000
        },
        "output": {
          "baseLatency": 0.0,
          "outputLatency": null
        },
        "recovered": false,
        "schema": 1,
        "tempo": {
          "beatsPerBar": 4.0,
          "bpm": 120.0,
          "cps": 0.5,
          "meter": "4/4",
          "tempoOverride": false
        },
        "timing": {
          "anchorCount": 1,
          "bar1Frame": null,
          "bar1HostNs": null,
          "clockOffsetNs": {
            "end": null,
            "start": null
          },
          "firstAnchor": [
            0,
            "1000000000"
          ],
          "outputStamps": []
        },
        "xruns": 0
      }
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "main-02"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav",
        "main-02.json",
        "main-02.wav"
      ],
      "result": {
        "ok": {
          "duration": 0.01,
          "frames": 10,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 108,
          "stem": "main-02"
        }
      }
    },
    {
      "do": "sidecar main-02",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav",
        "main-02.json",
        "main-02.wav"
      ],
      "result": {
        "app": {
          "version": "<app version>"
        },
        "backing": {
          "countIn": true,
          "loopSection": null,
          "metronome": false,
          "song": true,
          "songPath": "songs/main.js",
          "startSection": null
        },
        "input": {
          "channel": 0,
          "channels": 1,
          "deviceId": "fake:mic",
          "deviceName": "fake mic",
          "sampleRate": 1000
        },
        "output": {
          "baseLatency": 0.0,
          "outputLatency": null
        },
        "recovered": false,
        "schema": 1,
        "tempo": {
          "beatsPerBar": 4.0,
          "bpm": 120.0,
          "cps": 0.5,
          "meter": "4/4",
          "tempoOverride": false
        },
        "timing": {
          "anchorCount": 1,
          "bar1Frame": null,
          "bar1HostNs": null,
          "clockOffsetNs": {
            "end": null,
            "start": null
          },
          "firstAnchor": [
            0,
            "1000000000"
          ],
          "outputStamps": []
        },
        "xruns": 0
      }
    }
  ],
  "device-lost-armed": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "fault DeviceLost unplugged",
      "events": [
        {
          "error": {
            "kind": "deviceLost",
            "message": "the input device was lost: unplugged"
          },
          "state": "idle"
        }
      ],
      "files": [],
      "result": null
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    }
  ],
  "device-lost-take": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "fault DeviceLost unplugged",
      "events": [
        {
          "error": {
            "kind": "deviceLost",
            "message": "the input device was lost: unplugged"
          },
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "sidecar main-01",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "app": {
          "version": "<app version>"
        },
        "backing": {
          "countIn": true,
          "loopSection": null,
          "metronome": false,
          "song": true,
          "songPath": "songs/main.js",
          "startSection": null
        },
        "input": {
          "channel": 0,
          "channels": 1,
          "deviceId": "fake:mic",
          "deviceName": "fake mic",
          "sampleRate": 1000
        },
        "output": {
          "baseLatency": 0.0,
          "outputLatency": null
        },
        "recovered": false,
        "schema": 1,
        "tempo": {
          "beatsPerBar": 4.0,
          "bpm": 120.0,
          "cps": 0.5,
          "meter": "4/4",
          "tempoOverride": false
        },
        "timing": {
          "anchorCount": 1,
          "bar1Frame": null,
          "bar1HostNs": null,
          "clockOffsetNs": {
            "end": null,
            "start": null
          },
          "firstAnchor": [
            0,
            "1000000000"
          ],
          "outputStamps": []
        },
        "xruns": 0
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    },
    {
      "do": "stop bar1 1250000000",
      "events": [
        {
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": false,
              "loopSection": null,
              "metronome": true,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": "verse"
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.005,
              "outputLatency": 0.02
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 3.0,
              "bpm": 90.0,
              "cps": 0.5,
              "meter": "6/8",
              "tempoOverride": true
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": 250.0,
              "bar1HostNs": "1250000000",
              "clockOffsetNs": {
                "end": null,
                "start": {
                  "offsetNs": "-12",
                  "rttNs": "40"
                }
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": [
                [
                  0.5,
                  "1000000123"
                ]
              ]
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    },
    {
      "do": "stop",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "err": {
          "kind": "other",
          "message": "no take in progress"
        }
      }
    },
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    }
  ],
  "device-lost-take-blocked": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "block publication",
      "events": [],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": null
    },
    {
      "do": "fault DeviceLost unplugged",
      "events": [
        {
          "error": {
            "kind": "deviceLost",
            "message": "the input device was lost: unplugged"
          },
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "error": {
            "kind": "exists",
            "message": "memo main-01 already exists"
          },
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": {
        "err": {
          "kind": "exists",
          "message": "memo main-01 already exists"
        }
      }
    },
    {
      "do": "remove the conflict",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    },
    {
      "do": "stop",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "err": {
          "kind": "other",
          "message": "no take in progress"
        }
      }
    }
  ],
  "disarm": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "disarm",
      "events": [
        {
          "state": "idle"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    },
    {
      "do": "disarm",
      "events": [
        {
          "state": "idle"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    }
  ],
  "ended-after-stop": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    },
    {
      "do": "take ended Cap at main-01's partial",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "status",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    }
  ],
  "ended-wrong-path": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "take ended Cap at other-01's partial",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "status",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": "<elapsed>",
          "permission": "authorized",
          "state": "recording",
          "stem": "main-01"
        }
      }
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    }
  ],
  "exclusive-input": [
    {
      "do": "usb lists without channels",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "write prefs fake:usb channel 2",
      "events": [],
      "files": [],
      "result": true
    },
    {
      "do": "next arm fails: microphone access is denied",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "arm",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "permissionDenied",
          "message": "microphone access denied"
        }
      }
    },
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 2,
          "device": {
            "bluetooth": false,
            "channels": 4,
            "id": "fake:usb",
            "isDefault": false,
            "name": "usb interface",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "devices",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "devices": [
            {
              "bluetooth": false,
              "channels": 1,
              "id": "fake:mic",
              "isDefault": true,
              "name": "fake mic",
              "sampleRate": 1000
            },
            {
              "bluetooth": false,
              "channels": 4,
              "id": "fake:usb",
              "isDefault": false,
              "name": "usb interface",
              "sampleRate": 1000
            }
          ],
          "selected": {
            "channel": 2,
            "deviceId": "fake:usb"
          }
        }
      }
    },
    {
      "do": "arm fake:usb channel 3",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 3,
          "device": {
            "bluetooth": false,
            "channels": 4,
            "id": "fake:usb",
            "isDefault": false,
            "name": "usb interface",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    }
  ],
  "failed-after-disarm": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "disarm",
      "events": [
        {
          "state": "idle"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    },
    {
      "do": "failed arm 1: the input device was lost: late",
      "events": [
        {
          "error": {
            "kind": "deviceLost",
            "message": "the input device was lost: late"
          },
          "state": "idle"
        }
      ],
      "files": [],
      "result": null
    }
  ],
  "invalid-start-tempo": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main cps 0",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    },
    {
      "do": "start main cps 0",
      "events": [
        {
          "state": "recording",
          "stem": "main-02"
        }
      ],
      "files": [
        ".main-02.wav.partial",
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "stem": "main-02"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-02.wav.partial",
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "stop bar1 1500000000 cps 0",
      "events": [
        {
          "state": "armed",
          "stem": "main-02"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav",
        "main-02.json",
        "main-02.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-02"
        }
      }
    }
  ],
  "no-prompt-stream-decides": [
    {
      "do": "no permission prompt",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "permission NotDetermined",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "notDetermined",
          "state": "idle",
          "stem": null
        }
      }
    },
    {
      "do": "next arm fails: microphone access is denied",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "arm",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "permissionDenied",
          "message": "microphone access denied"
        }
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "denied",
          "state": "idle",
          "stem": null
        }
      }
    },
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    }
  ],
  "prefs": [
    {
      "do": "write prefs fake:gone channel 2",
      "events": [],
      "files": [],
      "result": true
    },
    {
      "do": "prefs file",
      "events": [],
      "files": [],
      "result": "{\n  \"schema\": 1,\n  \"deviceId\": \"fake:gone\",\n  \"channel\": 2\n}\n"
    },
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "prefs file",
      "events": [],
      "files": [],
      "result": "{\n  \"schema\": 1,\n  \"deviceId\": \"fake:gone\",\n  \"channel\": 2\n}\n"
    },
    {
      "do": "arm channel 0",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "prefs file",
      "events": [],
      "files": [],
      "result": "{\n  \"schema\": 1,\n  \"deviceId\": \"fake:mic\",\n  \"channel\": 0\n}\n"
    },
    {
      "do": "arm fake:usb",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 4,
            "id": "fake:usb",
            "isDefault": false,
            "name": "usb interface",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "prefs file",
      "events": [],
      "files": [],
      "result": "{\n  \"schema\": 1,\n  \"deviceId\": \"fake:usb\",\n  \"channel\": 0\n}\n"
    }
  ],
  "publish-retry": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "block publication",
      "events": [],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": null
    },
    {
      "do": "stop bar1 1500000000",
      "events": [
        {
          "error": {
            "kind": "exists",
            "message": "memo main-01 already exists"
          },
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": {
        "err": {
          "kind": "exists",
          "message": "memo main-01 already exists"
        }
      }
    },
    {
      "do": "stop",
      "events": [
        {
          "error": {
            "kind": "exists",
            "message": "memo main-01 already exists"
          },
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": {
        "err": {
          "kind": "exists",
          "message": "memo main-01 already exists"
        }
      }
    },
    {
      "do": "remove the conflict",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "stop bar1 1500000000",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": false,
              "loopSection": null,
              "metronome": true,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": "verse"
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.005,
              "outputLatency": 0.02
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 3.0,
              "bpm": 90.0,
              "cps": 0.5,
              "meter": "6/8",
              "tempoOverride": true
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": 500.0,
              "bar1HostNs": "1500000000",
              "clockOffsetNs": {
                "end": null,
                "start": {
                  "offsetNs": "-12",
                  "rttNs": "40"
                }
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": [
                [
                  0.5,
                  "1000000123"
                ]
              ]
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    },
    {
      "do": "stop",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "err": {
          "kind": "other",
          "message": "no take in progress"
        }
      }
    }
  ],
  "quit-take": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "quit",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "sidecar main-01",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "app": {
          "version": "<app version>"
        },
        "backing": {
          "countIn": true,
          "loopSection": null,
          "metronome": false,
          "song": true,
          "songPath": "songs/main.js",
          "startSection": null
        },
        "input": {
          "channel": 0,
          "channels": 1,
          "deviceId": "fake:mic",
          "deviceName": "fake mic",
          "sampleRate": 1000
        },
        "output": {
          "baseLatency": 0.0,
          "outputLatency": null
        },
        "recovered": false,
        "schema": 1,
        "tempo": {
          "beatsPerBar": 4.0,
          "bpm": 120.0,
          "cps": 0.5,
          "meter": "4/4",
          "tempoOverride": false
        },
        "timing": {
          "anchorCount": 1,
          "bar1Frame": null,
          "bar1HostNs": null,
          "clockOffsetNs": {
            "end": null,
            "start": null
          },
          "firstAnchor": [
            0,
            "1000000000"
          ],
          "outputStamps": []
        },
        "xruns": 0
      }
    },
    {
      "do": "quit",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    }
  ],
  "rearm": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "arm fake:usb channel 2",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 2,
          "device": {
            "bluetooth": false,
            "channels": 4,
            "id": "fake:usb",
            "isDefault": false,
            "name": "usb interface",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 2,
          "device": {
            "bluetooth": false,
            "channels": 4,
            "id": "fake:usb",
            "isDefault": false,
            "name": "usb interface",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    }
  ],
  "rearm-fails-to-idle": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "next arm fails: audio backend: refused",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "arm fake:usb",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "other",
          "message": "audio backend: refused"
        }
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    }
  ],
  "reload-idle": [
    {
      "do": "reload",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "reload",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    }
  ],
  "reload-take": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "reload",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "status",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    },
    {
      "do": "stop",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "err": {
          "kind": "other",
          "message": "no take in progress"
        }
      }
    },
    {
      "do": "sidecar main-01",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "app": {
          "version": "<app version>"
        },
        "backing": {
          "countIn": true,
          "loopSection": null,
          "metronome": false,
          "song": true,
          "songPath": "songs/main.js",
          "startSection": null
        },
        "input": {
          "channel": 0,
          "channels": 1,
          "deviceId": "fake:mic",
          "deviceName": "fake mic",
          "sampleRate": 1000
        },
        "output": {
          "baseLatency": 0.0,
          "outputLatency": null
        },
        "recovered": false,
        "schema": 1,
        "tempo": {
          "beatsPerBar": 4.0,
          "bpm": 120.0,
          "cps": 0.5,
          "meter": "4/4",
          "tempoOverride": false
        },
        "timing": {
          "anchorCount": 1,
          "bar1Frame": null,
          "bar1HostNs": null,
          "clockOffsetNs": {
            "end": null,
            "start": null
          },
          "firstAnchor": [
            0,
            "1000000000"
          ],
          "outputStamps": []
        },
        "xruns": 0
      }
    },
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    }
  ],
  "repair": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "stale partial memo-09",
      "events": [],
      "files": [
        ".main-01.wav.partial",
        ".memo-09.wav.partial"
      ],
      "result": null
    },
    {
      "do": "repair on open",
      "events": [],
      "files": [
        ".main-01.wav.partial",
        ".memo-09.wav.partial"
      ],
      "result": {
        "ok": "skipped"
      }
    },
    {
      "do": "fault DeviceLost unplugged",
      "events": [
        {
          "error": {
            "kind": "deviceLost",
            "message": "the input device was lost: unplugged"
          },
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        ".memo-09.wav.partial",
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "repair on open",
      "events": [],
      "files": [
        ".memo-09.wav.partial",
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": "skipped"
      }
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        ".memo-09.wav.partial",
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    },
    {
      "do": "repair on open",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "repaired": {
            "discarded": [
              ".memo-09.wav.partial"
            ],
            "recovered": [],
            "skipped": []
          }
        }
      }
    }
  ],
  "settle-by-arm": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "block publication",
      "events": [],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "error": {
            "kind": "exists",
            "message": "memo main-01 already exists"
          },
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": {
        "err": {
          "kind": "exists",
          "message": "memo main-01 already exists"
        }
      }
    },
    {
      "do": "remove the conflict",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "sidecar main-01",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "app": {
          "version": "<app version>"
        },
        "backing": {
          "countIn": true,
          "loopSection": null,
          "metronome": false,
          "song": true,
          "songPath": "songs/main.js",
          "startSection": null
        },
        "input": {
          "channel": 0,
          "channels": 1,
          "deviceId": "fake:mic",
          "deviceName": "fake mic",
          "sampleRate": 1000
        },
        "output": {
          "baseLatency": 0.0,
          "outputLatency": null
        },
        "recovered": false,
        "schema": 1,
        "tempo": {
          "beatsPerBar": 4.0,
          "bpm": 120.0,
          "cps": 0.5,
          "meter": "4/4",
          "tempoOverride": false
        },
        "timing": {
          "anchorCount": 1,
          "bar1Frame": null,
          "bar1HostNs": null,
          "clockOffsetNs": {
            "end": null,
            "start": null
          },
          "firstAnchor": [
            0,
            "1000000000"
          ],
          "outputStamps": []
        },
        "xruns": 0
      }
    }
  ],
  "settle-by-disarm": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "block publication",
      "events": [],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "error": {
            "kind": "exists",
            "message": "memo main-01 already exists"
          },
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": {
        "err": {
          "kind": "exists",
          "message": "memo main-01 already exists"
        }
      }
    },
    {
      "do": "remove the conflict",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "disarm",
      "events": [
        {
          "state": "idle"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    },
    {
      "do": "sidecar main-01",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "app": {
          "version": "<app version>"
        },
        "backing": {
          "countIn": true,
          "loopSection": null,
          "metronome": false,
          "song": true,
          "songPath": "songs/main.js",
          "startSection": null
        },
        "input": {
          "channel": 0,
          "channels": 1,
          "deviceId": "fake:mic",
          "deviceName": "fake mic",
          "sampleRate": 1000
        },
        "output": {
          "baseLatency": 0.0,
          "outputLatency": null
        },
        "recovered": false,
        "schema": 1,
        "tempo": {
          "beatsPerBar": 4.0,
          "bpm": 120.0,
          "cps": 0.5,
          "meter": "4/4",
          "tempoOverride": false
        },
        "timing": {
          "anchorCount": 1,
          "bar1Frame": null,
          "bar1HostNs": null,
          "clockOffsetNs": {
            "end": null,
            "start": null
          },
          "firstAnchor": [
            0,
            "1000000000"
          ],
          "outputStamps": []
        },
        "xruns": 0
      }
    }
  ],
  "settle-by-quit": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "block publication",
      "events": [],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "error": {
            "kind": "exists",
            "message": "memo main-01 already exists"
          },
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": {
        "err": {
          "kind": "exists",
          "message": "memo main-01 already exists"
        }
      }
    },
    {
      "do": "remove the conflict",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "quit",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "sidecar main-01",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "app": {
          "version": "<app version>"
        },
        "backing": {
          "countIn": true,
          "loopSection": null,
          "metronome": false,
          "song": true,
          "songPath": "songs/main.js",
          "startSection": null
        },
        "input": {
          "channel": 0,
          "channels": 1,
          "deviceId": "fake:mic",
          "deviceName": "fake mic",
          "sampleRate": 1000
        },
        "output": {
          "baseLatency": 0.0,
          "outputLatency": null
        },
        "recovered": false,
        "schema": 1,
        "tempo": {
          "beatsPerBar": 4.0,
          "bpm": 120.0,
          "cps": 0.5,
          "meter": "4/4",
          "tempoOverride": false
        },
        "timing": {
          "anchorCount": 1,
          "bar1Frame": null,
          "bar1HostNs": null,
          "clockOffsetNs": {
            "end": null,
            "start": null
          },
          "firstAnchor": [
            0,
            "1000000000"
          ],
          "outputStamps": []
        },
        "xruns": 0
      }
    }
  ],
  "settle-by-reload": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "block publication",
      "events": [],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "error": {
            "kind": "exists",
            "message": "memo main-01 already exists"
          },
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": {
        "err": {
          "kind": "exists",
          "message": "memo main-01 already exists"
        }
      }
    },
    {
      "do": "remove the conflict",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "reload",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "sidecar main-01",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "app": {
          "version": "<app version>"
        },
        "backing": {
          "countIn": true,
          "loopSection": null,
          "metronome": false,
          "song": true,
          "songPath": "songs/main.js",
          "startSection": null
        },
        "input": {
          "channel": 0,
          "channels": 1,
          "deviceId": "fake:mic",
          "deviceName": "fake mic",
          "sampleRate": 1000
        },
        "output": {
          "baseLatency": 0.0,
          "outputLatency": null
        },
        "recovered": false,
        "schema": 1,
        "tempo": {
          "beatsPerBar": 4.0,
          "bpm": 120.0,
          "cps": 0.5,
          "meter": "4/4",
          "tempoOverride": false
        },
        "timing": {
          "anchorCount": 1,
          "bar1Frame": null,
          "bar1HostNs": null,
          "clockOffsetNs": {
            "end": null,
            "start": null
          },
          "firstAnchor": [
            0,
            "1000000000"
          ],
          "outputStamps": []
        },
        "xruns": 0
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    }
  ],
  "settle-by-start": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "block publication",
      "events": [],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "error": {
            "kind": "exists",
            "message": "memo main-01 already exists"
          },
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial",
        "main-01.json"
      ],
      "result": {
        "err": {
          "kind": "exists",
          "message": "memo main-01 already exists"
        }
      }
    },
    {
      "do": "remove the conflict",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-02"
        }
      ],
      "files": [
        ".main-02.wav.partial",
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "stem": "main-02"
        }
      }
    },
    {
      "do": "sidecar main-01",
      "events": [],
      "files": [
        ".main-02.wav.partial",
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "app": {
          "version": "<app version>"
        },
        "backing": {
          "countIn": true,
          "loopSection": null,
          "metronome": false,
          "song": true,
          "songPath": "songs/main.js",
          "startSection": null
        },
        "input": {
          "channel": 0,
          "channels": 1,
          "deviceId": "fake:mic",
          "deviceName": "fake mic",
          "sampleRate": 1000
        },
        "output": {
          "baseLatency": 0.0,
          "outputLatency": null
        },
        "recovered": false,
        "schema": 1,
        "tempo": {
          "beatsPerBar": 4.0,
          "bpm": 120.0,
          "cps": 0.5,
          "meter": "4/4",
          "tempoOverride": false
        },
        "timing": {
          "anchorCount": 1,
          "bar1Frame": null,
          "bar1HostNs": null,
          "clockOffsetNs": {
            "end": null,
            "start": null
          },
          "firstAnchor": [
            0,
            "1000000000"
          ],
          "outputStamps": []
        },
        "xruns": 0
      }
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "main-02"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav",
        "main-02.json",
        "main-02.wav"
      ],
      "result": {
        "ok": {
          "duration": 0.01,
          "frames": 10,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 108,
          "stem": "main-02"
        }
      }
    }
  ],
  "stale-failed": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "arm fake:usb",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 4,
            "id": "fake:usb",
            "isDefault": false,
            "name": "usb interface",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "failed arm 1: the input device was lost: old",
      "events": [],
      "files": [],
      "result": null
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 4,
            "id": "fake:usb",
            "isDefault": false,
            "name": "usb interface",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "failed arm 2: the input device was lost: now",
      "events": [
        {
          "error": {
            "kind": "deviceLost",
            "message": "the input device was lost: now"
          },
          "state": "idle"
        }
      ],
      "files": [],
      "result": null
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    }
  ],
  "start-after-loss": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "fault DeviceLost unplugged",
      "events": [
        {
          "error": {
            "kind": "deviceLost",
            "message": "the input device was lost: unplugged"
          },
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "start none",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "err": {
          "kind": "other",
          "message": "the recorder is not armed"
        }
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": null,
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "idle",
          "stem": null
        }
      }
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "idle",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    }
  ],
  "start-memo-error": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "replace recordings/ with a file",
      "events": [],
      "files": null,
      "result": null
    },
    {
      "do": "start none",
      "events": [],
      "files": null,
      "result": {
        "err": {
          "kind": "other",
          "message": "<root>/recordings: not a folder"
        }
      }
    }
  ],
  "start-names": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": "<elapsed>",
          "permission": "authorized",
          "state": "recording",
          "stem": "main-01"
        }
      }
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 0.01,
          "frames": 10,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 108,
          "stem": "main-01"
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-02"
        }
      ],
      "files": [
        ".main-02.wav.partial",
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "stem": "main-02"
        }
      }
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "main-02"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav",
        "main-02.json",
        "main-02.wav"
      ],
      "result": {
        "ok": {
          "duration": 0.01,
          "frames": 10,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 108,
          "stem": "main-02"
        }
      }
    },
    {
      "do": "start none",
      "events": [
        {
          "state": "recording",
          "stem": "memo-01"
        }
      ],
      "files": [
        ".memo-01.wav.partial",
        "main-01.json",
        "main-01.wav",
        "main-02.json",
        "main-02.wav"
      ],
      "result": {
        "ok": {
          "stem": "memo-01"
        }
      }
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "memo-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav",
        "main-02.json",
        "main-02.wav",
        "memo-01.json",
        "memo-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 0.01,
          "frames": 10,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 108,
          "stem": "memo-01"
        }
      }
    },
    {
      "do": "start a/b",
      "events": [
        {
          "state": "recording",
          "stem": "memo-02"
        }
      ],
      "files": [
        ".memo-02.wav.partial",
        "main-01.json",
        "main-01.wav",
        "main-02.json",
        "main-02.wav",
        "memo-01.json",
        "memo-01.wav"
      ],
      "result": {
        "ok": {
          "stem": "memo-02"
        }
      }
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "memo-02"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav",
        "main-02.json",
        "main-02.wav",
        "memo-01.json",
        "memo-01.wav",
        "memo-02.json",
        "memo-02.wav"
      ],
      "result": {
        "ok": {
          "duration": 0.01,
          "frames": 10,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 108,
          "stem": "memo-02"
        }
      }
    }
  ],
  "start-no-project": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start with no project open",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "noProject",
          "message": "no project is open"
        }
      }
    }
  ],
  "start-not-armed": [
    {
      "do": "start none",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "other",
          "message": "the recorder is not armed"
        }
      }
    }
  ],
  "start-stale-root": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start at root + -old",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "projectChanged",
          "message": "the project changed"
        }
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    }
  ],
  "start-timeout": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start none without audio",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "other",
          "message": "no audio arrived from the input"
        }
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    }
  ],
  "stop-bar1-outside": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "stop bar1 500000000",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": false,
              "loopSection": null,
              "metronome": true,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": "verse"
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.005,
              "outputLatency": 0.02
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 3.0,
              "bpm": 90.0,
              "cps": 0.5,
              "meter": "6/8",
              "tempoOverride": true
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": "500000000",
              "clockOffsetNs": {
                "end": null,
                "start": {
                  "offsetNs": "-12",
                  "rttNs": "40"
                }
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": [
                [
                  0.5,
                  "1000000123"
                ]
              ]
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    }
  ],
  "stop-invalid-fields": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "stop meter four",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    }
  ],
  "stop-nothing": [
    {
      "do": "stop",
      "events": [],
      "files": [],
      "result": {
        "err": {
          "kind": "other",
          "message": "no take in progress"
        }
      }
    },
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    },
    {
      "do": "stop",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "err": {
          "kind": "other",
          "message": "no take in progress"
        }
      }
    }
  ],
  "stop-other-project": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "open beta",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    },
    {
      "do": "beta recordings",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": []
      }
    }
  ],
  "stop-with-fields": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "stop bar1 1500000000",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": false,
              "loopSection": null,
              "metronome": true,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": "verse"
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.005,
              "outputLatency": 0.02
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 3.0,
              "bpm": 90.0,
              "cps": 0.5,
              "meter": "6/8",
              "tempoOverride": true
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": 500.0,
              "bar1HostNs": "1500000000",
              "clockOffsetNs": {
                "end": null,
                "start": {
                  "offsetNs": "-12",
                  "rttNs": "40"
                }
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": [
                [
                  0.5,
                  "1000000123"
                ]
              ]
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    },
    {
      "do": "status",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    }
  ],
  "stop-without-fields": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": 1.0,
          "frames": 1000,
          "hasSidecar": true,
          "sampleRate": 1000,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 1,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": [
                0,
                "1000000000"
              ],
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": 4068,
          "stem": "main-01"
        }
      }
    }
  ],
  "write-error": [
    {
      "do": "arm",
      "events": [
        {
          "state": "armed"
        }
      ],
      "files": [],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "start main",
      "events": [
        {
          "state": "recording",
          "stem": "main-01"
        }
      ],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": {
        "ok": {
          "stem": "main-01"
        }
      }
    },
    {
      "do": "push frames 10..1000",
      "events": [],
      "files": [
        ".main-01.wav.partial"
      ],
      "result": null
    },
    {
      "do": "take ended WriteError(\"disk full\") at main-01's partial",
      "events": [
        {
          "error": {
            "kind": "other",
            "message": "the take could not be written: disk full"
          },
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    },
    {
      "do": "status",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "channel": 0,
          "device": {
            "bluetooth": false,
            "channels": 1,
            "id": "fake:mic",
            "isDefault": true,
            "name": "fake mic",
            "sampleRate": 1000
          },
          "elapsedSeconds": null,
          "permission": "authorized",
          "state": "armed",
          "stem": null
        }
      }
    },
    {
      "do": "stop",
      "events": [
        {
          "state": "armed",
          "stem": "main-01"
        }
      ],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": {
        "ok": {
          "duration": "<live>",
          "frames": "<live>",
          "hasSidecar": true,
          "sampleRate": null,
          "sidecar": {
            "app": {
              "version": "<app version>"
            },
            "backing": {
              "countIn": true,
              "loopSection": null,
              "metronome": false,
              "song": true,
              "songPath": "songs/main.js",
              "startSection": null
            },
            "input": {
              "channel": 0,
              "channels": 1,
              "deviceId": "fake:mic",
              "deviceName": "fake mic",
              "sampleRate": 1000
            },
            "output": {
              "baseLatency": 0.0,
              "outputLatency": null
            },
            "recovered": false,
            "schema": 1,
            "tempo": {
              "beatsPerBar": 4.0,
              "bpm": 120.0,
              "cps": 0.5,
              "meter": "4/4",
              "tempoOverride": false
            },
            "timing": {
              "anchorCount": 0,
              "bar1Frame": null,
              "bar1HostNs": null,
              "clockOffsetNs": {
                "end": null,
                "start": null
              },
              "firstAnchor": null,
              "outputStamps": []
            },
            "xruns": 0
          },
          "sidecarError": null,
          "size": "<live>",
          "stem": "main-01"
        }
      }
    },
    {
      "do": "quit",
      "events": [],
      "files": [
        "main-01.json",
        "main-01.wav"
      ],
      "result": null
    }
  ]
}"#;
// --- expected: end ---
