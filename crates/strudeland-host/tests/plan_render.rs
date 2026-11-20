//! `Host::plan_render`: every chunk of a render planned in one call, equal to planning each
//! chunk's request with `Host::plan_job`; the length requests and their flagged errors; the
//! planning timeout, which the same `Host` survives.

mod common;

use std::time::{Duration, Instant};

use common::{SONG, corpus_song, request};
use strudeland_engine::EngineJob;
use strudeland_host::{
    ExportPreset, Host, HostError, Layout, LengthRequest, LookbackSec, Mode, PlannedRender,
    RenderPlanRequest, RenderRequest,
};

fn plan_request(length: LengthRequest) -> RenderPlanRequest {
    RenderPlanRequest {
        length,
        preset: None,
        mode: Mode::Mix,
        sample_rate: 44_100,
        include_events: None,
        timeout_sec: None,
    }
}

fn window(from_bar: u32, to_bar: u32, layout: Layout) -> RenderPlanRequest {
    plan_request(LengthRequest::Window {
        from_bar,
        to_bar,
        layout,
    })
}

/// The page request of chunk `i` (`chunkRequest(base, plan)` with overlap 0).
fn chunk_request(planned: &PlannedRender, i: usize) -> RenderRequest {
    let plan = &planned.chunks[i].plan;
    RenderRequest {
        layout: planned.window.layout,
        from_bar: plan.from_bar,
        to_bar: plan.to_bar,
        overlap_bars: 0,
        lookback_sec: LookbackSec::Seconds(plan.lookback_sec),
        mode: Mode::Mix,
        sample_rate: 44_100,
    }
}

/// The chunk windows are contiguous, cover the window and parse as engine jobs.
fn assert_covers(name: &str, planned: &PlannedRender) {
    assert!(!planned.chunks.is_empty(), "{name}: no chunks");
    let mut at = planned.window.from_bar;
    for (i, chunk) in planned.chunks.iter().enumerate() {
        assert_eq!(chunk.plan.from_bar, at, "{name} chunk {i}: not contiguous");
        assert!(chunk.plan.to_bar > at, "{name} chunk {i}: empty");
        assert_eq!(
            chunk.plan.c1, chunk.plan.to_bar,
            "{name} chunk {i}: overlap"
        );
        assert!(chunk.plan.c0 <= chunk.plan.from_bar, "{name} chunk {i}: c0");
        assert!(chunk.json.is_ascii(), "{name} chunk {i}: not ASCII");
        EngineJob::from_json(&chunk.json).unwrap_or_else(|e| panic!("{name} chunk {i}: {e}"));
        at = chunk.plan.to_bar;
    }
    assert_eq!(at, planned.window.to_bar, "{name}: the chunks end early");
}

#[test]
fn chunks_equal_plan_job_of_each_chunk_request() {
    let host = Host::new().unwrap();
    let cases = [
        ("pad-seam", "pad-seam.js", window(0, 20, Layout::Linear)),
        (
            "reverb-long",
            "reverb-long.js",
            window(0, 20, Layout::Linear),
        ),
        ("song-clip", "song-linear.js", window(0, 12, Layout::Clip)),
        ("fx-chain", "fx-chain.js", window(0, 12, Layout::Linear)),
        (
            "song-linear tail",
            "song-linear.js",
            window(3, 8, Layout::Linear),
        ),
    ];
    let mut multi = 0;
    for (name, file, req) in cases {
        let code = corpus_song(file);
        let planned = host
            .plan_render(&code, &req)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_covers(name, &planned);
        if planned.chunks.len() > 1 {
            multi += 1;
        }
        for i in 0..planned.chunks.len() {
            let single = host
                .plan_job(&code, &chunk_request(&planned, i))
                .unwrap_or_else(|e| panic!("{name} chunk {i}: {e}"));
            assert!(
                single.json == planned.chunks[i].json,
                "{name} chunk {i}: the batch job differs from plan_job's"
            );
            let info: serde_json::Value = serde_json::from_str(&single.info).unwrap();
            let plan = &planned.chunks[i].plan;
            assert_eq!(info["plan"]["c0"], plan.c0, "{name} chunk {i}");
            assert_eq!(info["plan"]["frames"], plan.frames, "{name} chunk {i}");
            assert_eq!(
                info["plan"]["lookbackSecUsed"].as_f64().map(f64::to_bits),
                Some(plan.lookback_sec_used.to_bits()),
                "{name} chunk {i}"
            );
        }
    }
    assert!(multi >= 2, "the cases must include multi-chunk renders");
}

