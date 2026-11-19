//! `Host`: the bundle loads, every corpus fixture plans into a valid engine job, requests are
//! isolated, results are deterministic across hosts and threads, and every `HostError` variant
//! is reachable.

mod common;

use std::time::Instant;

use common::{SONG, corpus_song, repo_root, request};
use serde_json::Value;
use strudeland_engine::EngineJob;
use strudeland_host::{Host, HostError, Layout, LookbackSec, Mode, RenderRequest};

#[test]
fn the_bundle_loads_and_installs_the_entry() {
    // eval_script runs without the bundle; a query proves the bundle evaluated and its entry ran.
    let host = Host::new().unwrap();
    assert_eq!(
        host.eval_script("typeof strudelandHost").unwrap(),
        "undefined"
    );
    let events = host.query(SONG, 0.0, 1.0, Layout::Linear).unwrap();
    let events: Value = serde_json::from_str(&events).unwrap();
    assert_eq!(events.as_array().map(Vec::len), Some(3), "{events}");
}

#[test]
fn every_corpus_fixture_plans_a_valid_engine_job() {
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("tests/render/corpus.json")).unwrap(),
    )
    .unwrap();
    let fixtures = manifest["fixtures"].as_array().unwrap();
    assert!(!fixtures.is_empty());
    // Four hosts in parallel, each planning every fourth fixture, to bound the wall time.
    let jobs: Vec<(String, String, RenderRequest)> = fixtures
        .iter()
        .map(|fixture| {
            let name = fixture["name"].as_str().unwrap().to_string();
            let code = corpus_song(fixture["file"].as_str().unwrap());
            let window = fixture["windowBars"].as_array().unwrap();
            let layout: Layout = serde_json::from_value(fixture["layout"].clone()).unwrap();
            let req = RenderRequest {
                layout,
                from_bar: u32::try_from(window[0].as_u64().unwrap()).unwrap(),
                to_bar: u32::try_from(window[1].as_u64().unwrap()).unwrap(),
                overlap_bars: 0,
                lookback_sec: LookbackSec::Auto,
                mode: Mode::Mix,
                sample_rate: 44_100,
            };
            (name, code, req)
        })
        .collect();
    let jobs = std::sync::Arc::new(jobs);
    let workers: Vec<_> = (0..4)
        .map(|w| {
            let jobs = jobs.clone();
            std::thread::spawn(move || {
                let host = Host::new().unwrap();
                let mut timings = Vec::new();
                for (name, code, req) in jobs.iter().skip(w).step_by(4) {
                    let started = Instant::now();
                    let planned = host
                        .plan_job(code, req)
                        .unwrap_or_else(|e| panic!("{name}: {e}"));
                    timings.push((name.clone(), started.elapsed().as_millis()));
                    assert!(planned.json.is_ascii(), "{name}: job is not ASCII");
                    let info: serde_json::Value = serde_json::from_str(&planned.info).unwrap();
                    assert_eq!(
                        info["jsonLength"].as_u64(),
                        Some(planned.json.len() as u64),
                        "{name}: jsonLength"
                    );
                    EngineJob::from_json(&planned.json).unwrap_or_else(|e| panic!("{name}: {e}"));
                }
                timings
            })
        })
        .collect();
    let timings: Vec<(String, u128)> = workers
        .into_iter()
        .flat_map(|w| w.join().unwrap())
        .collect();
    assert_eq!(timings.len(), fixtures.len());
    eprintln!("plan ms per fixture: {timings:?}");
}

#[test]
fn a_request_does_not_see_the_previous_ones_globals() {
    // Song A defines a global, a registered method and a transpiler-level name; B reads them.
    // (`x` itself is a Strudel control, from @strudel/draw, so the global has its own name.)
    let song_a = "globalThis.hostLeakX = 1;\nconst hostLeak = 2;\nregister('hostTwice', (pat) => pat.fast(2));\n$: note(\"c e\").s(\"sine\").hostTwice()\n";
    let song_b = "$: note(globalThis.hostLeakX === undefined && typeof hostLeak === 'undefined' && typeof hostTwice === 'undefined' ? 'c' : 'e').s(\"sine\")\n";
    let host = Host::new().unwrap();
    let a = host.plan_job(song_a, &request(2)).unwrap();
    let b_after_a = host.plan_job(song_b, &request(2)).unwrap();
    let b_fresh = Host::new().unwrap().plan_job(song_b, &request(2)).unwrap();
    assert_ne!(a.json, b_fresh.json);
    assert_eq!(b_after_a.json, b_fresh.json);
    assert!(b_fresh.json.contains("\"note\":\"c\""), "{}", b_fresh.json);
}

