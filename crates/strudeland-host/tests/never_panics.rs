//! Malformed songs and requests come back as `Err` (or `Ok`), never as a panic or an abort.

mod common;

use common::{SONG, request};
use strudeland_host::{
    Host, HostError, Layout, LengthRequest, LookbackSec, Mode, RenderPlanRequest, RenderRequest,
};

fn render_request(length: LengthRequest) -> RenderPlanRequest {
    RenderPlanRequest {
        length,
        mode: Mode::Mix,
        sample_rate: 44_100,
        include_events: Some(true),
        timeout_sec: None,
    }
}

fn garbage() -> Vec<(&'static str, String)> {
    let mut noise = String::new();
    let mut state: u32 = 0x1234_5678;
    for _ in 0..4096 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        noise.push(char::from(b' ' + (state % 95) as u8));
    }
    vec![
        ("empty", String::new()),
        ("spaces", "   \n\t ".into()),
        ("noise", noise),
        ("open parens", "(".repeat(100_000)),
        ("close braces", "}".repeat(10_000)),
        ("unterminated string", "$: note(\"c e".into()),
        ("unterminated template", "$: note(`c e".into()),
        ("nul and control bytes", "\0\u{1}\u{7f}$: \0".into()),
        ("lone meta", "const meta = {".into()),
        (
            "meta not an object",
            "const meta = 3;\n$: note(\"c\")".into(),
        ),
        ("deep recursion", "const f = (n) => f(n + 1); f(0);".into()),
        (
            "throw a non-error",
            "throw { toString() { throw 1 } };".into(),
        ),
        (
            "reject",
            "await Promise.reject(new Error('rejected'));".into(),
        ),
        ("pattern of nothing", "$: silence".into()),
        ("non-pattern output", "$: 42".into()),
        (
            "5 MB of comment",
            format!("{SONG}// {}\n", "x".repeat(5 << 20)),
        ),
        (
            "5 MB of code",
            format!("{SONG}{}", "1;\n".repeat((5 << 20) / 3)),
        ),
    ]
}

#[test]
fn malformed_songs_return_without_panicking() {
    let host = Host::new().unwrap();
    for (name, code) in garbage() {
        let planned = host.plan_job(&code, &request(2));
        if let Ok(job) = &planned {
            assert!(!job.json.is_empty(), "{name}");
        }
        let _ = host.query(&code, 0.0, 1.0, Layout::Linear);
        let _ = host.query(&code, 0.0, 1.0, Layout::Clip);
        for length in [
            LengthRequest::Clip { loops: None },
            LengthRequest::Clip { loops: Some(2) },
            LengthRequest::Section { name: "a".into() },
            LengthRequest::Length { bars: 2 },
        ] {
            if let Ok(planned) = host.plan_render(&code, &render_request(length)) {
                assert!(!planned.chunks.is_empty(), "{name}");
            }
        }
    }
    // The host survives all of it.
    assert!(host.plan_job(SONG, &request(2)).is_ok());
}

#[test]
fn malformed_scripts_return_without_panicking() {
    let host = Host::new().unwrap();
    for (name, code) in garbage() {
        let _ = host.eval_script(&code);
        let _ = name;
    }
    assert!(matches!(
        host.eval_script("(function f() { f() })()"),
        Err(HostError::Runtime(_))
    ));
    assert_eq!(host.eval_script("1").unwrap(), "1");
}

