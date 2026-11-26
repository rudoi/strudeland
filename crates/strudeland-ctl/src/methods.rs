//! Every method of the control protocol, with its typed params and result.
//!
//! The webview validates params (`screenshot`'s are checked by the app). These types are what a
//! request the webview accepts decodes into and what a result parses into; the wire examples
//! in `tests/wire/` are the cross-check every side reads. Field names on the wire are
//! camelCase; absent optional request fields are omitted, while nullable result fields are
//! `null`.

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
    InstrumentApply,
}

impl Method {
    /// Every method, in table order.
    pub const ALL: [Method; 16] = [
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
        Method::InstrumentApply,
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
            Method::InstrumentApply => "instrument.apply",
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
    type Params: Serialize + DeserializeOwned;
    type Result: Serialize + DeserializeOwned;
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
/// `instrument.apply`: apply a project pin to a selected pattern in the active song.
pub struct InstrumentApply;

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
impl Rpc for InstrumentApply {
    const METHOD: Method = Method::InstrumentApply;
    type Params = InstrumentApplyParams;
    type Result = InstrumentApplyResult;
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

/// A conditional edit of the active song. Offsets are UTF-16, with an exclusive end.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentApplyParams {
    pub root: String,
    /// Optional assertion of the active song's project-relative path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<String>,
    pub id: String,
    pub from: u32,
    pub to: u32,
    /// The exact active buffer from `code.get`.
    pub expect: String,
}

/// The edited song and its replacement range, in UTF-16.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentApplyResult {
    pub song: String,
    pub from: u32,
    pub to: u32,
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

/// `events`' limit when `limit` is absent.
pub const EVENTS_LIMIT_DEFAULT: u32 = 2000;
/// The largest `limit` the webview accepts.
pub const EVENTS_LIMIT_MAX: u32 = 100_000;

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
    /// Absent means [`EVENTS_LIMIT_DEFAULT`].
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
    /// Last section of a temporary multi-section loop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_through: Option<String>,
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
    use crate::wire_fixtures;
    use serde_json::{Value, json};

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
                "project.open",
                "instrument.apply"
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
    fn song_new_and_switch_params() {
        assert!(serde_json::from_str::<SongNewParams>("{}").is_err());
        assert!(serde_json::from_str::<SongSwitchParams>("{}").is_err());
    }

    #[test]
    fn song_rename_params() {
        assert!(serde_json::from_str::<SongRenameParams>(r#"{"song":"main"}"#).is_err());
    }

    #[test]
    fn project_open_params() {
        assert!(serde_json::from_str::<ProjectOpenParams>("{}").is_err());
    }

    #[test]
    fn song_results() {
        assert!(serde_json::from_str::<SongResult>("{}").is_err());
        assert!(
            serde_json::from_str::<SongDeleteResult>(r#"{"song":{"path":"p","title":"t"}}"#)
                .is_err()
        );
    }

    #[test]
    fn project_open_result_uses_already_open_key() {
        assert!(
            serde_json::from_str::<ProjectOpenResult>(
                r#"{"project":{"name":"demo","root":"/p"},"already_open":true}"#
            )
            .is_err()
        );
    }

    #[test]
    fn empty_params() {
        // Unknown members are not rejected in Rust; the webview validates.
        assert_eq!(
            serde_json::from_str::<Empty>(r#"{"x":1}"#).unwrap(),
            Empty {}
        );
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
    fn older_state_defaults_the_new_fields() {
        let s: AppState =
            serde_json::from_value(wire_fixtures::result(Method::State, "minimal")).unwrap();
        assert_eq!(s.playhead, None);
        assert_eq!(s.transport.view, TransportView::Song);
        assert_eq!(s.transport.loop_section, None);
        assert_eq!(s.transport.armed_section, None);
        assert_eq!(s.output_error, None);
        assert_eq!(s.engine, EngineId::Superdough);
        assert_eq!(s.live, None);
        assert_eq!(s.levels, None);
        assert!(!s.vim);
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
    fn state_result_missing_a_field_is_an_error() {
        let e = serde_json::from_value::<AppState>(json!({"project": null}));
        assert!(e.is_err());
    }

    #[test]
    fn stop_result_is_camel_case() {
        assert!(serde_json::from_str::<StopResult>(r#"{"was_playing":true}"#).is_err());
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
    fn transport_view_defaults_to_song() {
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
