//! Command behaviour over a [`Transport`], so it is testable without a socket.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use strudeland_ctl::{
    AppState, CallError, CodeGetResult, CodeSetParams, EvalMode, EventsParams, Method, PingResult,
    ProjectOpenParams, SaveParams, ScreenshotParams, SongDeleteParams, SongNewParams, SongParams,
    SongRenameParams, SongSwitchParams, StopResult, codes,
};
use strudeland_project::{InstrumentLibrary, Project, ProjectError, RecordingEntry, Sidecar};
use strudeland_render::export::{self, Finished, ReplacePolicy};
use strudeland_transcribe::load::{Span, load_wav_span};
use strudeland_transcribe::report::BackingReport;
use strudeland_transcribe::{
    GridHint, Hinted, Options, TranscribeError, Transcription, parse_key_override, parse_meter,
    transcribe,
};

use crate::cli::{
    Cli, CodeCommand, Command, InstrumentCommand, MemoCommand, ProjectCommand, SkillCommand,
    SongCommand, TranscribeArgs,
};
use crate::docs;
use crate::output::{self, Report};
use crate::render::{self, ProgressLines};
use crate::skill;

mod instruments;
mod mcp;
use instruments::run_instrument;

pub const EXIT_OK: i32 = 0;
pub const EXIT_APP_ERROR: i32 = 1;
pub const EXIT_NOT_RUNNING: i32 = 2;
pub const EXIT_TIMEOUT: i32 = 3;
pub const EXIT_USAGE: i32 = 4;

/// Sends one request and returns the raw `result`.
pub trait Transport {
    fn call(&mut self, method: Method, params: Value) -> Result<Value, CallError>;
}

pub struct Env<'a> {
    pub stdin: &'a mut dyn Read,
    pub stdin_is_tty: bool,
    pub stdout: &'a mut dyn Write,
    pub stdout_is_tty: bool,
    pub stderr: &'a mut dyn Write,
}

/// Why a command stopped before it could print its result.
enum Failure {
    Usage(String),
    Call(CallError),
    Render(strudeland_render::RenderError),
    Project(ProjectError),
    Transcribe(TranscribeError),
}

impl Failure {
    /// The client's read timed out, or the app reported that the webview did not answer in time.
    fn is_timeout(&self) -> bool {
        match self {
            Failure::Call(CallError::Timeout) => true,
            Failure::Call(CallError::Rpc(e)) => e.code == codes::TIMEOUT,
            _ => false,
        }
    }

    fn exit_code(&self) -> i32 {
        if self.is_timeout() {
            return EXIT_TIMEOUT;
        }
        match self {
            Failure::Usage(_) => EXIT_USAGE,
            Failure::Project(ProjectError::InvalidRecordingName(_)) => EXIT_USAGE,
            Failure::Project(_) | Failure::Transcribe(_) => EXIT_APP_ERROR,
            Failure::Render(e) => match e.exit_class() {
                strudeland_render::ExitClass::Usage => EXIT_USAGE,
                strudeland_render::ExitClass::Failed => EXIT_APP_ERROR,
            },
            Failure::Call(CallError::NotRunning { .. }) => EXIT_NOT_RUNNING,
            Failure::Call(CallError::Rpc(e)) if e.code == codes::NOT_READY => EXIT_TIMEOUT,
            Failure::Call(_) => EXIT_APP_ERROR,
        }
    }

    /// A line of advice that follows the error for a refusal the caller can act on.
    fn hint(&self) -> Option<&'static str> {
        match self {
            Failure::Call(CallError::Rpc(e)) if e.code == codes::UNSAVED => {
                Some(output::UNSAVED_HINT)
            }
            Failure::Call(CallError::Rpc(e)) if e.code == codes::BUSY => Some(output::BUSY_HINT),
            Failure::Render(strudeland_render::RenderError::Plan(
                strudeland_render::HostError::Timeout { .. },
            )) => Some(output::PLAN_TIMEOUT_HINT),
            _ => None,
        }
    }

    fn message(&self) -> String {
        let text = match self {
            Failure::Usage(m) => m.clone(),
            Failure::Render(e) => e.to_string(),
            Failure::Project(e) => e.to_string(),
            Failure::Transcribe(e) => e.to_string(),
            Failure::Call(CallError::Rpc(e)) => e.message.clone(),
            Failure::Call(e) => e.to_string(),
        };
        if self.is_timeout() {
            format!("error: {text} {}", output::TIMEOUT_HINT)
        } else {
            format!("error: {text}")
        }
    }
}

impl From<ProjectError> for Failure {
    fn from(error: ProjectError) -> Self {
        Self::Project(error)
    }
}

impl From<CallError> for Failure {
    fn from(e: CallError) -> Self {
        Failure::Call(e)
    }
}

/// Runs a command and returns the process exit code.
pub fn run(cli: &Cli, transport: &mut dyn Transport, env: &mut Env) -> i32 {
    run_with(cli, transport, &mut export::RealPipeline, env)
}

/// [`run`] with the export pipeline injected.
pub fn run_with(
    cli: &Cli,
    transport: &mut dyn Transport,
    pipeline: &mut dyn export::Pipeline,
    env: &mut Env,
) -> i32 {
    match execute(cli, transport, pipeline, env) {
        Ok(code) => code,
        Err(f) => {
            let _ = writeln!(env.stderr, "{}", f.message());
            if let Some(hint) = f.hint() {
                let _ = writeln!(env.stderr, "{hint}");
            }
            f.exit_code()
        }
    }
}

fn call<T: DeserializeOwned>(
    transport: &mut dyn Transport,
    method: Method,
    params: Value,
) -> Result<(Value, T), Failure> {
    let raw = transport.call(method, params)?;
    let typed = T::deserialize(&raw)
        .map_err(|e| Failure::Call(CallError::Protocol(format!("invalid {method} result: {e}"))))?;
    Ok((raw, typed))
}

fn params<P: serde::Serialize>(p: &P) -> Value {
    serde_json::to_value(p).expect("params serialize")
}

/// Prints a command's output: the raw result on one line for `--json`, else the human text.
fn emit(env: &mut Env, json: bool, raw: &Value, human: Report) -> i32 {
    if json {
        let _ = writeln!(env.stdout, "{raw}");
    } else {
        let _ = env.stdout.write_all(human.stdout.as_bytes());
    }
    let _ = env.stderr.write_all(human.stderr.as_bytes());
    let _ = env.stdout.flush();
    if human.failed {
        EXIT_APP_ERROR
    } else {
        EXIT_OK
    }
}

/// Calls `method`, then prints its result with [`emit`], formatting the human text with `format`.
fn report<T: DeserializeOwned>(
    transport: &mut dyn Transport,
    env: &mut Env,
    json: bool,
    method: Method,
    params: Value,
    format: impl FnOnce(&T) -> Report,
) -> Result<i32, Failure> {
    let (raw, r) = call(transport, method, params)?;
    Ok(emit(env, json, &raw, format(&r)))
}

/// A report with only stdout text.
fn stdout_only(stdout: String) -> Report {
    Report {
        stdout,
        ..Report::default()
    }
}

fn execute(
    cli: &Cli,
    transport: &mut dyn Transport,
    pipeline: &mut dyn export::Pipeline,
    env: &mut Env,
) -> Result<i32, Failure> {
    let json = cli.json;
    match &cli.command {
        Command::Mcp => Ok(mcp::serve(cli.socket.clone(), cli.timeout, env.stderr)),
        Command::Render(args) => run_render(args, json, pipeline, env),
        Command::Instrument { command } => run_instrument(
            command,
            json,
            transport,
            env,
            &current_dir,
            InstrumentLibrary::open_default,
        ),
        Command::Docs { name, search, .. } => {
            Ok(run_docs(name.as_deref(), search.as_deref(), json, env))
        }
        Command::Memo { command } => run_memo(command, json, transport, env, &current_dir),
        Command::Transcribe(args) => run_transcribe(args, json, transport, env, &current_dir),
        Command::Skill {
            command: SkillCommand::Print,
        } => Ok(print_skill(json, env)),
        Command::Skill {
            command: SkillCommand::Install { project },
        } => {
            let user = skill::claude_dir(
                std::env::var_os("CLAUDE_CONFIG_DIR"),
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .or_else(|| {
                        if cfg!(windows) {
                            std::env::var_os("USERPROFILE")
                        } else {
                            None
                        }
                    }),
            );
            install_skill(project.as_deref(), user.as_deref(), &current_dir, json, env)
        }
        Command::Ping => {
            let mut not_ready = false;
            let code = report(
                transport,
                env,
                json,
                Method::Ping,
                json!({}),
                |r: &PingResult| {
                    not_ready = !r.attached;
                    output::format_ping(r)
                },
            )?;
            Ok(if not_ready && code == EXIT_OK {
                EXIT_TIMEOUT
            } else {
                code
            })
        }
        Command::State => report(
            transport,
            env,
            json,
            Method::State,
            json!({}),
            |s: &AppState| stdout_only(output::format_state(s)),
        ),
        Command::Stop => report(
            transport,
            env,
            json,
            Method::Stop,
            json!({}),
            |r: &StopResult| stdout_only(output::format_stop(r.was_playing)),
        ),
        Command::Events {
            bars,
            section,
            all,
            limit,
        } => {
            let p = params(&EventsParams {
                bars: *bars,
                section: section.clone(),
                all: all.then_some(true),
                limit: *limit,
            });
            report(
                transport,
                env,
                json,
                Method::Events,
                p,
                output::format_events,
            )
        }
        Command::Screenshot { path } => {
            let path = path
                .as_deref()
                .map(|p| {
                    Ok::<_, Failure>(absolute(p, &current_dir()?).to_string_lossy().into_owned())
                })
                .transpose()?;
            let p = params(&ScreenshotParams { path });
            report(
                transport,
                env,
                json,
                Method::Screenshot,
                p,
                output::format_screenshot,
            )
        }
        Command::Song { command } => match command {
            SongCommand::New { name } => report(
                transport,
                env,
                json,
                Method::SongNew,
                params(&SongNewParams { name: name.clone() }),
                output::format_song_new,
            ),
            SongCommand::Switch { song } => report(
                transport,
                env,
                json,
                Method::SongSwitch,
                params(&SongSwitchParams { song: song.clone() }),
                output::format_song_switch,
            ),
            SongCommand::Rename { song, name } => report(
                transport,
                env,
                json,
                Method::SongRename,
                params(&SongRenameParams {
                    song: song.clone(),
                    name: name.clone(),
                }),
                output::format_song_rename,
            ),
            SongCommand::Delete { song, discard } => report(
                transport,
                env,
                json,
                Method::SongDelete,
                params(&SongDeleteParams {
                    song: song.clone(),
                    discard: discard.then_some(true),
                }),
                output::format_song_delete,
            ),
        },
        Command::Save { song, all } => {
            let p = params(&SaveParams {
                song: song.clone(),
                all: all.then_some(true),
            });
            report(transport, env, json, Method::Save, p, output::format_save)
        }
        Command::Project {
            command: ProjectCommand::Open { path, discard },
        } => {
            let path = absolute(path, &current_dir()?)
                .to_string_lossy()
                .into_owned();
            let p = params(&ProjectOpenParams {
                path,
                discard: discard.then_some(true),
            });
            report(
                transport,
                env,
                json,
                Method::ProjectOpen,
                p,
                output::format_project_open,
            )
        }
        Command::Eval { song } => eval_like(transport, env, json, Method::Eval, song),
        Command::Play { song } => eval_like(transport, env, json, Method::Play, song),
        Command::Code {
            command: CodeCommand::Get { song },
        } => {
            let p = params(&SongParams { song: song.clone() });
            let (raw, r): (Value, CodeGetResult) = call(transport, Method::CodeGet, p)?;
            if json {
                let _ = writeln!(env.stdout, "{raw}");
            } else {
                let _ = env.stdout.write_all(r.code.as_bytes());
                if env.stdout_is_tty && !r.code.is_empty() && !r.code.ends_with('\n') {
                    let _ = env.stdout.write_all(b"\n");
                }
            }
            let _ = env.stdout.flush();
            Ok(EXIT_OK)
        }
        Command::Code {
            command:
                CodeCommand::Set {
                    song,
                    file,
                    eval,
                    no_eval,
                    expect_file,
                },
        } => {
            let code = read_code(file.as_deref(), env)?;
            let expect = match expect_file {
                Some(path) => Some(read_utf8_file(path, &path.display().to_string())?),
                None => None,
            };
            let eval = if *eval {
                Some(EvalMode::Always)
            } else if *no_eval {
                Some(EvalMode::Never)
            } else {
                None
            };
            let p = params(&CodeSetParams {
                song: song.clone(),
                code,
                eval,
                expect,
            });
            report(transport, env, json, Method::CodeSet, p, output::format_set)
        }
    }
}

