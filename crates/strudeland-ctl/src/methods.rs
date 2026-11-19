//! Every method of the control protocol, with its typed params and result.
//!
//! The webview validates params, except for `screenshot`, which the app checks itself. These types
//! are the contract the CLI and the other clients build requests from and parse results into. Field
//! names on the wire are camelCase; absent optional request fields are omitted, while nullable
//! result fields are `null`.

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A method name on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Ping,
    State,
    CodeGet,
    CodeSet,
    Eval,
    Play,
    Stop,
    Events,
    Screenshot,
    SongNew,
    SongSwitch,
    SongRename,
    SongDelete,
    Save,
    ProjectOpen,
}

impl Method {
    /// Every method, in table order.
    pub const ALL: [Method; 15] = [
        Method::Ping,
        Method::State,
        Method::CodeGet,
        Method::CodeSet,
        Method::Eval,
        Method::Play,
        Method::Stop,
        Method::Events,
        Method::Screenshot,
        Method::SongNew,
        Method::SongSwitch,
        Method::SongRename,
        Method::SongDelete,
        Method::Save,
        Method::ProjectOpen,
    ];

    /// The wire name, e.g. `"code.get"`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Method::Ping => "ping",
            Method::State => "state",
            Method::CodeGet => "code.get",
            Method::CodeSet => "code.set",
            Method::Eval => "eval",
            Method::Play => "play",
            Method::Stop => "stop",
            Method::Events => "events",
            Method::Screenshot => "screenshot",
            Method::SongNew => "song.new",
            Method::SongSwitch => "song.switch",
            Method::SongRename => "song.rename",
            Method::SongDelete => "song.delete",
            Method::Save => "save",
            Method::ProjectOpen => "project.open",
        }
    }

    /// The method with this wire name, or `None` for an unknown one.
    pub fn parse(s: &str) -> Option<Method> {
        Method::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A method with its typed params and result; implemented by one marker type per method.
pub trait Rpc {
    const METHOD: Method;
    type Params: Serialize;
    type Result: DeserializeOwned;
}

/// `ping`: answered by the app itself, even before the webview attaches.
pub struct Ping;
/// `state`: a summary of the open project, songs and playback.
pub struct StateCall;
/// `code.get`: the buffer of a song.
pub struct CodeGet;
/// `code.set`: replace a song's buffer as one undo step.
pub struct CodeSet;
/// `eval`: evaluate a song's buffer.
pub struct EvalCall;
/// `play`: the same as `eval`, named for readability.
pub struct Play;
/// `stop`: stop playback.
pub struct Stop;
/// `events`: the events of the evaluated song over a bar range, with source locations.
pub struct EventsCall;
/// `screenshot`: a PNG of the app window, answered by the app itself.
pub struct ScreenshotCall;
/// `song.new`: create a song from the template and switch to it.
pub struct SongNewCall;
/// `song.switch`: show a song in the editor.
pub struct SongSwitchCall;
/// `song.rename`: rename a song's file, keeping its buffer.
pub struct SongRenameCall;
/// `song.delete`: delete a song's file.
pub struct SongDeleteCall;
/// `save`: write song buffers to disk.
pub struct SaveCall;
/// `project.open`: open another project.
pub struct ProjectOpenCall;

impl Rpc for Ping {
    const METHOD: Method = Method::Ping;
    type Params = Empty;
    type Result = PingResult;
}
impl Rpc for StateCall {
    const METHOD: Method = Method::State;
    type Params = Empty;
    type Result = AppState;
}
impl Rpc for CodeGet {
    const METHOD: Method = Method::CodeGet;
    type Params = SongParams;
    type Result = CodeGetResult;
}
impl Rpc for CodeSet {
    const METHOD: Method = Method::CodeSet;
    type Params = CodeSetParams;
    type Result = CodeSetResult;
}
impl Rpc for EvalCall {
    const METHOD: Method = Method::Eval;
    type Params = SongParams;
    type Result = EvalOutcome;
}
impl Rpc for Play {
    const METHOD: Method = Method::Play;
    type Params = SongParams;
    type Result = EvalOutcome;
}
impl Rpc for Stop {
    const METHOD: Method = Method::Stop;
    type Params = Empty;
    type Result = StopResult;
}
impl Rpc for EventsCall {
    const METHOD: Method = Method::Events;
    type Params = EventsParams;
    type Result = EventsResult;
}
impl Rpc for ScreenshotCall {
    const METHOD: Method = Method::Screenshot;
    type Params = ScreenshotParams;
    type Result = ScreenshotResult;
}
impl Rpc for SongNewCall {
    const METHOD: Method = Method::SongNew;
    type Params = SongNewParams;
    type Result = SongResult;
}
impl Rpc for SongSwitchCall {
    const METHOD: Method = Method::SongSwitch;
    type Params = SongSwitchParams;
    type Result = SongResult;
}
impl Rpc for SongRenameCall {
    const METHOD: Method = Method::SongRename;
    type Params = SongRenameParams;
    type Result = SongRenameResult;
}
impl Rpc for SongDeleteCall {
    const METHOD: Method = Method::SongDelete;
    type Params = SongDeleteParams;
    type Result = SongDeleteResult;
}
impl Rpc for SaveCall {
    const METHOD: Method = Method::Save;
    type Params = SaveParams;
    type Result = SaveResult;
}
impl Rpc for ProjectOpenCall {
    const METHOD: Method = Method::ProjectOpen;
    type Params = ProjectOpenParams;
    type Result = ProjectOpenResult;
}

/// The params of `ping`, `state` and `stop`: `{}`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Empty {}

/// The params of `code.get`, `eval` and `play`. `song` is a file stem (`main`) or a rel path
/// (`songs/main.js`); absent means the active song.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<String>,
}

