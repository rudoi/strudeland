//! `Host`: a dedicated worker thread that serves requests one at a time, each in a fresh JS
//! V8 isolate (isolates are not `Send`, so they never leave the thread).

use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::engine::{self, Arg, Session};
use crate::{HostError, Layout, PlannedRender, RenderPlanRequest, RenderRequest, natives};

/// The worker thread's stack: deep enough for the transpiler on large songs.
const WORKER_STACK: usize = 64 << 20;

/// A planned job: the job's JSON text (ASCII, byte-equal to the render page's) and its
/// `PageJobInfo` as the bundle's `JSON.stringify` wrote it (one line; not byte-compared with the
/// page's: the page and the host capture errors differently). Parse `info` where it is read.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedJob {
    pub json: String,
    pub info: String,
}

/// Where one `plan_job`'s time went, measured on the worker (for `bench`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanTimings {
    /// The new isolate and context, the natives and the bundle's evaluation.
    pub load: Duration,
    /// The bundle's phase marks (`evaluating`, `querying`, `settled`) with their instants;
    /// empty unless profiling.
    pub marks: Vec<(String, Instant)>,
    /// The result read-out.
    pub json: Duration,
}

type Reply<T> = Sender<Result<T, HostError>>;

/// One request for the worker, with the channel its typed reply goes back on.
enum Work {
    Plan {
        code: String,
        request_json: String,
        profile: bool,
        timeout: Option<Duration>,
        reply: Reply<(PlannedJob, PlanTimings)>,
    },
    PlanRender {
        code: String,
        request_json: String,
        timeout: Option<Duration>,
        reply: Reply<PlannedRender>,
    },
    Query {
        code: String,
        from: f64,
        to: f64,
        layout: Layout,
        reply: Reply<String>,
    },
    Eval {
        source: String,
        reply: Reply<String>,
    },
}

/// The headless host. `Send + Sync`; calls block and are served one at a time per `Host`;
/// two `Host`s run in parallel. Dropping it stops and joins the worker.
pub struct Host {
    sender: Mutex<Option<Sender<Work>>>,
    worker: Option<JoinHandle<()>>,
}

impl Host {
    /// Starts the worker thread. Fails with [`HostError::Runtime`] when the thread cannot be
    /// spawned.
    pub fn new() -> Result<Host, HostError> {
        let (sender, inbox) = channel::<Work>();
        let worker = std::thread::Builder::new()
            .name("strudeland-host".into())
            .stack_size(WORKER_STACK)
            .spawn(move || {
                while let Ok(work) = inbox.recv() {
                    serve(work);
                }
            })
            .map_err(|e| HostError::Runtime(format!("starting the host thread: {e}")))?;
        Ok(Host {
            sender: Mutex::new(Some(sender)),
            worker: Some(worker),
        })
    }

    /// Plans `request`'s engine job for the song `code`.
    pub fn plan_job(&self, code: &str, request: &RenderRequest) -> Result<PlannedJob, HostError> {
        self.plan_job_with_timeout(code, request, None)
    }

    /// [`Host::plan_job`] that gives up with [`HostError::Timeout`] when the planning takes longer
    /// than `timeout` (`None`: no limit). The same `Host` serves the next request afterwards.
    pub fn plan_job_with_timeout(
        &self,
        code: &str,
        request: &RenderRequest,
        timeout: Option<Duration>,
    ) -> Result<PlannedJob, HostError> {
        self.plan_job_timed(code, request, false, timeout)
            .map(|(job, _)| job)
    }

    /// [`Host::plan_job`] with the worker's timings; `profile` turns the bundle's phase marks on.
    pub(crate) fn plan_job_timed(
        &self,
        code: &str,
        request: &RenderRequest,
        profile: bool,
        timeout: Option<Duration>,
    ) -> Result<(PlannedJob, PlanTimings), HostError> {
        let request_json = request
            .validate()
            .and_then(|()| request.to_json())
            .map_err(|e| HostError::Plan(format!("invalid request: {e}")))?;
        self.send(|reply| Work::Plan {
            code: code.to_string(),
            request_json,
            profile,
            timeout,
            reply,
        })
    }

