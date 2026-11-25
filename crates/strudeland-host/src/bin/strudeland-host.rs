//! `strudeland-host`: the headless host's harness binary (not a `strudeland` subcommand).
//!
//! ```text
//! strudeland-host plan --song <file> --request <json|file> [--out <file>] [--timeout <secs>]
//! strudeland-host plan-render --song <file> --request <json|file> [--out <file>]
//! strudeland-host query --song <file> --from <c> --to <c> [--layout clip|linear]
//! strudeland-host bench [--bars N] [--lengths FILE] [--profile] [--out-dir DIR] <song…>
//! ```
//!
//! `plan` writes the job JSON to `--out` (or stdout, no trailing newline) and exactly one line
//! on stderr: the job's info JSON. `query` writes the events JSON on stdout. Exit codes: 0 ok;
//! 1 host error (stderr one line `strudeland-host: <kind>: <message>`, nothing on stdout); 2 bad
//! input (stderr `strudeland-host: <message>` and the usage). `--request` is JSON text when its
//! first non-space character is `{`, else a file path. `plan --timeout` bounds the planning
//! (seconds, greater than 0 and at most 86400; past it the host error is `timeout: …`).
//!
//! `plan-render` plans every chunk of a render in one call (`--request` is a `RenderPlanRequest`:
//! `{"length":{"kind":"clip"|"section"|"length"|"window",…},"preset"?,"includeEvents"?,
//! "timeoutSec"?,"instruments"?}`) and writes the plan as one JSON line to `--out` (or stdout, no
//! trailing newline), nothing on stderr. A bad length request (the bundle's `bad request: …`) is
//! bad input; a length the song cannot satisfy is a host error (`meta: …`, naming the flag).
//!
//! `bench` plans each song three times on fresh hosts and writes one JSON document on stdout
//! (medians, job size and hash, peak memory, the budget report) and a table on stderr; exit 0 when
//! every song planned, 1 when one failed (listed; the others still run), 2 for bad input. A budget
//! miss is in the document's `budget`, not in the exit code.

use std::fs;
use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

use serde::de::DeserializeOwned;
use strudeland_host::bench;
use strudeland_host::{
    Host, HostError, Layout, MAX_TIMEOUT_SEC, RenderPlanRequest, RenderRequest, timeout_from_secs,
};

const USAGE: &str = "usage: strudeland-host plan --song <file> --request <json|file> [--out <file>] [--timeout <secs>]\n       strudeland-host plan-render --song <file> --request <json|file> [--out <file>]\n       strudeland-host query --song <file> --from <c> --to <c> [--layout clip|linear]\n       strudeland-host bench [--bars N] [--lengths FILE] [--profile] [--out-dir DIR] <song…>";

/// How a command failed: bad input (the message, then the usage; exit 2) or a host error (one
/// line; exit 1).
enum Exit {
    BadInput(String),
    Host(HostError),
}

impl From<HostError> for Exit {
    /// The bundle's `bad request: …` plan error is bad input; every other error is a host error.
    fn from(e: HostError) -> Self {
        match e {
            HostError::Plan(m) if m.starts_with("bad request:") => Exit::BadInput(m),
            e => Exit::Host(e),
        }
    }
}

/// Bad input: the message, then the usage; exit 2.
fn bad_input(message: impl std::fmt::Display) -> ExitCode {
    eprintln!(
        "strudeland-host: {}\n{USAGE}",
        one_line(&message.to_string())
    );
    ExitCode::from(2)
}

/// A host error on one line; exit 1.
fn host_error(e: &HostError) -> ExitCode {
    eprintln!("strudeland-host: {}", one_line(&e.to_string()));
    ExitCode::from(1)
}

/// The exit code of a command's result, its failure reported on stderr.
fn finish(result: Result<(), Exit>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Exit::BadInput(message)) => bad_input(message),
        Err(Exit::Host(e)) => host_error(&e),
    }
}

fn one_line(text: &str) -> String {
    text.replace('\r', "").replace('\n', "\\n")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("plan") => {
            match parse_flags(rest, &["--song", "--request"], &["--out", "--timeout"]) {
                Ok((req, opt)) => finish(run_plan(
                    &req[0],
                    &req[1],
                    opt[0].as_deref(),
                    opt[1].as_deref(),
                )),
                Err(e) => bad_input(e),
            }
        }
        Some("plan-render") => match parse_flags(rest, &["--song", "--request"], &["--out"]) {
            Ok((req, opt)) => finish(run_plan_render(&req[0], &req[1], opt[0].as_deref())),
            Err(e) => bad_input(e),
        },
        Some("query") => match parse_flags(rest, &["--song", "--from", "--to"], &["--layout"]) {
            Ok((req, opt)) => finish(run_query(&req[0], &req[1], &req[2], opt[0].as_deref())),
            Err(e) => bad_input(e),
        },
        Some("bench") => match bench::parse_args(rest) {
            Ok(args) => run_bench(&args),
            Err(e) => bad_input(e),
        },
        Some(other) => bad_input(format!("unknown command {other}")),
        None => bad_input("missing command"),
    }
}

type Parsed = (Vec<String>, Vec<Option<String>>);

