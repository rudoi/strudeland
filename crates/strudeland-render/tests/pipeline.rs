//! The whole pipeline on corpus songs: plan through the host, render on the worker pool, join,
//! and a 32-bit float WAV; the length requests; the refusal of unsupported material.

mod common;

use common::{Recorder, corpus_song, options, window};
use strudeland_engine::wav::read_f32;
use strudeland_host::{HostError, Layout, LengthRequest};
use strudeland_render::encode::encode_once;
use strudeland_render::encode::verify::decode;
use strudeland_render::join::window_frames;
use strudeland_render::level::{integrated_lufs, true_peak_db};
use strudeland_render::{
    BitDepth, EncoderOptions, ExitClass, Format, Loudness, OutputOptions, OutputTarget, Progress,
    RenderError, RenderOptions, finish, render, render_audio, write_output,
};

/// `Planned` first, then one `ChunkRendered` per chunk with distinct indices, then the rest.
fn assert_progress(events: &[Progress], chunks: usize, tail: &[Progress]) {
    assert_eq!(
        events.first(),
        Some(&Progress::Planned { chunks }),
        "{events:?}"
    );
    let mut indices: Vec<usize> = events
        .iter()
        .skip(1)
        .take(chunks)
        .map(|p| match p {
            Progress::ChunkRendered { index, of } => {
                assert_eq!(*of, chunks, "{events:?}");
                *index
            }
            other => panic!("expected ChunkRendered, got {other:?}"),
        })
        .collect();
    indices.sort_unstable();
    assert_eq!(indices, (0..chunks).collect::<Vec<_>>(), "{events:?}");
    assert_eq!(&events[1 + chunks..], tail, "{events:?}");
}

#[test]
fn sine_basic_renders_to_a_float_wav_of_the_exact_length() {
    let code = corpus_song("sine-basic.js");
    let opts = options(LengthRequest::Length { bars: 4 });
    let rec = Recorder::new();
    let out = render(&code, &opts, &|p| rec.record(p)).unwrap();
    let frames = window_frames(0, 4, 0.5);
    assert_eq!(frames, 352_800);
    assert_eq!(out.summary.frames, frames);
    assert_eq!(out.summary.seconds, 8.0);
    assert_eq!(out.summary.sample_rate, 44_100);
    assert_eq!(out.summary.format, Some(Format::Wav));
    assert_eq!(out.summary.bytes, Some(out.bytes.len() as u64));
    assert_eq!(
        (
            out.summary.window.from_bar,
            out.summary.window.to_bar,
            out.summary.window.layout
        ),
        (0, 4, Layout::Linear)
    );
    assert!(out.summary.jobs >= 1 && out.summary.jobs <= out.summary.chunks);
    assert_eq!(out.summary.warnings, Vec::<String>::new());
    let wav = read_f32(&out.bytes).unwrap();
    assert_eq!(wav.sample_rate, 44_100);
    assert_eq!(wav.channels.len(), 2);
    assert!(wav.channels.iter().all(|c| c.len() as u64 == frames));
    assert!(
        wav.channels.iter().flatten().any(|&x| x != 0.0),
        "the render is silent"
    );
    assert!(wav.channels.iter().flatten().all(|x| x.is_finite()));
    assert_progress(
        &rec.events(),
        out.summary.chunks,
        &[Progress::Encoded { pass: 1 }],
    );
}

#[test]
fn the_float_wav_holds_the_joined_audio_untouched() {
    let code = corpus_song("sine-basic.js");
    let opts = options(window(0, 2, Layout::Linear));
    let audio = render_audio(&code, &opts, &|_| {}).unwrap();
    let out = finish(&audio, &opts.output, &|_| {}).unwrap();
    let wav = read_f32(&out.bytes).unwrap();
    let bits = |c: &[Vec<f32>]| -> Vec<Vec<u32>> {
        c.iter()
            .map(|ch| ch.iter().map(|x| x.to_bits()).collect())
            .collect()
    };
    assert_eq!(bits(&wav.channels), bits(&audio.channels));
    assert_eq!(audio.channels[0].len(), 176_400);
    // `finish` again on the same audio gives the same bytes.
    assert_eq!(
        finish(&audio, &opts.output, &|_| {}).unwrap().bytes,
        out.bytes
    );
}

