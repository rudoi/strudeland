//! The globals the host installs before `host.js` runs. See the determinism policy in the host
//! README. These include `Math` overrides from `jsmath`, a throwing
//! `Math.random`, zero clocks (`Date.now`, `performance.now` and the no-argument `Date` forms),
//! inert timers, console capture, and the browser globals removed:
//! the lists and their V8 installer.
//!
//! No native may panic (release builds abort on panic): they convert their arguments with V8's
//! `ToNumber`/`ToString`, so a bad argument throws a JS exception instead.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use strudeland_engine::jsmath;

use crate::HostError;

/// The message of the error `Math.random()` throws.
pub const RANDOM_MESSAGE: &str = "Math.random is disabled in the strudeland host: patterns must be deterministic (use rand, irand or perlin)";

/// Replaces `Date` so its no-argument forms read the frozen clock: `new Date()` is the epoch (as
/// `Date.now()` is 0) and `Date()` is `new Date(0).toString()`; with arguments `new Date(...)`
/// constructs as the original does. `Date.prototype`, `instanceof`, subclassing (`new.target`),
/// `Date.parse`, `Date.UTC` and the zero `Date.now` are kept; `name` is `Date`, `length` 7.
pub(crate) const DATE_PRELUDE: &str = r#"(() => {
  'use strict';
  const NativeDate = globalThis.Date;
  const Date = function Date(...args) {
    if (new.target === undefined) return new NativeDate(0).toString();
    return Reflect.construct(NativeDate, args.length === 0 ? [0] : args, new.target);
  };
  const hidden = (value) => ({ value, writable: true, enumerable: false, configurable: true });
  Object.defineProperties(Date, {
    length: { value: 7, writable: false, enumerable: false, configurable: true },
    prototype: { value: NativeDate.prototype, writable: false, enumerable: false, configurable: false },
    now: hidden(NativeDate.now),
    parse: hidden(NativeDate.parse),
    UTC: hidden(NativeDate.UTC),
  });
  Object.defineProperty(NativeDate.prototype, 'constructor', hidden(Date));
  Object.defineProperty(globalThis, 'Date', hidden(Date));
})();"#;

/// The environment variable that lets `console.warn`/`console.error` through to stderr.
pub const LOG_ENV: &str = "STRUDELAND_HOST_LOG";

/// A one-argument `Math` function.
pub type Unary = fn(f64) -> f64;

/// A two-argument `Math` function.
pub type Binary = fn(f64, f64) -> f64;

/// One-argument `Math` functions replaced by `jsmath` (name, implementation).
pub const UNARY_OVERRIDES: [(&str, Unary); 7] = [
    ("sin", jsmath::sin),
    ("cos", jsmath::cos),
    ("tanh", jsmath::tanh),
    ("exp", jsmath::exp),
    ("expm1", jsmath::expm1),
    ("log1p", jsmath::log1p),
    ("cosh", jsmath::cosh),
];

/// Two-argument `Math` functions replaced by `jsmath`.
pub const BINARY_OVERRIDES: [(&str, Binary); 1] = [("pow", jsmath::pow)];

/// Console methods whose output is always discarded: every method of the WHATWG Console
/// standard but `warn` and `error`, plus V8's `profile`, `profileEnd` and `timeStamp`, so bundled
/// code calling any of them (core's `Hap` constructor calls `console.assert`) runs as in Chromium.
pub(crate) const CONSOLE_DISCARDED: [&str; 20] = [
    "log",
    "info",
    "debug",
    "trace",
    "dir",
    "dirxml",
    "assert",
    "clear",
    "count",
    "countReset",
    "group",
    "groupCollapsed",
    "groupEnd",
    "table",
    "time",
    "timeEnd",
    "timeLog",
    "profile",
    "profileEnd",
    "timeStamp",
];

/// Console methods written to stderr when [`LOG_ENV`] is `1`, else discarded.
pub(crate) const CONSOLE_LOGGED: [&str; 2] = ["warn", "error"];

/// Timer functions: each returns the next integer id and never calls its callback.
pub(crate) const TIMERS_SET: [&str; 2] = ["setTimeout", "setInterval"];

/// Timer cancellers: accept anything, do nothing.
pub(crate) const TIMERS_CLEAR: [&str; 2] = ["clearTimeout", "clearInterval"];

/// Globals that must not exist in the host (removed if V8 defines them).
pub const ABSENT: [&str; 9] = [
    "window",
    "document",
    "navigator",
    "fetch",
    "AudioContext",
    "localStorage",
    "requestAnimationFrame",
    "queueMicrotask",
    "structuredClone",
];

