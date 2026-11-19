//! Hostile input never panics (the release profile aborts on a panic): every case returns an
//! error or audio.

mod common;

use std::time::Duration;

use common::{corpus_song, options, window};
use strudeland_host::{HostError, Layout, LengthRequest};
use strudeland_render::{RenderError, RenderOptions, render};

#[test]
fn hostile_input_returns_errors() {
    let unsupported_only = "$: s(\"zzfx\")\n";
    let cases: Vec<(&str, String, RenderOptions)> = vec![
        (
            "garbage",
            "\u{0}\u{1}}}}{{{ ]] @@ \u{fffd}".to_string(),
            options(LengthRequest::Length { bars: 1 }),
        ),
        (
            "empty",
            String::new(),
            options(LengthRequest::Length { bars: 1 }),
        ),
        (
            "unsupported only",
            unsupported_only.to_string(),
            options(LengthRequest::Length { bars: 2 }),
        ),
        (
            "zero jobs",
            corpus_song("sine-basic.js"),
            RenderOptions {
                jobs: Some(0),
                ..options(LengthRequest::Length { bars: 1 })
            },
        ),
        (
            "a 1 ms timeout",
            corpus_song("late-feedback.js"),
            RenderOptions {
                plan_timeout: Some(Duration::from_millis(1)),
                ..options(LengthRequest::Length { bars: 64 })
            },
        ),
        (
            "an empty window",
            corpus_song("sine-basic.js"),
            options(window(3, 3, Layout::Linear)),
        ),
        (
            "a window past the bar limit",
            corpus_song("sine-basic.js"),
            options(window(0, u32::MAX, Layout::Linear)),
        ),
        (
            "zero bars",
            corpus_song("sine-basic.js"),
            options(LengthRequest::Length { bars: 0 }),
        ),
        (
            "zero loops",
            corpus_song("song-linear.js"),
            options(LengthRequest::Clip { loops: Some(0) }),
        ),
    ];
    for (name, code, opts) in cases {
        let r = render(&code, &opts, &|_| {});
        let Err(e) = r else {
            panic!("{name}: rendered");
        };
        match name {
            "unsupported only" => {
                assert!(matches!(e, RenderError::Unsupported(_)), "{name}: {e:?}")
            }
            "zero jobs" => assert!(matches!(e, RenderError::Usage { .. }), "{name}: {e:?}"),
            "a 1 ms timeout" => assert!(
                matches!(e, RenderError::Plan(HostError::Timeout { millis: 1 })),
                "{name}: {e:?}"
            ),
            _ => assert!(matches!(e, RenderError::Plan(_)), "{name}: {e:?}"),
        }
    }
}

#[test]
fn a_window_past_the_song_renders() {
    // Past the last note: silence of the exact length, not an error.
    let code = corpus_song("song-linear.js");
    let out = render(&code, &options(window(40, 42, Layout::Linear)), &|_| {}).unwrap();
    assert_eq!(out.summary.frames, 176_400);
}