#[test]
fn a_song_planned_twice_gives_identical_jobs() {
    let code = corpus_song("sine-basic.js");
    let host = Host::new().unwrap();
    let first = host.plan_job(&code, &request(4)).unwrap();
    let second = host.plan_job(&code, &request(4)).unwrap();
    assert_eq!(first, second);
}

#[test]
fn two_hosts_on_two_threads_agree() {
    let code = corpus_song("sine-basic.js");
    let reference = Host::new().unwrap().plan_job(&code, &request(4)).unwrap();
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let code = code.clone();
            std::thread::spawn(move || Host::new().unwrap().plan_job(&code, &request(4)).unwrap())
        })
        .collect();
    for t in threads {
        assert_eq!(t.join().unwrap(), reference);
    }
}

#[test]
fn one_host_shared_by_threads_serves_each_call() {
    let host = std::sync::Arc::new(Host::new().unwrap());
    let reference = host.query(SONG, 0.0, 2.0, Layout::Linear).unwrap();
    let threads: Vec<_> = (0..3)
        .map(|_| {
            let host = host.clone();
            std::thread::spawn(move || host.query(SONG, 0.0, 2.0, Layout::Linear).unwrap())
        })
        .collect();
    for t in threads {
        assert_eq!(t.join().unwrap(), reference);
    }
}