#[test]
fn length_requests_on_song_linear() {
    let code = corpus_song("song-linear.js");
    let cases = [
        (
            "clip",
            LengthRequest::Clip { loops: None },
            (0, 12, Layout::Clip),
        ),
        (
            "section body",
            LengthRequest::Section {
                name: "body".into(),
            },
            (2, 6, Layout::Linear),
        ),
        (
            "length 5",
            LengthRequest::Length { bars: 5 },
            (0, 5, Layout::Clip),
        ),
        (
            "loops 3",
            LengthRequest::Clip { loops: Some(3) },
            (0, 16, Layout::Clip),
        ),
    ];
    for (name, length, (from, to, layout)) in cases {
        let opts = RenderOptions {
            jobs: Some(2),
            ..options(length)
        };
        let rec = Recorder::new();
        let audio = render_audio(&code, &opts, &|p| rec.record(p))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            (
                audio.summary.window.from_bar,
                audio.summary.window.to_bar,
                audio.summary.window.layout
            ),
            (from, to, layout),
            "{name}"
        );
        let frames = window_frames(from, to, 0.5);
        assert_eq!(audio.summary.frames, frames, "{name}");
        assert_eq!(audio.channels.len(), 2, "{name}");
        assert!(
            audio.channels.iter().all(|c| c.len() as u64 == frames),
            "{name}"
        );
        assert_eq!(audio.summary.chunks, audio.chunks.len(), "{name}");
        assert_eq!(audio.summary.jobs, 2.min(audio.chunks.len()), "{name}");
        assert_eq!(audio.cps, 0.5, "{name}");
        assert_eq!(audio.structure.sections.len(), 3, "{name}");
        assert_eq!(audio.events, None, "{name}");
        assert_progress(&rec.events(), audio.chunks.len(), &[]);
    }
}

#[test]
fn events_only_when_asked() {
    let code = corpus_song("sine-basic.js");
    let opts = RenderOptions {
        include_events: true,
        ..options(window(0, 1, Layout::Linear))
    };
    let audio = render_audio(&code, &opts, &|_| {}).unwrap();
    let events = audio.events.as_ref().and_then(|e| e.as_array()).unwrap();
    assert!(!events.is_empty());
    assert!(events.iter().all(|e| e["s"] == "sine"));
}

#[test]
fn jobs_are_clamped_to_the_chunks_and_zero_is_a_usage_error() {
    let code = corpus_song("sine-basic.js");
    let opts = RenderOptions {
        jobs: Some(64),
        ..options(window(0, 2, Layout::Linear))
    };
    let audio = render_audio(&code, &opts, &|_| {}).unwrap();
    assert_eq!(audio.summary.chunks, 1);
    assert_eq!(audio.summary.jobs, 1);

    let rec = Recorder::new();
    let opts = RenderOptions {
        jobs: Some(0),
        ..options(window(0, 2, Layout::Linear))
    };
    let e = render(&code, &opts, &|p| rec.record(p)).unwrap_err();
    assert!(
        matches!(e, RenderError::Usage { flag: "--jobs", .. }),
        "{e:?}"
    );
    assert_eq!(e.exit_class(), ExitClass::Usage);
    // Rejected before planning: no progress at all.
    assert_eq!(rec.events(), vec![]);
}

#[test]
fn a_multi_chunk_render_with_several_workers_equals_one_worker() {
    let code = corpus_song("sine-basic.js");
    let one = render_audio(
        &code,
        &RenderOptions {
            jobs: Some(1),
            ..options(window(0, 20, Layout::Linear))
        },
        &|_| {},
    )
    .unwrap();
    assert!(one.chunks.len() >= 2, "{} chunks", one.chunks.len());
    let four = render_audio(
        &code,
        &RenderOptions {
            jobs: Some(4),
            ..options(window(0, 20, Layout::Linear))
        },
        &|_| {},
    )
    .unwrap();
    assert_eq!(four.summary.jobs, 4.min(four.chunks.len()));
    assert!(
        one.channels == four.channels,
        "the worker count changed the audio"
    );
    assert_eq!(one.channels[0].len() as u64, window_frames(0, 20, 0.5));
}