/// The params of `code.set`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodeSetParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<String>,
    pub code: String,
    /// Absent means `auto`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval: Option<EvalMode>,
    /// The exact text last read; the call fails with `SONG_CHANGED` when the buffer differs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expect: Option<String>,
}

/// When `code.set` evaluates.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EvalMode {
    /// Only when the song is the one already playing.
    Auto,
    Always,
    Never,
}

/// The audio context's state.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Audio {
    Running,
    /// No user gesture yet: evaluating does not start playback.
    Suspended,
}

/// A song's open conflict bar.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Conflict {
    Changed,
    Deleted,
}

/// The result of `ping`. `version` is the app's version.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PingResult {
    pub version: String,
    pub attached: bool,
}

/// The open project.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRef {
    pub name: String,
    pub root: String,
}

/// One song of the open project; `path` is the rel path.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SongSummary {
    pub path: String,
    pub title: String,
    pub dirty: bool,
    pub conflict: Option<Conflict>,
}

/// The song that is playing.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Playing {
    pub path: String,
    pub title: String,
}

/// The live engine playing the song.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum EngineId {
    #[default]
    Superdough,
    Rust,
}

/// The Rust engine's per-batch decode time over the last second, in microseconds.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatchDecode {
    pub p50: u64,
    pub max: u64,
}

/// The app's `playback_schedule` round trip over the last 256 batches, in milliseconds.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
pub struct InvokeRtt {
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

/// The Rust engine's live counters and windowed timings.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LiveStats {
    pub underruns: u64,
    pub late_haps: u64,
    pub haps: u64,
    pub batches: u64,
    pub skipped_haps: u64,
    pub render_headroom_ms: f64,
    pub batch_decode_us: BatchDecode,
    pub invoke_rtt_ms: InvokeRtt,
}

/// The master output's held levels since the last play or evaluate.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Levels {
    /// Held peak per channel in dBFS; `null` for silence (a peak of 0).
    pub peak_db: [Option<f64>; 2],
    /// Held sum of both channels' over counts.
    pub overs: u64,
    /// Held lowest limiter gain in dB (0 or negative).
    pub limit_db: f64,
    /// The current limiter preference.
    pub limiter: bool,
}

/// The result of `state`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub project: Option<ProjectRef>,
    pub songs: Vec<SongSummary>,
    pub active: Option<String>,
    pub playing: Option<Playing>,
    pub started: bool,
    pub cps: f64,
    /// Whole today, but `f64` so a fractional bpm still parses.
    pub bpm: f64,
    /// `"N/D"`, e.g. `"4/4"`.
    pub meter: String,
    pub audio: Audio,
    pub eval_error: Option<String>,
    pub meta_error: Option<String>,
    pub tempo_override: bool,
    /// The playing song's `meta.seed`, when it has one. Absent from an older app's reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<f64>,
    /// Absent from an older app's reply.
    #[serde(default)]
    pub playhead: Option<Playhead>,
    /// Absent from an older app's reply.
    #[serde(default)]
    pub transport: TransportSummary,
    /// The sound error the status bar shows. Absent from an older app's reply.
    #[serde(default)]
    pub output_error: Option<String>,
    /// The engine playing the song. Absent from an older app's reply.
    #[serde(default)]
    pub engine: EngineId,
    /// The Rust engine's stats, once it has reported. Absent from an older app's reply.
    #[serde(default)]
    pub live: Option<LiveStats>,
    /// The master levels, once a frame has arrived since the last reset. Absent from an older
    /// app's reply.
    #[serde(default)]
    pub levels: Option<Levels>,
    /// Whether the editor has vim keybindings on. Absent from an older app's reply.
    #[serde(default)]
    pub vim: bool,
}