#[test]
fn export_requests_cross_the_bundle_boundary_and_exclude_the_ending() {
    let host = Host::new().unwrap();
    let song = corpus_song("song-linear.js");
    let request = plan_request(LengthRequest::Clip { loops: None });
    let regular = host.plan_render(&song, &request).unwrap();
    assert!(regular.export.is_none());
    for (preset, from, to) in [
        (ExportPreset::IntroLoop, 0, 6),
        (ExportPreset::TaggedLoop, 0, 6),
        (ExportPreset::FoldedLoop, 2, 6),
        (ExportPreset::OneShot, 0, 12),
    ] {
        let planned = host
            .plan_render(
                &song,
                &RenderPlanRequest {
                    preset: Some(preset),
                    ..request.clone()
                },
            )
            .unwrap();
        assert_eq!((planned.window.from_bar, planned.window.to_bar), (from, to));
        let export = planned.export.unwrap();
        assert_eq!(
            export.loop_start_bar,
            (preset != ExportPreset::OneShot).then_some(2)
        );
        let last = &planned.chunks.last().unwrap().plan;
        assert_eq!(last.c1, to);
        assert_eq!(
            last.frames - last.window_start_frame,
            u64::from(last.to_bar - last.from_bar) * 88_200 + export.tail_frames
        );
        assert_eq!(export.tail_frames == 0, preset == ExportPreset::TaggedLoop);
    }
}

#[test]
fn length_requests_resolve_against_the_meta() {
    let host = Host::new().unwrap();
    let song = corpus_song("song-linear.js");
    let cases = [
        (LengthRequest::Clip { loops: None }, 0, 12, Layout::Clip, 2),
        (
            LengthRequest::Clip { loops: Some(3) },
            0,
            16,
            Layout::Clip,
            3,
        ),
        (
            LengthRequest::Section {
                name: "body".into(),
            },
            2,
            6,
            Layout::Linear,
            2,
        ),
        (LengthRequest::Length { bars: 4 }, 0, 4, Layout::Clip, 2),
    ];
    for (length, from, to, layout, loops) in cases {
        let planned = host
            .plan_render(&song, &plan_request(length.clone()))
            .unwrap_or_else(|e| panic!("{length:?}: {e}"));
        assert_eq!(
            (
                planned.window.from_bar,
                planned.window.to_bar,
                planned.window.layout
            ),
            (from, to, layout),
            "{length:?}"
        );
        assert_eq!(planned.structure.loops, loops, "{length:?}");
        assert_eq!(planned.structure.sections.len(), 3, "{length:?}");
        assert!(planned.structure.sections[1].is_loop, "{length:?}");
        assert_eq!(planned.cps, 0.5, "{length:?}");
        assert_covers(&format!("{length:?}"), &planned);
    }
    // A song without sections renders a length on the linear timeline.
    let planned = host
        .plan_render(SONG, &plan_request(LengthRequest::Length { bars: 3 }))
        .unwrap();
    assert_eq!(
        (
            planned.window.from_bar,
            planned.window.to_bar,
            planned.window.layout
        ),
        (0, 3, Layout::Linear)
    );
    assert_eq!(planned.structure.clip_bars, 0);
    assert_eq!(planned.errors, vec![]);
}