/// The values of `required` (each present) and `optional` flags, each in the order given;
/// every flag takes one value and may appear once, in any order on the command line.
fn parse_flags(args: &[String], required: &[&str], optional: &[&str]) -> Result<Parsed, String> {
    let names: Vec<&str> = required.iter().chain(optional).copied().collect();
    let mut values: Vec<Option<String>> = vec![None; names.len()];
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let Some(i) = names.iter().position(|f| f == flag) else {
            return Err(format!("unknown argument {flag}"));
        };
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        if values[i].replace(value.clone()).is_some() {
            return Err(format!("{flag} given twice"));
        }
    }
    let optional_values = values.split_off(required.len());
    let required_values = required
        .iter()
        .zip(values)
        .map(|(f, v)| v.ok_or_else(|| format!("missing {f}")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((required_values, optional_values))
}

fn read_text(path: &str, what: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("reading {what} {path}: {e}"))
}

/// The request from `--request`: JSON text when it starts with `{` (after spaces), else a file.
fn read_request_json<T: DeserializeOwned>(arg: &str) -> Result<T, String> {
    let text = if arg.trim_start().starts_with('{') {
        arg.to_string()
    } else {
        read_text(arg, "the request")?
    };
    serde_json::from_str(&text).map_err(|e| format!("invalid request: {e}"))
}

/// `plan`'s request, validated.
fn read_request(arg: &str) -> Result<RenderRequest, String> {
    let request: RenderRequest = read_request_json(arg)?;
    request
        .validate()
        .map_err(|e| format!("invalid request: {e}"))?;
    Ok(request)
}

/// `plan-render`'s request, validated.
fn read_plan_request(arg: &str) -> Result<RenderPlanRequest, String> {
    let request: RenderPlanRequest = read_request_json(arg)?;
    request
        .validate()
        .map_err(|e| format!("invalid request: {e}"))?;
    Ok(request)
}

/// `--timeout`'s seconds: a number greater than 0 and at most [`MAX_TIMEOUT_SEC`].
fn parse_timeout(text: &str) -> Result<Duration, String> {
    text.trim()
        .parse::<f64>()
        .ok()
        .and_then(timeout_from_secs)
        .ok_or_else(|| {
            format!(
                "--timeout must be a number of seconds greater than 0 and at most {MAX_TIMEOUT_SEC}, got {text}"
            )
        })
}

/// Writes `text` to `out`, or to stdout without a trailing newline.
fn write_output(text: &str, out: Option<&str>) -> Result<(), String> {
    match out {
        Some(path) => fs::write(path, text).map_err(|e| format!("writing {path}: {e}")),
        None => write_stdout(text),
    }
}

fn write_stdout(text: &str) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| format!("writing stdout: {e}"))
}

fn run_plan(
    song: &str,
    request: &str,
    out: Option<&str>,
    timeout: Option<&str>,
) -> Result<(), Exit> {
    let code = read_text(song, "the song").map_err(Exit::BadInput)?;
    let request = read_request(request).map_err(Exit::BadInput)?;
    let timeout = timeout
        .map(parse_timeout)
        .transpose()
        .map_err(Exit::BadInput)?;
    let planned = Host::new()?.plan_job_with_timeout(&code, &request, timeout)?;
    write_output(&planned.json, out).map_err(Exit::BadInput)?;
    // The bundle's `JSON.stringify` of the info: one line.
    eprintln!("{}", planned.info);
    Ok(())
}

fn run_plan_render(song: &str, request: &str, out: Option<&str>) -> Result<(), Exit> {
    let code = read_text(song, "the song").map_err(Exit::BadInput)?;
    let request = read_plan_request(request).map_err(Exit::BadInput)?;
    let planned = Host::new()?.plan_render(&code, &request)?;
    let text = serde_json::to_string(&planned)
        .map_err(|e| HostError::Runtime(format!("serializing the plan: {e}")))?;
    write_output(&text, out).map_err(Exit::BadInput)
}

fn parse_cycle(flag: &str, text: &str) -> Result<f64, String> {
    text.trim()
        .parse::<f64>()
        .map_err(|_| format!("{flag} must be a number, got {text}"))
}

fn run_query(song: &str, from: &str, to: &str, layout: Option<&str>) -> Result<(), Exit> {
    let code = read_text(song, "the song").map_err(Exit::BadInput)?;
    let from = parse_cycle("--from", from).map_err(Exit::BadInput)?;
    let to = parse_cycle("--to", to).map_err(Exit::BadInput)?;
    let layout = match layout {
        None => Layout::Linear,
        Some(text) => text
            .parse::<Layout>()
            .map_err(|_| Exit::BadInput(format!("--layout must be clip or linear, got {text}")))?,
    };
    let events = Host::new()?.query(&code, from, to, layout)?;
    write_stdout(&events).map_err(Exit::BadInput)
}

fn run_bench(args: &bench::BenchArgs) -> ExitCode {
    eprintln!("{}", bench::TABLE_HEADER);
    let report = match bench::run(args, |song| eprintln!("{}", bench::format_row(song))) {
        Ok(r) => r,
        Err(e) => return bad_input(e),
    };
    eprintln!("{}", bench::format_budget(&report.budget));
    let failed: Vec<&str> = report
        .songs
        .iter()
        .filter(|s| !s.ok)
        .map(|s| s.name.as_str())
        .collect();
    if !failed.is_empty() {
        eprintln!("failed: {}", failed.join(", "));
    }
    let text = match serde_json::to_string_pretty(&report) {
        Ok(t) => t,
        Err(e) => return host_error(&HostError::Runtime(format!("serializing the report: {e}"))),
    };
    if let Err(e) = write_stdout(&format!("{text}\n")) {
        return bad_input(e);
    }
    if report.all_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