/// The result of `code.get`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodeGetResult {
    pub song: String,
    pub title: String,
    pub code: String,
    pub dirty: bool,
}

/// The result of `eval` and `play`, and `code.set`'s `evaluated`. A failed evaluate is
/// `ok: false` with an `error`, not an RPC error.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalOutcome {
    pub ok: bool,
    pub song: String,
    pub title: String,
    pub started: bool,
    pub audio: Audio,
    pub error: Option<String>,
}

/// The result of `code.set`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodeSetResult {
    pub song: String,
    pub changed: bool,
    pub dirty: bool,
    pub conflict: Option<Conflict>,
    pub evaluated: Option<EvalOutcome>,
}

/// The result of `stop`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StopResult {
    pub was_playing: bool,
}

/// The params of `events`. `bars` and `section` are mutually exclusive.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct EventsParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bars: Option<BarRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// The CLI sends `Some(true)` only for `--all`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all: Option<bool>,
    /// Absent means 2000.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// 1-based, inclusive song bars.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BarRange {
    pub from: u32,
    pub to: u32,
}

/// The result of `events`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventsResult {
    /// The evaluated song.
    pub song: SongRef,
    /// The clamped queried range, 1-based inclusive.
    pub from: u32,
    pub to: u32,
    /// The song's buffer differs from the evaluated text.
    pub stale: bool,
    /// More events matched than `limit`.
    pub truncated: bool,
    /// `"N/D"`.
    pub meter: String,
    pub tempo_override: bool,
    pub events: Vec<Event>,
}

/// A song by rel path and title, as in [`SongSummary`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SongRef {
    pub path: String,
    pub title: String,
}

/// The params of `song.new`: a new stem, validated by the project library.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SongNewParams {
    pub name: String,
}

/// The params of `song.switch`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SongSwitchParams {
    pub song: String,
}

/// The params of `song.rename`. Absent `song` means the active song.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SongRenameParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<String>,
    pub name: String,
}

/// The params of `song.delete`. Absent `song` means the active song.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongDeleteParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<String>,
    /// Delete even with unsaved edits. The CLI sends `Some(true)` only for `--discard`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discard: Option<bool>,
}

/// The params of `save`. `song` conflicts with `all: true`; neither means the active song.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SaveParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<String>,
    /// The CLI sends `Some(true)` only for `--all`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all: Option<bool>,
}

/// The params of `project.open`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOpenParams {
    /// Absolute.
    pub path: String,
    /// Open even with unsaved edits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discard: Option<bool>,
}

/// The result of `song.new` and `song.switch`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SongResult {
    pub song: SongRef,
}

/// The result of `song.rename`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SongRenameResult {
    pub from: SongRef,
    pub to: SongRef,
}

/// The result of `song.delete`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SongDeleteResult {
    /// The deleted song as it was before.
    pub song: SongRef,
    /// True only when it was the playing song.
    pub stopped: bool,
}

/// The result of `save`; empty when nothing needed saving.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    pub saved: Vec<SongRef>,
}

/// The result of `project.open`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOpenResult {
    pub project: ProjectRef,
    pub already_open: bool,
}

/// One event of the evaluated pattern.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// 1-based song bar containing `begin`. 0 or less for a continuation (under `all`) whose note
    /// began before the song's first bar.
    pub bar: i64,
    /// 1-based position in the bar, in units of 1/meter.den notes.
    pub beat: f64,
    /// Song-bar position; bar 1 starts at 0.
    pub begin: f64,
    pub end: f64,
    /// False only under `all`, for continuations and haps without a whole.
    pub onset: bool,
    /// Null for a non-`song()` pattern or a position past the song's end.
    pub section: Option<SectionPos>,
    pub value: serde_json::Map<String, serde_json::Value>,
    /// Empty when the value has no source position.
    pub locations: Vec<Location>,
}

/// A position in a section; `bar` is 1-based within the section.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SectionPos {
    pub name: String,
    pub bar: u32,
}

/// Source position in the evaluated text. `line`/`col` are 1-based; `col` counts UTF-16 code
/// units like JS string indices. `end_line`/`end_col` are just after the last character.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    /// The source line containing the start, trimmed and cut to 120 chars.
    pub text: String,
}

/// The params of `screenshot`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotParams {
    /// Absolute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The result of `screenshot`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotResult {
    /// The file written.
    pub path: String,
    /// Pixel size of the PNG.
    pub width: u32,
    pub height: u32,
}

/// Where the scheduler is.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Playhead {
    /// The scheduler's cycle, >= 0.
    pub cycle: f64,
    /// 1-based; null for a non-`song()` pattern or past the end.
    pub song_bar: Option<u32>,
    /// Null exactly when `song_bar` is null.
    pub section: Option<SectionPos>,
}

