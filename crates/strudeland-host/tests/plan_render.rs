//! `Host::plan_render`: every chunk of a render planned in one call, equal to the single-chunk
//! plan of each chunk's request; the length requests and their flagged errors; the bad requests;
//! request isolation; the planning timeout, which the same `Host` survives.

mod common;

use std::time::{Duration, Instant};

use common::{SONG, corpus_song, job, request, single_chunk};
use strudeland_engine::{EngineJob, Value};
use strudeland_host::{
    ExportPreset, Host, HostError, Layout, LengthProblem, LengthRequest, LookbackSec, Mode,
    PlannedChunk, PlannedRender, PlannedSection, RenderPlanRequest, SingleChunk,
};

fn plan_request(length: LengthRequest) -> RenderPlanRequest {
    RenderPlanRequest {
        length,
        instruments: Default::default(),
        preset: None,
        include_events: None,
        chunk: None,
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

/// The single-chunk request of chunk `i` (`chunkRequest(base, plan)` with overlap 0).
fn chunk_request(planned: &PlannedRender, i: usize) -> RenderPlanRequest {
    let plan = &planned.chunks[i].plan;
    single_chunk(
        plan.from_bar,
        plan.to_bar,
        planned.window.layout,
        SingleChunk {
            overlap_bars: 0,
            lookback_sec: LookbackSec::Seconds(plan.lookback_sec),
            mode: Mode::Mix,
        },
    )
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
fn chunks_equal_the_single_chunk_plan_of_each_chunk_request() {
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
                .plan_render(&code, &chunk_request(&planned, i))
                .unwrap_or_else(|e| panic!("{name} chunk {i}: {e}"));
            assert!(
                job(&single) == planned.chunks[i].json,
                "{name} chunk {i}: the batch job differs from the single-chunk plan's"
            );
            let plan = &planned.chunks[i].plan;
            // Every field, `lookback_sec_used` and `lookback_sec` bit for bit (`PartialEq` on f64).
            assert_eq!(&single.chunks[0].plan, plan, "{name} chunk {i}");
            assert_eq!(
                single.chunks[0].plan.lookback_sec_used.to_bits(),
                plan.lookback_sec_used.to_bits(),
                "{name} chunk {i}"
            );
            assert!(single.chunks[0].info.is_some(), "{name} chunk {i}: info");
            assert!(planned.chunks[i].info.is_none(), "{name} chunk {i}: info");
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
        let has_intro = matches!(preset, ExportPreset::IntroLoop | ExportPreset::TaggedLoop);
        assert_eq!(export.tail_frames == 0, has_intro);
        assert_eq!(export.repeat.is_some(), has_intro);
        if let Some(repeat) = export.repeat {
            let first = &repeat.chunks.first().unwrap().plan;
            let last = &repeat.chunks.last().unwrap().plan;
            assert_eq!((first.c0, first.from_bar, last.to_bar), (2, 2, 6));
            assert!(repeat.tail_frames > 0);
            assert_eq!(
                last.frames - last.window_start_frame,
                u64::from(last.to_bar - last.from_bar) * 88_200 + repeat.tail_frames
            );
        }
    }
}

#[test]
fn loop_passes_reuse_the_same_notes_for_patterns_that_depend_on_the_query_start() {
    let code = r#"
const meta = {
  bpm: 120,
  sections: [{ name: 'intro', bars: 1 }, { name: 'body', bars: 1, loop: true }],
};
"<220 440>".stepBind(x => freq(x)).s('sine')
"#;
    let planned = Host::new()
        .unwrap()
        .plan_render(
            code,
            &RenderPlanRequest {
                preset: Some(ExportPreset::TaggedLoop),
                ..plan_request(LengthRequest::Clip { loops: None })
            },
        )
        .unwrap();
    assert!(planned.errors.is_empty());
    let body_frequencies = |chunks: &[PlannedChunk]| {
        let job = EngineJob::from_json(&chunks[0].json).unwrap();
        job.pauses
            .iter()
            .flat_map(|pause| &pause.haps)
            .filter(|hap| hap.cycle == 1.0)
            .map(|hap| job.values[hap.v as usize]["freq"].clone())
            .collect::<Vec<_>>()
    };
    let opening = body_frequencies(&planned.chunks);
    let repeat = planned.export.unwrap().repeat.unwrap();
    assert_eq!(opening, [Value::Num(220.0)]);
    assert_eq!(body_frequencies(&repeat.chunks), opening);
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
fn sections_name_the_layout_bars_of_a_real_plan() {
    let host = Host::new().unwrap();
    let song = corpus_song("song-linear.js");
    let runs = |length: LengthRequest| {
        host.plan_render(&song, &plan_request(length.clone()))
            .unwrap_or_else(|e| panic!("{length:?}: {e}"))
            .sections
    };
    let run = |from_bar: u32, to_bar: u32, name: &str| PlannedSection {
        from_bar,
        to_bar,
        name: name.to_string(),
    };
    assert_eq!(
        runs(LengthRequest::Clip { loops: None }),
        [run(0, 2, "intro"), run(2, 10, "body"), run(10, 12, "tail")]
    );
    // The lookback bars before the section's first bar are named too.
    assert_eq!(
        runs(LengthRequest::Section {
            name: "body".into()
        }),
        [run(0, 2, "intro"), run(2, 6, "body")]
    );
}

#[test]
fn length_errors_are_meta_with_their_problem() {
    let host = Host::new().unwrap();
    let sections = corpus_song("song-linear.js");
    let plain = corpus_song("sine-basic.js");
    let n = "note('c').s('sine')";
    let long_loop = |loops: &str| {
        format!(
            "const meta = {{ bpm: 120, {loops}sections: [{{ name: 'a', bars: 2 }}, \
             {{ name: 'b', bars: 10000, loop: true }}] }};\n\
             song(meta, {{ a: {n}, b: {n} }})\n"
        )
    };
    // Eleven 10000-bar sections, `last` on the last one's entry, `tail` after the sections.
    let eleven = |last: &str, tail: &str| {
        let secs: Vec<String> = (0..11)
            .map(|i| {
                let extra = if i == 10 { last } else { "" };
                format!("{{ name: 's{i}', bars: 10000{extra} }}")
            })
            .collect();
        let parts: Vec<String> = (0..11).map(|i| format!("s{i}: {n}")).collect();
        format!(
            "const meta = {{ bpm: 120, sections: [{}]{tail} }};\nsong(meta, {{ {} }})\n",
            secs.join(", "),
            parts.join(", ")
        )
    };
    let looped = long_loop("");
    let meta_loops = long_loop("loops: 10, ");
    let long_plain = eleven("", "");
    let last_loop = eleven(", loop: true", "");
    let range = eleven("", ", loop: { from: 's9', through: 's10' }");
    let preset = |length: LengthRequest| RenderPlanRequest {
        preset: Some(ExportPreset::TaggedLoop),
        ..plan_request(length)
    };
    let clip = LengthRequest::Clip { loops: None };
    let cases = [
        (
            plain.as_str(),
            plan_request(clip.clone()),
            LengthProblem::NoSections,
            "this song has no sections; pass --length N to say how many bars to render",
        ),
        (
            plain.as_str(),
            plan_request(LengthRequest::Section { name: "a".into() }),
            LengthProblem::SectionWithoutSections,
            "this song has no sections; --section needs meta.sections",
        ),
        (
            sections.as_str(),
            plan_request(LengthRequest::Section {
                name: "drop".into(),
            }),
            LengthProblem::UnknownSection {
                name: "drop".into(),
                sections: vec!["intro".into(), "body".into(), "tail".into()],
            },
            "unknown section 'drop' (sections: intro, body, tail)",
        ),
        (
            sections.as_str(),
            plan_request(LengthRequest::Length { bars: 13 }),
            LengthProblem::LengthPastEnd {
                bars: 13,
                clip_bars: 12,
            },
            "--length 13 is past the end of the song (12 bars)",
        ),
        (
            plain.as_str(),
            plan_request(LengthRequest::Clip { loops: Some(2) }),
            LengthProblem::LoopsWithoutLoop,
            "--loops needs meta.loop or a section with loop: true in meta.sections",
        ),
        (
            sections.as_str(),
            preset(LengthRequest::Length { bars: 2 }),
            LengthProblem::PresetWithLength,
            "loop presets cannot be combined with --section, --length or --loops",
        ),
        (
            looped.as_str(),
            plan_request(LengthRequest::Clip { loops: Some(1000) }),
            LengthProblem::LoopsPastLimit {
                loops: 1000,
                clip_bars: 10000002,
                limit: 100000,
            },
            "--loops 1000 makes the clip 10000002 bars, past the render limit of 100000 bars",
        ),
        (
            meta_loops.as_str(),
            plan_request(clip.clone()),
            LengthProblem::ClipPastLimit {
                clip_bars: 100002,
                limit: 100000,
                has_loop: true,
            },
            "the song's clip is 100002 bars, past the render limit of 100000 bars; pass --length N or --loops N",
        ),
        (
            long_plain.as_str(),
            plan_request(clip.clone()),
            LengthProblem::ClipPastLimit {
                clip_bars: 110000,
                limit: 100000,
                has_loop: false,
            },
            "the song's clip is 110000 bars, past the render limit of 100000 bars; pass --length N",
        ),
        (
            long_plain.as_str(),
            plan_request(LengthRequest::Section { name: "s10".into() }),
            LengthProblem::SectionPastLimit {
                name: "s10".into(),
                to_bar: 110000,
                limit: 100000,
            },
            "section 's10' ends at bar 110000, past the render limit of 100000 bars",
        ),
        (
            last_loop.as_str(),
            preset(clip.clone()),
            LengthProblem::SectionPastLimit {
                name: "s10".into(),
                to_bar: 110000,
                limit: 100000,
            },
            "section 's10' ends at bar 110000, past the render limit of 100000 bars",
        ),
        (
            plain.as_str(),
            preset(clip.clone()),
            LengthProblem::PresetWithoutLoop,
            "--preset needs meta.loop or a section with loop: true in meta.sections",
        ),
        (
            range.as_str(),
            preset(clip.clone()),
            LengthProblem::LoopPastLimit {
                end: 110000,
                limit: 100000,
            },
            "the loop ends at bar 110000, past the render limit of 100000 bars",
        ),
    ];
    for (code, request, want, message) in cases {
        match host.plan_render(code, &request) {
            Err(HostError::MetaFlag {
                problem,
                message: m,
            }) => {
                assert_eq!(problem, Some(want), "{request:?}");
                assert_eq!(m, message, "{request:?}");
            }
            other => panic!("{request:?}: {other:?}"),
        }
    }
    // A bad meta header is a plain meta error, without a length problem.
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
    let mut bad_pin = window(0, 4, Layout::Linear);
    let definition = serde_json::json!({"schema":1,"parameters":{},"voices":[
        {"name":"tone","chain":[{"control":"s","value":"sine"}]}
    ]});
    bad_pin.instruments.insert(
        "../lead".into(),
        serde_json::from_value(definition).unwrap(),
    );
    let cases = [
        bad_pin,
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
}

#[test]
fn a_bad_length_request_is_the_bundles_bad_request() {
    let host = Host::new().unwrap();
    let cases = [
        window(4, 4, Layout::Linear),
        window(0, 100_001, Layout::Linear),
        plan_request(LengthRequest::Length { bars: 0 }),
        plan_request(LengthRequest::Length { bars: 100_001 }),
        plan_request(LengthRequest::Clip { loops: Some(0) }),
        plan_request(LengthRequest::Clip { loops: Some(1001) }),
        // A single chunk: the window, its overlap past the limit, a lookback that is not
        // finite and >= 0 (serde writes NaN as `null`), a chunk with a preset or without a window.
        RenderPlanRequest {
            length: LengthRequest::Window {
                from_bar: 4,
                to_bar: 4,
                layout: Layout::Linear,
            },
            ..request(4)
        },
        RenderPlanRequest {
            length: LengthRequest::Window {
                from_bar: 0,
                to_bar: 200_000,
                layout: Layout::Linear,
            },
            ..request(4)
        },
        RenderPlanRequest {
            chunk: Some(SingleChunk {
                overlap_bars: 99_997,
                lookback_sec: LookbackSec::Seconds(0.0),
                mode: Mode::Mix,
            }),
            ..request(4)
        },
        RenderPlanRequest {
            chunk: Some(SingleChunk {
                overlap_bars: 0,
                lookback_sec: LookbackSec::Seconds(f64::NAN),
                mode: Mode::Mix,
            }),
            ..request(4)
        },
        RenderPlanRequest {
            chunk: Some(SingleChunk {
                overlap_bars: 0,
                lookback_sec: LookbackSec::Seconds(-1.0),
                mode: Mode::Mix,
            }),
            ..request(4)
        },
        RenderPlanRequest {
            preset: Some(ExportPreset::TaggedLoop),
            ..request(4)
        },
        RenderPlanRequest {
            length: LengthRequest::Length { bars: 4 },
            ..request(4)
        },
    ];
    for req in cases {
        match host.plan_render(SONG, &req) {
            Err(HostError::Plan(m)) => assert!(m.starts_with("bad request: "), "{req:?}: {m}"),
            other => panic!("{req:?}: {other:?}"),
        }
    }
    assert_eq!(
        host.plan_render(SONG, &plan_request(LengthRequest::Clip { loops: Some(0) })),
        Err(HostError::Plan(
            "bad request: --loops must be an integer from 1 to 1000".into()
        ))
    );
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
    let single = host
        .plan_render(
            &code,
            &RenderPlanRequest {
                timeout_sec: Some(600.0),
                ..request(4)
            },
        )
        .unwrap();
    assert_eq!(single, host.plan_render(&code, &request(4)).unwrap());
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
    let result = host.plan_render(
        later,
        &RenderPlanRequest {
            timeout_sec: Some(0.5),
            ..request(2)
        },
    );
    assert_timeout("spin after an await", started, result);
    assert!(host.plan_render(SONG, &request(2)).is_ok());
}

#[test]
fn the_heap_limit_is_not_a_timeout() {
    let host = Host::new().unwrap();
    let hog = "const a = [];\nwhile (true) a.push(new Array(1e5).fill(1.5));\n";
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
    assert!(host.plan_render(SONG, &request(2)).is_ok());
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
    // A song waiting on a timer: timers never fire, so it never settles.
    let timer =
        "await new Promise((resolve) => setTimeout(resolve, 10));\n$: note(\"c\").s(\"sine\")\n";
    assert_eq!(
        host.plan_render(timer, &request(2)),
        Err(HostError::Unsettled)
    );
    assert_eq!(
        host.query(timer, 0.0, 1.0, Layout::Linear),
        Err(HostError::Unsettled)
    );
    // The host still serves the next request.
    assert!(host.plan_render(SONG, &request(2)).is_ok());
    match host.plan_render(
        "throw new Error('boom-in-render');\n",
        &window(0, 2, Layout::Linear),
    ) {
        Err(HostError::Eval(m)) => assert!(m.contains("boom-in-render"), "{m}"),
        other => panic!("{other:?}"),
    }
}