#[test]
fn samples_are_refused_and_nothing_is_rendered() {
    let code = corpus_song("samples.js");
    let rec = Recorder::new();
    let e = render(&code, &options(window(0, 2, Layout::Linear)), &|p| {
        rec.record(p)
    })
    .unwrap_err();
    let RenderError::Unsupported(list) = &e else {
        panic!("{e:?}");
    };
    let sounds: Vec<&str> = list.iter().map(|u| u.s.as_str()).collect();
    assert_eq!(sounds, vec!["hat", "kick", "tone"]);
    assert!(
        list.iter()
            .all(|u| u.layout_bar == 0 && u.section.is_none())
    );
    assert_eq!(
        e.to_string(),
        "the song uses sounds or controls the engine cannot play yet; nothing was written:\n  s=hat first at bar 1\n  s=kick first at bar 1\n  s=tone first at bar 1"
    );
    assert_eq!(e.exit_class(), ExitClass::Failed);
    // Planned, then refused: no chunk was rendered.
    assert!(matches!(rec.events()[..], [Progress::Planned { .. }]));
}

/// An unsupported voice in the intro whose release rings into the body: a `--section body`
/// render plans the intro's haps in its lookback, so the engine would play them; refused.
const LOOKBACK_SONG: &str = r#"const meta = {
  bpm: 120,
  meter: '4/4',
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 2 },
  ],
};

$: song(meta, {
  intro: s("zzfx").release(4),
  body: note("c3").s('sine'),
})
"#;

#[test]
fn an_unsupported_hap_in_the_lookback_refuses_a_section() {
    let opts = options(LengthRequest::Section {
        name: "body".into(),
    });
    let e = render_audio(LOOKBACK_SONG, &opts, &|_| {}).unwrap_err();
    let RenderError::Unsupported(list) = &e else {
        panic!("{e:?}");
    };
    assert_eq!(list.len(), 1, "{e}");
    assert_eq!(list[0].s, "zzfx");
    assert!(list[0].layout_bar < 2, "{e}");
    assert_eq!(list[0].section.as_deref(), Some("intro"));
    assert!(
        e.to_string().ends_with(&format!(
            "first at bar {} (section 'intro')",
            list[0].layout_bar + 1
        )),
        "{e}"
    );
    // The clip's intro bars name the intro too.
    let e = render_audio(
        LOOKBACK_SONG,
        &options(LengthRequest::Clip { loops: None }),
        &|_| {},
    )
    .unwrap_err();
    assert_eq!(
        e.to_string(),
        "the song uses sounds or controls the engine cannot play yet; nothing was written:\n  s=zzfx first at bar 1 (section 'intro')"
    );
}

#[test]
fn length_errors_are_usage_errors() {
    let code = corpus_song("sine-basic.js");
    let e = render_audio(
        &code,
        &options(LengthRequest::Clip { loops: None }),
        &|_| {},
    )
    .unwrap_err();
    assert!(
        matches!(&e, RenderError::Plan(HostError::MetaFlag { flag, .. }) if flag == "--length"),
        "{e:?}"
    );
    assert_eq!(e.exit_class(), ExitClass::Usage);
    // The message alone, without the `meta:` prefix.
    assert_eq!(
        e.to_string(),
        "this song has no sections; pass --length N to say how many bars to render"
    );
}