/// The song transport's mode.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TransportSummary {
    pub view: TransportView,
    /// Section name while `view` is `loop`, else null.
    pub loop_section: Option<String>,
    /// Section name armed while stopped, else null.
    pub armed_section: Option<String>,
}

/// Which view of the song the transport plays.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TransportView {
    #[default]
    Song,
    Loop,
    Release,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// Parses `wire` into `T`, checks it equals `expected`, and that it serializes back to
    /// exactly the same JSON value.
    fn round_trip<T>(wire: &str, expected: T)
    where
        T: Serialize + DeserializeOwned + PartialEq + fmt::Debug,
    {
        let parsed: T = serde_json::from_str(wire).unwrap();
        assert_eq!(parsed, expected);
        let back = serde_json::to_value(&parsed).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(wire).unwrap());
    }

    #[test]
    fn method_names_round_trip() {
        let names: Vec<_> = Method::ALL.iter().map(|m| m.as_str()).collect();
        assert_eq!(
            names,
            [
                "ping",
                "state",
                "code.get",
                "code.set",
                "eval",
                "play",
                "stop",
                "events",
                "screenshot",
                "song.new",
                "song.switch",
                "song.rename",
                "song.delete",
                "save",
                "project.open"
            ]
        );
        for m in Method::ALL {
            assert_eq!(Method::parse(m.as_str()), Some(m));
            assert_eq!(m.to_string(), m.as_str());
        }
    }

    #[test]
    fn unknown_method_is_none() {
        for s in ["nope", "", "Ping", "code", "code.", "code_get", " ping"] {
            assert_eq!(Method::parse(s), None, "{s:?}");
        }
    }

    #[test]
    fn markers_carry_their_method() {
        assert_eq!(Ping::METHOD, Method::Ping);
        assert_eq!(StateCall::METHOD, Method::State);
        assert_eq!(CodeGet::METHOD, Method::CodeGet);
        assert_eq!(CodeSet::METHOD, Method::CodeSet);
        assert_eq!(EvalCall::METHOD, Method::Eval);
        assert_eq!(Play::METHOD, Method::Play);
        assert_eq!(Stop::METHOD, Method::Stop);
        assert_eq!(EventsCall::METHOD, Method::Events);
        assert_eq!(ScreenshotCall::METHOD, Method::Screenshot);
        assert_eq!(SongNewCall::METHOD, Method::SongNew);
        assert_eq!(SongSwitchCall::METHOD, Method::SongSwitch);
        assert_eq!(SongRenameCall::METHOD, Method::SongRename);
        assert_eq!(SongDeleteCall::METHOD, Method::SongDelete);
        assert_eq!(SaveCall::METHOD, Method::Save);
        assert_eq!(ProjectOpenCall::METHOD, Method::ProjectOpen);
        assert_eq!(Method::ALL.len(), 15);
    }

    fn song_ref(path: &str, title: &str) -> SongRef {
        SongRef {
            path: path.into(),
            title: title.into(),
        }
    }

    #[test]
    fn song_new_and_switch_params() {
        round_trip(
            r#"{"name":"main2"}"#,
            SongNewParams {
                name: "main2".into(),
            },
        );
        round_trip(
            r#"{"song":"verse"}"#,
            SongSwitchParams {
                song: "verse".into(),
            },
        );
        assert!(serde_json::from_str::<SongNewParams>("{}").is_err());
        assert!(serde_json::from_str::<SongSwitchParams>("{}").is_err());
    }

    #[test]
    fn song_rename_params() {
        round_trip(
            r#"{"name":"intro"}"#,
            SongRenameParams {
                song: None,
                name: "intro".into(),
            },
        );
        round_trip(
            r#"{"song":"main","name":"intro"}"#,
            SongRenameParams {
                song: Some("main".into()),
                name: "intro".into(),
            },
        );
        assert!(serde_json::from_str::<SongRenameParams>(r#"{"song":"main"}"#).is_err());
    }

    #[test]
    fn song_delete_params_omit_absent_optionals() {
        round_trip("{}", SongDeleteParams::default());
        round_trip(
            r#"{"song":"verse","discard":true}"#,
            SongDeleteParams {
                song: Some("verse".into()),
                discard: Some(true),
            },
        );
    }

    #[test]
    fn save_params_omit_absent_optionals() {
        round_trip("{}", SaveParams::default());
        round_trip(
            r#"{"song":"main"}"#,
            SaveParams {
                song: Some("main".into()),
                all: None,
            },
        );
        round_trip(
            r#"{"all":true}"#,
            SaveParams {
                song: None,
                all: Some(true),
            },
        );
    }

    #[test]
    fn project_open_params() {
        round_trip(
            r#"{"path":"/p"}"#,
            ProjectOpenParams {
                path: "/p".into(),
                discard: None,
            },
        );
        round_trip(
            r#"{"path":"/p","discard":true}"#,
            ProjectOpenParams {
                path: "/p".into(),
                discard: Some(true),
            },
        );
        assert!(serde_json::from_str::<ProjectOpenParams>("{}").is_err());
    }

    #[test]
    fn song_results() {
        round_trip(
            r#"{"song":{"path":"songs/a.js","title":"a"}}"#,
            SongResult {
                song: song_ref("songs/a.js", "a"),
            },
        );
        assert!(serde_json::from_str::<SongResult>("{}").is_err());
        round_trip(
            r#"{"from":{"path":"songs/a.js","title":"a"},"to":{"path":"songs/b.js","title":"b"}}"#,
            SongRenameResult {
                from: song_ref("songs/a.js", "a"),
                to: song_ref("songs/b.js", "b"),
            },
        );
        round_trip(
            r#"{"song":{"path":"songs/a.js","title":"a"},"stopped":true}"#,
            SongDeleteResult {
                song: song_ref("songs/a.js", "a"),
                stopped: true,
            },
        );
        assert!(
            serde_json::from_str::<SongDeleteResult>(r#"{"song":{"path":"p","title":"t"}}"#)
                .is_err()
        );
    }

    #[test]
    fn save_result() {
        round_trip(r#"{"saved":[]}"#, SaveResult { saved: vec![] });
        round_trip(
            r#"{"saved":[{"path":"songs/a.js","title":"a"}]}"#,
            SaveResult {
                saved: vec![song_ref("songs/a.js", "a")],
            },
        );
    }

    #[test]
    fn project_open_result_uses_already_open_key() {
        round_trip(
            r#"{"project":{"name":"demo","root":"/p"},"alreadyOpen":false}"#,
            ProjectOpenResult {
                project: ProjectRef {
                    name: "demo".into(),
                    root: "/p".into(),
                },
                already_open: false,
            },
        );
        assert!(
            serde_json::from_str::<ProjectOpenResult>(
                r#"{"project":{"name":"demo","root":"/p"},"already_open":true}"#
            )
            .is_err()
        );
    }

    #[test]
    fn empty_params() {
        round_trip("{}", Empty {});
        // Unknown members are not rejected in Rust; the webview validates.
        assert_eq!(
            serde_json::from_str::<Empty>(r#"{"x":1}"#).unwrap(),
            Empty {}
        );
    }

    #[test]
    fn song_params_omit_an_absent_song() {
        round_trip("{}", SongParams { song: None });
        round_trip(
            r#"{"song":"main"}"#,
            SongParams {
                song: Some("main".into()),
            },
        );
        round_trip(
            r#"{"song":"songs/verse.js"}"#,
            SongParams {
                song: Some("songs/verse.js".into()),
            },
        );
    }

    #[test]
    fn code_set_params_full_example() {
        round_trip(
            r#"{"song":"main","code":"$: s(\"bd sd\")\n","eval":"auto","expect":"$: s(\"bd\")\n"}"#,
            CodeSetParams {
                song: Some("main".into()),
                code: "$: s(\"bd sd\")\n".into(),
                eval: Some(EvalMode::Auto),
                expect: Some("$: s(\"bd\")\n".into()),
            },
        );
    }

    #[test]
    fn code_set_params_omit_absent_optionals() {
        round_trip(
            r#"{"code":"x","expect":"y"}"#,
            CodeSetParams {
                code: "x".into(),
                expect: Some("y".into()),
                ..Default::default()
            },
        );
        let p = CodeSetParams {
            code: "x".into(),
            ..Default::default()
        };
        assert_eq!(serde_json::to_string(&p).unwrap(), r#"{"code":"x"}"#);
    }

    #[test]
    fn code_set_params_need_code() {
        assert!(serde_json::from_str::<CodeSetParams>(r#"{"song":"main"}"#).is_err());
    }

    #[test]
    fn enums_are_lowercase() {
        assert_eq!(
            serde_json::to_value([EvalMode::Auto, EvalMode::Always, EvalMode::Never]).unwrap(),
            json!(["auto", "always", "never"])
        );
        assert_eq!(
            serde_json::to_value([Audio::Running, Audio::Suspended]).unwrap(),
            json!(["running", "suspended"])
        );
        assert_eq!(
            serde_json::to_value([Conflict::Changed, Conflict::Deleted]).unwrap(),
            json!(["changed", "deleted"])
        );
        assert!(serde_json::from_str::<EvalMode>(r#""Auto""#).is_err());
        assert!(serde_json::from_str::<Audio>(r#""closed""#).is_err());
    }

    #[test]
    fn ping_result() {
        round_trip(
            r#"{"version":"0.1.0","attached":true}"#,
            PingResult {
                version: "0.1.0".into(),
                attached: true,
            },
        );
    }

    const STATE_EXAMPLE: &str = r#"{"project":{"name":"demo","root":"/Users/me/demo"},"songs":[{"path":"songs/main.js","title":"Main Theme","dirty":true,"conflict":null}],"active":"songs/main.js","playing":{"path":"songs/main.js","title":"Main Theme"},"started":true,"cps":0.5,"bpm":120,"meter":"4/4","audio":"running","evalError":null,"metaError":null,"tempoOverride":false,"playhead":null,"transport":{"view":"song","loopSection":null,"armedSection":null},"outputError":null,"engine":"superdough","live":null,"levels":null,"vim":false}"#;

    #[test]
    fn state_result_example() {
        // The app sends `"bpm":120`; Rust writes it back as `120.0`, which is the same number.
        let wire = STATE_EXAMPLE.replace(r#""bpm":120,"#, r#""bpm":120.0,"#);
        let from_app: AppState = serde_json::from_str(STATE_EXAMPLE).unwrap();
        assert_eq!(from_app, serde_json::from_str::<AppState>(&wire).unwrap());
        round_trip(
            &wire,
            AppState {
                project: Some(ProjectRef {
                    name: "demo".into(),
                    root: "/Users/me/demo".into(),
                }),
                songs: vec![SongSummary {
                    path: "songs/main.js".into(),
                    title: "Main Theme".into(),
                    dirty: true,
                    conflict: None,
                }],
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
                transport: TransportSummary::default(),
                output_error: None,
                engine: EngineId::Superdough,
                live: None,
                levels: None,
                vim: false,
            },
        );
    }

    #[test]
    fn state_with_engine_and_live_round_trips() {
        let live = LiveStats {
            underruns: 1,
            late_haps: 2,
            haps: 300,
            batches: 40,
            skipped_haps: 5,
            render_headroom_ms: 38.5,
            batch_decode_us: BatchDecode { p50: 120, max: 900 },
            invoke_rtt_ms: InvokeRtt {
                p50: 1.5,
                p95: 4.1,
                max: 9.0,
            },
        };
        let mut s: AppState = serde_json::from_str(STATE_EXAMPLE).unwrap();
        s.engine = EngineId::Rust;
        s.live = Some(live);
        let text = serde_json::to_string(&s).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["engine"], "rust");
        assert_eq!(v["live"]["lateHaps"], 2);
        assert_eq!(v["live"]["batchDecodeUs"]["p50"], 120);
        assert_eq!(v["live"]["invokeRttMs"]["p95"], 4.1);
        assert_eq!(serde_json::from_str::<AppState>(&text).unwrap(), s);
    }

    #[test]
    fn state_without_engine_and_live_defaults_them() {
        let s: AppState = serde_json::from_str(STATE_EXAMPLE).unwrap();
        assert_eq!(s.engine, EngineId::Superdough);
        assert_eq!(s.live, None);
        assert_eq!(s.levels, None);
    }

    #[test]
    fn state_with_levels_round_trips() {
        let mut s: AppState = serde_json::from_str(STATE_EXAMPLE).unwrap();
        s.levels = Some(Levels {
            peak_db: [None, Some(2.1)],
            overs: 12,
            limit_db: -2.3,
            limiter: true,
        });
        let text = serde_json::to_string(&s).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["levels"]["peakDb"], json!([null, 2.1]));
        assert_eq!(v["levels"]["limitDb"], -2.3);
        assert_eq!(serde_json::from_str::<AppState>(&text).unwrap(), s);
    }

    #[test]
    fn state_result_without_a_project() {
        let s: AppState = serde_json::from_value(json!({
            "project": null, "songs": [], "active": null, "playing": null,
            "started": false, "cps": 0.5, "bpm": 120.5, "meter": "3/4", "audio": "suspended",
            "evalError": "boom", "metaError": null, "tempoOverride": true
        }))
        .unwrap();
        assert_eq!(s.project, None);
        assert!(s.songs.is_empty());
        assert_eq!(s.bpm, 120.5);
        assert_eq!(s.audio, Audio::Suspended);
        assert_eq!(s.eval_error.as_deref(), Some("boom"));
        assert!(s.tempo_override);
        assert_eq!(s.seed, None);
        // Nullable result fields serialize as null, not omitted.
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["project"], Value::Null);
        assert_eq!(v["metaError"], Value::Null);
        // An absent seed is left out, not null.
        assert!(v.as_object().unwrap().get("seed").is_none());
    }

    #[test]
    fn state_result_with_a_seed_round_trips() {
        let s: AppState = serde_json::from_value(json!({
            "project": null, "songs": [], "active": null, "playing": null,
            "started": false, "cps": 0.5, "bpm": 120.0, "meter": "4/4", "audio": "suspended",
            "evalError": null, "metaError": null, "tempoOverride": false, "seed": -3.5
        }))
        .unwrap();
        assert_eq!(s.seed, Some(-3.5));
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["seed"], json!(-3.5));
        assert_eq!(serde_json::from_value::<AppState>(v).unwrap(), s);
    }

    #[test]
    fn state_result_with_a_conflict() {
        let s: SongSummary = serde_json::from_str(
            r#"{"path":"songs/bridge.js","title":"Bridge","dirty":false,"conflict":"changed"}"#,
        )
        .unwrap();
        assert_eq!(s.conflict, Some(Conflict::Changed));
    }

    #[test]
    fn state_result_missing_a_field_is_an_error() {
        let e = serde_json::from_value::<AppState>(json!({"project": null}));
        assert!(e.is_err());
    }

    #[test]
    fn code_get_result_example() {
        round_trip(
            r#"{"song":"songs/main.js","title":"Main Theme","code":"$: s(\"bd\")\n","dirty":false}"#,
            CodeGetResult {
                song: "songs/main.js".into(),
                title: "Main Theme".into(),
                code: "$: s(\"bd\")\n".into(),
                dirty: false,
            },
        );
    }

    #[test]
    fn code_set_result_example() {
        round_trip(
            r#"{"song":"songs/main.js","changed":true,"dirty":true,"conflict":null,"evaluated":{"ok":true,"song":"songs/main.js","title":"Main Theme","started":true,"audio":"running","error":null}}"#,
            CodeSetResult {
                song: "songs/main.js".into(),
                changed: true,
                dirty: true,
                conflict: None,
                evaluated: Some(EvalOutcome {
                    ok: true,
                    song: "songs/main.js".into(),
                    title: "Main Theme".into(),
                    started: true,
                    audio: Audio::Running,
                    error: None,
                }),
            },
        );
    }

    #[test]
    fn code_set_result_without_evaluate() {
        round_trip(
            r#"{"song":"songs/main.js","changed":false,"dirty":false,"conflict":"deleted","evaluated":null}"#,
            CodeSetResult {
                song: "songs/main.js".into(),
                changed: false,
                dirty: false,
                conflict: Some(Conflict::Deleted),
                evaluated: None,
            },
        );
    }

    #[test]
    fn eval_outcome_examples() {
        round_trip(
            r#"{"ok":false,"song":"songs/main.js","title":"Main Theme","started":false,"audio":"running","error":"sound nope not found!"}"#,
            EvalOutcome {
                ok: false,
                song: "songs/main.js".into(),
                title: "Main Theme".into(),
                started: false,
                audio: Audio::Running,
                error: Some("sound nope not found!".into()),
            },
        );
        round_trip(
            r#"{"ok":true,"song":"songs/verse.js","title":"Verse","started":false,"audio":"suspended","error":null}"#,
            EvalOutcome {
                ok: true,
                song: "songs/verse.js".into(),
                title: "Verse".into(),
                started: false,
                audio: Audio::Suspended,
                error: None,
            },
        );
    }

    #[test]
    fn stop_result_is_camel_case() {
        round_trip(r#"{"wasPlaying":true}"#, StopResult { was_playing: true });
        assert!(serde_json::from_str::<StopResult>(r#"{"was_playing":true}"#).is_err());
    }

    #[test]
    fn state_without_the_new_fields_defaults_them() {
        let s: AppState = serde_json::from_value(json!({
            "project": null, "songs": [], "active": null, "playing": null,
            "started": false, "cps": 0.5, "bpm": 120, "meter": "4/4", "audio": "running",
            "evalError": null, "metaError": null, "tempoOverride": false
        }))
        .unwrap();
        assert_eq!(s.playhead, None);
        assert_eq!(s.transport.view, TransportView::Song);
        assert_eq!(s.transport.loop_section, None);
        assert_eq!(s.transport.armed_section, None);
        assert_eq!(s.output_error, None);
        assert_eq!(s.engine, EngineId::Superdough);
        assert_eq!(s.live, None);
        assert!(!s.vim);
    }

    #[test]
    fn state_with_playhead_and_transport() {
        let s: AppState = serde_json::from_value(json!({
            "project": null, "songs": [], "active": null, "playing": null,
            "started": true, "cps": 0.5, "bpm": 120, "meter": "4/4", "audio": "running",
            "evalError": null, "metaError": null, "tempoOverride": false,
            "playhead": {"cycle": 5.5, "songBar": 6, "section": {"name": "verse", "bar": 2}},
            "transport": {"view": "loop", "loopSection": "verse", "armedSection": null},
            "outputError": "no sound",
            "vim": true
        }))
        .unwrap();
        assert_eq!(s.playhead.as_ref().unwrap().song_bar, Some(6));
        assert_eq!(s.transport.view, TransportView::Loop);
        assert_eq!(s.transport.loop_section.as_deref(), Some("verse"));
        assert_eq!(s.output_error.as_deref(), Some("no sound"));
        assert!(s.vim);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["transport"]["armedSection"], Value::Null);
    }

    #[test]
    fn events_params_omit_absent_fields() {
        round_trip("{}", EventsParams::default());
        round_trip(
            r#"{"bars":{"from":2,"to":4},"all":true,"limit":10}"#,
            EventsParams {
                bars: Some(BarRange { from: 2, to: 4 }),
                section: None,
                all: Some(true),
                limit: Some(10),
            },
        );
        round_trip(
            r#"{"section":"verse"}"#,
            EventsParams {
                section: Some("verse".into()),
                ..Default::default()
            },
        );
    }

    fn example_event() -> Event {
        Event {
            bar: 3,
            beat: 2.5,
            begin: 2.125,
            end: 2.25,
            onset: true,
            section: Some(SectionPos {
                name: "verse".into(),
                bar: 1,
            }),
            value: serde_json::from_str(r#"{"s":"bd","gain":0.8}"#).unwrap(),
            locations: vec![Location {
                line: 4,
                col: 7,
                end_line: 4,
                end_col: 9,
                text: "$: s(\"bd\")".into(),
            }],
        }
    }

    #[test]
    fn events_result_round_trips() {
        round_trip(
            r#"{"song":{"path":"songs/main.js","title":"Main"},"from":1,"to":4,"stale":false,"truncated":true,"meter":"4/4","tempoOverride":false,"events":[{"bar":3,"beat":2.5,"begin":2.125,"end":2.25,"onset":true,"section":{"name":"verse","bar":1},"value":{"s":"bd","gain":0.8},"locations":[{"line":4,"col":7,"endLine":4,"endCol":9,"text":"$: s(\"bd\")"}]}]}"#,
            EventsResult {
                song: SongRef {
                    path: "songs/main.js".into(),
                    title: "Main".into(),
                },
                from: 1,
                to: 4,
                stale: false,
                truncated: true,
                meter: "4/4".into(),
                tempo_override: false,
                events: vec![example_event()],
            },
        );
    }

    #[test]
    fn event_before_the_first_bar_has_a_non_positive_bar() {
        let e = Event {
            bar: -1,
            section: None,
            onset: false,
            ..example_event()
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["bar"], -1);
        assert_eq!(serde_json::from_value::<Event>(v).unwrap(), e);
    }

    #[test]
    fn event_without_a_section_or_locations() {
        let e = Event {
            section: None,
            locations: vec![],
            onset: false,
            ..example_event()
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["section"], Value::Null);
        assert_eq!(v["locations"], json!([]));
        assert_eq!(v["onset"], json!(false));
        assert_eq!(serde_json::from_value::<Event>(v).unwrap(), e);
    }

    #[test]
    fn location_is_camel_case() {
        round_trip(
            r#"{"line":1,"col":2,"endLine":3,"endCol":4,"text":"x"}"#,
            Location {
                line: 1,
                col: 2,
                end_line: 3,
                end_col: 4,
                text: "x".into(),
            },
        );
    }

    #[test]
    fn screenshot_types() {
        round_trip("{}", ScreenshotParams::default());
        round_trip(
            r#"{"path":"/tmp/a.png"}"#,
            ScreenshotParams {
                path: Some("/tmp/a.png".into()),
            },
        );
        round_trip(
            r#"{"path":"/tmp/a.png","width":1600,"height":1000}"#,
            ScreenshotResult {
                path: "/tmp/a.png".into(),
                width: 1600,
                height: 1000,
            },
        );
    }

    #[test]
    fn playhead_and_transport_round_trip() {
        round_trip(
            r#"{"cycle":4.5,"songBar":null,"section":null}"#,
            Playhead {
                cycle: 4.5,
                song_bar: None,
                section: None,
            },
        );
        round_trip(
            r#"{"view":"release","loopSection":null,"armedSection":"chorus"}"#,
            TransportSummary {
                view: TransportView::Release,
                loop_section: None,
                armed_section: Some("chorus".into()),
            },
        );
        assert_eq!(TransportSummary::default().view, TransportView::Song);
        assert_eq!(
            serde_json::to_value([
                TransportView::Song,
                TransportView::Loop,
                TransportView::Release
            ])
            .unwrap(),
            json!(["song", "loop", "release"])
        );
    }
}