#[test]
fn odd_request_numbers_are_errors() {
    let host = Host::new().unwrap();
    let cases = [
        RenderRequest {
            lookback_sec: LookbackSec::Seconds(f64::NAN),
            ..request(2)
        },
        RenderRequest {
            lookback_sec: LookbackSec::Seconds(f64::NEG_INFINITY),
            ..request(2)
        },
        RenderRequest {
            lookback_sec: LookbackSec::Seconds(-0.5),
            ..request(2)
        },
        RenderRequest {
            from_bar: u32::MAX,
            to_bar: u32::MAX,
            ..request(2)
        },
        RenderRequest {
            from_bar: 0,
            to_bar: u32::MAX,
            overlap_bars: u32::MAX,
            ..request(2)
        },
        RenderRequest {
            sample_rate: 0,
            ..request(2)
        },
    ];
    for req in cases {
        assert!(
            matches!(host.plan_job(SONG, &req), Err(HostError::Plan(_))),
            "{req:?}"
        );
    }
    let bad_renders = [
        RenderPlanRequest {
            timeout_sec: Some(f64::NAN),
            ..render_request(LengthRequest::Length { bars: 2 })
        },
        RenderPlanRequest {
            timeout_sec: Some(f64::MAX),
            ..render_request(LengthRequest::Length { bars: 2 })
        },
        RenderPlanRequest {
            timeout_sec: Some(-1.0),
            ..render_request(LengthRequest::Length { bars: 2 })
        },
        render_request(LengthRequest::Length { bars: u32::MAX }),
        render_request(LengthRequest::Clip {
            loops: Some(u32::MAX),
        }),
        render_request(LengthRequest::Window {
            from_bar: u32::MAX,
            to_bar: u32::MAX,
            layout: Layout::Linear,
        }),
        render_request(LengthRequest::Window {
            from_bar: 0,
            to_bar: u32::MAX,
            layout: Layout::Clip,
        }),
        RenderPlanRequest {
            sample_rate: 0,
            ..render_request(LengthRequest::Length { bars: 2 })
        },
    ];
    for req in bad_renders {
        assert!(
            matches!(host.plan_render(SONG, &req), Err(HostError::Plan(_))),
            "{req:?}"
        );
    }
    // Odd section names reach the bundle and come back as errors.
    for name in ["", "\0", "'", "\u{1F600}", &"x".repeat(1 << 20)] {
        let song = "const meta = { sections: [{ name: 'a', bars: 2 }] };\n$: note(\"c\")\n";
        assert!(
            matches!(
                host.plan_render(
                    song,
                    &render_request(LengthRequest::Section { name: name.into() })
                ),
                Err(HostError::MetaFlag { .. })
            ),
            "{name:?}"
        );
    }
    // A huge but valid lookback is planned or refused, never a panic.
    let _ = host.plan_job(
        SONG,
        &RenderRequest {
            lookback_sec: LookbackSec::Seconds(1e12),
            ..request(2)
        },
    );
    for (from, to) in [
        (f64::NAN, f64::NAN),
        (f64::MAX, f64::INFINITY),
        (-f64::MAX, f64::MAX),
        (1e300, 1e300 + 1.0),
        (-0.0, 0.0),
    ] {
        assert!(
            host.query(SONG, from, to, Layout::Linear).is_err(),
            "{from}..{to}"
        );
    }
}

#[test]
fn a_hundred_thousand_bar_window_returns() {
    let host = Host::new().unwrap();
    let sparse = "$: note(\"c\").s(\"sine\").slow(20000)\n";
    let window = RenderRequest {
        to_bar: 100_000,
        ..request(2)
    };
    // Planned or refused (the job may be too long for the planner), but it returns.
    match host.plan_job(sparse, &window) {
        Ok(job) => assert!(!job.json.is_empty()),
        Err(e) => assert!(matches!(e, HostError::Plan(_)), "{e}"),
    }
    let tail = RenderRequest {
        from_bar: 99_999,
        to_bar: 100_000,
        ..request(2)
    };
    let _ = host.plan_job(sparse, &tail);
    // The batch planner over the widest window: planned or refused, but it returns.
    let widest = render_request(LengthRequest::Window {
        from_bar: 0,
        to_bar: 100_000,
        layout: Layout::Linear,
    });
    match host.plan_render(sparse, &widest) {
        Ok(planned) => assert!(!planned.chunks.is_empty()),
        Err(e) => assert!(matches!(e, HostError::Plan(_)), "{e}"),
    }
}

#[test]
fn exhausting_the_heap_is_a_runtime_error_not_an_abort() {
    let host = Host::new().unwrap();
    let hog = "const a = [];\nwhile (true) a.push(new Array(1e5).fill(1.5));\n";
    for result in [
        host.eval_script(hog).map(|_| ()),
        host.plan_job(hog, &request(2)).map(|_| ()),
    ] {
        match result {
            Err(HostError::Runtime(m)) => assert!(m.contains("ran out of memory"), "{m}"),
            other => panic!("{other:?}"),
        }
    }
    // The next request gets a fresh heap.
    assert!(host.plan_job(SONG, &request(2)).is_ok());
}