#[test]
fn a_clip_past_the_render_limit_is_a_usage_error_not_an_abort() {
    // A 10000-bar loop section: --loops 1000 asks for about ten million bars.
    let code = "const meta = { bpm: 120, sections: [{ name: 'a', bars: 2 }, \
                { name: 'b', bars: 10000, loop: true }] };\n\
                song(meta, { a: note('c').s('sine'), b: note('e').s('sine') })\n";
    let e = render_audio(
        code,
        &options(LengthRequest::Clip { loops: Some(1000) }),
        &|_| {},
    )
    .unwrap_err();
    assert!(
        matches!(&e, RenderError::Plan(HostError::MetaFlag { flag, .. }) if flag == "--loops"),
        "{e:?}"
    );
    assert_eq!(e.exit_class(), ExitClass::Usage);
    assert_eq!(
        e.to_string(),
        "--loops 1000 makes the clip 10000002 bars, past the render limit of 100000 bars"
    );
}

#[test]
fn eval_errors_fail_the_render() {
    let e = render_audio(
        "$: note(",
        &options(LengthRequest::Length { bars: 1 }),
        &|_| {},
    )
    .unwrap_err();
    assert!(matches!(&e, RenderError::Plan(HostError::Eval(_))), "{e:?}");
    assert_eq!(e.exit_class(), ExitClass::Failed);
}

/// A 24-bit PCM WAV's samples as planar floats (test-local reader).
fn read_pcm24(b: &[u8]) -> Vec<Vec<f32>> {
    assert_eq!(&b[0..4], b"RIFF");
    assert_eq!(u16::from_le_bytes([b[34], b[35]]), 24);
    let channels = usize::from(u16::from_le_bytes([b[22], b[23]]));
    let data = u32::from_le_bytes([b[40], b[41], b[42], b[43]]) as usize;
    let mut out = vec![Vec::new(); channels];
    for (i, s) in b[44..44 + data].chunks_exact(3).enumerate() {
        let v = i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8;
        out[i % channels].push(v as f32 / 8_388_608.0);
    }
    out
}

#[test]
fn every_format_end_to_end_with_the_default_loudness() {
    let code = corpus_song("sine-basic.js");
    let base = options(LengthRequest::Length { bars: 4 });
    let audio = render_audio(&code, &base, &|_| {}).unwrap();
    for format in Format::ALL {
        let output = OutputOptions {
            loudness: Loudness::default(),
            encoder: EncoderOptions::default_for(format),
        };
        let rec = Recorder::new();
        let out = finish(&audio, &output, &|p| rec.record(p)).unwrap();
        let l = out.summary.loudness.as_ref().unwrap();
        assert_eq!(out.summary.format, Some(format));
        assert_eq!(out.summary.bytes, Some(out.bytes.len() as u64));
        assert!(l.enabled);
        assert_eq!(l.target_lufs, Some(-23.0));
        assert_eq!(l.ceiling_dbtp, Some(-1.0));
        assert!((l.output_lufs + 23.0).abs() <= 0.1, "{format}: {l:?}");
        assert!(l.output_true_peak_db <= -1.0, "{format}: {l:?}");
        assert!((1..=3).contains(&l.encodes), "{format}: {l:?}");
        let passes: Vec<Progress> = (1..=l.encodes)
            .map(|pass| Progress::Encoded { pass })
            .collect();
        assert_eq!(rec.events(), passes, "{format}");
        assert!(
            out.summary
                .warnings
                .iter()
                .all(|w| !w.starts_with("lossy encode")),
            "{format}: {:?}",
            out.summary.warnings
        );
        if format == Format::Wav {
            assert_eq!(l.decoded_true_peak_db, None);
            assert_eq!(l.encodes, 1);
            // The file itself measures -23 LUFS within 0.1 LU.
            let pcm = read_pcm24(&out.bytes);
            assert_eq!(pcm[0].len() as u64, out.summary.frames);
            let lufs = integrated_lufs(&pcm, 44_100);
            assert!((lufs + 23.0).abs() <= 0.1, "wav measures {lufs} LUFS");
        } else {
            let dp = l.decoded_true_peak_db.unwrap();
            assert!(dp <= -1.0, "{format}: decoded true peak {dp}");
            let decoded = decode(format, &out.bytes).unwrap();
            assert_eq!(true_peak_db(&decoded.channels), dp, "{format}");
        }
    }
}

