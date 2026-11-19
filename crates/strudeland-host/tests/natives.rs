//! The natives installed before the bundle: `jsmath` overrides, a throwing `Math.random`, zero
//! clocks, inert timers, console capture and the absent browser globals.

use std::process::Command;
use std::sync::OnceLock;

use strudeland_engine::jsmath;
use strudeland_host::natives::{
    ABSENT, BINARY_OVERRIDES, LOG_ENV, RANDOM_MESSAGE, UNARY_OVERRIDES, Unary,
};
use strudeland_host::{Host, HostError};

fn host() -> &'static Host {
    static HOST: OnceLock<Host> = OnceLock::new();
    HOST.get_or_init(|| Host::new().unwrap())
}

fn eval(source: &str) -> String {
    host()
        .eval_script(source)
        .unwrap_or_else(|e| panic!("{source}: {e}"))
}

const UNARY_INPUTS: [f64; 18] = [
    0.0,
    -0.0,
    0.5,
    -1.5,
    1.0,
    -1.0,
    3.0,
    10.0,
    100.0,
    709.0,
    -745.0,
    1e-300,
    1e300,
    std::f64::consts::FRAC_PI_4,
    f64::MIN_POSITIVE,
    f64::NAN,
    f64::INFINITY,
    f64::NEG_INFINITY,
];

const BINARY_INPUTS: [(f64, f64); 18] = [
    (2.0, 0.5),
    (2.0, 10.0),
    (10.0, -3.0),
    (0.5, 3.0),
    (-8.0, 1.0 / 3.0),
    (-2.0, 3.0),
    (0.0, -1.0),
    (-0.0, -1.0),
    (-0.0, 3.0),
    (1.0, f64::NAN),
    (f64::NAN, 0.0),
    (f64::NAN, 1.0),
    (-1.0, f64::INFINITY),
    (f64::INFINITY, -0.5),
    (f64::NEG_INFINITY, 3.0),
    (1.000_000_1, 1e9),
    (0.3, 4.0),
    (1e300, 2.0),
];

/// JS that decodes the hex bit pattern into a number.
fn js_number(x: f64) -> String {
    format!("num(0x{:016x}n)", x.to_bits())
}

/// The JS prelude: `num(bits)` and `bits(x)` (hex, 16 digits).
const PRELUDE: &str = "const dv = new DataView(new ArrayBuffer(8));
const num = (b) => (dv.setBigUint64(0, b), dv.getFloat64(0));
const bits = (x) => (dv.setFloat64(0, x), dv.getBigUint64(0).toString(16).padStart(16, '0'));";

fn js_bits(expressions: &[String]) -> Vec<u64> {
    let source = format!(
        "{PRELUDE}\n[{}].map(bits).join(',')",
        expressions.join(", ")
    );
    eval(&source)
        .split(',')
        .map(|h| u64::from_str_radix(h, 16).unwrap())
        .collect()
}

fn same(a: u64, b: u64) -> bool {
    a == b || (f64::from_bits(a).is_nan() && f64::from_bits(b).is_nan())
}

#[test]
fn overrides_keep_the_original_name_and_length() {
    for (name, _) in UNARY_OVERRIDES {
        assert_eq!(
            eval(&format!("Math.{name}.name + ',' + Math.{name}.length")),
            format!("{name},1"),
            "{name}"
        );
    }
    for (name, _) in BINARY_OVERRIDES {
        assert_eq!(
            eval(&format!("Math.{name}.name + ',' + Math.{name}.length")),
            format!("{name},2"),
            "{name}"
        );
    }
    assert_eq!(eval("Math.pow.name + ',' + Math.pow.length"), "pow,2");
    // Like the built-ins they replace, the overrides are not constructors.
    assert_eq!(
        eval("try { new Math.sin(1); 'constructed' } catch (e) { e instanceof TypeError }"),
        "true"
    );
    assert_eq!(eval("'prototype' in Math.pow"), "false");
    assert_eq!(eval("Math.sin.name + ',' + Math.sin.length"), "sin,1");
}

#[test]
fn the_eight_overrides_are_the_jsmath_list() {
    let mut names: Vec<&str> = UNARY_OVERRIDES
        .iter()
        .map(|(n, _)| *n)
        .chain(BINARY_OVERRIDES.iter().map(|(n, _)| *n))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["cos", "cosh", "exp", "expm1", "log1p", "pow", "sin", "tanh"]
    );
}