/// `docs`: served from the embedded file, never touching the transport. Without a name or
/// `--search` it lists (clap requires exactly one of the three).
fn run_docs(name: Option<&str>, search: Option<&str>, json: bool, env: &mut Env) -> i32 {
    let entries = docs::entries();
    let (text, value) = if let Some(name) = name {
        match docs::lookup(entries, name) {
            Some(e) => (output::format_docs_entry(e), json!(e)),
            None => {
                let _ = writeln!(env.stderr, "error: no docs for '{name}'");
                let suggestions = docs::suggest(entries, name);
                if !suggestions.is_empty() {
                    let _ = writeln!(env.stderr, "did you mean: {}", suggestions.join(", "));
                }
                return EXIT_APP_ERROR;
            }
        }
    } else if let Some(term) = search {
        let hits = docs::search(entries, term);
        if hits.is_empty() {
            let _ = writeln!(env.stderr, "error: nothing matches '{term}'");
            return EXIT_APP_ERROR;
        }
        (output::format_docs_search(&hits), json!(hits))
    } else {
        let names = docs::list(entries);
        (output::format_docs_list(&names), json!(names))
    };
    if json {
        let _ = writeln!(env.stdout, "{value}");
    } else {
        let _ = env.stdout.write_all(text.as_bytes());
    }
    let _ = env.stdout.flush();
    EXIT_OK
}

/// `render`: offline, never touching the transport. Progress goes to stderr from this thread.
fn run_render(
    args: &crate::cli::RenderArgs,
    json: bool,
    pipeline: &mut dyn export::Pipeline,
    env: &mut Env,
) -> Result<i32, Failure> {
    let code = read_utf8_file(&args.song, &args.song.display().to_string())?;
    let prepared = render::prepare(args).map_err(Failure::Render)?;
    let instruments = match Project::find(&args.song) {
        Some((project, _)) => project.instrument_snapshot().map_err(Failure::Project)?,
        None => Default::default(),
    };
    let preset = prepared.preset().is_some();
    let summary = {
        let lines = ProgressLines::new(&mut *env.stderr);
        let finished = prepared
            .run(
                pipeline,
                &code,
                ReplacePolicy::Always,
                None,
                instruments,
                &|e| lines.event(e, preset),
            )
            .map_err(|e| Failure::Render(render::failure(&lines, e)))?;
        if let Finished::Stems { export, .. } = &finished {
            lines.warnings(&export.summary.warnings);
        }
        render::summary(&finished, json)
    };
    let _ = env.stdout.write_all(summary.as_bytes());
    let _ = env.stdout.flush();
    Ok(EXIT_OK)
}

/// The project an offline command works on: `--project` (no walk up, no fallthrough), the project the current
/// directory is in, then the project open in the running app. Every failure of that last step is
/// swallowed: it is a fallback.
fn offline_project(
    flag: Option<&Path>,
    transport: &mut dyn Transport,
    cwd: &dyn Fn() -> Result<PathBuf, Failure>,
    find_local: impl FnOnce(&Path) -> Result<Option<Project>, ProjectError>,
) -> Result<Project, Failure> {
    let cwd = cwd()?;
    if let Some(dir) = flag {
        return Project::open(&absolute(dir, &cwd)).map_err(Failure::Project);
    }
    if let Some(p) = find_local(&cwd).map_err(Failure::Project)? {
        return Ok(p);
    }
    let from_app = call::<AppState>(transport, Method::State, json!({}))
        .ok()
        .and_then(|(_, s)| s.project)
        .and_then(|p| Project::open(Path::new(&p.root)).ok());
    from_app.ok_or_else(|| {
        Failure::Usage(
            "not in a strudeland project; run it inside a project folder, pass --project DIR, or open a project in the app"
                .into(),
        )
    })
}

/// The memo `name` among `entries`. An invalid name is a usage error, a missing memo a plain
/// failure, and a memo whose sidecar is unreadable fails with the sidecar's error.
fn find_memo<'a>(
    project: &Project,
    entries: &'a [RecordingEntry],
    name: &str,
) -> Result<&'a RecordingEntry, Failure> {
    let Some(entry) = entries.iter().find(|e| e.stem == name) else {
        project.read_sidecar(name).map_err(Failure::Project)?;
        return Err(Failure::Project(ProjectError::RecordingNotFound(
            name.to_string(),
        )));
    };
    if entry.sidecar_error.is_some() {
        project.read_sidecar(name).map_err(Failure::Project)?;
    }
    Ok(entry)
}

/// `memo list` and `memo show`: offline reads through `strudeland-project`.
fn run_memo(
    command: &MemoCommand,
    json: bool,
    transport: &mut dyn Transport,
    env: &mut Env,
    cwd: &dyn Fn() -> Result<PathBuf, Failure>,
) -> Result<i32, Failure> {
    let (flag, name) = match command {
        MemoCommand::List { project } => (project, None),
        MemoCommand::Show { name, project } => (project, Some(name)),
    };
    let project = offline_project(flag.as_deref(), transport, cwd, |dir| {
        Ok(Project::find_from_dir(dir))
    })?;
    let root = project.root().to_string_lossy().into_owned();
    let entries = project.recordings().map_err(Failure::Project)?;
    let report = match name {
        None if json => stdout_only(format!("{}\n", output::memo_list_json(&root, &entries))),
        None => output::format_memo_list(&entries),
        Some(name) => {
            let entry = find_memo(&project, &entries, name)?;
            if json {
                stdout_only(format!("{}\n", output::memo_show_json(&root, entry)))
            } else {
                stdout_only(output::format_memo_show(&root, entry))
            }
        }
    };
    let _ = env.stderr.write_all(report.stderr.as_bytes());
    let _ = env.stdout.write_all(report.stdout.as_bytes());
    let _ = env.stdout.flush();
    Ok(EXIT_OK)
}

/// A transcription failure as a command failure, with the error's own exit code.
fn transcribe_failure(e: TranscribeError) -> Failure {
    if e.exit_code() == EXIT_APP_ERROR {
        Failure::Transcribe(e)
    } else {
        Failure::Usage(e.to_string())
    }
}

/// The grid hint of `transcribe`: each of the tempo, the meter and bar 1 from its flag, else from
/// the `trusted` sidecar (bar 1 is its `bar1Frame` over the memo's sample `rate`).
fn build_hint(
    args: &TranscribeArgs,
    trusted: Option<&Sidecar>,
    rate: Option<u32>,
) -> Result<GridHint, Failure> {
    let pick = |flag: Option<f64>, sidecar: Option<f64>| {
        flag.map(Hinted::flag).or(sidecar.map(Hinted::sidecar))
    };
    let beats_flag = args
        .meter
        .as_deref()
        .map(parse_meter)
        .transpose()
        .map_err(transcribe_failure)?;
    let sidecar_origin = trusted
        .and_then(|s| s.timing.bar1_frame)
        .zip(rate)
        .map(|(frame, rate)| frame / f64::from(rate));
    Ok(GridHint {
        bpm: pick(args.bpm, trusted.map(|s| s.tempo.bpm)),
        beats_per_bar: pick(beats_flag, trusted.map(|s| s.tempo.beats_per_bar)),
        origin_seconds: pick(args.bar1, sidecar_origin),
        meter: args
            .meter
            .clone()
            .or_else(|| trusted.map(|s| s.tempo.meter.clone())),
        // A tempo the take never heard: no song and no metronome played.
        tempo_unheard: trusted.is_some_and(|s| !s.backing.song && !s.backing.metronome),
    })
}

/// `transcribe`: a memo's audio to notes and code, offline. Reads the memo and its sidecar, never
/// the song, and writes nothing.
fn run_transcribe(
    args: &TranscribeArgs,
    json: bool,
    transport: &mut dyn Transport,
    env: &mut Env,
    cwd: &dyn Fn() -> Result<PathBuf, Failure>,
) -> Result<i32, Failure> {
    if args.debug && !json {
        return Err(Failure::Usage("--debug needs --json".into()));
    }
    let project = offline_project(args.project.as_deref(), transport, cwd, |dir| {
        Ok(Project::find_from_dir(dir))
    })?;
    let name = &args.name;
    let entries = project.recordings().map_err(Failure::Project)?;
    let entry = find_memo(&project, &entries, name)?;

    let sidecar = entry.sidecar.as_ref();
    let recovered = sidecar.is_some_and(|s| s.recovered);
    // A recovered take's tempo, meter and bar 1 are placeholders: only flags supply them.
    let trusted = sidecar.filter(|s| !s.recovered);
    let mut cli_warnings = Vec::new();
    if recovered {
        cli_warnings
            .push("the memo was recovered after a crash; its tempo is a placeholder".to_string());
    }
    if sidecar.is_none() && !entry.has_sidecar {
        cli_warnings.push("no sidecar; the grid is inferred from the audio".to_string());
    }

    let rate = entry
        .sample_rate
        .or_else(|| sidecar.map(|s| s.input.sample_rate));
    let hint = build_hint(args, trusted, rate)?;
    let key = args
        .key
        .as_deref()
        .map(parse_key_override)
        .transpose()
        .map_err(transcribe_failure)?;

    let span = resolve_span(args, &hint)?;

    let audio =
        load_wav_span(&project.recording_wav_path(name), span).map_err(transcribe_failure)?;
    let opts = Options {
        grid: args.grid,
        key,
        tuning_cents: args.tuning,
        shift_ms: args.shift,
        span_start_seconds: audio.start_seconds,
        memo_seconds: Some(audio.memo_seconds),
        last_bar: args.bars.map(|(_, to)| to),
        debug: args.debug,
    };
    let mut result =
        transcribe(&audio.samples, audio.sample_rate, &hint, &opts).map_err(transcribe_failure)?;
    result.memo = Some(name.clone());
    result.backing = sidecar.map(|s| BackingReport {
        song: s.backing.song_path.clone(),
        start_section: s.backing.start_section.clone(),
    });
    cli_warnings.append(&mut result.warnings);
    result.warnings = cli_warnings;

    print_transcription(&result, json, env);
    Ok(EXIT_OK)
}

/// The span `transcribe` reads: `--bars` through the hint's grid (its bar 1 moved by `--shift`, as
/// the grid's is), `--start`/`--end`, or the whole memo.
fn resolve_span(args: &TranscribeArgs, hint: &GridHint) -> Result<Span, Failure> {
    Ok(match (args.bars, args.start, args.end) {
        (Some((from, to)), _, _) => {
            let (start, end) = hint
                .bar_span(from, to, args.shift)
                .ok_or_else(|| transcribe_failure(TranscribeError::BarsNeedGrid))?;
            Span::Seconds {
                start,
                end: Some(end),
            }
        }
        (None, Some(start), Some(end)) => Span::Seconds {
            start,
            end: Some(end),
        },
        _ => Span::Whole,
    })
}

/// Writes a transcription: the JSON line or the human report on stdout, its warnings on stderr.
fn print_transcription(result: &Transcription, json: bool, env: &mut Env) {
    let mut report = if json {
        stdout_only(format!("{}\n", output::transcription_json(result)))
    } else {
        output::format_transcription(result)
    };
    for w in &result.warnings {
        report.stderr.push_str(&format!("warning: {w}\n"));
    }
    let _ = env.stderr.write_all(report.stderr.as_bytes());
    let _ = env.stdout.write_all(report.stdout.as_bytes());
    let _ = env.stdout.flush();
}

fn current_dir() -> Result<PathBuf, Failure> {
    std::env::current_dir()
        .map_err(|e| Failure::Usage(format!("cannot read the current directory: {e}")))
}

/// `skill print`: the embedded text, never touching the transport.
fn print_skill(json: bool, env: &mut Env) -> i32 {
    if json {
        let _ = writeln!(env.stdout, "{}", json!({"skill": skill::SKILL}));
    } else {
        let _ = env.stdout.write_all(skill::SKILL.as_bytes());
    }
    let _ = env.stdout.flush();
    EXIT_OK
}

/// `skill install`: into `project`'s `.claude` (relative to the current directory, read only
/// then) if given, else into the user's Claude config folder `user`.
fn install_skill(
    project: Option<&Path>,
    user: Option<&Path>,
    cwd: &dyn Fn() -> Result<PathBuf, Failure>,
    json: bool,
    env: &mut Env,
) -> Result<i32, Failure> {
    let base = match project {
        Some(dir) => {
            let dir = absolute(dir, &cwd()?);
            if !dir.join("project.json").is_file() {
                let _ = writeln!(env.stderr, "warning: no project.json in {}", dir.display());
            }
            dir.join(".claude")
        }
        None => user
            .ok_or_else(|| Failure::Usage("HOME is not set; use --project to install here".into()))?
            .to_path_buf(),
    };
    let path = base.join(skill::REL_PATH);
    let outcome = match skill::install(&path) {
        Ok(o) => o.as_str(),
        Err(e) => {
            let _ = writeln!(env.stderr, "error: cannot write {}: {e}", path.display());
            return Ok(EXIT_APP_ERROR);
        }
    };
    if json {
        let path = path.to_string_lossy();
        let _ = writeln!(env.stdout, "{}", json!({"path": path, "outcome": outcome}));
    } else {
        let _ = writeln!(env.stdout, "{outcome} {}", path.display());
    }
    let _ = env.stdout.flush();
    Ok(EXIT_OK)
}

/// `path` against `cwd` when relative: the app's working directory is unrelated to the caller's.
/// `.` components are dropped so the reported path reads cleanly; no canonicalizing and no `..`
/// folding (that would change the meaning across symlinks).
fn absolute(path: &Path, cwd: &Path) -> PathBuf {
    let rest = path
        .components()
        .filter(|c| !matches!(c, std::path::Component::CurDir));
    if path.is_absolute() {
        rest.collect()
    } else {
        cwd.join(rest.collect::<PathBuf>())
    }
}