#[test]
fn the_summary_holds_the_loudness_and_timings() {
    let code = corpus_song("sine-basic.js");
    let opts = RenderOptions {
        output: OutputOptions {
            loudness: Loudness::default(),
            encoder: EncoderOptions::default_for(Format::Ogg),
        },
        ..options(window(0, 2, Layout::Linear))
    };
    let audio = render_audio(&code, &opts, &|_| {}).unwrap();
    assert_eq!(audio.summary.loudness, None);
    assert_eq!(audio.summary.format, None);
    let out = finish(&audio, &opts.output, &|_| {}).unwrap();
    let s = &out.summary;
    let l = s.loudness.as_ref().unwrap();
    assert!(
        s.timings.level_ms > 0.0 && s.timings.encode_ms > 0.0,
        "{:?}",
        s.timings
    );
    assert_eq!(s.timings.plan_ms, audio.summary.timings.plan_ms);
    assert_eq!(s.timings.render_ms, audio.summary.timings.render_ms);
    assert_eq!(
        s.timings.total_ms,
        audio.summary.timings.total_ms + (s.timings.level_ms + s.timings.encode_ms)
    );
    // The sine is louder than the default target, so it is turned down, never limited: before
    // and after, both measured.
    assert!(l.input_lufs > -23.0, "{l:?}");
    assert_eq!(l.gain_db, -23.0 - l.input_lufs, "{l:?}");
    assert_eq!(l.limiter_samples, 0, "{l:?}");
    assert_eq!(l.encodes, 1, "{l:?}");
    assert_eq!(l.limiter_ceiling_dbtp, Some(-1.0), "{l:?}");
    assert!(l.output_true_peak_db < l.input_true_peak_db, "{l:?}");

    let json: serde_json::Value = serde_json::to_value(s).unwrap();
    let keys =
        |v: &serde_json::Value| -> Vec<String> { v.as_object().unwrap().keys().cloned().collect() };
    let mut want = vec![
        "format",
        "bytes",
        "window",
        "frames",
        "seconds",
        "sampleRate",
        "chunks",
        "jobs",
        "loudness",
        "timings",
        "warnings",
    ];
    let mut got = keys(&json);
    got.sort();
    want.sort();
    assert_eq!(got, want);
    let mut got = keys(&json["loudness"]);
    got.sort();
    let mut want = vec![
        "enabled",
        "targetLufs",
        "ceilingDbtp",
        "limiterCeilingDbtp",
        "inputLufs",
        "inputTruePeakDb",
        "gainDb",
        "limiterSamples",
        "outputLufs",
        "outputTruePeakDb",
        "decodedTruePeakDb",
        "encodes",
    ];
    want.sort();
    assert_eq!(got, want);
    let mut got = keys(&json["timings"]);
    got.sort();
    assert_eq!(
        got,
        vec!["encodeMs", "levelMs", "planMs", "renderMs", "totalMs"]
    );
    assert_eq!(json["format"], "ogg");
    assert_eq!(json["loudness"]["enabled"], true);
    assert_eq!(json["loudness"]["targetLufs"], -23.0);
    assert_eq!(json["loudness"]["ceilingDbtp"], -1.0);
    assert_eq!(
        json["loudness"]["limiterCeilingDbtp"],
        l.limiter_ceiling_dbtp.unwrap()
    );
    assert_eq!(json["loudness"]["encodes"], l.encodes);
    assert_eq!(
        json["loudness"]["decodedTruePeakDb"],
        l.decoded_true_peak_db.unwrap()
    );
}