#[test]
fn unary_overrides_return_the_jsmath_bits() {
    let cases: [(&str, Unary); 7] = [
        ("sin", jsmath::sin),
        ("cos", jsmath::cos),
        ("tanh", jsmath::tanh),
        ("exp", jsmath::exp),
        ("expm1", jsmath::expm1),
        ("log1p", jsmath::log1p),
        ("cosh", jsmath::cosh),
    ];
    for (name, f) in cases {
        let exprs: Vec<String> = UNARY_INPUTS
            .iter()
            .map(|&x| format!("Math.{name}({})", js_number(x)))
            .collect();
        let got = js_bits(&exprs);
        for (&x, g) in UNARY_INPUTS.iter().zip(got) {
            let want = f(x).to_bits();
            assert!(
                same(g, want),
                "Math.{name}({x:e}): host {g:016x}, jsmath {want:016x}"
            );
        }
    }
}

#[test]
fn pow_returns_the_jsmath_bits() {
    let exprs: Vec<String> = BINARY_INPUTS
        .iter()
        .map(|&(x, y)| format!("Math.pow({}, {})", js_number(x), js_number(y)))
        .collect();
    let got = js_bits(&exprs);
    for (&(x, y), g) in BINARY_INPUTS.iter().zip(got) {
        let want = jsmath::pow(x, y).to_bits();
        assert!(
            same(g, want),
            "Math.pow({x:e}, {y:e}): host {g:016x}, jsmath {want:016x}"
        );
    }
}

#[test]
fn arguments_go_through_to_number() {
    let got = js_bits(&[
        "Math.sin('0.5')".into(),
        "Math.sin(true)".into(),
        "Math.sin(null)".into(),
        "Math.sin()".into(),
        "Math.sin(undefined)".into(),
        "Math.pow(2)".into(),
        "Math.pow()".into(),
        "Math.pow('2', '3')".into(),
        "Math.cos({ valueOf: () => 1 })".into(),
        "Math.exp(1, 99)".into(),
    ]);
    let want = [
        jsmath::sin(0.5),
        jsmath::sin(1.0),
        jsmath::sin(0.0),
        f64::NAN,
        f64::NAN,
        f64::NAN,
        f64::NAN,
        8.0,
        jsmath::cos(1.0),
        jsmath::exp(1.0),
    ];
    for (i, (g, w)) in got.into_iter().zip(want).enumerate() {
        assert!(same(g, w.to_bits()), "case {i}: {g:016x} vs {w:e}");
    }
}

#[test]
fn a_throwing_conversion_propagates_in_argument_order() {
    assert_eq!(
        eval(
            "try { Math.sin({ valueOf() { throw new Error('v') } }); 'no' } catch (e) { e.message }"
        ),
        "v"
    );
    assert_eq!(
        eval("try { Math.cos(Symbol('s')); 'no' } catch (e) { e instanceof TypeError }"),
        "true"
    );
    assert_eq!(
        eval(
            "const seen = []; Math.pow({ valueOf() { seen.push('x'); return 2 } }, { valueOf() { seen.push('y'); return 3 } }) + seen.join('')"
        ),
        "8xy"
    );
}