#[test]
fn length_errors_are_meta_with_their_flag() {
    let host = Host::new().unwrap();
    let sections = corpus_song("song-linear.js");
    let plain = corpus_song("sine-basic.js");
    let cases = [
        (
            plain.as_str(),
            LengthRequest::Clip { loops: None },
            "--length",
            "this song has no sections; pass --length N to say how many bars to render",
        ),
        (
            plain.as_str(),
            LengthRequest::Section { name: "a".into() },
            "--section",
            "this song has no sections; --section needs meta.sections",
        ),
        (
            sections.as_str(),
            LengthRequest::Section {
                name: "drop".into(),
            },
            "--section",
            "unknown section 'drop' (sections: intro, body, tail)",
        ),
        (
            sections.as_str(),
            LengthRequest::Length { bars: 13 },
            "--length",
            "--length 13 is past the end of the song (12 bars)",
        ),
        (
            plain.as_str(),
            LengthRequest::Clip { loops: Some(2) },
            "--loops",
            "--loops needs a section with loop: true in meta.sections",
        ),
    ];
    for (code, length, flag, message) in cases {
        assert_eq!(
            host.plan_render(code, &plan_request(length.clone())),
            Err(HostError::MetaFlag {
                flag: flag.into(),
                message: message.into()
            }),
            "{length:?}"
        );
    }
    // A bad meta header has no flag.
    match host.plan_render(
        "const meta = { bpm: 'x' };\n$: note(\"c\")\n",
        &plan_request(LengthRequest::Length { bars: 2 }),
    ) {
        Err(HostError::Meta(m)) => assert!(m.contains("bpm"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_invalid_request_is_plan() {
    let host = Host::new().unwrap();
    let cases = [
        RenderPlanRequest {
            mode: Mode::Stems,
            ..window(0, 4, Layout::Linear)
        },
        RenderPlanRequest {
            sample_rate: 48_000,
            ..window(0, 4, Layout::Linear)
        },
        window(4, 4, Layout::Linear),
        window(0, 100_001, Layout::Linear),
        plan_request(LengthRequest::Length { bars: 0 }),
        plan_request(LengthRequest::Clip { loops: Some(0) }),
        RenderPlanRequest {
            timeout_sec: Some(0.0),
            ..window(0, 4, Layout::Linear)
        },
        RenderPlanRequest {
            timeout_sec: Some(f64::NAN),
            ..window(0, 4, Layout::Linear)
        },
    ];
    for req in cases {
        match host.plan_render(SONG, &req) {
            Err(HostError::Plan(m)) => assert!(m.starts_with("invalid request: "), "{req:?}: {m}"),
            other => panic!("{req:?}: {other:?}"),
        }
    }
    match host.plan_render(
        SONG,
        &RenderPlanRequest {
            mode: Mode::Stems,
            ..window(0, 4, Layout::Linear)
        },
    ) {
        Err(HostError::Plan(m)) => assert_eq!(
            m,
            "invalid request: mode: stems is not supported yet; v1 renders the mix"
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn events_only_when_asked() {
    let host = Host::new().unwrap();
    let code = corpus_song("sine-basic.js");
    let without = host
        .plan_render(&code, &window(0, 4, Layout::Linear))
        .unwrap();
    assert_eq!(without.events, None);
    let with = host
        .plan_render(
            &code,
            &RenderPlanRequest {
                include_events: Some(true),
                ..window(0, 4, Layout::Linear)
            },
        )
        .unwrap();
    let events = with.events.as_ref().unwrap().as_array().unwrap();
    let queried: serde_json::Value =
        serde_json::from_str(&host.query(&code, 0.0, 4.0, Layout::Linear).unwrap()).unwrap();
    assert_eq!(events.len(), queried.as_array().unwrap().len());
    assert_eq!(with.chunks, without.chunks);
    let off = host
        .plan_render(
            &code,
            &RenderPlanRequest {
                include_events: Some(false),
                ..window(0, 4, Layout::Linear)
            },
        )
        .unwrap();
    assert_eq!(off, without);
}

#[test]
fn the_timeout_does_not_move_the_plan() {
    let host = Host::new().unwrap();
    let code = corpus_song("pad-seam.js");
    let plain = host
        .plan_render(&code, &window(0, 20, Layout::Linear))
        .unwrap();
    let timed = host
        .plan_render(
            &code,
            &RenderPlanRequest {
                timeout_sec: Some(600.0),
                ..window(0, 20, Layout::Linear)
            },
        )
        .unwrap();
    assert_eq!(plain, timed);
    let job = host
        .plan_job_with_timeout(&code, &request(4), Some(Duration::from_secs(600)))
        .unwrap();
    assert_eq!(job, host.plan_job(&code, &request(4)).unwrap());
}

#[test]
fn a_render_does_not_see_the_previous_ones_globals() {
    let song_a = "globalThis.hostLeakX = 1;\nconst hostLeak = 2;\nregister('hostTwice', (pat) => pat.fast(2));\n$: note(\"c e\").s(\"sine\").hostTwice()\n";
    let song_b = "$: note(globalThis.hostLeakX === undefined && typeof hostLeak === 'undefined' && typeof hostTwice === 'undefined' ? 'c' : 'e').s(\"sine\")\n";
    let req = window(0, 2, Layout::Linear);
    let host = Host::new().unwrap();
    let a = host.plan_render(song_a, &req).unwrap();
    let b_after_a = host.plan_render(song_b, &req).unwrap();
    let b_fresh = Host::new().unwrap().plan_render(song_b, &req).unwrap();
    assert_ne!(a.chunks, b_fresh.chunks);
    assert_eq!(b_after_a, b_fresh);
    assert!(b_fresh.chunks[0].json.contains("\"note\":\"c\""));
}

const SPIN: &str = "while (true) {}\n$: note(\"c\").s(\"sine\")\n";

/// `result` is the 500 ms timeout, and it came back soon after the deadline.
fn assert_timeout<T: std::fmt::Debug>(what: &str, started: Instant, result: Result<T, HostError>) {
    let elapsed = started.elapsed();
    assert_eq!(
        result.map(|_| ()),
        Err(HostError::Timeout { millis: 500 }),
        "{what}"
    );
    assert!(elapsed >= Duration::from_millis(500), "{what}: {elapsed:?}");
    assert!(elapsed < Duration::from_secs(30), "{what}: {elapsed:?}");
}

#[test]
fn a_spinning_song_times_out_and_the_host_serves_the_next_request() {
    let host = Host::new().unwrap();
    let started = Instant::now();
    let result = host.plan_job_with_timeout(SPIN, &request(2), Some(Duration::from_millis(500)));
    assert_timeout("plan_job_with_timeout", started, result);
    assert!(host.plan_job(SONG, &request(2)).is_ok());

    let started = Instant::now();
    let result = host.plan_render(
        SPIN,
        &RenderPlanRequest {
            timeout_sec: Some(0.5),
            ..window(0, 2, Layout::Linear)
        },
    );
    assert_timeout("plan_render", started, result);
    assert!(
        host.plan_render(SONG, &window(0, 2, Layout::Linear))
            .is_ok()
    );

    // A spin in a microtask (after an await) is terminated in the drain.
    let later = "await Promise.resolve();\nwhile (true) {}\n";
    let started = Instant::now();
    let result = host.plan_job_with_timeout(later, &request(2), Some(Duration::from_millis(500)));
    assert_timeout("spin after an await", started, result);
    assert!(host.plan_job(SONG, &request(2)).is_ok());
}

#[test]
fn the_heap_limit_is_not_a_timeout() {
    let host = Host::new().unwrap();
    let hog = "const a = [];\nwhile (true) a.push(new Array(1e5).fill(1.5));\n";
    let result = host.plan_job_with_timeout(hog, &request(2), Some(Duration::from_secs(600)));
    match result {
        Err(HostError::Runtime(m)) => assert!(m.contains("ran out of memory"), "{m}"),
        other => panic!("{other:?}"),
    }
    let result = host.plan_render(
        hog,
        &RenderPlanRequest {
            timeout_sec: Some(600.0),
            ..window(0, 2, Layout::Linear)
        },
    );
    match result {
        Err(HostError::Runtime(m)) => assert!(m.contains("ran out of memory"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(host.plan_job(SONG, &request(2)).is_ok());
}

#[test]
fn a_waiting_song_is_unsettled_not_a_timeout() {
    // The timeout only stops running JS: a promise that never settles is still unsettled, at once.
    let host = Host::new().unwrap();
    let never = "await new Promise(() => {});\n$: note(\"c\").s(\"sine\")\n";
    let started = Instant::now();
    assert_eq!(
        host.plan_render(
            never,
            &RenderPlanRequest {
                timeout_sec: Some(600.0),
                ..window(0, 2, Layout::Linear)
            },
        ),
        Err(HostError::Unsettled)
    );
    assert!(started.elapsed() < Duration::from_secs(60));
    match host.plan_render(
        "throw new Error('boom-in-render');\n",
        &window(0, 2, Layout::Linear),
    ) {
        Err(HostError::Eval(m)) => assert!(m.contains("boom-in-render"), "{m}"),
        other => panic!("{other:?}"),
    }
}
