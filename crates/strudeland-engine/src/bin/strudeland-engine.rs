//! `strudeland-engine`: the render harness's entry to the engine.
//!
//! ```text
//! strudeland-engine render --job <file|-> --out <file.wav>
//! strudeland-engine signatures --job <file|->
//! strudeland-engine controls --json
//! ```
//!
//! `render` exit codes: 0 rendered without errors; 1 rendered with render errors (summary on
//! stdout, WAV written); 2 bad input or I/O (message on stderr, no WAV). `signatures` prints the
//! job's unsupported signatures (`[{"s":…,"keys":[…],"firstBar":…}]`, one line, `[]` when the
//! engine plays everything) and exits 0 for any valid job, 2 for bad input or I/O.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use strudeland_engine::controls::unsupported_signatures;
use strudeland_engine::{EngineJob, SAMPLE_RATE, controls_json, render, wav};

const USAGE: &str = "usage: strudeland-engine render --job <file|-> --out <file.wav>\n       strudeland-engine signatures --job <file|->\n       strudeland-engine controls --json";

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("strudeland-engine: {message}");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("render") => match parse_flags(&args[1..], &["--job", "--out"]) {
            Ok(v) => run_render(&v[0], &v[1]),
            Err(e) => fail(format!("{e}\n{USAGE}")),
        },
        Some("signatures") => match parse_flags(&args[1..], &["--job"]) {
            Ok(v) => run_signatures(&v[0]),
            Err(e) => fail(format!("{e}\n{USAGE}")),
        },
        Some("controls") if args[1..] == ["--json"] => {
            println!("{}", controls_json());
            ExitCode::SUCCESS
        }
        _ => fail(USAGE),
    }
}

/// The values of `flags` (each required, each taking one value, in any order), returned in the
/// order of `flags`.
fn parse_flags(args: &[String], flags: &[&str]) -> Result<Vec<String>, String> {
    let mut values: Vec<Option<String>> = vec![None; flags.len()];
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let Some(i) = flags.iter().position(|f| f == flag) else {
            return Err(format!("unknown argument {flag}"));
        };
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        if values[i].replace(value.clone()).is_some() {
            return Err(format!("{flag} given twice"));
        }
    }
    flags
        .iter()
        .zip(values)
        .map(|(f, v)| v.ok_or_else(|| format!("missing {f}")))
        .collect()
}

fn read_job(path: &str) -> Result<String, String> {
    let mut text = String::new();
    if path == "-" {
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|e| format!("reading stdin: {e}"))?;
    } else {
        File::open(path)
            .and_then(|mut f| f.read_to_string(&mut text))
            .map_err(|e| format!("reading {path}: {e}"))?;
    }
    Ok(text)
}

fn run_signatures(job_path: &str) -> ExitCode {
    let text = match read_job(job_path) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let job = match EngineJob::from_json(&text) {
        Ok(j) => j,
        Err(e) => return fail(format!("invalid job:\n{e}")),
    };
    match serde_json::to_string(&unsupported_signatures(&job)) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(e) => fail(e),
    }
}

fn run_render(job_path: &str, out_path: &str) -> ExitCode {
    let text = match read_job(job_path) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let job = match EngineJob::from_json(&text) {
        Ok(j) => j,
        Err(e) => return fail(format!("invalid job:\n{e}")),
    };
    let out = Path::new(out_path);
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !parent.is_dir() {
        return fail(format!(
            "output directory {} does not exist",
            parent.display()
        ));
    }
    let frames = usize::try_from(job.frames).unwrap_or(usize::MAX);
    if wav::data_size(job.channels.count as usize, frames).is_none() {
        return fail("the WAV would be larger than 4 GiB");
    }
    let started = Instant::now();
    let rendered = match render(&job) {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    let render_ms = started.elapsed().as_millis();
    let written = File::create(out)
        .map_err(wav::WavError::from)
        .and_then(|f| {
            let mut w = BufWriter::new(f);
            wav::write_f32(&mut w, &rendered.channels, SAMPLE_RATE)?;
            w.flush()?;
            Ok(())
        });
    if let Err(e) = written {
        return fail(format!("writing {out_path}: {e}"));
    }
    let summary = serde_json::json!({
        "ok": rendered.errors.is_empty(),
        "frames": job.frames,
        "channels": job.channels.count,
        "errors": rendered.errors,
        "renderMs": u64::try_from(render_ms).unwrap_or(u64::MAX),
    });
    println!("{summary}");
    if rendered.errors.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