#[test]
fn loudness_off_reports_the_measures_and_leaves_the_audio_untouched() {
    let code = corpus_song("sine-basic.js");
    let opts = options(window(0, 2, Layout::Linear));
    let audio = render_audio(&code, &opts, &|_| {}).unwrap();
    for encoder in [
        EncoderOptions::Wav {
            bit_depth: BitDepth::F32,
        },
        EncoderOptions::default_for(Format::Mp3),
    ] {
        let output = OutputOptions {
            loudness: Loudness::Off,
            encoder,
        };
        let out = finish(&audio, &output, &|_| {}).unwrap();
        let l = out.summary.loudness.as_ref().unwrap();
        assert!(!l.enabled);
        assert_eq!(
            (l.target_lufs, l.ceiling_dbtp, l.limiter_ceiling_dbtp),
            (None, None, None)
        );
        assert_eq!(l.gain_db, 0.0);
        assert_eq!(l.limiter_samples, 0);
        assert_eq!(l.encodes, 1);
        assert_eq!(l.input_lufs, l.output_lufs);
        assert_eq!(l.input_lufs, integrated_lufs(&audio.channels, 44_100));
        assert_eq!(
            out.bytes,
            encode_once(&audio.channels, 44_100, &encoder).unwrap()
        );
        if encoder.format() == Format::Wav {
            assert_eq!(l.decoded_true_peak_db, None);
        } else {
            assert_eq!(
                l.decoded_true_peak_db,
                Some(true_peak_db(
                    &decode(Format::Mp3, &out.bytes).unwrap().channels
                ))
            );
        }
    }
}