/// Whether `console.warn`/`console.error` go to stderr (read once per request).
pub(crate) fn log_enabled() -> bool {
    std::env::var(LOG_ENV).is_ok_and(|v| v == "1")
}

/// Writes one console line to stderr, ignoring write failures (a closed stderr must not panic).
fn write_log(level: &str, text: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "console.{level}: {text}");
}

/// The global the bundle calls to mark a phase (`bench --profile`).
pub(crate) const MARK: &str = "__strudelandMark";

/// The per-isolate counter behind the timer ids (fresh with every request's isolate).
struct TimerIds(u32);

/// The phase marks of one request: `Some` when profiling (each label with the monotonic instant
/// it was marked at), `None` when the marks are no-ops.
pub(crate) struct Marks(pub(crate) Option<Vec<(String, std::time::Instant)>>);

/// Per-isolate state the natives need, set before the context is created; `profile` turns the
/// phase marks on.
pub(crate) fn prepare_isolate(isolate: &mut v8::Isolate, profile: bool) {
    isolate.set_slot(TimerIds(0));
    isolate.set_slot(Marks(profile.then(Vec::new)));
}

fn runtime(what: &str) -> HostError {
    HostError::Runtime(format!("installing the natives: {what}"))
}

fn js_string<'s>(
    scope: &v8::PinScope<'s, '_>,
    text: &str,
) -> Result<v8::Local<'s, v8::String>, HostError> {
    v8::String::new(scope, text).ok_or_else(|| runtime(text))
}

/// A native function with the given `name` and `length`; not a constructor, like the built-ins
/// it replaces.
fn function<'s>(
    scope: &v8::PinScope<'s, '_>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    name: &str,
    length: i32,
    data: Option<v8::Local<'s, v8::Value>>,
) -> Result<v8::Local<'s, v8::Function>, HostError> {
    let mut builder = v8::Function::builder(callback)
        .length(length)
        .constructor_behavior(v8::ConstructorBehavior::Throw);
    if let Some(data) = data {
        builder = builder.data(data);
    }
    let func = builder.build(scope).ok_or_else(|| runtime(name))?;
    func.set_name(js_string(scope, name)?);
    Ok(func)
}

fn set<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), HostError> {
    let key = js_string(scope, name)?;
    match object.set(scope, key.into(), value) {
        Some(true) => Ok(()),
        _ => Err(runtime(name)),
    }
}

fn get_object<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Object>, HostError> {
    let key = js_string(scope, name)?;
    object
        .get(scope, key.into())
        .and_then(|v| v.to_object(scope))
        .ok_or_else(|| runtime(name))
}

/// Installs every native on the current context's global object.
pub(crate) fn install(scope: &mut v8::PinScope<'_, '_>, log: bool) -> Result<(), HostError> {
    let global = scope.get_current_context().global(scope);

    let math = get_object(scope, global, "Math")?;
    install_overrides(scope, math, UNARY_OVERRIDES.map(|(name, _)| name), unary, 1)?;
    install_overrides(
        scope,
        math,
        BINARY_OVERRIDES.map(|(name, _)| name),
        binary,
        2,
    )?;
    let random = function(scope, random, "random", 0, None)?;
    set(scope, math, "random", random.into())?;

    // One zero clock for `Date.now` and `performance.now`.
    let now = function(scope, zero, "now", 0, None)?;
    let date = get_object(scope, global, "Date")?;
    set(scope, date, "now", now.into())?;
    let performance = v8::Object::new(scope);
    set(scope, performance, "now", now.into())?;
    set(scope, global, "performance", performance.into())?;
    // After the zero `Date.now`, which the replacement keeps.
    run_prelude(scope, DATE_PRELUDE, "Date")?;

    for name in TIMERS_SET {
        let func = function(scope, set_timer, name, 0, None)?;
        set(scope, global, name, func.into())?;
    }
    for name in TIMERS_CLEAR {
        let func = function(scope, nothing, name, 0, None)?;
        set(scope, global, name, func.into())?;
    }

    let console = v8::Object::new(scope);
    for name in CONSOLE_DISCARDED {
        let func = function(scope, nothing, name, 0, None)?;
        set(scope, console, name, func.into())?;
    }
    for name in CONSOLE_LOGGED {
        // The level as data when logging; without data the call is discarded.
        let data = if log {
            Some(js_string(scope, name)?.into())
        } else {
            None
        };
        let func = function(scope, console_logged, name, 0, data)?;
        set(scope, console, name, func.into())?;
    }
    set(scope, global, "console", console.into())?;

    let mark = function(scope, mark, MARK, 1, None)?;
    set(scope, global, MARK, mark.into())?;

    for name in ABSENT {
        let key = js_string(scope, name)?;
        if global.delete(scope, key.into()) != Some(true) {
            return Err(runtime(name));
        }
    }
    Ok(())
}