    /// Plans every chunk of a render of the song `code` in one call: the bundle resolves the
    /// length request against the song's `meta`, evaluates the song once and plans each chunk's
    /// engine job. Standard jobs match [`Host::plan_job`] byte for byte. Loop exports can also
    /// plan an isolated repeat from the same selected body events. `request.timeout_sec` bounds
    /// the whole call ([`HostError::Timeout`]). An invalid request is [`HostError::Plan`]
    /// `invalid request: …`; a length request the song cannot
    /// satisfy is [`HostError::MetaFlag`] naming the CLI flag.
    pub fn plan_render(
        &self,
        code: &str,
        request: &RenderPlanRequest,
    ) -> Result<PlannedRender, HostError> {
        let request_json = request
            .validate()
            .and_then(|()| request.to_json())
            .map_err(|e| HostError::Plan(format!("invalid request: {e}")))?;
        let timeout = request.timeout();
        self.send(|reply| Work::PlanRender {
            code: code.to_string(),
            request_json,
            timeout,
            reply,
        })
    }

    /// The song's events (`RenderEvent[]` JSON) with onsets in `[from, to)` layout cycles.
    /// The bundle rejects a window that is not finite, empty or wider than 4096 cycles
    /// ([`HostError::Plan`] `bad window`).
    pub fn query(
        &self,
        code: &str,
        from: f64,
        to: f64,
        layout: Layout,
    ) -> Result<String, HostError> {
        self.send(|reply| Work::Query {
            code: code.to_string(),
            from,
            to,
            layout,
            reply,
        })
    }

    /// Diagnostic hook for tests: a fresh context with the natives installed and **no**
    /// bundle; evaluates `source` as a script and returns `String(completion value)`. A thrown
    /// exception is [`HostError::Runtime`].
    pub fn eval_script(&self, source: &str) -> Result<String, HostError> {
        self.send(|reply| Work::Eval {
            source: source.to_string(),
            reply,
        })
    }

