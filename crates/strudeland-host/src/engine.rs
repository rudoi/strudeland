//! The embedded V8 (the raw `v8` crate): the platform is initialised once per process, and every
//! request gets its own isolate and context ([`with_context`]), dropped when the request ends.
//!
//! Nothing here may panic (release builds abort on panic): every fallible V8 call is mapped to a
//! [`HostError::Runtime`].

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Once, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::HostError;
use crate::natives;

/// V8's stack limit in KiB, below the worker thread's 64 MiB stack (`host.rs`), so that deep
/// recursion in a song or in the transpiler throws a `RangeError` instead of overflowing.
const STACK_KIB: usize = 48 << 10;

/// An argument to [`Session::call_global`].
#[derive(Debug, Clone, Copy)]
pub(crate) enum Arg<'a> {
    Str(&'a str),
    Num(f64),
}

static PLATFORM: Once = Once::new();

/// Initialises V8 once per process (flags, the default platform).
fn init_platform() {
    PLATFORM.call_once(|| {
        v8::V8::set_flags_from_string(&format!("--stack-size={STACK_KIB}"));
        // The hardened macOS sidecar cannot allocate executable memory for JIT code.
        if cfg!(feature = "jitless") {
            v8::V8::set_flags_from_string("--jitless");
        }
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}

/// Turns a heap about to run out into a terminated execution (an error) instead of V8's fatal
/// out-of-memory abort, and records a termination by the planning watchdog.
struct HeapGuard {
    handle: v8::IsolateHandle,
    hit: AtomicBool,
    /// Set by the watchdog before it terminates the execution.
    timed_out: Arc<AtomicBool>,
    /// The watchdog's deadline in whole milliseconds (rounded up), for [`HostError::Timeout`].
    timeout_millis: u64,
}

impl HeapGuard {
    /// The error a terminated execution stands for: the watchdog's timeout (checked first), the
    /// heap limit, or `None` when neither terminated it.
    fn termination(&self) -> Option<HostError> {
        if self.timed_out.load(Ordering::SeqCst) {
            return Some(HostError::Timeout {
                millis: self.timeout_millis,
            });
        }
        self.hit.load(Ordering::SeqCst).then(|| {
            HostError::Runtime("the JS heap ran out of memory; execution was terminated".into())
        })
    }
}

/// The planning watchdog: a thread that terminates the isolate's execution at the deadline
/// unless it is cancelled first. [`Watchdog::stop`] cancels and joins it.
struct Watchdog {
    cancel: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Watchdog {
    fn start(
        handle: v8::IsolateHandle,
        timed_out: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<Watchdog, HostError> {
        let cancel = Arc::new((Mutex::new(false), Condvar::new()));
        let flag = Arc::clone(&cancel);
        let thread = std::thread::Builder::new()
            .name("strudeland-host-watchdog".into())
            .spawn(move || {
                let (lock, condvar) = &*flag;
                let cancelled = lock.lock().unwrap_or_else(PoisonError::into_inner);
                // Measured from the wait's start, so a timeout past the clock's range never comes.
                let (_cancelled, wait) = condvar
                    .wait_timeout_while(cancelled, timeout, |cancelled| !*cancelled)
                    .unwrap_or_else(PoisonError::into_inner);
                if wait.timed_out() {
                    timed_out.store(true, Ordering::SeqCst);
                    handle.terminate_execution();
                }
            })
            .map_err(|e| HostError::Runtime(format!("starting the planning watchdog: {e}")))?;
        Ok(Watchdog {
            cancel,
            thread: Some(thread),
        })
    }

    /// Cancels the watchdog and joins its thread (idempotent).
    fn stop(&mut self) {
        let (lock, condvar) = &*self.cancel;
        *lock.lock().unwrap_or_else(PoisonError::into_inner) = true;
        condvar.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `timeout` in whole milliseconds, rounded up (saturating).
fn millis_ceil(timeout: Duration) -> u64 {
    u64::try_from(timeout.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

extern "C" fn near_heap_limit(data: *mut c_void, current: usize, _initial: usize) -> usize {
    // SAFETY: `data` is the `HeapGuard` boxed in `with_context`, which outlives the isolate.
    let guard = unsafe { &*(data as *const HeapGuard) };
    guard.hit.store(true, Ordering::SeqCst);
    guard.handle.terminate_execution();
    // Room for the terminated script to unwind.
    current.saturating_mul(2)
}

/// One request's isolate and context, entered.
pub(crate) struct Session<'a, 's, 'i> {
    scope: &'a mut v8::PinScope<'s, 'i>,
    heap: &'a HeapGuard,
    text_limit: Option<usize>,
    error_limit: Option<usize>,
}

/// Runs `body` in a fresh isolate and context with the natives installed; `log` lets
/// `console.warn`/`console.error` through to stderr, `profile` turns the phase marks on, and
/// `timeout` (when given) starts a watchdog that terminates the execution at the deadline, which
/// the session reports as [`HostError::Timeout`]. The watchdog is cancelled and joined before the
/// isolate drops, whatever the body returned.
pub(crate) fn with_context<R>(
    log: bool,
    profile: bool,
    timeout: Option<Duration>,
    body: impl FnOnce(&mut Session<'_, '_, '_>) -> Result<R, HostError>,
) -> Result<R, HostError> {
    init_platform();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let timed_out = Arc::new(AtomicBool::new(false));
    let heap = Box::new(HeapGuard {
        handle: isolate.thread_safe_handle(),
        hit: AtomicBool::new(false),
        timed_out: Arc::clone(&timed_out),
        timeout_millis: timeout.map_or(0, millis_ceil),
    });
    let heap_ptr: *const HeapGuard = &*heap;
    isolate.add_near_heap_limit_callback(near_heap_limit, heap_ptr as *mut c_void);
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    natives::prepare_isolate(&mut isolate, profile);
    let mut watchdog = match timeout {
        Some(t) => match Watchdog::start(isolate.thread_safe_handle(), timed_out, t) {
            Ok(w) => Some(w),
            Err(e) => {
                drop(isolate);
                drop(heap);
                return Err(e);
            }
        },
        None => None,
    };
    let result = run_in_context(&mut isolate, &heap, log, body);
    // The watchdog stops before the isolate drops: its handle must not outlive it. A termination
    // that lands after the body returned is harmless (the isolate is dropped next).
    if let Some(w) = watchdog.as_mut() {
        w.stop();
    }
    drop(watchdog);
    // The isolate goes first: the heap callback points at `heap`.
    drop(isolate);
    drop(heap);
    result
}

fn run_in_context<R>(
    isolate: &mut v8::OwnedIsolate,
    heap: &HeapGuard,
    log: bool,
    body: impl FnOnce(&mut Session<'_, '_, '_>) -> Result<R, HostError>,
) -> Result<R, HostError> {
    v8::scope!(let handle_scope, isolate);
    let context = v8::Context::new(handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(handle_scope, context);
    natives::install(scope, log)?;
    let mut session = Session {
        scope,
        heap,
        text_limit: None,
        error_limit: None,
    };
    body(&mut session)
}

impl Session<'_, '_, '_> {
    /// Bound the requested strings and any exception text before making Rust-owned copies.
    pub(crate) fn limit_text(&mut self, result_bytes: usize, error_bytes: usize) {
        self.text_limit = Some(result_bytes);
        self.error_limit = Some(error_bytes);
    }

    fn terminated(&self) -> Option<HostError> {
        self.heap.termination()
    }

    /// Evaluates `source` as a classic script; returns `String(completion value)`.
    pub(crate) fn eval_script(&mut self, source: &str) -> Result<String, HostError> {
        let heap = self.heap;
        let scope = &mut *self.scope;
        v8::tc_scope!(let tc, scope);
        let code = v8::String::new(tc, source)
            .ok_or_else(|| HostError::Runtime("the script is too long for a JS string".into()))?;
        let result = v8::Script::compile(tc, code, None)
            .and_then(|script| script.run(tc))
            .and_then(|value| value.to_string(tc));
        match result {
            Some(text) => copy_text(tc, text, self.text_limit),
            None => Err(caught(tc, heap, self.error_limit)),
        }
    }

    /// Calls `globalThis[object][method](...args)` with `this` the object, ignoring its result.
    pub(crate) fn call_global(
        &mut self,
        object: &str,
        method: &str,
        args: &[Arg<'_>],
    ) -> Result<(), HostError> {
        let heap = self.heap;
        let scope = &mut *self.scope;
        v8::tc_scope!(let tc, scope);
        let context = tc.get_current_context();
        let global = context.global(tc);
        let target = key(tc, object)
            .and_then(|k| global.get(tc, k))
            .and_then(|v| v.to_object(tc));
        let Some(target) = target else {
            return Err(if tc.has_caught() {
                caught(tc, heap, self.error_limit)
            } else {
                HostError::Runtime(format!("globalThis.{object} is not an object"))
            });
        };
        let func = key(tc, method)
            .and_then(|k| target.get(tc, k))
            .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok());
        let Some(func) = func else {
            return Err(if tc.has_caught() {
                caught(tc, heap, self.error_limit)
            } else {
                HostError::Runtime(format!("{object}.{method} is not a function"))
            });
        };
        let mut values: Vec<v8::Local<v8::Value>> = Vec::with_capacity(args.len());
        for arg in args {
            values.push(match *arg {
                Arg::Str(s) => v8::String::new(tc, s)
                    .ok_or_else(|| {
                        HostError::Runtime("an argument is too long for a JS string".into())
                    })?
                    .into(),
                Arg::Num(x) => v8::Number::new(tc, x).into(),
            });
        }
        match func.call(tc, target.into(), &values) {
            Some(_) => Ok(()),
            None => Err(caught(tc, heap, self.error_limit)),
        }
    }

    /// Runs pending promise jobs until none is left (timers never fire). V8 swallows exceptions
    /// thrown by microtasks; a promise they reject stays rejected.
    pub(crate) fn drain_microtasks(&mut self) -> Result<(), HostError> {
        self.scope.perform_microtask_checkpoint();
        match self.terminated() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// The phase marks recorded so far (empty unless profiling), taken out of the isolate.
    pub(crate) fn take_marks(&mut self) -> Vec<(String, std::time::Instant)> {
        match self.scope.get_slot_mut::<natives::Marks>() {
            Some(natives::Marks(Some(marks))) => std::mem::take(marks),
            _ => Vec::new(),
        }
    }

    /// `globalThis[name]` when it is a string, else `None`.
    pub(crate) fn read_global_string(&mut self, name: &str) -> Result<Option<String>, HostError> {
        let heap = self.heap;
        let scope = &mut *self.scope;
        v8::tc_scope!(let tc, scope);
        let context = tc.get_current_context();
        let global = context.global(tc);
        match key(tc, name).and_then(|k| global.get(tc, k)) {
            Some(value) => match v8::Local::<v8::String>::try_from(value) {
                Ok(text) => copy_text(tc, text, self.text_limit).map(Some),
                Err(_) => Ok(None),
            },
            None => Err(caught(tc, heap, self.error_limit)),
        }
    }
}

/// The embedded V8's version (`152.2.0`'s V8, e.g. `14.x.y.z`).
pub fn v8_version() -> &'static str {
    v8::V8::get_version()
}

/// This process's peak resident set size in bytes (0 when unavailable).
#[cfg(unix)]
pub(crate) fn max_rss_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage writes into the valid, zeroed buffer.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return 0;
    }
    // SAFETY: getrusage succeeded and initialized the result.
    let usage = unsafe { usage.assume_init() };
    crate::bench::rss_bytes(usage.ru_maxrss, cfg!(target_os = "macos"))
}

/// This process's peak working set in bytes (0 when unavailable).
#[cfg(windows)]
pub(crate) fn max_rss_bytes() -> u64 {
    use windows_sys::Win32::System::{
        ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };
    let size = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size,
        ..Default::default()
    };
    // SAFETY: GetCurrentProcess is a valid pseudo-handle and counters is writable for size bytes.
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) } == 0 {
        return 0;
    }
    counters.PeakWorkingSetSize as u64
}

/// A property key, or `None` if the name cannot become a JS string.
fn key<'s>(scope: &v8::PinScope<'s, '_>, name: &str) -> Option<v8::Local<'s, v8::Value>> {
    v8::String::new(scope, name).map(Into::into)
}

/// Check UTF-8 byte size before allocating Rust storage. The UTF-16 length is a cheap
/// early rejection for large strings and leaves the UTF-8 length pass bounded.
fn copy_text(
    scope: &v8::Isolate,
    text: v8::Local<'_, v8::String>,
    limit: Option<usize>,
) -> Result<String, HostError> {
    if limit.is_some_and(|limit| text.length() > limit || text.utf8_length(scope) > limit) {
        return Err(HostError::Runtime(
            "JavaScript text exceeded the inspection size limit".into(),
        ));
    }
    Ok(text.to_rust_string_lossy(scope))
}

/// The error for a failed V8 call: the termination (the watchdog's timeout, then the heap
/// limit), or the caught exception as text (`String(e)`, so an `Error` gives `Name: message`).
fn caught(
    tc: &mut v8::PinnedRef<'_, v8::TryCatch<v8::HandleScope>>,
    heap: &HeapGuard,
    limit: Option<usize>,
) -> HostError {
    if let Some(e) = heap.termination() {
        return e;
    }
    if tc.has_terminated() {
        return HostError::Runtime("execution was terminated".into());
    }
    let Some(exception) = tc.exception() else {
        return HostError::Runtime("a JS call failed without an exception".into());
    };
    tc.reset();
    v8::tc_scope!(let inner, tc);
    match exception.to_string(inner) {
        Some(text) => match copy_text(inner, text, limit) {
            Ok(message) => HostError::Runtime(message),
            Err(error) => error,
        },
        None => HostError::Runtime("an exception that cannot be converted to a string".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn millis_round_up() {
        assert_eq!(millis_ceil(Duration::from_millis(500)), 500);
        assert_eq!(millis_ceil(Duration::from_nanos(1)), 1);
        assert_eq!(millis_ceil(Duration::from_nanos(1_000_001)), 2);
        assert_eq!(millis_ceil(Duration::ZERO), 0);
        assert_eq!(millis_ceil(Duration::from_secs(86_400)), 86_400_000);
        assert_eq!(millis_ceil(Duration::MAX), u64::MAX);
    }

    #[test]
    fn readout_bounds_completion_global_and_exception_bytes() {
        let result = with_context(false, false, None, |session| {
            session.limit_text(16, 16);
            let too_large =
                HostError::Runtime("JavaScript text exceeded the inspection size limit".into());
            // A code-unit length check alone would accept this string.
            assert_eq!(
                session.eval_script("'é'.repeat(12)"),
                Err(too_large.clone())
            );
            session.eval_script("globalThis.testOut = 'é'.repeat(12); null")?;
            assert_eq!(
                session.read_global_string("testOut"),
                Err(too_large.clone())
            );
            assert_eq!(session.eval_script("throw 'é'.repeat(12)"), Err(too_large));
            assert_eq!(session.eval_script("'é'.repeat(8)")?, "é".repeat(8));
            Ok(())
        });
        assert_eq!(result, Ok(()));
    }
}