#[test]
fn a_rendered_file_is_written_and_overwritten() {
    let code = corpus_song("sine-basic.js");
    let opts = RenderOptions {
        output: OutputOptions {
            loudness: Loudness::default(),
            encoder: EncoderOptions::default_for(Format::Opus),
        },
        ..options(window(0, 1, Layout::Linear))
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.opus");
    std::fs::write(&path, b"old").unwrap();
    let rec = Recorder::new();
    let out = render(&code, &opts, &|p| rec.record(p)).unwrap();
    let written = write_output(&out, &OutputTarget::file(&path), &|p| rec.record(p)).unwrap();
    assert_eq!(written, path);
    assert_eq!(std::fs::read(&path).unwrap(), out.bytes);
    assert_eq!(
        rec.events().last(),
        Some(&Progress::Written { path: path.clone() })
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

const GAME_LOOP: &str = r#"
const meta = {
  bpm: 137, meter: '7/8', loops: 3,
  sections: [
    { name: 'intro', bars: 1 },
    { name: 'body', bars: 1, loop: true },
    { name: 'ending', bars: 1 },
  ],
};
song(meta, {
  intro: s('sine').freq(220).gain(0.25).room(0.15).roomsize(0.5),
  body: s('sine').freq(440).gain(0.12).release(0.15),
  ending: s('square').freq(880).gain(0.5),
})
"#;

#[test]
fn split_intro_plays_once_and_the_body_can_repeat_without_the_intro_or_ending() {
    use strudeland_render::{ExportPreset, ExportRole, render_export};
    let mut opts = options(LengthRequest::Clip { loops: None });
    opts.jobs = Some(2);
    let recorder = Recorder::new();
    let files = render_export(GAME_LOOP, &opts, ExportPreset::IntroLoop, &|p| {
        recorder.record(p)
    })
    .unwrap();
    assert_progress(
        &recorder.events(),
        files[0].output.summary.chunks,
        &[Progress::Encoded { pass: 1 }],
    );
    let [intro, body] = files.as_slice() else {
        panic!("expected an intro and body")
    };
    assert_eq!(
        (intro.role, body.role),
        (ExportRole::Intro, ExportRole::Loop)
    );
    assert_eq!(intro.loop_points, None);
    let intro = read_f32(&intro.output.bytes).unwrap();
    let body_wav = read_f32(&body.output.bytes).unwrap();
    let cps = (137.0 / 60.0) / 3.5;
    let intro_frames = window_frames(0, 1, cps) as usize;
    let loop_frames = window_frames(1, 2, cps) as usize;
    assert_eq!(intro.channels[0].len(), intro_frames + loop_frames);
    assert_eq!(body_wav.channels[0].len(), loop_frames);
    let points = body.loop_points.unwrap();
    assert_eq!(
        (points.start_frame, points.length_frames),
        (0, loop_frames as u64)
    );

    let altered = GAME_LOOP
        .replace("freq(220)", "freq(660)")
        .replace("freq(880)", "freq(990)");
    opts.jobs = Some(1);
    let other = render_export(&altered, &opts, ExportPreset::IntroLoop, &|_| {}).unwrap();
    assert_ne!(files[0].output.bytes, other[0].output.bytes);
    assert_eq!(files[1].output.bytes, other[1].output.bytes);
    let other_intro = read_f32(&other[0].output.bytes).unwrap();
    assert_ne!(
        intro.channels[0][intro_frames..intro_frames + 1024],
        other_intro.channels[0][intro_frames..intro_frames + 1024],
        "intro effects should carry into the first pass"
    );

    let folded = render_export(GAME_LOOP, &opts, ExportPreset::FoldedLoop, &|_| {}).unwrap();
    let tagged = render_export(GAME_LOOP, &opts, ExportPreset::TaggedLoop, &|_| {}).unwrap();
    for (files, first_frames) in [
        (&folded, loop_frames),
        (&tagged, intro_frames + loop_frames),
    ] {
        let file = &files[0];
        let points = file.loop_points.unwrap();
        let wav = read_f32(&file.output.bytes).unwrap();
        assert_eq!(points.start_frame, first_frames as u64);
        assert_eq!(points.length_frames, loop_frames as u64);
        assert_eq!(wav.channels[0].len(), first_frames + loop_frames);
        for (channel, repeat) in wav.channels.iter().zip(&body_wav.channels) {
            assert_eq!(&channel[first_frames..], repeat);
            assert_eq!(channel[first_frames - 1], *repeat.last().unwrap());
        }
    }
    let playback: Vec<f32> = intro.channels[0]
        .iter()
        .chain(&body_wav.channels[0])
        .chain(&body_wav.channels[0])
        .copied()
        .collect();
    let frequency_at = |start: usize| {
        let frames = &playback[start + 4410..start + 8820];
        frames
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count()
            * 10
    };
    assert!(frequency_at(0).abs_diff(220) <= 10);
    assert!(frequency_at(intro_frames).abs_diff(440) <= 10);
    assert!(frequency_at(intro_frames + loop_frames).abs_diff(440) <= 10);
    assert!(frequency_at(intro_frames + 2 * loop_frames).abs_diff(440) <= 10);
}

#[test]
fn delayed_and_reverberant_body_tails_enter_on_the_repeat() {
    use strudeland_render::{ExportPreset, render_export};
    const BODY: &str = r#"
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'body', bars: 1, loop: true }] };
song(meta, { body: note("~ ~ ~ a4").s('sine').gain(0.35).release(0.02).FX })
"#;
    let opts = options(LengthRequest::Clip { loops: None });
    let frames = 11_025;
    for fx in [
        "delay(0.8).delaytime(0.8).delayfeedback(0)",
        "room(0.8).roomsize(0.7)",
    ] {
        let code = BODY.replace("FX", fx);
        let reference = render(&code, &opts, &|_| {}).unwrap();
        let reference = read_f32(&reference.bytes).unwrap();
        let files = render_export(&code, &opts, ExportPreset::IntroLoop, &|_| {}).unwrap();
        let [opening, repeat] = files.as_slice() else {
            panic!("a split export always has a one-time opening and a repeat")
        };
        assert!(opening.loop_points.is_none());
        assert_eq!(repeat.loop_points.unwrap().start_frame, 0);
        let opening = read_f32(&opening.output.bytes).unwrap();
        let repeat = read_f32(&repeat.output.bytes).unwrap();
        for ((first, steady), cold) in opening
            .channels
            .iter()
            .zip(&repeat.channels)
            .zip(&reference.channels)
        {
            assert_eq!(first.len(), frames, "{fx}");
            assert_eq!(steady.len(), frames, "{fx}");
            assert_eq!(&first[..frames - 441], &cold[..frames - 441], "{fx}");
            assert!(
                first[..frames / 2].iter().all(|&sample| sample == 0.0),
                "{fx}"
            );
            assert!(
                steady[..frames / 2]
                    .iter()
                    .any(|sample| sample.abs() > 1e-4),
                "{fx}"
            );
            assert_eq!(first.last(), steady.last(), "{fx}");
        }
    }
}