    /// Sends the work `make` builds around a fresh reply channel and waits for the reply.
    fn send<T>(&self, make: impl FnOnce(Reply<T>) -> Work) -> Result<T, HostError> {
        let (reply, answer) = channel();
        let work = make(reply);
        {
            let guard = self
                .sender
                .lock()
                .map_err(|_| HostError::Runtime("the host's channel is poisoned".into()))?;
            let sender = guard
                .as_ref()
                .ok_or_else(|| HostError::Runtime("the host is stopped".into()))?;
            sender
                .send(work)
                .map_err(|_| HostError::Runtime("the host's worker thread has stopped".into()))?;
        }
        answer
            .recv()
            .map_err(|_| HostError::Runtime("the host's worker thread has stopped".into()))?
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.sender.lock() {
            guard.take();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The bundle's result slot (`globalThis.__strudelandResult`): `{ ok: true, value }` or
/// `{ ok: false, kind, error, flag? }` (`flag` names the CLI flag of a length error).
#[derive(Deserialize)]
struct Slot {
    ok: bool,
    value: Option<String>,
    kind: Option<String>,
    error: Option<String>,
    flag: Option<String>,
}

/// Reads the result slot after the drain: its value, or the error its `kind` names.
fn read_slot(text: &str) -> Result<String, HostError> {
    if text == "null" || text == "undefined" {
        return Err(HostError::Unsettled);
    }
    let slot: Slot = serde_json::from_str(text)
        .map_err(|e| HostError::Runtime(format!("malformed result slot: {e}")))?;
    match slot {
        Slot {
            ok: true,
            value: Some(value),
            ..
        } => Ok(value),
        Slot {
            ok: false,
            kind: Some(kind),
            error: Some(error),
            flag,
            ..
        } => Err(match (kind.as_str(), flag) {
            ("meta", Some(flag)) => HostError::MetaFlag {
                flag,
                message: error,
            },
            ("meta", None) => HostError::Meta(error),
            ("eval", _) => HostError::Eval(error),
            ("plan", _) => HostError::Plan(error),
            (other, _) => HostError::Runtime(format!("unknown result kind {other}: {error}")),
        }),
        Slot { ok, .. } => Err(HostError::Runtime(format!(
            "malformed result slot: ok {ok} without its fields"
        ))),
    }
}

const READ_SLOT: &str = "(() => { const r = globalThis.__strudelandResult; return r === undefined ? 'null' : JSON.stringify(r); })()";

/// Serves one request in a fresh isolate and context and sends the reply (a dropped receiver
/// is ignored).
fn serve(work: Work) {
    let start = Instant::now();
    let log = natives::log_enabled();
    match work {
        Work::Plan {
            code,
            request_json,
            profile,
            timeout,
            reply,
        } => {
            let result = engine::with_context(log, profile, timeout, |engine| {
                let load = load_bundle(engine, start)?;
                engine.call_global(
                    "strudelandHost",
                    "planJob",
                    &[Arg::Str(&code), Arg::Str(&request_json)],
                )?;
                engine.drain_microtasks()?;
                let drained = Instant::now();
                let info = read_slot(&engine.eval_script(READ_SLOT)?)?;
                let json = engine
                    .read_global_string("__strudelandJob")?
                    .ok_or_else(|| HostError::Runtime("planJob settled without a job".into()))?;
                let timings = PlanTimings {
                    load,
                    marks: engine.take_marks(),
                    json: drained.elapsed(),
                };
                Ok((PlannedJob { json, info }, timings))
            });
            let _ = reply.send(result);
        }
        Work::PlanRender {
            code,
            request_json,
            timeout,
            reply,
        } => {
            let result = engine::with_context(log, false, timeout, |engine| {
                load_bundle(engine, start)?;
                engine.call_global(
                    "strudelandHost",
                    "planRender",
                    &[Arg::Str(&code), Arg::Str(&request_json)],
                )?;
                engine.drain_microtasks()?;
                // A failure is in the slot; its value is unused on success.
                read_slot(&engine.eval_script(READ_SLOT)?)?;
                let text = engine
                    .read_global_string("__strudelandRender")?
                    .ok_or_else(|| {
                        HostError::Runtime("planRender settled without a plan".into())
                    })?;
                serde_json::from_str::<PlannedRender>(&text)
                    .map_err(|e| HostError::Runtime(format!("malformed plan: {e}")))
            });
            let _ = reply.send(result);
        }
        Work::Query {
            code,
            from,
            to,
            layout,
            reply,
        } => {
            let result = engine::with_context(log, false, None, |engine| {
                load_bundle(engine, start)?;
                engine.call_global(
                    "strudelandHost",
                    "query",
                    &[
                        Arg::Str(&code),
                        Arg::Num(from),
                        Arg::Num(to),
                        Arg::Str(layout.as_str()),
                    ],
                )?;
                engine.drain_microtasks()?;
                read_slot(&engine.eval_script(READ_SLOT)?)
            });
            let _ = reply.send(result);
        }
        Work::Eval { source, reply } => {
            let result =
                engine::with_context(log, false, None, |engine| engine.eval_script(&source));
            let _ = reply.send(result);
        }
    }
}

/// Evaluates the bundle; the time since `start` (the isolate, context, natives and bundle).
fn load_bundle(engine: &mut Session<'_, '_, '_>, start: Instant) -> Result<Duration, HostError> {
    engine
        .eval_script(crate::bundle::BUNDLE)
        .map_err(|e| match e {
            // The deadline can pass while the bundle loads; that is still the request's timeout.
            HostError::Timeout { .. } => e,
            other => HostError::Runtime(format!("loading the bundle: {other}")),
        })?;
    Ok(start.elapsed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slot_maps_to_results_and_kinds() {
        assert_eq!(read_slot(r#"{"ok":true,"value":"v"}"#), Ok("v".into()));
        assert_eq!(
            read_slot(r#"{"ok":false,"kind":"meta","error":"m"}"#),
            Err(HostError::Meta("m".into()))
        );
        assert_eq!(
            read_slot(r#"{"ok":false,"kind":"eval","error":"e"}"#),
            Err(HostError::Eval("e".into()))
        );
        assert_eq!(
            read_slot(r#"{"ok":false,"kind":"plan","error":"p"}"#),
            Err(HostError::Plan("p".into()))
        );
        assert_eq!(
            read_slot(r#"{"ok":false,"kind":"meta","error":"m","flag":"--length"}"#),
            Err(HostError::MetaFlag {
                flag: "--length".into(),
                message: "m".into()
            })
        );
        // Only `meta` carries a flag; on another kind it is ignored.
        assert_eq!(
            read_slot(r#"{"ok":false,"kind":"plan","error":"p","flag":"--length"}"#),
            Err(HostError::Plan("p".into()))
        );
        assert_eq!(
            read_slot(r#"{"ok":false,"kind":"meta","error":"m","flag":null}"#),
            Err(HostError::Meta("m".into()))
        );
        assert_eq!(read_slot("null"), Err(HostError::Unsettled));
        assert_eq!(read_slot("undefined"), Err(HostError::Unsettled));
    }

    #[test]
    fn a_malformed_slot_is_a_runtime_error() {
        for text in [
            r#"{"ok":false,"kind":"other","error":"x"}"#,
            r#"{"ok":true}"#,
            r#"{"ok":false,"value":"v"}"#,
            r#"{"ok":"yes","value":"v"}"#,
            r#"{"ok":true,"value":3}"#,
            r#"{"ok":false,"kind":"meta","error":"m","flag":3}"#,
            "3",
            "",
        ] {
            assert!(
                matches!(read_slot(text), Err(HostError::Runtime(_))),
                "{text}: {:?}",
                read_slot(text)
            );
        }
    }

    #[test]
    fn host_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Host>();
    }
}