#[test]
fn math_random_throws_the_clear_error() {
    assert_eq!(
        eval("try { Math.random(); 'no' } catch (e) { (e instanceof Error) + '|' + e.message }"),
        format!("true|{RANDOM_MESSAGE}")
    );
    match host().eval_script("Math.random()") {
        Err(HostError::Runtime(m)) => assert!(m.contains(RANDOM_MESSAGE), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(RANDOM_MESSAGE.contains("Math.random is disabled"));
}

#[test]
fn clocks_are_zero() {
    assert_eq!(eval("Date.now()"), "0");
    assert_eq!(eval("performance.now()"), "0");
    assert_eq!(eval("Object.keys(performance).join()"), "now");
    assert_eq!(
        eval("Object.is(Date.now(), 0) && Object.is(performance.now(), 0)"),
        "true"
    );
}

#[test]
fn the_no_argument_date_forms_read_the_frozen_clock() {
    assert_eq!(eval("new Date().getTime()"), "0");
    assert_eq!(eval("Object.is(+new Date(), 0)"), "true");
    assert_eq!(eval("new Date().toISOString()"), "1970-01-01T00:00:00.000Z");
    assert_eq!(eval("Date() === new Date(0).toString()"), "true");
    assert_eq!(eval("typeof Date()"), "string");
    assert_eq!(eval("Date.now()"), "0");
}

#[test]
fn date_with_arguments_and_its_statics_work_as_in_chromium() {
    assert_eq!(eval("new Date(86400000).getTime()"), "86400000");
    assert_eq!(eval("new Date(2020, 0, 2).getFullYear()"), "2020");
    assert_eq!(
        eval("new Date('2020-01-02T03:04:05Z').toISOString()"),
        "2020-01-02T03:04:05.000Z"
    );
    assert_eq!(eval("new Date(undefined).getTime()"), "NaN");
    assert_eq!(eval("Date.UTC(2020, 0)"), "1577836800000");
    assert_eq!(eval("Date.parse('1970-01-01T00:00:01Z')"), "1000");
    assert_eq!(
        eval(
            "[new Date() instanceof Date, Object.prototype.toString.call(new Date()), \
             Date.name, Date.length, new Date().constructor === Date, \
             Object.keys(Date).length, Object.getOwnPropertyDescriptor(globalThis, 'Date').enumerable].join()"
        ),
        "true,[object Date],Date,7,true,0,false"
    );
    assert_eq!(
        eval(
            "class D extends Date { constructor(...a) { super(...a); } } \
             const d = new D(); const e = new D(5); \
             [d instanceof D, d instanceof Date, d.getTime(), e.getTime()].join()"
        ),
        "true,true,0,5"
    );
}

#[test]
fn timers_return_increasing_ids_and_never_fire() {
    assert_eq!(
        eval(
            "globalThis.fired = 0; const f = () => { globalThis.fired++ };
             [setTimeout(f, 0), setTimeout(f), setInterval(f, 1), setTimeout()].join()"
        ),
        "1,2,3,4"
    );
    assert_eq!(
        eval(
            "clearTimeout(1); clearTimeout(); clearInterval('x'); clearInterval({}); clearTimeout(null); 'ok'"
        ),
        "ok"
    );
    // A fresh context per call: the ids restart and nothing a timer would do has happened.
    assert_eq!(eval("setTimeout(() => {}, 0)"), "1");
    assert_eq!(eval("typeof globalThis.fired"), "undefined");
}

#[test]
fn browser_globals_are_absent() {
    for name in ABSENT {
        assert_eq!(eval(&format!("typeof {name}")), "undefined", "{name}");
        assert_eq!(eval(&format!("'{name}' in globalThis")), "false", "{name}");
    }
    for name in [
        "window",
        "document",
        "fetch",
        "AudioContext",
        "localStorage",
        "queueMicrotask",
        "structuredClone",
    ] {
        assert!(ABSENT.contains(&name), "{name}");
    }
    for name in ["std", "os", "require", "process"] {
        assert_eq!(eval(&format!("typeof {name}")), "undefined", "{name}");
    }
}

#[test]
fn console_methods_exist_and_return_undefined() {
    assert_eq!(
        eval(
            "[console.log('a'), console.info('b'), console.debug('c'), console.trace('d'), console.dir({}), console.warn('w', 1), console.error(new Error('e'))].map(String).join()"
        ),
        "undefined,undefined,undefined,undefined,undefined,undefined,undefined"
    );
}

#[test]
fn the_other_console_methods_exist_and_do_nothing() {
    // Core's `Hap` constructor calls `console.assert` for a stateful hap.
    for call in [
        "console.assert(false, 'x')",
        "console.assert(true)",
        "console.dirxml({})",
        "console.clear()",
        "console.count('c')",
        "console.countReset('c')",
        "console.group('g')",
        "console.groupCollapsed()",
        "console.groupEnd()",
        "console.table([1, 2])",
        "console.time('t')",
        "console.timeLog('t')",
        "console.timeEnd('t')",
        "console.profile()",
        "console.profileEnd()",
        "console.timeStamp()",
    ] {
        assert_eq!(eval(&format!("String({call})")), "undefined", "{call}");
    }
}

fn song_file(code: &str) -> tempfile::NamedTempFile {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), code).unwrap();
    file
}

fn query_stderr(log: Option<&str>) -> String {
    let song = song_file(
        "console.warn('host-log-probe', 1);\nconsole.log('quiet-probe');\n$: note(\"c\").s(\"sine\")\n",
    );
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_strudeland-host"));
    cmd.args(["query", "--song"])
        .arg(song.path())
        .args(["--from", "0", "--to", "1"]);
    cmd.env_remove(LOG_ENV);
    if let Some(v) = log {
        cmd.env(LOG_ENV, v);
    }
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    String::from_utf8(out.stderr).unwrap()
}

#[test]
fn console_warn_is_discarded_unless_the_log_variable_is_1() {
    assert_eq!(query_stderr(None), "");
    assert_eq!(query_stderr(Some("0")), "");
    assert_eq!(query_stderr(Some("true")), "");
    let logged = query_stderr(Some("1"));
    assert!(
        logged.contains("console.warn: host-log-probe 1"),
        "{logged}"
    );
    assert!(!logged.contains("quiet-probe"), "{logged}");
}