#[test]
fn one_shot_renders_a_delayed_echo_without_triggering_the_next_bar() {
    use strudeland_render::{ExportPreset, render_export};
    let code = "const meta = { bpm: 120 }; s('sine').freq(220).gain(0.25).delay(0.5).delaytime(0.2).delayfeedback(0.01)";
    let opts = options(LengthRequest::Length { bars: 1 });
    let one = render_export(code, &opts, ExportPreset::OneShot, &|_| {}).unwrap();
    let wav = read_f32(&one[0].output.bytes).unwrap();
    let end = window_frames(0, 1, 0.5) as usize;
    assert!(wav.channels[0].len() > end);
    assert!(
        wav.channels[0][end + 4000..end + 8000]
            .iter()
            .any(|x| x.abs() > 0.0001)
    );

    // The next cycle would still be sounding well beyond the previous note and its first echo.
    let extended = render(code, &options(LengthRequest::Length { bars: 2 }), &|_| {}).unwrap();
    let extended = read_f32(&extended.bytes).unwrap();
    assert!(extended.channels[0][end + 25_000].abs() > 0.0001);
    let late_shot = wav.channels[0].get(end + 25_000).copied().unwrap_or(0.0);
    assert!(late_shot.abs() < 0.0001);
}

#[test]
fn one_shot_retains_the_reverb_tail_in_the_encoded_file() {
    use strudeland_render::{ExportPreset, render_export};
    let code = "const meta = { bpm: 120 }; s('sine').gain(0.3).room(0.7).roomsize(0.4)";
    let opts = options(LengthRequest::Length { bars: 1 });
    let shot = render_export(code, &opts, ExportPreset::OneShot, &|_| {}).unwrap();
    let raw = read_f32(&shot[0].output.bytes).unwrap();
    let end = window_frames(0, 1, 0.5) as usize;
    assert_eq!(shot[0].loop_points, None);
    assert_eq!(shot[0].output.summary.frames, raw.channels[0].len() as u64);
    assert!(
        raw.channels[0][end + 4410..]
            .iter()
            .any(|x| x.abs() > 0.0001)
    );
}

#[test]
fn one_shot_preserves_filter_ringing_after_the_amp_release() {
    use strudeland_render::{ExportPreset, render_export};
    let opts = options(LengthRequest::Length { bars: 1 });
    let after_release = window_frames(0, 1, 2.0) as usize + 442;
    for voice in [
        "s('square').lpf(3000).lpq(20)",
        "s('sine').freq(660).vowel('a')",
    ] {
        let code = format!(
            r#"const meta = {{ bpm: 240, meter: '2/4' }};
note("~ ~ ~ a3").{voice}.release(0.01)"#
        );
        let shot = render_export(&code, &opts, ExportPreset::OneShot, &|_| {}).unwrap();
        let raw = read_f32(&shot[0].output.bytes).unwrap();
        let ring = raw.channels[0]
            .get(after_release..after_release + 128)
            .unwrap_or_else(|| panic!("{voice} must ring past its amp release"));
        assert!(ring.iter().any(|x| x.abs() > 0.001), "{voice}");
    }
}

#[test]
fn a_short_hot_loop_still_respects_the_static_peak_ceiling() {
    use strudeland_render::{ExportPreset, render_export};
    // Both passes together must stay below the 400 ms loudness gating window.
    let code = r#"const meta = { bpm: 360, meter: '1/4', sections: [{ name: 'body', bars: 1, loop: true }] };
song(meta, { body: s('square').gain(4) })"#;
    let mut opts = options(LengthRequest::Clip { loops: None });
    opts.output.loudness = Loudness::default();
    let files = render_export(code, &opts, ExportPreset::FoldedLoop, &|_| {}).unwrap();
    let summary = &files[0].output.summary;
    let loudness = summary.loudness.as_ref().unwrap();
    assert!(!loudness.input_lufs.is_finite());
    assert!(loudness.gain_db < 0.0);
    assert!(loudness.output_true_peak_db <= -1.0 + 1e-5);
    assert!(
        summary
            .warnings
            .iter()
            .any(|w| w.contains("true-peak ceiling still applies"))
    );
}
