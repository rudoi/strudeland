//! The batch path (one host plan of every chunk, parallel engine renders, the joiner) against the
//! per-chunk path the harness takes (a single-chunk `plan_render` per chunk request on a fresh
//! host, one engine render each, the same joiner): the job texts are equal and the joined audio is
//! bit for bit the same.

mod common;

use common::{corpus_song, options, window};
use strudeland_engine::EngineJob;
use strudeland_host::{
    ChunkWindow, Host, Layout, LengthRequest, LookbackSec, Mode, RenderPlanRequest, SingleChunk,
};
use strudeland_render::join::Joiner;
use strudeland_render::render_audio;

/// The single-chunk request at `plan`'s bars and lookback: linear, overlap 0, mix.
fn single(plan: &ChunkWindow) -> RenderPlanRequest {
    RenderPlanRequest {
        instruments: Default::default(),
        preset: None,
        length: LengthRequest::Window {
            from_bar: plan.from_bar,
            to_bar: plan.to_bar,
            layout: Layout::Linear,
        },
        include_events: None,
        chunk: Some(SingleChunk {
            overlap_bars: 0,
            lookback_sec: LookbackSec::Seconds(plan.lookback_sec),
            mode: Mode::Mix,
        }),
        timeout_sec: None,
    }
}

#[test]
fn batch_and_per_chunk_renders_join_bit_identically() {
    let code = corpus_song("sine-basic.js");
    let length = window(0, 20, Layout::Linear);
    let mut opts = options(length.clone());
    opts.jobs = Some(3);
    let audio = render_audio(&code, &opts, &|_| {}).unwrap();
    assert!(
        audio.chunks.len() >= 2,
        "{} chunk(s): the test needs seams",
        audio.chunks.len()
    );
    assert_eq!(audio.chunks.first().unwrap().from_bar, 0);
    assert_eq!(audio.chunks.last().unwrap().to_bar, 20);

    // The batch's job texts, from one plan_render call.
    let planned = Host::new()
        .unwrap()
        .plan_render(
            &code,
            &RenderPlanRequest {
                instruments: Default::default(),
                preset: None,
                length,
                include_events: None,
                chunk: None,
                timeout_sec: None,
            },
        )
        .unwrap();
    assert_eq!(planned.chunks.len(), audio.chunks.len());
    assert_eq!(planned.cps.to_bits(), audio.cps.to_bits());

    let mut joiner = Joiner::for_plans(
        &audio.summary.window,
        &audio.chunks,
        2,
        audio.music_frames,
        0,
    )
    .unwrap();
    for (k, (plan, batch)) in audio.chunks.iter().zip(&planned.chunks).enumerate() {
        assert_eq!(&batch.plan, plan, "chunk {k}: the window");
        // A fresh host per chunk, as the harness's one process per request.
        let one = Host::new()
            .unwrap()
            .plan_render(&code, &single(plan))
            .unwrap();
        assert_eq!(one.chunks.len(), 1, "chunk {k}: a single-chunk plan");
        let job = &one.chunks[0].json;
        assert!(*job == batch.json, "chunk {k}: the job text differs");
        let rendered = strudeland_engine::render(&EngineJob::from_json(job).unwrap()).unwrap();
        assert!(
            rendered.errors.is_empty(),
            "chunk {k}: {:?}",
            rendered.errors
        );
        joiner.place_rendered(k, plan, &rendered.channels).unwrap();
    }
    let joined = joiner.finish().unwrap();

    assert_eq!(joined.len(), 2);
    assert_eq!(audio.channels.len(), 2);
    for (c, (a, b)) in audio.channels.iter().zip(&joined).enumerate() {
        assert_eq!(a.len(), b.len(), "channel {c}: length");
        assert_eq!(a.len() as u64, audio.summary.frames, "channel {c}: frames");
        if let Some(i) = a
            .iter()
            .zip(b)
            .position(|(x, y)| x.to_bits() != y.to_bits())
        {
            panic!(
                "channel {c}: frame {i} differs: batch {} per-chunk {}",
                a[i], b[i]
            );
        }
    }
    // Not silence: the comparison means something.
    assert!(audio.channels[0].iter().any(|&x| x.abs() > 0.01));
}