#[test]
fn a_bad_meta_header_is_meta() {
    let host = Host::new().unwrap();
    match host.plan_job("const meta = { bpm: 'x' };\n$: note(\"c\")\n", &request(2)) {
        Err(HostError::Meta(m)) => assert!(m.contains("bpm"), "{m}"),
        other => panic!("{other:?}"),
    }
    match host.query("const meta = { meter: '9/x' };\n", 0.0, 1.0, Layout::Linear) {
        Err(HostError::Meta(m)) => assert!(m.contains("meter"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_throwing_song_is_eval() {
    let host = Host::new().unwrap();
    match host.plan_job("throw new Error('boom-in-song');\n", &request(2)) {
        Err(HostError::Eval(m)) => assert!(m.contains("boom-in-song"), "{m}"),
        other => panic!("{other:?}"),
    }
    match host.plan_job("$: note(\"c\".s(\n", &request(2)) {
        Err(HostError::Eval(_)) => {}
        other => panic!("syntax error: {other:?}"),
    }
    match host.query(
        "throw new Error('boom-in-query');\n",
        0.0,
        1.0,
        Layout::Clip,
    ) {
        Err(HostError::Eval(m)) => assert!(m.contains("boom-in-query"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_song_calling_math_random_fails_with_the_clear_error() {
    let host = Host::new().unwrap();
    match host.plan_job(
        "const r = Math.random();\n$: note(\"c\").s(\"sine\")\n",
        &request(2),
    ) {
        Err(HostError::Eval(m)) => assert!(m.contains("Math.random is disabled"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_invalid_request_is_plan() {
    let host = Host::new().unwrap();
    let cases = [
        RenderRequest {
            from_bar: 4,
            to_bar: 4,
            ..request(4)
        },
        RenderRequest {
            to_bar: 200_000,
            ..request(4)
        },
        RenderRequest {
            to_bar: u32::MAX,
            overlap_bars: u32::MAX,
            ..request(4)
        },
        RenderRequest {
            lookback_sec: LookbackSec::Seconds(f64::NAN),
            ..request(4)
        },
        RenderRequest {
            lookback_sec: LookbackSec::Seconds(-1.0),
            ..request(4)
        },
        RenderRequest {
            sample_rate: 48_000,
            ..request(4)
        },
    ];
    for req in cases {
        match host.plan_job(SONG, &req) {
            Err(HostError::Plan(m)) => assert!(m.starts_with("invalid request: "), "{req:?}: {m}"),
            other => panic!("{req:?}: {other:?}"),
        }
    }
}

#[test]
fn a_song_waiting_on_a_timer_is_unsettled() {
    let host = Host::new().unwrap();
    let code =
        "await new Promise((resolve) => setTimeout(resolve, 10));\n$: note(\"c\").s(\"sine\")\n";
    assert_eq!(host.plan_job(code, &request(2)), Err(HostError::Unsettled));
    assert_eq!(
        host.query(code, 0.0, 1.0, Layout::Linear),
        Err(HostError::Unsettled)
    );
    let never = "await new Promise(() => {});\n$: note(\"c\").s(\"sine\")\n";
    assert_eq!(host.plan_job(never, &request(2)), Err(HostError::Unsettled));
    // The host still serves the next request.
    assert!(host.plan_job(SONG, &request(2)).is_ok());
}

#[test]
fn a_script_error_outside_the_entry_is_runtime() {
    let host = Host::new().unwrap();
    assert!(matches!(host.eval_script("("), Err(HostError::Runtime(_))));
    match host.eval_script("throw new TypeError('outside')") {
        Err(HostError::Runtime(m)) => assert_eq!(m, "TypeError: outside"),
        other => panic!("{other:?}"),
    }
    match host.eval_script("throw 42") {
        Err(HostError::Runtime(m)) => assert_eq!(m, "42"),
        other => panic!("{other:?}"),
    }
    assert_eq!(host.eval_script("1 + 1").unwrap(), "2");
    assert_eq!(host.eval_script("'a' + 'b'").unwrap(), "ab");
    assert_eq!(host.eval_script("undefined").unwrap(), "undefined");
}

#[test]
fn query_returns_events_for_both_layouts() {
    let host = Host::new().unwrap();
    let song = "const meta = { bpm: 120, sections: [{ name: 'a', bars: 2 }] };\n$: note(\"c e\").s(\"sine\")\n";
    for layout in [Layout::Linear, Layout::Clip] {
        let text = host.query(song, 0.0, 2.0, layout).unwrap();
        let events: Value = serde_json::from_str(&text).unwrap();
        let events = events.as_array().unwrap();
        assert_eq!(events.len(), 4, "{layout:?}: {text}");
        assert_eq!(events[0]["s"], "sine", "{layout:?}");
        assert_eq!(events[1]["note"], "e", "{layout:?}");
    }
}

#[test]
fn query_rejects_a_bad_window() {
    let host = Host::new().unwrap();
    for (from, to) in [
        (1.0, 1.0),
        (2.0, 1.0),
        (f64::NAN, 1.0),
        (0.0, f64::NAN),
        (0.0, f64::INFINITY),
        (f64::NEG_INFINITY, 0.0),
        (0.0, 4096.5),
        (-10.0, 4090.0),
    ] {
        assert_eq!(
            host.query(SONG, from, to, Layout::Linear),
            Err(HostError::Plan("bad window".into())),
            "{from}..{to}"
        );
    }
    assert!(host.query(SONG, 0.0, 4096.0, Layout::Linear).is_ok());
}

#[test]
fn the_info_describes_the_job() {
    let host = Host::new().unwrap();
    let planned = host
        .plan_job(&corpus_song("sine-basic.js"), &request(4))
        .unwrap();
    let info: serde_json::Value = serde_json::from_str(&planned.info).unwrap();
    assert!(!planned.info.contains('\n'), "the info is one line");
    for key in [
        "jsonLength",
        "events",
        "errors",
        "plan",
        "cps",
        "structure",
        "stemPairs",
    ] {
        assert!(info.get(key).is_some(), "{key}: {info}");
    }
    assert_eq!(info["cps"], 0.5);
    assert_eq!(info["errors"], serde_json::json!([]));
    let job: Value = serde_json::from_str(&planned.json).unwrap();
    assert_eq!(job["schema"], 2);
    assert_eq!(job["sampleRate"], 44_100);
}

#[test]
fn a_failure_core_logs_is_in_the_info_errors() {
    // core's `queryArc` logs `[query] error: …` and answers [], as the page records from its
    // `strudel.log` event.
    let host = Host::new().unwrap();
    let song = format!(
        "note('c').withValue(() => {{ throw new Error('inner query broke'); }}).queryArc(0, 1);\n{SONG}"
    );
    let planned = host.plan_job(&song, &request(2)).unwrap();
    let info: Value = serde_json::from_str(&planned.info).unwrap();
    assert_eq!(
        info["errors"],
        serde_json::json!([{ "kind": "other", "message": "[query] error: inner query broke" }])
    );
}