/// Runs `source`, a trusted script of the host's own, in the current context.
fn run_prelude(
    scope: &mut v8::PinScope<'_, '_>,
    source: &str,
    what: &str,
) -> Result<(), HostError> {
    v8::tc_scope!(let tc, scope);
    let code = js_string(tc, source)?;
    v8::Script::compile(tc, code, None)
        .and_then(|script| script.run(tc))
        .map(|_| ())
        .ok_or_else(|| runtime(what))
}

/// Installs `callback` on `math` under each of `names`, with its index in the table (in order)
/// as the function's data.
fn install_overrides<'s>(
    scope: &v8::PinScope<'s, '_>,
    math: v8::Local<'s, v8::Object>,
    names: impl IntoIterator<Item = &'static str>,
    callback: impl v8::MapFnTo<v8::FunctionCallback> + Copy,
    length: i32,
) -> Result<(), HostError> {
    for (index, name) in (0_i32..).zip(names) {
        let data = v8::Integer::new(scope, index).into();
        let func = function(scope, callback, name, length, Some(data))?;
        set(scope, math, name, func.into())?;
    }
    Ok(())
}

/// `ToNumber(arguments[i])`, `NaN` when missing; `None` when the conversion threw (the
/// exception is pending and propagates when the callback returns).
fn number_arg(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments,
    i: i32,
) -> Option<f64> {
    args.get(i).number_value(scope)
}

/// The override table index stored as the function's data.
fn table_index(scope: &mut v8::PinScope<'_, '_>, args: &v8::FunctionCallbackArguments) -> usize {
    args.data()
        .int32_value(scope)
        .and_then(|i| usize::try_from(i).ok())
        .unwrap_or(usize::MAX)
}

fn unary(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(&(_, f)) = UNARY_OVERRIDES.get(table_index(scope, &args)) else {
        return throw(scope, "a Math override lost its implementation");
    };
    if let Some(x) = number_arg(scope, &args, 0) {
        rv.set_double(f(x));
    }
}

fn binary(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(&(_, f)) = BINARY_OVERRIDES.get(table_index(scope, &args)) else {
        return throw(scope, "a Math override lost its implementation");
    };
    // ToNumber(x) then ToNumber(y), as the spec orders them.
    let Some(x) = number_arg(scope, &args, 0) else {
        return;
    };
    if let Some(y) = number_arg(scope, &args, 1) {
        rv.set_double(f(x, y));
    }
}

/// Throws `Error(message)`.
fn throw(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    if let Some(text) = v8::String::new(scope, message) {
        let error = v8::Exception::error(scope, text);
        scope.throw_exception(error);
    }
}

fn random(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    throw(scope, RANDOM_MESSAGE);
}

fn zero(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    rv.set_int32(0);
}

fn set_timer(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match scope.get_slot_mut::<TimerIds>() {
        Some(ids) => {
            ids.0 = ids.0.saturating_add(1);
            rv.set_uint32(ids.0);
        }
        None => throw(scope, "the timer ids are missing"),
    }
}

/// `__strudelandMark(label)`: records `String(label)` with the current monotonic instant when
/// profiling, else does nothing; returns `undefined` either way, so a song cannot read a clock.
fn mark(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let now = std::time::Instant::now();
    let profiling = matches!(scope.get_slot::<Marks>(), Some(Marks(Some(_))));
    if !profiling {
        return;
    }
    // A throwing `toString` propagates.
    let Some(label) = args.get(0).to_string(scope) else {
        return;
    };
    let label = label.to_rust_string_lossy(scope);
    if let Some(Marks(Some(marks))) = scope.get_slot_mut::<Marks>() {
        marks.push((label, now));
    }
}

fn nothing(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
}

fn console_logged(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let data = args.data();
    if !data.is_string() {
        return;
    }
    let level = data.to_rust_string_lossy(scope);
    let mut parts = Vec::with_capacity(usize::try_from(args.length()).unwrap_or(0));
    for i in 0..args.length() {
        // A throwing `toString` propagates.
        let Some(text) = args.get(i).to_string(scope) else {
            return;
        };
        parts.push(text.to_rust_string_lossy(scope));
    }
    write_log(&level, &parts.join(" "));
}