fn eval_like(
    transport: &mut dyn Transport,
    env: &mut Env,
    json: bool,
    method: Method,
    song: &Option<String>,
) -> Result<i32, Failure> {
    let p = params(&SongParams { song: song.clone() });
    report(transport, env, json, method, p, output::format_eval)
}

fn read_utf8_file(path: &Path, label: &str) -> Result<String, Failure> {
    let bytes = std::fs::read(path)
        .map_err(|e| Failure::Usage(format!("cannot read {}: {e}", path.display())))?;
    String::from_utf8(bytes).map_err(|_| Failure::Usage(format!("{label} is not valid UTF-8")))
}

/// The code for `code set`: `--file F`, or stdin for `-` or no `--file`. Read as is.
fn read_code(file: Option<&str>, env: &mut Env) -> Result<String, Failure> {
    match file {
        Some(f) if f != "-" => read_utf8_file(Path::new(f), f),
        other => {
            if other.is_none() && env.stdin_is_tty {
                return Err(Failure::Usage(
                    "pass --file or pipe the code on stdin".into(),
                ));
            }
            let mut bytes = Vec::new();
            env.stdin
                .read_to_end(&mut bytes)
                .map_err(|e| Failure::Usage(format!("cannot read stdin: {e}")))?;
            String::from_utf8(bytes).map_err(|_| Failure::Usage("stdin is not valid UTF-8".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::collections::VecDeque;
    use strudeland_ctl::RpcError;

    #[derive(Default)]
    struct Fake {
        calls: Vec<(Method, Value)>,
        replies: VecDeque<Result<Value, CallError>>,
    }

    impl Fake {
        fn with(reply: Result<Value, CallError>) -> Fake {
            Fake {
                calls: vec![],
                replies: VecDeque::from([reply]),
            }
        }
        fn ok(v: Value) -> Fake {
            Fake::with(Ok(v))
        }
    }

    impl Transport for Fake {
        fn call(&mut self, method: Method, params: Value) -> Result<Value, CallError> {
            self.calls.push((method, params));
            self.replies.pop_front().expect("unexpected call")
        }
    }

    struct Out {
        code: i32,
        stdout: Vec<u8>,
        stderr: String,
    }

    impl Out {
        fn out(&self) -> &str {
            std::str::from_utf8(&self.stdout).unwrap()
        }
    }

    struct Io<'a> {
        stdin: &'a [u8],
        stdin_tty: bool,
        stdout_tty: bool,
    }

    const IO: Io = Io {
        stdin: b"",
        stdin_tty: false,
        stdout_tty: false,
    };

    fn go(args: &[&str], fake: &mut Fake, io: Io) -> Out {
        let cli =
            Cli::try_parse_from(std::iter::once("strudeland").chain(args.iter().copied())).unwrap();
        let mut stdin = io.stdin;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = {
            let mut env = Env {
                stdin: &mut stdin,
                stdin_is_tty: io.stdin_tty,
                stdout: &mut stdout,
                stdout_is_tty: io.stdout_tty,
                stderr: &mut stderr,
            };
            run(&cli, fake, &mut env)
        };
        Out {
            code,
            stdout,
            stderr: String::from_utf8(stderr).unwrap(),
        }
    }

    fn state_json() -> Value {
        json!({
            "project": {"name": "demo", "root": "/Users/me/demo"},
            "songs": [
                {"path": "songs/main.js", "title": "Main Theme", "dirty": true, "conflict": null},
                {"path": "songs/verse.js", "title": "Verse", "dirty": false, "conflict": "changed"}
            ],
            "active": "songs/main.js",
            "playing": {"path": "songs/main.js", "title": "Main Theme"},
            "started": true, "cps": 0.5, "bpm": 120, "meter": "4/4", "audio": "running",
            "evalError": null, "metaError": null, "tempoOverride": false
        })
    }

    fn outcome(ok: bool, started: bool, audio: &str, error: Value) -> Value {
        json!({"ok": ok, "song": "songs/main.js", "title": "Main Theme", "started": started,
               "audio": audio, "error": error})
    }

    fn rpc(code: i64, message: &str) -> CallError {
        CallError::Rpc(RpcError::new(code, message))
    }

    #[test]
    fn state_human_and_json() {
        let mut f = Fake::ok(state_json());
        let o = go(&["state"], &mut f, IO);
        assert_eq!(o.code, 0);
        assert_eq!(f.calls, vec![(Method::State, json!({}))]);
        assert_eq!(
            o.out(),
            "project: demo (/Users/me/demo)\nsongs:\n  ● main   \"Main Theme\"   active, playing\n    verse  \"Verse\"        conflict: changed\ntempo: 120 bpm 4/4\naudio: running\nplaying: Main Theme\n"
        );
        assert_eq!(o.stderr, "");

        let mut f = Fake::ok(state_json());
        let o = go(&["state", "--json"], &mut f, IO);
        assert_eq!(o.code, 0);
        assert_eq!(o.out().matches('\n').count(), 1);
        assert!(o.out().ends_with('\n'));
        assert_eq!(
            serde_json::from_str::<Value>(o.out()).unwrap(),
            state_json()
        );
    }

    #[test]
    fn ping_cases() {
        let mut f = Fake::ok(json!({"version": "0.1.0", "attached": true}));
        let o = go(&["ping"], &mut f, IO);
        assert_eq!(
            (o.code, o.out(), o.stderr.as_str()),
            (0, "strudeland 0.1.0 is running\n", "")
        );
        assert_eq!(f.calls, vec![(Method::Ping, json!({}))]);

        let mut f = Fake::ok(json!({"version": "0.1.0", "attached": false}));
        let o = go(&["ping"], &mut f, IO);
        assert_eq!(o.code, 3);
        assert_eq!(o.out(), "");
        assert_eq!(o.stderr, "strudeland 0.1.0 is running but not ready\n");

        let mut f = Fake::ok(json!({"version": "0.1.0", "attached": false}));
        let o = go(&["ping", "--json"], &mut f, IO);
        assert_eq!(o.code, 3);
        assert_eq!(
            serde_json::from_str::<Value>(o.out()).unwrap(),
            json!({"version": "0.1.0", "attached": false})
        );
    }

    #[test]
    fn stop_cases() {
        let mut f = Fake::ok(json!({"wasPlaying": true}));
        let o = go(&["stop"], &mut f, IO);
        assert_eq!((o.code, o.out()), (0, "stopped\n"));
        assert_eq!(f.calls, vec![(Method::Stop, json!({}))]);
        let mut f = Fake::ok(json!({"wasPlaying": false}));
        assert_eq!(go(&["stop"], &mut f, IO).out(), "not playing\n");
    }

    #[test]
    fn eval_and_play() {
        let mut f = Fake::ok(outcome(true, true, "running", Value::Null));
        let o = go(&["eval"], &mut f, IO);
        assert_eq!((o.code, o.out()), (0, "playing Main Theme\n"));
        assert_eq!(f.calls, vec![(Method::Eval, json!({}))]);

        let mut f = Fake::ok(outcome(true, true, "running", Value::Null));
        let o = go(&["play", "--song", "verse"], &mut f, IO);
        assert_eq!((o.code, o.out()), (0, "playing Main Theme\n"));
        assert_eq!(f.calls, vec![(Method::Play, json!({"song": "verse"}))]);
    }

    #[test]
    fn eval_suspended_warns_and_exits_1() {
        let mut f = Fake::ok(outcome(true, false, "suspended", Value::Null));
        let o = go(&["eval"], &mut f, IO);
        assert_eq!(o.code, 1);
        assert_eq!(o.out(), "evaluated Main Theme (not playing)\n");
        assert_eq!(
            o.stderr,
            "warning: audio is suspended: click in the strudeland window to enable it\n"
        );
    }

    #[test]
    fn eval_stopped_meanwhile_is_ok() {
        let mut f = Fake::ok(outcome(true, false, "running", Value::Null));
        let o = go(&["eval"], &mut f, IO);
        assert_eq!(
            (o.code, o.out(), o.stderr.as_str()),
            (0, "evaluated Main Theme (not playing)\n", "")
        );
    }

    #[test]
    fn eval_failure_goes_to_stderr_and_exits_1() {
        let mut f = Fake::ok(outcome(
            false,
            false,
            "running",
            json!("meta: bpm must be a number (1:20)"),
        ));
        let o = go(&["eval"], &mut f, IO);
        assert_eq!(o.code, 1);
        assert_eq!(o.out(), "");
        assert_eq!(o.stderr, "error: meta: bpm must be a number (1:20)\n");
    }

    #[test]
    fn eval_json_failure_keeps_stdout_json_only() {
        let mut f = Fake::ok(outcome(false, false, "running", json!("bad")));
        let o = go(&["eval", "--json"], &mut f, IO);
        assert_eq!(o.code, 1);
        assert_eq!(o.out().matches('\n').count(), 1);
        assert_eq!(o.stderr, "error: bad\n");
    }

    #[test]
    fn code_get_newline_rule() {
        let result =
            json!({"song": "songs/main.js", "title": "Main", "code": "a\nb", "dirty": false});
        let mut f = Fake::ok(result.clone());
        let o = go(&["code", "get"], &mut f, IO);
        assert_eq!((o.code, o.out()), (0, "a\nb"));
        assert_eq!(f.calls, vec![(Method::CodeGet, json!({}))]);

        let mut f = Fake::ok(result.clone());
        let tty = Io {
            stdout_tty: true,
            ..IO
        };
        assert_eq!(go(&["code", "get"], &mut f, tty).out(), "a\nb\n");

        let with_nl =
            json!({"song": "songs/main.js", "title": "Main", "code": "a\n", "dirty": false});
        let mut f = Fake::ok(with_nl.clone());
        let tty = Io {
            stdout_tty: true,
            ..IO
        };
        assert_eq!(go(&["code", "get"], &mut f, tty).out(), "a\n");

        let empty = json!({"song": "songs/main.js", "title": "Main", "code": "", "dirty": false});
        let mut f = Fake::ok(empty);
        let tty = Io {
            stdout_tty: true,
            ..IO
        };
        assert_eq!(go(&["code", "get"], &mut f, tty).out(), "");
    }

    #[test]
    fn code_get_json_is_exact_even_on_a_tty() {
        let result =
            json!({"song": "songs/main.js", "title": "Main", "code": "a\nb", "dirty": true});
        let mut f = Fake::ok(result.clone());
        let tty = Io {
            stdout_tty: true,
            ..IO
        };
        let o = go(&["code", "get", "--json", "--song", "main"], &mut f, tty);
        assert_eq!(serde_json::from_str::<Value>(o.out()).unwrap(), result);
        assert_eq!(f.calls, vec![(Method::CodeGet, json!({"song": "main"}))]);
    }

    fn set_result(changed: bool, dirty: bool, conflict: Value, evaluated: Value) -> Value {
        json!({"song": "songs/main.js", "changed": changed, "dirty": dirty,
               "conflict": conflict, "evaluated": evaluated})
    }

    #[test]
    fn code_set_from_stdin_is_not_trimmed() {
        let mut f = Fake::ok(set_result(
            true,
            true,
            Value::Null,
            outcome(true, true, "running", Value::Null),
        ));
        let io = Io {
            stdin: b"  s(\"bd\")\n\n",
            ..IO
        };
        let o = go(&["code", "set"], &mut f, io);
        assert_eq!(o.code, 0);
        assert_eq!(o.out(), "main: updated (unsaved)\nplaying Main Theme\n");
        assert_eq!(
            f.calls,
            vec![(Method::CodeSet, json!({"code": "  s(\"bd\")\n\n"}))]
        );
    }

    #[test]
    fn code_set_dash_reads_stdin_even_on_a_tty() {
        let mut f = Fake::ok(set_result(false, false, Value::Null, Value::Null));
        let io = Io {
            stdin: b"x",
            stdin_tty: true,
            ..IO
        };
        let o = go(&["code", "set", "--file", "-", "--no-eval"], &mut f, io);
        assert_eq!((o.code, o.out()), (0, "main: no change\n"));
        assert_eq!(
            f.calls,
            vec![(Method::CodeSet, json!({"code": "x", "eval": "never"}))]
        );
    }

    #[test]
    fn code_set_from_file_with_eval_and_expect() {
        let dir = tempfile::tempdir().unwrap();
        let new = dir.path().join("new.js");
        let old = dir.path().join("old.js");
        std::fs::write(&new, "new\r\ncode\n").unwrap();
        std::fs::write(&old, "old\n").unwrap();
        let mut f = Fake::ok(set_result(true, false, Value::Null, Value::Null));
        let o = go(
            &[
                "code",
                "set",
                "--song",
                "verse",
                "--file",
                new.to_str().unwrap(),
                "--eval",
                "--expect-file",
                old.to_str().unwrap(),
            ],
            &mut f,
            IO,
        );
        assert_eq!((o.code, o.out()), (0, "main: updated\n"));
        assert_eq!(
            f.calls,
            vec![(
                Method::CodeSet,
                json!({"song": "verse", "code": "new\r\ncode\n", "eval": "always", "expect": "old\n"})
            )]
        );
    }

    #[test]
    fn code_set_conflict_note() {
        let mut f = Fake::ok(set_result(
            true,
            true,
            json!("changed"),
            outcome(false, false, "running", json!("nope")),
        ));
        let o = go(&["code", "set"], &mut f, Io { stdin: b"c", ..IO });
        assert_eq!(o.code, 1);
        assert_eq!(
            o.out(),
            "main: updated (unsaved)\nnote: the file changed on disk; the conflict bar is still open\n"
        );
        assert_eq!(o.stderr, "error: nope\n");
    }

    #[test]
    fn code_set_input_errors_are_usage_errors() {
        // stdin is a terminal and no --file
        let mut f = Fake::default();
        let o = go(
            &["code", "set"],
            &mut f,
            Io {
                stdin_tty: true,
                ..IO
            },
        );
        assert_eq!(o.code, 4);
        assert_eq!(o.stderr, "error: pass --file or pipe the code on stdin\n");
        assert!(f.calls.is_empty());

        // invalid UTF-8 on stdin
        let mut f = Fake::default();
        let o = go(
            &["code", "set"],
            &mut f,
            Io {
                stdin: &[0xff, 0xfe],
                ..IO
            },
        );
        assert_eq!(o.code, 4);
        assert_eq!(o.stderr, "error: stdin is not valid UTF-8\n");

        // invalid UTF-8 in --file
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.js");
        std::fs::write(&bad, [0xff]).unwrap();
        let mut f = Fake::default();
        let o = go(
            &["code", "set", "--file", bad.to_str().unwrap()],
            &mut f,
            IO,
        );
        assert_eq!(o.code, 4);
        assert!(
            o.stderr.ends_with("bad.js is not valid UTF-8\n"),
            "{}",
            o.stderr
        );

        // missing --file
        let missing = dir.path().join("missing.js");
        let mut f = Fake::default();
        let o = go(
            &["code", "set", "--file", missing.to_str().unwrap()],
            &mut f,
            IO,
        );
        assert_eq!(o.code, 4);
        assert!(o.stderr.starts_with("error: cannot read "), "{}", o.stderr);

        // missing --expect-file
        let mut f = Fake::default();
        let o = go(
            &["code", "set", "--expect-file", missing.to_str().unwrap()],
            &mut f,
            Io { stdin: b"x", ..IO },
        );
        assert_eq!(o.code, 4);
        assert!(o.stderr.contains("missing.js"), "{}", o.stderr);
        assert!(f.calls.is_empty());

        // invalid UTF-8 in --expect-file
        let mut f = Fake::default();
        let o = go(
            &["code", "set", "--expect-file", bad.to_str().unwrap()],
            &mut f,
            Io { stdin: b"x", ..IO },
        );
        assert_eq!(o.code, 4);
        assert!(f.calls.is_empty());
    }

    #[test]
    fn call_error_mapping() {
        let cases: Vec<(CallError, i32, &str)> = vec![
            (
                CallError::NotRunning {
                    path: "/s/x.sock".into(),
                },
                2,
                if cfg!(windows) {
                    "error: strudeland is not running (no pipe at /s/x.sock)\n"
                } else {
                    "error: strudeland is not running (no socket at /s/x.sock)\n"
                },
            ),
            (
                CallError::Timeout,
                3,
                "error: the app did not answer in time (raise it with --timeout)\n",
            ),
            (
                rpc(codes::TIMEOUT, "timeout"),
                3,
                "error: timeout (raise it with --timeout)\n",
            ),
            (
                rpc(codes::NOT_READY, "app not ready"),
                3,
                "error: app not ready\n",
            ),
            (
                rpc(codes::NO_PROJECT, "no project open"),
                1,
                "error: no project open\n",
            ),
            (
                rpc(codes::SONG_CHANGED, "the song changed since you read it"),
                1,
                "error: the song changed since you read it\n",
            ),
            (
                CallError::TooLarge { bytes: 2_000_000 },
                1,
                "error: request too large (2000000 bytes; the app reads at most 1048576)\n",
            ),
            (
                CallError::Protocol("bad".into()),
                1,
                "error: protocol error: bad\n",
            ),
            (
                CallError::Io(std::io::Error::other("pipe broke")),
                1,
                "error: pipe broke\n",
            ),
        ];
        for (err, code, msg) in cases {
            let mut f = Fake::with(Err(err));
            let o = go(&["state"], &mut f, IO);
            assert_eq!((o.code, o.stderr.as_str(), o.out()), (code, msg, ""));
        }
    }

    #[test]
    fn malformed_result_is_an_app_error() {
        let mut f = Fake::ok(json!({"unexpected": 1}));
        let o = go(&["state"], &mut f, IO);
        assert_eq!(o.code, 1);
        assert!(
            o.stderr
                .starts_with("error: protocol error: invalid state result"),
            "{}",
            o.stderr
        );
    }

    fn events_json() -> Value {
        json!({
            "song": {"path": "songs/main.js", "title": "Main Theme"},
            "from": 1, "to": 1, "stale": true, "truncated": false, "meter": "4/4",
            "tempoOverride": false,
            "events": [{
                "bar": 1, "beat": 1.0, "begin": 0.0, "end": 0.25, "onset": true,
                "section": {"name": "intro", "bar": 1},
                "value": {"s": "bd"},
                "locations": [{"line": 7, "col": 15, "endLine": 7, "endCol": 17, "text": "x"}]
            }]
        })
    }

    #[test]
    fn events_params_per_flag_combination() {
        let cases: Vec<(Vec<&str>, Value)> = vec![
            (vec!["events"], json!({})),
            (vec!["events", "--all"], json!({"all": true})),
            (
                vec!["events", "--bars", "2-4", "--limit", "10"],
                json!({"bars": {"from": 2, "to": 4}, "limit": 10}),
            ),
            (
                vec!["events", "--bars", "3"],
                json!({"bars": {"from": 3, "to": 3}}),
            ),
            (
                vec!["events", "--section", "intro", "--all"],
                json!({"section": "intro", "all": true}),
            ),
        ];
        for (args, expected) in cases {
            let mut f = Fake::ok(events_json());
            let o = go(&args, &mut f, IO);
            assert_eq!(f.calls, vec![(Method::Events, expected)], "{args:?}");
            assert_eq!(
                o.out(),
                "song: main \"Main Theme\"\nbars: 1 (4/4)\n\nbar 1  intro 1\n  1  s=bd @ 7:15\n"
            );
        }
    }

    #[test]
    fn events_warnings_go_to_stderr_with_exit_0() {
        let mut f = Fake::ok(events_json());
        let o = go(&["events"], &mut f, IO);
        assert_eq!(o.code, 0);
        assert_eq!(
            o.stderr,
            "warning: the song changed since it was evaluated; positions refer to the evaluated text\n"
        );
    }

    #[test]
    fn events_json_is_the_raw_result() {
        let mut f = Fake::ok(events_json());
        let o = go(&["events", "--json"], &mut f, IO);
        assert_eq!(o.code, 0);
        assert_eq!(o.out().matches('\n').count(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(o.out()).unwrap(),
            events_json()
        );
    }

    #[test]
    fn events_error_codes() {
        let mut f = Fake::with(Err(rpc(
            codes::NOTHING_EVALUATED,
            "nothing has been evaluated",
        )));
        let o = go(&["events"], &mut f, IO);
        assert_eq!((o.code, o.out()), (1, ""));
        assert_eq!(o.stderr, "error: nothing has been evaluated\n");
        let mut f = Fake::with(Err(rpc(codes::INVALID_PARAMS, "unknown section 'x'")));
        let o = go(&["events", "--section", "x"], &mut f, IO);
        assert_eq!(
            (o.code, o.stderr.as_str()),
            (1, "error: unknown section 'x'\n")
        );
        let mut f = Fake::with(Err(CallError::Timeout));
        assert_eq!(go(&["events"], &mut f, IO).code, 3);
    }

    #[test]
    fn absolute_joins_only_relative_paths() {
        let cwd = std::env::temp_dir().join("work/dir");
        let absolute_path = std::env::temp_dir().join("s.png");
        assert_eq!(absolute(&absolute_path, &cwd), absolute_path);
        assert_eq!(absolute(Path::new("s.png"), &cwd), cwd.join("s.png"));
        assert_eq!(
            absolute(Path::new("../shots/s.png"), &cwd),
            cwd.join("../shots/s.png"),
            "`..` is kept"
        );
        assert_eq!(absolute(Path::new("./s.png"), &cwd), cwd.join("s.png"));
    }

    fn shot(path: &str) -> Value {
        json!({"path": path, "width": 800, "height": 600})
    }

    #[test]
    fn screenshot_params() {
        let mut f = Fake::ok(shot("/tmp/strudeland-screenshot-1.png"));
        let o = go(&["screenshot"], &mut f, IO);
        assert_eq!(
            (o.code, o.out(), o.stderr.as_str()),
            (0, "wrote /tmp/strudeland-screenshot-1.png (800x600)\n", "")
        );
        assert_eq!(f.calls, vec![(Method::Screenshot, json!({}))]);

        let path = std::env::temp_dir().join("a.png");
        let path = path.to_str().unwrap();
        let mut f = Fake::ok(shot(path));
        go(&["screenshot", path], &mut f, IO);
        assert_eq!(f.calls[0].1, json!({"path": path}));

        let cwd = std::env::current_dir().unwrap();
        let mut f = Fake::ok(shot("x"));
        go(&["screenshot", "out/s.png"], &mut f, IO);
        assert_eq!(
            f.calls[0].1,
            json!({"path": cwd.join("out").join("s.png").to_str().unwrap()})
        );
    }

    #[test]
    fn screenshot_json_and_errors() {
        let mut f = Fake::ok(shot("/tmp/a.png"));
        let o = go(&["--json", "screenshot"], &mut f, IO);
        assert_eq!(o.out(), format!("{}\n", shot("/tmp/a.png")));
        let mut f = Fake::with(Err(rpc(codes::SCREENSHOT_FAILED, "no main window")));
        let o = go(&["screenshot"], &mut f, IO);
        assert_eq!(
            (o.code, o.out(), o.stderr.as_str()),
            (1, "", "error: no main window\n")
        );
        let mut f = Fake::with(Err(rpc(codes::TIMEOUT, "the app did not answer in 10s")));
        assert_eq!(go(&["screenshot"], &mut f, IO).code, 3);
    }

    fn song_ref(stem: &str) -> Value {
        json!({"path": format!("songs/{stem}.js"), "title": stem})
    }

    #[test]
    fn song_new_and_switch() {
        let mut f = Fake::ok(json!({"song": song_ref("main2")}));
        let o = go(&["song", "new", "main2"], &mut f, IO);
        assert_eq!(
            (o.code, o.out(), o.stderr.as_str()),
            (0, "created main2\n", "")
        );
        assert_eq!(f.calls, vec![(Method::SongNew, json!({"name": "main2"}))]);

        let mut f = Fake::ok(json!({"song": song_ref("verse")}));
        let o = go(&["song", "switch", "verse"], &mut f, IO);
        assert_eq!((o.code, o.out()), (0, "switched to verse\n"));
        assert_eq!(
            f.calls,
            vec![(Method::SongSwitch, json!({"song": "verse"}))]
        );
    }

    #[test]
    fn song_rename_params_and_output() {
        let reply = json!({"from": song_ref("main"), "to": song_ref("intro")});
        let mut f = Fake::ok(reply.clone());
        let o = go(&["song", "rename", "intro"], &mut f, IO);
        assert_eq!((o.code, o.out()), (0, "renamed main \u{2192} intro\n"));
        assert_eq!(
            f.calls,
            vec![(Method::SongRename, json!({"name": "intro"}))]
        );

        let mut f = Fake::ok(reply);
        go(&["song", "rename", "--song", "main", "intro"], &mut f, IO);
        assert_eq!(f.calls[0].1, json!({"song": "main", "name": "intro"}));
    }

    #[test]
    fn song_delete_params_and_output() {
        let mut f = Fake::ok(json!({"song": song_ref("verse"), "stopped": false}));
        let o = go(&["song", "delete"], &mut f, IO);
        assert_eq!((o.code, o.out()), (0, "deleted verse\n"));
        assert_eq!(f.calls, vec![(Method::SongDelete, json!({}))]);

        let mut f = Fake::ok(json!({"song": song_ref("verse"), "stopped": true}));
        let o = go(
            &["song", "delete", "--song", "verse", "--discard"],
            &mut f,
            IO,
        );
        assert_eq!(o.out(), "deleted verse\nstopped playback\n");
        assert_eq!(f.calls[0].1, json!({"song": "verse", "discard": true}));
    }

    #[test]
    fn save_params_and_output() {
        let mut f = Fake::ok(json!({"saved": []}));
        let o = go(&["save"], &mut f, IO);
        assert_eq!((o.code, o.out()), (0, "nothing to save\n"));
        assert_eq!(f.calls, vec![(Method::Save, json!({}))]);

        let mut f = Fake::ok(json!({"saved": [song_ref("main")]}));
        let o = go(&["save", "--song", "main"], &mut f, IO);
        assert_eq!(o.out(), "saved main\n");
        assert_eq!(f.calls[0].1, json!({"song": "main"}));

        let mut f = Fake::ok(json!({"saved": [song_ref("main"), song_ref("verse")]}));
        let o = go(&["save", "--all"], &mut f, IO);
        assert_eq!(o.out(), "saved main, verse\n");
        assert_eq!(f.calls[0].1, json!({"all": true}));
    }

    #[test]
    fn project_open_params_and_output() {
        let opened = json!({"project": {"name": "demo", "root": "/path"}, "alreadyOpen": false});
        let mut f = Fake::ok(opened);
        let path = std::env::temp_dir().join("demo");
        let path = path.to_str().unwrap();
        let o = go(&["project", "open", path], &mut f, IO);
        assert_eq!((o.code, o.out()), (0, "opened demo (/path)\n"));
        assert_eq!(f.calls, vec![(Method::ProjectOpen, json!({"path": path}))]);

        let cwd = std::env::current_dir().unwrap();
        let again = json!({"project": {"name": "demo", "root": "/path"}, "alreadyOpen": true});
        let mut f = Fake::ok(again);
        let o = go(&["project", "open", "rel/demo", "--discard"], &mut f, IO);
        assert_eq!(o.out(), "demo is already open\n");
        assert_eq!(
            f.calls[0].1,
            json!({"path": cwd.join("rel").join("demo").to_str().unwrap(), "discard": true})
        );
    }

    #[test]
    fn new_commands_print_the_raw_result_under_json() {
        let reply = json!({"song": song_ref("main2")});
        let mut f = Fake::ok(reply.clone());
        let o = go(&["--json", "song", "new", "main2"], &mut f, IO);
        assert_eq!(o.out(), format!("{reply}\n"));
        let reply = json!({"saved": []});
        let mut f = Fake::ok(reply.clone());
        let o = go(&["save", "--json"], &mut f, IO);
        assert_eq!(o.out(), format!("{reply}\n"));
    }

    #[test]
    fn unsaved_and_busy_exit_1_with_a_hint() {
        let msg = "'a' has unsaved changes (use discard to delete it anyway)";
        let mut f = Fake::with(Err(rpc(codes::UNSAVED, msg)));
        let o = go(&["song", "delete"], &mut f, IO);
        assert_eq!(
            (o.code, o.out().to_string(), o.stderr.clone()),
            (
                1,
                String::new(),
                format!("error: {msg}\n{}\n", output::UNSAVED_HINT)
            )
        );

        let msg = "the app is waiting on an open dialog; try again after it closes";
        let mut f = Fake::with(Err(rpc(codes::BUSY, msg)));
        let o = go(&["project", "open", "/p"], &mut f, IO);
        assert_eq!(
            (o.code, o.stderr.clone()),
            (1, format!("error: {msg}\n{}\n", output::BUSY_HINT))
        );
    }

    #[test]
    fn the_hint_goes_to_stderr_under_json_too() {
        let mut f = Fake::with(Err(rpc(codes::UNSAVED, "x")));
        let o = go(&["--json", "project", "open", "/p"], &mut f, IO);
        assert_eq!(
            (o.code, o.out().to_string(), o.stderr.clone()),
            (
                1,
                String::new(),
                format!("error: x\n{}\n", output::UNSAVED_HINT)
            )
        );
    }

    #[test]
    fn song_exists_and_open_failed_exit_1_without_a_hint() {
        let mut f = Fake::with(Err(rpc(
            codes::SONG_EXISTS,
            "song songs/a.js already exists",
        )));
        let o = go(&["song", "new", "a"], &mut f, IO);
        assert_eq!(
            (o.code, o.stderr.as_str()),
            (1, "error: song songs/a.js already exists\n")
        );
        let mut f = Fake::with(Err(rpc(
            codes::OPEN_FAILED,
            "/x is not a strudeland project (no project.json)",
        )));
        let o = go(&["project", "open", "/x"], &mut f, IO);
        assert_eq!(
            (o.code, o.stderr.as_str()),
            (
                1,
                "error: /x is not a strudeland project (no project.json)\n"
            )
        );
    }

    #[test]
    fn timeouts_of_the_new_commands_still_exit_3() {
        let mut f = Fake::with(Err(rpc(codes::TIMEOUT, "the app did not answer in 10s")));
        assert_eq!(go(&["save", "--all"], &mut f, IO).code, 3);
        let mut f = Fake::with(Err(CallError::Timeout));
        assert_eq!(go(&["project", "open", "/p"], &mut f, IO).code, 3);
    }

    fn skill_run(
        project: Option<&str>,
        user: Option<&Path>,
        cwd: &Path,
        json: bool,
    ) -> (Result<i32, Failure>, String, String) {
        let (mut stdin, mut stdout, mut stderr) = (&b""[..], Vec::new(), Vec::new());
        let result = {
            let mut env = Env {
                stdin: &mut stdin,
                stdin_is_tty: false,
                stdout: &mut stdout,
                stdout_is_tty: false,
                stderr: &mut stderr,
            };
            let cwd = || Ok(cwd.to_path_buf());
            install_skill(project.map(Path::new), user, &cwd, json, &mut env)
        };
        (
            result,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn skill_print_matches_and_never_touches_the_transport() {
        let mut f = Fake::default();
        let o = go(&["skill", "print"], &mut f, IO);
        assert_eq!((o.code, o.out()), (0, skill::SKILL));
        let o = go(&["--json", "skill", "print"], &mut f, IO);
        assert_eq!(o.code, 0);
        let v: Value = serde_json::from_str(o.out()).unwrap();
        assert_eq!(v, json!({"skill": skill::SKILL}));
        assert!(f.calls.is_empty());
    }

    #[test]
    fn skill_install_user_level_human_and_json() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let path = home.path().join(skill::REL_PATH);
        let (r, out, err) = skill_run(None, Some(home.path()), cwd.path(), false);
        assert!(path.is_file());
        assert_eq!((r.ok(), err.as_str()), (Some(0), ""));
        assert_eq!(out, format!("installed {}\n", path.display()));
        let (_, out, _) = skill_run(None, Some(home.path()), cwd.path(), true);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v,
            json!({"path": path.display().to_string(), "outcome": "unchanged"})
        );
    }

    #[test]
    fn skill_install_without_home_is_a_usage_error() {
        let cwd = tempfile::tempdir().unwrap();
        let (r, out, _) = skill_run(None, None, cwd.path(), false);
        let Err(f @ Failure::Usage(_)) = r else {
            panic!("expected a usage failure")
        };
        assert_eq!(f.exit_code(), EXIT_USAGE);
        assert!(f.message().contains("HOME"));
        assert_eq!(out, "");
    }

    #[test]
    fn skill_install_project_paths_and_warning() {
        let cwd = tempfile::tempdir().unwrap();
        std::fs::create_dir(cwd.path().join("music")).unwrap();
        let music = cwd.path().join("music");
        let (r, out, err) = skill_run(Some("./music"), None, cwd.path(), false);
        assert_eq!(r.ok(), Some(0));
        assert_eq!(
            out,
            format!(
                "installed {}\n",
                music.join(".claude").join(skill::REL_PATH).display()
            )
        );
        assert_eq!(
            err,
            format!("warning: no project.json in {}\n", music.display())
        );

        std::fs::write(music.join("project.json"), "{}").unwrap();
        let (_, out, err) = skill_run(Some("music"), None, cwd.path(), false);
        assert!(out.starts_with("unchanged "));
        assert_eq!(err, "");

        // A bare `--project` is `.`, the current directory.
        let (_, out, _) = skill_run(Some("."), None, cwd.path(), false);
        assert_eq!(
            out,
            format!(
                "installed {}\n",
                cwd.path().join(".claude").join(skill::REL_PATH).display()
            )
        );
    }

    #[test]
    fn skill_install_user_level_never_reads_the_cwd() {
        let home = tempfile::tempdir().unwrap();
        let (mut stdin, mut stdout, mut stderr) = (&b""[..], Vec::new(), Vec::new());
        let mut env = Env {
            stdin: &mut stdin,
            stdin_is_tty: false,
            stdout: &mut stdout,
            stdout_is_tty: false,
            stderr: &mut stderr,
        };
        let cwd = || -> Result<PathBuf, Failure> { panic!("cwd read for a user-level install") };
        let r = install_skill(None, Some(home.path()), &cwd, false, &mut env);
        assert_eq!(r.ok(), Some(EXIT_OK));
    }

    #[test]
    fn skill_install_io_error_exits_app_error() {
        let cwd = tempfile::tempdir().unwrap();
        std::fs::write(cwd.path().join(".claude"), "a file").unwrap();
        let (r, out, err) = skill_run(Some("."), None, cwd.path(), false);
        assert_eq!(r.ok(), Some(EXIT_APP_ERROR));
        assert_eq!(out, "");
        assert!(err.contains("error: cannot write "), "{err}");
    }

    #[test]
    fn docs_hit_never_touches_the_transport() {
        let mut f = Fake::default();
        let o = go(&["docs", "sound"], &mut f, IO);
        assert_eq!(o.code, 0);
        assert!(o.out().starts_with("s (synonyms: sound)\n"), "{}", o.out());
        assert!(f.calls.is_empty());
        assert_eq!(o.stderr, "");
    }

    #[test]
    fn docs_miss_prints_suggestions_on_stderr() {
        let mut f = Fake::default();
        let o = go(&["docs", "fas"], &mut f, IO);
        assert_eq!(o.code, 1);
        assert_eq!(o.out(), "");
        assert!(
            o.stderr
                .starts_with("error: no docs for 'fas'\ndid you mean: fast")
        );
        let o = go(&["docs", "fats"], &mut f, IO);
        assert_eq!(
            (o.code, o.stderr.as_str()),
            (1, "error: no docs for 'fats'\n")
        );
    }

    #[test]
    fn docs_search_and_list() {
        let mut f = Fake::default();
        let o = go(&["docs", "--search", "Speed up a pattern"], &mut f, IO);
        assert_eq!(o.code, 0);
        assert!(o.out().starts_with("fast "), "{}", o.out());
        let o = go(&["docs", "--search", "qqqqzzzz"], &mut f, IO);
        assert_eq!(o.code, 1);
        assert_eq!(o.out(), "");
        assert_eq!(o.stderr, "error: nothing matches 'qqqqzzzz'\n");
        let o = go(&["docs", "--list"], &mut f, IO);
        assert_eq!(o.code, 0);
        assert!(o.out().lines().any(|l| l == "fast"));
        assert!(o.out().lines().any(|l| l == "song"));
    }

    #[test]
    fn docs_json_shapes() {
        let mut f = Fake::default();
        let o = go(&["--json", "docs", "sound"], &mut f, IO);
        assert_eq!(o.out().lines().count(), 1);
        let v: Value = serde_json::from_str(o.out()).unwrap();
        assert_eq!(v["name"], "s");
        assert_eq!(v["synonyms"], json!(["sound"]));
        let o = go(
            &["docs", "--search", "Speed up a pattern", "--json"],
            &mut f,
            IO,
        );
        let v: Value = serde_json::from_str(o.out()).unwrap();
        assert!(v.as_array().unwrap().iter().any(|e| e["name"] == "fast"));
        let o = go(&["docs", "--list", "--json"], &mut f, IO);
        let v: Value = serde_json::from_str(o.out()).unwrap();
        assert!(v.as_array().unwrap().contains(&json!("fast")));
        let o = go(&["docs", "fats", "--json"], &mut f, IO);
        assert_eq!((o.code, o.out()), (1, ""));
        assert!(f.calls.is_empty());
    }

    #[test]
    fn instrument_cli_lists_pins_and_requires_the_current_fingerprint_to_update() {
        let temp = tempfile::tempdir().unwrap();
        let lib = temp.path().join("library");
        std::fs::create_dir(&lib).unwrap();
        let project = Project::create(&temp.path().join("p"), "test").unwrap();
        let root = project.root().to_str().unwrap();
        let invoke = |args: &[&str]| -> Result<Value, String> {
            let cli = Cli::try_parse_from(
                ["strudeland", "instrument"]
                    .into_iter()
                    .chain(args.iter().copied()),
            )
            .unwrap();
            let Command::Instrument { command } = cli.command else {
                unreachable!()
            };
            let mut stdout = Vec::new();
            let mut env = Env {
                stdin: &mut std::io::empty(),
                stdin_is_tty: false,
                stdout: &mut stdout,
                stdout_is_tty: false,
                stderr: &mut Vec::new(),
            };
            let status = run_instrument(
                &command,
                true,
                &mut Fake::default(),
                &mut env,
                &current_dir,
                || Ok(InstrumentLibrary::at(lib.clone())),
            )
            .map_err(|e| e.message())?;
            assert_eq!(status, EXIT_OK);
            Ok(serde_json::from_slice(&stdout).unwrap())
        };
        let definition = |sound: &str| {
            json!({"schema": 1, "parameters": {}, "voices": [
                {"name": "main", "chain": [{"control": "s", "value": sound}]}
            ]})
            .to_string()
        };
        assert_eq!(invoke(&["library"]).unwrap()["entries"], json!([]));
        let input = temp.path().join("source.json");
        let create = ["create", "lead", "--file", input.to_str().unwrap()];
        std::fs::write(&input, "{}").unwrap();
        assert!(invoke(&create).is_err());
        std::fs::write(&input, definition("sine")).unwrap();
        let created = invoke(&create).unwrap();
        assert_eq!(created["id"], "lead");
        assert!(invoke(&create).is_err());
        let library = invoke(&["library"]).unwrap();
        let pin = ["pin", "lead", "--project", root];
        let entry = invoke(&pin).unwrap();
        assert_eq!(library["entries"][0], entry);
        assert_eq!(entry["id"], "lead");
        assert!(invoke(&pin).is_err());
        std::fs::write(lib.join("lead.json"), definition("sawtooth")).unwrap();
        let update = [
            "update",
            "lead",
            "--expect",
            entry["fingerprint"].as_str().unwrap(),
            "--project",
            root,
        ];
        let changed = invoke(&update).unwrap();
        assert_ne!(changed["fingerprint"], entry["fingerprint"]);
        assert!(invoke(&update).is_err());
        assert_eq!(
            invoke(&["list", "--project", root]).unwrap()["entries"],
            json!([changed])
        );
    }

    #[test]
    fn instrument_apply_preserves_the_exact_snapshot_and_returns_the_replacement_range() {
        let temp = tempfile::tempdir().unwrap();
        let project = Project::create(&temp.path().join("demo"), "test").unwrap();
        let before = temp.path().join("before.js");
        let source = "// 🎵\r\n note(\"c4\")\n";
        std::fs::write(&before, source).unwrap();
        let mut args = vec![
            "instrument",
            "apply",
            "lead",
            "--project",
            project.root().to_str().unwrap(),
            "--from",
            "8",
            "--to",
            "18",
            "--expect-file",
            before.to_str().unwrap(),
        ];
        let result = json!({"song":"songs/main.js","from":8,"to":48});
        let mut transport = Fake::ok(result.clone());
        let human = go(&args, &mut transport, IO);
        assert_eq!(
            (human.code, human.out()),
            (0, "applied lead to songs/main.js [8, 48)\n")
        );
        let mut expected =
            json!({"root":project.root(),"id":"lead","from":8,"to":18,"expect":source});
        assert_eq!(
            transport.calls,
            vec![(Method::InstrumentApply, expected.clone())]
        );

        args.extend(["--song", "songs/main.js", "--json"]);
        let mut transport = Fake::ok(result.clone());
        let machine = go(&args, &mut transport, IO);
        assert_eq!(
            serde_json::from_str::<Value>(machine.out()).unwrap(),
            result
        );
        expected["song"] = json!("songs/main.js");
        assert_eq!(transport.calls, vec![(Method::InstrumentApply, expected)]);
        std::fs::remove_file(&before).unwrap();
        assert_eq!(go(&args, &mut Fake::default(), IO).code, EXIT_USAGE);
    }

    #[test]
    fn instrument_pin_does_not_fall_back_past_an_invalid_local_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let invalid = temp.path().join("invalid");
        std::fs::create_dir(&invalid).unwrap();
        std::fs::write(invalid.join("project.json"), "{").unwrap();
        let open = Project::create(&temp.path().join("open"), "open").unwrap();
        let mut state = state_json();
        state["project"] = json!({ "name": "open", "root": open.root() });
        let mut transport = Fake::ok(state);
        let mut output = Vec::new();
        let mut env = Env {
            stdin: &mut std::io::empty(),
            stdin_is_tty: false,
            stdout: &mut output,
            stdout_is_tty: false,
            stderr: &mut Vec::new(),
        };
        let error = run_instrument(
            &InstrumentCommand::Pin {
                id: "lead".into(),
                project: None,
            },
            true,
            &mut transport,
            &mut env,
            &|| Ok(invalid.clone()),
            || panic!("the library must not open"),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            Failure::Project(ProjectError::InvalidManifest(_))
        ));
        assert!(transport.calls.is_empty());
        assert!(open.instrument_snapshot().unwrap().is_empty());
    }

    // --- memo ---

    fn wav_bytes(rate: u32, frames: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        let mut fmt = [0u8; 16];
        fmt[0..2].copy_from_slice(&3u16.to_le_bytes());
        fmt[2..4].copy_from_slice(&1u16.to_le_bytes());
        fmt[4..8].copy_from_slice(&rate.to_le_bytes());
        fmt[12..14].copy_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&fmt);
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(frames * 4).to_le_bytes());
        out.extend(std::iter::repeat_n(0u8, frames as usize * 4));
        out
    }

    fn sidecar_value(bpm: f64, bar1: Value) -> Value {
        let host = if bar1.is_null() {
            Value::Null
        } else {
            json!("1234567890123")
        };
        json!({
            "schema": 1, "createdAt": "2026-11-13T09:30:00Z", "app": {"version": "0.1.0"},
            "input": {"deviceId": "BuiltInMicrophoneDevice", "deviceName": "MacBook Pro Microphone",
                "channel": 0, "channels": 1, "sampleRate": 48000},
            "output": {"baseLatency": 0.01, "outputLatency": 0.02},
            "tempo": {"bpm": bpm, "meter": "4/4", "cps": 0.5, "beatsPerBar": 4.0, "tempoOverride": false},
            "backing": {"song": true, "metronome": true, "countIn": true,
                "songPath": "songs/main.js", "startSection": "verse", "loopSection": null},
            "timing": {"bar1Frame": bar1, "bar1HostNs": host,
                "clockOffsetNs": {"start": null, "end": null}, "anchors": [], "outputStamps": []},
            "xruns": 0, "recovered": false
        })
    }

    /// A project with `main-01` (valid sidecar), `old` (no sidecar, unreadable WAV) and `bad`.
    fn memo_project_dir() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let p = Project::create(&tmp.path().join("demo"), "demo").unwrap();
        let rec = p.root().join("recordings");
        std::fs::create_dir_all(&rec).unwrap();
        std::fs::write(rec.join("main-01.wav"), wav_bytes(48000, 594432)).unwrap();
        std::fs::write(
            rec.join("main-01.json"),
            sidecar_value(120.0, json!(96000.0)).to_string(),
        )
        .unwrap();
        std::fs::write(rec.join("old.wav"), b"junk").unwrap();
        (tmp, p.root().to_path_buf())
    }

    fn add_bad(root: &Path) {
        let rec = root.join("recordings");
        std::fs::write(rec.join("bad.wav"), wav_bytes(48000, 480)).unwrap();
        std::fs::write(rec.join("bad.json"), "{nope").unwrap();
    }

    fn memo_run(args: &[&str], cwd: &Path, fake: &mut Fake) -> (i32, String, String) {
        let cli =
            Cli::try_parse_from(std::iter::once("strudeland").chain(args.iter().copied())).unwrap();
        let (mut stdin, mut stdout, mut stderr) = (&b""[..], Vec::new(), Vec::new());
        let result = {
            let mut env = Env {
                stdin: &mut stdin,
                stdin_is_tty: false,
                stdout: &mut stdout,
                stdout_is_tty: false,
                stderr: &mut stderr,
            };
            let Command::Memo { command } = &cli.command else {
                panic!("not a memo command")
            };
            let cwd = || Ok(cwd.to_path_buf());
            run_memo(command, cli.json, fake, &mut env, &cwd)
        };
        let mut stderr = String::from_utf8(stderr).unwrap();
        let code = match result {
            Ok(c) => c,
            Err(f) => {
                stderr.push_str(&format!("{}\n", f.message()));
                f.exit_code()
            }
        };
        (code, String::from_utf8(stdout).unwrap(), stderr)
    }

    #[test]
    fn memo_list_human_in_a_project() {
        let (_t, root) = memo_project_dir();
        let mut f = Fake::default();
        let (code, out, err) = memo_run(&["memo", "list"], &root, &mut f);
        assert_eq!(code, 0);
        assert_eq!(err, "");
        assert_eq!(
            out,
            "main-01  0:12.4  120 bpm  4/4  song+metronome+count-in  bar 1 known\n\
             old      ?       -        -    -                        -\n"
        );
        assert!(f.calls.is_empty());
    }

    #[test]
    fn memo_list_walks_up_from_a_subfolder() {
        let (_t, root) = memo_project_dir();
        let mut f = Fake::default();
        let (code, out, _) = memo_run(&["memo", "list"], &root.join("songs"), &mut f);
        assert_eq!(code, 0);
        assert!(out.starts_with("main-01"));
    }

    #[test]
    fn memo_list_with_a_bad_sidecar_warns_and_exits_0() {
        let (_t, root) = memo_project_dir();
        add_bad(&root);
        let mut f = Fake::default();
        let (code, out, err) = memo_run(&["memo", "list"], &root, &mut f);
        assert_eq!(code, 0);
        assert!(
            out.contains("bad      0:00.0  -        -    -                        bad sidecar\n")
        );
        assert!(err.starts_with("warning: invalid memo sidecar: recordings/bad.json: "));
    }

    #[test]
    fn memo_list_empty_and_json() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Project::create(&tmp.path().join("demo"), "demo").unwrap();
        let mut f = Fake::default();
        let (code, out, _) = memo_run(&["memo", "list"], p.root(), &mut f);
        assert_eq!((code, out.as_str()), (0, "no memos\n"));
        let (code, out, _) = memo_run(&["--json", "memo", "list"], p.root(), &mut f);
        assert_eq!(code, 0);
        let root = p.root().to_string_lossy().into_owned();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, json!({"project": root, "memos": []}));
    }

    #[test]
    fn memo_list_json_shape() {
        let (_t, root) = memo_project_dir();
        let mut f = Fake::default();
        let (_, out, _) = memo_run(&["--json", "memo", "list"], &root, &mut f);
        assert_eq!(out.lines().count(), 1);
        let v: Value = serde_json::from_str(&out).unwrap();
        let r = root.to_string_lossy();
        assert_eq!(v["project"], json!(r));
        let m = &v["memos"][0];
        assert_eq!(m["name"], "main-01");
        assert_eq!(m["path"], json!(format!("{r}/recordings/main-01.wav")));
        assert_eq!(m["frames"], 594432);
        assert_eq!(m["sampleRate"], 48000);
        assert_eq!(m["duration"], 12.384);
        assert_eq!(m["hasSidecar"], true);
        assert_eq!(m["sidecarError"], Value::Null);
        assert_eq!(m["tempo"]["bpm"], 120.0);
        assert_eq!(m["backing"]["countIn"], true);
        assert_eq!(m["bar1Frame"], 96000.0);
        assert_eq!(m["bar1Seconds"], 2.0);
        assert_eq!(m["recovered"], false);
        let old = &v["memos"][1];
        assert_eq!(old["name"], "old");
        for k in [
            "duration",
            "frames",
            "sampleRate",
            "tempo",
            "backing",
            "bar1Frame",
            "bar1Seconds",
            "recovered",
        ] {
            assert_eq!(old[k], Value::Null, "{k}");
        }
        assert_eq!(old["hasSidecar"], false);
    }

    #[test]
    fn memo_show_human() {
        let (_t, root) = memo_project_dir();
        let mut f = Fake::default();
        let (code, out, err) = memo_run(&["memo", "show", "main-01"], &root, &mut f);
        let r = root.display();
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(
            out,
            format!(
                "main-01\npath: {r}/recordings/main-01.wav\n\
                 duration: 12.384 s (594432 frames at 48000 Hz)\n\
                 input: MacBook Pro Microphone (channel 1 of 1)\n\
                 tempo: 120 bpm 4/4\n\
                 backing: song, metronome, count-in\n\
                 song: songs/main.js\nstart section: verse\n\
                 bar 1: frame 96000 (2.000 s)\nxruns: 0\n"
            )
        );
        // A trailing .wav is ignored.
        let (code, out2, _) = memo_run(&["memo", "show", "main-01.wav"], &root, &mut f);
        assert_eq!((code, out2), (0, out));
    }

    #[test]
    fn memo_show_without_a_sidecar_and_unreadable_wav() {
        let (_t, root) = memo_project_dir();
        let mut f = Fake::default();
        let (code, out, _) = memo_run(&["memo", "show", "old"], &root, &mut f);
        assert_eq!(code, 0);
        assert_eq!(
            out,
            format!(
                "old\npath: {}/recordings/old.wav\nduration: unknown\nsidecar: none\n",
                root.display()
            )
        );
        let (_, out, _) = memo_run(&["--json", "memo", "show", "old"], &root, &mut f);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v,
            json!({"name": "old", "path": format!("{}/recordings/old.wav", root.display()),
                "duration": null, "frames": null, "sampleRate": null, "bar1Seconds": null,
                "hasSidecar": false})
        );
    }

    #[test]
    fn memo_show_json_has_sidecar_keys_and_derived_fields() {
        let (_t, root) = memo_project_dir();
        let mut f = Fake::default();
        let (_, out, _) = memo_run(&["--json", "memo", "show", "main-01"], &root, &mut f);
        assert_eq!(out.lines().count(), 1);
        let v: Value = serde_json::from_str(&out).unwrap();
        let mut want = sidecar_value(120.0, json!(96000.0));
        let m = want.as_object_mut().unwrap();
        m.insert("name".into(), json!("main-01"));
        m.insert(
            "path".into(),
            json!(format!("{}/recordings/main-01.wav", root.display())),
        );
        m.insert("duration".into(), json!(12.384));
        m.insert("frames".into(), json!(594432));
        m.insert("sampleRate".into(), json!(48000));
        m.insert("bar1Seconds".into(), json!(2.0));
        m.insert("hasSidecar".into(), json!(true));
        assert_eq!(v, want);
    }

    #[test]
    fn memo_show_variants_override_recovered_and_no_clock() {
        let (_t, root) = memo_project_dir();
        let mut sc = sidecar_value(96.5, Value::Null);
        sc["tempo"]["tempoOverride"] = json!(true);
        sc["backing"] = json!({"song": false, "metronome": false, "countIn": false,
            "songPath": null, "startSection": null, "loopSection": "chorus"});
        sc["recovered"] = json!(true);
        sc["xruns"] = json!(3);
        std::fs::write(root.join("recordings/main-01.json"), sc.to_string()).unwrap();
        let mut f = Fake::default();
        let (_, out, _) = memo_run(&["memo", "show", "main-01"], &root, &mut f);
        assert!(out.contains("tempo: 96.5 bpm 4/4 (tempo override)\n"));
        assert!(out.ends_with(
            "backing: none\nloop section: chorus\nbar 1: unknown\nxruns: 3\nrecovered: yes\n"
        ));
        let (_, list, _) = memo_run(&["memo", "list"], &root, &mut f);
        assert!(
            list.starts_with("main-01  0:12.4  96.5 bpm  4/4  none  bar 1 unknown (recovered)\n")
        );
    }

    #[test]
    fn memo_show_errors_and_exit_codes() {
        let (_t, root) = memo_project_dir();
        add_bad(&root);
        let mut f = Fake::default();
        let (code, out, err) = memo_run(&["memo", "show", "nope"], &root, &mut f);
        assert_eq!(
            (code, out.as_str(), err.as_str()),
            (1, "", "error: memo nope not found\n")
        );
        let (code, out, err) = memo_run(&["memo", "show", "bad"], &root, &mut f);
        assert_eq!((code, out.as_str()), (1, ""));
        assert!(err.starts_with("error: invalid memo sidecar: recordings/bad.json: "));
        let (code, out, err) = memo_run(&["memo", "show", "a.b"], &root, &mut f);
        assert_eq!((code, out.as_str()), (4, ""));
        assert_eq!(
            err,
            "error: invalid memo name \"a.b\" (use letters, digits, space, - and _; 1 to 64 characters)\n"
        );
        let mut sc = sidecar_value(120.0, Value::Null);
        sc["schema"] = json!(2);
        std::fs::write(root.join("recordings/main-01.json"), sc.to_string()).unwrap();
        let (code, _, err) = memo_run(&["memo", "show", "main-01"], &root, &mut f);
        assert_eq!(
            (code, err.as_str()),
            (
                1,
                "error: unsupported memo sidecar schema 2 (this app reads schema 1)\n"
            )
        );
    }

    #[test]
    fn memo_project_flag_does_not_walk_up_or_fall_through() {
        let (_t, root) = memo_project_dir();
        let other = tempfile::tempdir().unwrap();
        let mut f = Fake::default();
        // The flag wins over the cwd's project.
        let (code, out, _) = memo_run(
            &["memo", "list", "--project", root.to_str().unwrap()],
            other.path(),
            &mut f,
        );
        assert_eq!(code, 0);
        assert!(out.starts_with("main-01"));
        // Relative to the cwd.
        let (code, out, _) = memo_run(
            &["memo", "list", "--project", "demo"],
            root.parent().unwrap(),
            &mut f,
        );
        assert_eq!(code, 0);
        assert!(out.starts_with("main-01"));
        // A subfolder is not walked up from, and the app is not asked.
        let (code, out, err) = memo_run(&["memo", "list", "--project", "songs"], &root, &mut f);
        assert_eq!((code, out.as_str()), (1, ""));
        assert_eq!(
            err,
            format!(
                "error: {} is not a strudeland project (no project.json)\n",
                root.join("songs").canonicalize().unwrap().display()
            )
        );
        assert!(f.calls.is_empty());
    }

    #[test]
    fn memo_falls_back_to_the_running_apps_project() {
        let (_t, root) = memo_project_dir();
        let elsewhere = tempfile::tempdir().unwrap();
        let mut state = state_json();
        state["project"] = json!({"name": "demo", "root": root.to_string_lossy()});
        let mut f = Fake::ok(state);
        let (code, out, _) = memo_run(&["memo", "list"], elsewhere.path(), &mut f);
        assert_eq!(code, 0);
        assert!(out.starts_with("main-01"));
        assert_eq!(f.calls.len(), 1);
        assert_eq!(f.calls[0].0, Method::State);
    }

    #[test]
    fn memo_without_any_project_is_a_usage_error_whatever_the_app_says() {
        let elsewhere = tempfile::tempdir().unwrap();
        let want = "error: not in a strudeland project; run it inside a project folder, pass --project DIR, or open a project in the app\n";
        let mut no_project = state_json();
        no_project["project"] = Value::Null;
        let replies = [
            Err(CallError::NotRunning { path: "/s".into() }),
            Err(CallError::Timeout),
            Err(rpc(codes::NOT_READY, "not ready")),
            Ok(no_project),
        ];
        for reply in replies {
            let mut f = Fake::with(reply);
            let (code, out, err) = memo_run(&["memo", "list"], elsewhere.path(), &mut f);
            assert_eq!((code, out.as_str(), err.as_str()), (4, "", want));
        }
    }

    // --- transcribe ---

    use strudeland_project::Sidecar;
    use strudeland_transcribe::eval::synth::{Case, SynthSettings, write_wav};

    const TAKE_RATE: u32 = 16_000;

    /// A short synthetic melody: `bars` bars of the clean generator at seed 1000.
    fn short_case(bars: u32) -> Case {
        let mut case = Case::generate(1000, &SynthSettings::default());
        case.bars = bars;
        case.truth.retain(|n| n.bar <= bars);
        case
    }

    fn take_sidecar(case: &Case, edit: impl FnOnce(&mut Value)) -> Sidecar {
        let mut v = sidecar_value(case.bpm, json!(case.origin_seconds * f64::from(TAKE_RATE)));
        edit(&mut v);
        serde_json::from_value(v).unwrap()
    }

    /// A project holding `take` (the synthetic melody) with a sidecar that matches it.
    fn transcribe_project(bars: u32) -> (tempfile::TempDir, PathBuf, Case) {
        let tmp = tempfile::tempdir().unwrap();
        let p = Project::create(&tmp.path().join("demo"), "demo").unwrap();
        std::fs::create_dir_all(p.root().join("recordings")).unwrap();
        let case = short_case(bars);
        write_wav(
            &p.recording_wav_path("take"),
            &case.render(TAKE_RATE),
            TAKE_RATE,
        )
        .unwrap();
        p.write_sidecar("take", &take_sidecar(&case, |_| {}))
            .unwrap();
        (tmp, p.root().to_path_buf(), case)
    }

    fn transcribe_run(args: &[&str], cwd: &Path) -> (i32, String, String) {
        let mut f = Fake::default();
        let mut all = vec!["transcribe"];
        all.extend(args);
        let cli = Cli::try_parse_from(std::iter::once("strudeland").chain(all)).unwrap();
        let (mut stdin, mut stdout, mut stderr) = (&b""[..], Vec::new(), Vec::new());
        let result = {
            let mut env = Env {
                stdin: &mut stdin,
                stdin_is_tty: false,
                stdout: &mut stdout,
                stdout_is_tty: false,
                stderr: &mut stderr,
            };
            let Command::Transcribe(a) = &cli.command else {
                panic!("not a transcribe command")
            };
            let cwd = || Ok(cwd.to_path_buf());
            run_transcribe(a, cli.json, &mut f, &mut env, &cwd)
        };
        let mut stderr = String::from_utf8(stderr).unwrap();
        let code = match result {
            Ok(c) => c,
            Err(fail) => {
                stderr.push_str(&format!("{}\n", fail.message()));
                fail.exit_code()
            }
        };
        (code, String::from_utf8(stdout).unwrap(), stderr)
    }

    fn json_run(args: &[&str], cwd: &Path) -> (i32, Value, String) {
        let mut all = vec!["--json"];
        all.extend(args);
        let (code, out, err) = transcribe_run(&all, cwd);
        let v = if out.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&out).unwrap()
        };
        (code, v, err)
    }

    fn keys(v: &Value) -> Vec<&str> {
        let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        k.sort_unstable();
        k
    }

    #[test]
    fn transcribe_human_output() {
        let (_t, root, case) = transcribe_project(2);
        let (code, out, err) = transcribe_run(&["take"], &root);
        assert_eq!((code, err.as_str()), (0, ""), "{out}");
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("memo: take  0:"), "{out}");
        assert!(lines[0].ends_with("(bars 1-2)"), "{out}");
        assert_eq!(
            lines[1],
            format!(
                "grid: {} bpm, 4/4, 16 steps per bar (tempo sidecar, phase sidecar)",
                case.bpm
            )
        );
        assert!(lines[2].starts_with("tuning: "), "{out}");
        assert!(out.contains("\nnote(\""), "{out}");
        assert!(out.contains("\nbar:beat  note  length  conf\n"), "{out}");
        assert!(out.contains("\n1:1  "), "{out}");
    }

    #[test]
    fn transcribe_json_shape_has_every_key() {
        let (_t, root, _) = transcribe_project(2);
        let (code, v, err) = json_run(&["take"], &root);
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(
            keys(&v),
            [
                "backing",
                "dropped",
                "duration",
                "grid",
                "key",
                "memo",
                "notation",
                "notes",
                "timingSpread",
                "tuning",
                "warnings"
            ]
        );
        assert_eq!(v["memo"], "take");
        assert_eq!(
            keys(&v["grid"]),
            [
                "beatsPerBar",
                "bpm",
                "confidence",
                "division",
                "meter",
                "origin",
                "source",
                "stepsPerBar",
                "suggestedDivision",
                "suggestedShiftMs"
            ]
        );
        assert_eq!(keys(&v["grid"]["source"]), ["phase", "tempo"]);
        assert_eq!(keys(&v["tuning"]), ["cents", "source"]);
        assert_eq!(
            keys(&v["notation"]),
            ["firstBar", "lastBar", "n", "note", "scale"]
        );
        assert_eq!(v["notation"]["firstBar"], 1);
        assert_eq!(v["notation"]["lastBar"], 2);
        assert_eq!(
            keys(&v["notes"][0]),
            [
                "alternative",
                "bar",
                "cents",
                "components",
                "confidence",
                "degree",
                "end",
                "midi",
                "name",
                "snapErrorMs",
                "start",
                "step",
                "steps",
                "tiedToNext"
            ]
        );
        assert_eq!(
            keys(&v["notes"][0]["components"]),
            ["fit", "onset", "stability", "voiced"]
        );
        assert_eq!(keys(&v["backing"]), ["song", "startSection"]);
        assert!(v.get("frames").is_none() && v.get("onsets").is_none());
    }

    #[test]
    fn transcribe_json_is_one_line() {
        let (_t, root, _) = transcribe_project(2);
        let (code, out, _) = transcribe_run(&["--json", "take"], &root);
        assert_eq!(code, 0);
        assert!(
            out.ends_with("}\n") && out.matches('\n').count() == 1,
            "{out}"
        );
    }

    #[test]
    fn transcribe_debug_needs_json() {
        let (_t, root, _) = transcribe_project(2);
        let (code, out, err) = transcribe_run(&["take", "--debug"], &root);
        assert_eq!((code, out.as_str()), (4, ""));
        assert_eq!(err, "error: --debug needs --json\n");
        let (code, v, _) = json_run(&["take", "--debug"], &root);
        assert_eq!(code, 0);
        assert!(v["frames"].as_array().is_some_and(|f| !f.is_empty()));
        assert!(v["onsets"].is_array());
    }

    #[test]
    fn transcribe_debug_needs_json_before_any_lookup() {
        // A usage error is reported before the project or the memo is looked up.
        let (_t, root, _) = transcribe_project(1);
        let (code, out, err) = transcribe_run(&["nope", "--debug"], &root);
        assert_eq!(
            (code, out.as_str(), err.as_str()),
            (4, "", "error: --debug needs --json\n")
        );
        let elsewhere = tempfile::tempdir().unwrap();
        let (code, _, err) = transcribe_run(&["take", "--debug"], elsewhere.path());
        assert_eq!((code, err.as_str()), (4, "error: --debug needs --json\n"));
    }

    #[test]
    fn transcribe_missing_memo_exit_1() {
        let (_t, root, _) = transcribe_project(1);
        let (code, out, err) = transcribe_run(&["nope"], &root);
        assert_eq!(
            (code, out.as_str(), err.as_str()),
            (1, "", "error: memo nope not found\n")
        );
    }

    #[test]
    fn transcribe_bad_name_exit_4() {
        let (_t, root, _) = transcribe_project(1);
        let (code, out, err) = transcribe_run(&["a.b"], &root);
        assert_eq!((code, out.as_str()), (4, ""));
        assert!(err.starts_with("error: invalid memo name \"a.b\""), "{err}");
    }

    #[test]
    fn transcribe_no_project_exit_4() {
        let elsewhere = tempfile::tempdir().unwrap();
        let mut f = Fake::with(Err(CallError::NotRunning { path: "/s".into() }));
        let cli = Cli::try_parse_from(["strudeland", "transcribe", "take"]).unwrap();
        let Command::Transcribe(a) = &cli.command else {
            panic!()
        };
        let (mut stdin, mut stdout, mut stderr) = (&b""[..], Vec::new(), Vec::new());
        let mut env = Env {
            stdin: &mut stdin,
            stdin_is_tty: false,
            stdout: &mut stdout,
            stdout_is_tty: false,
            stderr: &mut stderr,
        };
        let cwd = || Ok(elsewhere.path().to_path_buf());
        let Err(fail) = run_transcribe(a, false, &mut f, &mut env, &cwd) else {
            panic!("expected a failure")
        };
        assert_eq!(fail.exit_code(), 4);
        assert!(
            fail.message()
                .starts_with("error: not in a strudeland project")
        );
        assert!(stdout.is_empty());
    }

    #[test]
    fn transcribe_bars_without_grid_exit_4() {
        let (_t, root, case) = transcribe_project(2);
        std::fs::remove_file(root.join("recordings/take.json")).unwrap();
        let (code, out, err) = transcribe_run(&["take", "--bars", "1-2"], &root);
        assert_eq!((code, out.as_str()), (4, ""));
        assert_eq!(
            err,
            "error: --bars needs a known grid (sidecar bar 1, or --bpm, --meter and --bar1)\n"
        );
        // Flags can supply the grid.
        let bpm = case.bpm.to_string();
        let bar1 = case.origin_seconds.to_string();
        let (code, _, _) = transcribe_run(
            &[
                "take", "--bars", "1-2", "--bpm", &bpm, "--meter", "4/4", "--bar1", &bar1,
            ],
            &root,
        );
        assert_eq!(code, 0);
    }

    #[test]
    fn transcribe_bars_with_unheard_tempo_exit_4() {
        // No song and no metronome (so no bar1Frame either): the sidecar tempo is checked against
        // the audio and may be replaced, so with --bar1 alone the bars are not known in advance.
        let (_t, root, case) = transcribe_project(2);
        let unheard = take_sidecar(&case, |v| {
            v["backing"]["song"] = json!(false);
            v["backing"]["metronome"] = json!(false);
            v["backing"]["countIn"] = json!(false);
            v["timing"]["bar1Frame"] = Value::Null;
            v["timing"]["bar1HostNs"] = Value::Null;
        });
        Project::open(&root)
            .unwrap()
            .write_sidecar("take", &unheard)
            .unwrap();
        let bar1 = case.origin_seconds.to_string();
        let (code, out, err) = transcribe_run(&["take", "--bars", "1-2", "--bar1", &bar1], &root);
        assert_eq!((code, out.as_str()), (4, ""));
        assert_eq!(
            err,
            "error: --bars needs a known grid (sidecar bar 1, or --bpm, --meter and --bar1)\n"
        );
        // A flag tempo is not second-guessed: with it the grid is known.
        let bpm = case.bpm.to_string();
        let (code, v, err) = json_run(
            &["take", "--bars", "1-2", "--bar1", &bar1, "--bpm", &bpm],
            &root,
        );
        assert_eq!(code, 0, "{err}");
        assert_eq!(
            v["grid"]["source"],
            json!({"tempo": "flag", "phase": "flag"})
        );
        assert_eq!(v["notation"]["firstBar"], 1);
    }

    #[test]
    fn transcribe_over_10_minutes_exit_4() {
        let (_t, root, _) = transcribe_project(1);
        // A header that claims 30 minutes of audio, with no samples behind it.
        let frames = 48_000u32 * 1832;
        let mut wav = wav_bytes(48000, 0);
        wav[28..32].copy_from_slice(&(48_000u32 * 4).to_le_bytes());
        wav[34..36].copy_from_slice(&32u16.to_le_bytes());
        let data_at = wav.len() - 4;
        wav[data_at..].copy_from_slice(&(frames * 4).to_le_bytes());
        std::fs::write(root.join("recordings/long.wav"), wav).unwrap();
        let (code, out, err) = transcribe_run(&["long"], &root);
        assert_eq!((code, out.as_str()), (4, ""));
        assert_eq!(
            err,
            "error: the memo is 1832 s long; pass --start/--end or --bars to pick up to 10 minutes\n"
        );
    }

    #[test]
    fn transcribe_recovered_memo_warns() {
        let (_t, root, case) = transcribe_project(2);
        let recovered = take_sidecar(&case, |v| {
            v["recovered"] = json!(true);
            v["tempo"]["bpm"] = json!(120.0);
            v["timing"]["bar1Frame"] = Value::Null;
        });
        Project::open(&root)
            .unwrap()
            .write_sidecar("take", &recovered)
            .unwrap();
        let (code, v, err) = json_run(&["take"], &root);
        assert_eq!(code, 0);
        let first = "the memo was recovered after a crash; its tempo is a placeholder";
        assert_eq!(v["warnings"][0], first);
        assert!(err.starts_with(&format!("warning: {first}\n")), "{err}");
        // The placeholder tempo is not used: the grid is not from the sidecar.
        assert_ne!(v["grid"]["source"]["tempo"], "sidecar");
    }

    #[test]
    fn transcribe_no_sidecar_infers() {
        let (_t, root, _) = transcribe_project(2);
        std::fs::remove_file(root.join("recordings/take.json")).unwrap();
        let (code, v, err) = json_run(&["take"], &root);
        assert_eq!(code, 0, "{err}");
        assert_eq!(
            v["warnings"][0],
            "no sidecar; the grid is inferred from the audio"
        );
        assert!(err.contains("warning: no sidecar; the grid is inferred from the audio\n"));
        assert!(
            err.contains("warning: meter assumed 4/4; pass --meter\n"),
            "{err}"
        );
        assert!(v["backing"].is_null());
    }

    #[test]
    fn transcribe_no_notes_prints_no_notes() {
        let (_t, root, _) = transcribe_project(1);
        write_wav(
            &root.join("recordings/quiet.wav"),
            &vec![0.0; TAKE_RATE as usize],
            TAKE_RATE,
        )
        .unwrap();
        let (code, out, err) = transcribe_run(&["quiet"], &root);
        assert_eq!(code, 0);
        assert_eq!(out, "memo: quiet  0:01.0\nno notes\n");
        assert!(err.contains("warning: no notes found\n"), "{err}");
    }

    #[test]
    fn transcribe_bad_key_exit_4() {
        let (_t, root, _) = transcribe_project(1);
        let (code, out, err) = transcribe_run(&["take", "--key", "H:major"], &root);
        assert_eq!((code, out.as_str()), (4, ""));
        assert_eq!(
            err,
            "error: bad --key \"H:major\"; use ROOT:SCALE such as C:major\n"
        );
    }

    #[test]
    fn transcribe_unknown_scale_exit_4() {
        let (_t, root, _) = transcribe_project(1);
        let (code, _, err) = transcribe_run(&["take", "--key", "C:wobbly"], &root);
        assert_eq!(code, 4);
        assert!(
            err.starts_with("error: unknown scale \"wobbly\"; use one of: major, minor"),
            "{err}"
        );
    }

    #[test]
    fn transcribe_grid_does_not_divide_exit_4() {
        let (_t, root, _) = transcribe_project(1);
        let (code, out, err) = transcribe_run(&["take", "--meter", "3/8", "--grid", "4"], &root);
        assert_eq!((code, out.as_str()), (4, ""));
        assert_eq!(
            err,
            "error: --grid 4 does not divide a 3/8 bar into whole steps (1.5 steps); try a finer grid\n"
        );
    }

    #[test]
    fn transcribe_bad_meter_exit_4() {
        let (_t, root, _) = transcribe_project(1);
        let (code, _, err) = transcribe_run(&["take", "--meter", "4/3"], &root);
        assert_eq!(code, 4);
        assert_eq!(
            err,
            "error: bad --meter \"4/3\"; use N/D such as 4/4 or 6/8\n"
        );
    }

    #[test]
    fn transcribe_bar1_past_end_exit_1() {
        let (_t, root, _) = transcribe_project(1);
        let (code, out, err) = transcribe_run(&["take", "--bar1", "999"], &root);
        assert_eq!((code, out.as_str()), (1, ""));
        assert!(
            err.starts_with("error: bar 1 (999.000 s, from --bar1) is past the end of the audio ("),
            "{err}"
        );
    }

    #[test]
    fn transcribe_unreadable_wav_exit_1() {
        let (_t, root, _) = transcribe_project(1);
        std::fs::write(root.join("recordings/old.wav"), b"junk").unwrap();
        let (code, out, err) = transcribe_run(&["old"], &root);
        assert_eq!((code, out.as_str()), (1, ""));
        assert!(err.starts_with("error: cannot read "), "{err}");
        assert!(err.contains("old.wav"), "{err}");
    }

    #[test]
    fn transcribe_flags_override_sidecar() {
        let (_t, root, case) = transcribe_project(2);
        let (_, v, _) = json_run(&["take"], &root);
        assert_eq!(v["grid"]["bpm"], case.bpm);
        assert_eq!(
            v["grid"]["source"],
            json!({"tempo": "sidecar", "phase": "sidecar"})
        );
        assert_eq!(v["grid"]["meter"], "4/4");
        let bar1 = (case.origin_seconds + 0.01).to_string();
        let (code, v, _) = json_run(
            &[
                "take", "--bpm", "100", "--bar1", &bar1, "--meter", "2/4", "--key", "C:major",
            ],
            &root,
        );
        assert_eq!(code, 0);
        assert_eq!(v["grid"]["bpm"], 100.0);
        assert_eq!(v["grid"]["meter"], "2/4");
        assert_eq!(v["grid"]["beatsPerBar"], 2.0);
        assert_eq!(
            v["grid"]["source"],
            json!({"tempo": "flag", "phase": "flag"})
        );
        assert!(
            (v["grid"]["origin"].as_f64().unwrap() - (case.origin_seconds + 0.01)).abs() < 1e-6
        );
        assert_eq!(v["key"]["name"], "C major");
        assert_eq!(v["key"]["source"], "flag");
        // One field from a flag, the rest from the sidecar.
        let (_, v, _) = json_run(&["take", "--bpm", "100"], &root);
        assert_eq!(
            v["grid"]["source"],
            json!({"tempo": "flag", "phase": "sidecar"})
        );
        assert_eq!(v["grid"]["meter"], "4/4");
    }

    #[test]
    fn transcribe_bars_selects_the_span() {
        let (_t, root, case) = transcribe_project(3);
        let (code, v, err) = json_run(&["take", "--bars", "2-2"], &root);
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(v["notation"]["firstBar"], 2);
        assert_eq!(v["notation"]["lastBar"], 2);
        assert!((v["duration"].as_f64().unwrap() - case.bar_seconds()).abs() < 0.05);
        let notes = v["notes"].as_array().unwrap();
        assert!(!notes.is_empty());
        let bars: Vec<u64> = notes.iter().map(|n| n["bar"].as_u64().unwrap()).collect();
        assert!(bars.iter().all(|b| (1..=3).contains(b)), "{bars:?}");
        assert!(bars.contains(&2), "{bars:?}");
        let (_, out, _) = transcribe_run(&["take", "--bars", "2-2"], &root);
        assert!(out.lines().next().unwrap().ends_with("(bar 2)"), "{out}");
    }

    #[test]
    fn transcribe_bars_keeps_trailing_rest_bars() {
        // Notes in bars 1 and 2 of a three-bar take: --bars 2-3 is a two-bar pattern.
        let (_t, root, _) = transcribe_project(1);
        let mut case = short_case(2);
        case.bars = 3;
        write_wav(
            &root.join("recordings/rest.wav"),
            &case.render(TAKE_RATE),
            TAKE_RATE,
        )
        .unwrap();
        Project::open(&root)
            .unwrap()
            .write_sidecar("rest", &take_sidecar(&case, |_| {}))
            .unwrap();
        let (code, v, err) = json_run(&["rest", "--bars", "2-3"], &root);
        assert_eq!(code, 0, "{err}");
        assert_eq!(v["notation"]["firstBar"], 2);
        assert_eq!(v["notation"]["lastBar"], 3);
        let note = v["notation"]["note"].as_str().unwrap();
        assert!(note.starts_with("<[") && note.ends_with("] ~>"), "{note}");
        // Without --bars the pattern ends at the last bar with a note.
        let (_, v, _) = json_run(&["rest"], &root);
        assert_eq!(v["notation"]["lastBar"], 2);
    }

    #[test]
    fn transcribe_bars_with_shift_starts_at_its_bar() {
        // The span is cut from the shifted origin the grid uses, so it starts in bar 2.
        let (_t, root, _) = transcribe_project(3);
        for shift in ["40", "-40"] {
            let (code, v, err) = json_run(&["take", "--bars", "2-2", "--shift", shift], &root);
            assert_eq!(code, 0, "{err}");
            assert_eq!(v["notation"]["firstBar"], 2, "--shift {shift}");
            assert_eq!(v["notation"]["lastBar"], 2, "--shift {shift}");
        }
    }

    #[test]
    fn transcribe_backing_comes_from_sidecar() {
        let (_t, root, _) = transcribe_project(1);
        let (_, v, _) = json_run(&["take"], &root);
        assert_eq!(
            v["backing"],
            json!({"song": "songs/main.js", "startSection": "verse"})
        );
    }
}
