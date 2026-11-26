//! The joiner: ports of `contentOf` and the concatenation in `assembleWindow`
//! (`src/lib/render/chunks.ts`). Every frame count is the planner's (`contentFrames`,
//! `musicFrames`); nothing here recomputes one. Every check is an error, never a panic.

use strudeland_host::{ChunkWindow, MAX_BARS, PlannedWindow};

use crate::RenderError;

fn to_usize(n: u64, what: &str) -> Result<usize, RenderError> {
    usize::try_from(n).map_err(|_| RenderError::Render(format!("{what} ({n}) is too large")))
}

/// The frames of a chunk's buffer that its content keeps: `plan.window_start_frame` frames from
/// the head, then the musical content length from the plan.
fn kept_range(plan: &ChunkWindow) -> Result<std::ops::Range<usize>, RenderError> {
    let start = to_usize(plan.window_start_frame, "windowStartFrame")?;
    let len = to_usize(plan.content_frames, "content frames")?;
    let end = start
        .checked_add(len)
        .ok_or_else(|| RenderError::Render("content end overflows".into()))?;
    Ok(start..end)
}

/// The kept part of each channel ([`kept_range`]): a port of `contentOf`. A short channel is a
/// `Render` error.
fn kept<'a>(plan: &ChunkWindow, channels: &'a [Vec<f32>]) -> Result<Vec<&'a [f32]>, RenderError> {
    let range = kept_range(plan)?;
    channels
        .iter()
        .enumerate()
        .map(|(c, ch)| {
            ch.get(range.clone()).ok_or_else(|| {
                RenderError::Render(format!(
                    "contentOf: channel {c} has {} frames, needs {} ({} + {})",
                    ch.len(),
                    range.end,
                    range.start,
                    range.len()
                ))
            })
        })
        .collect()
}

/// A chunk's bars and channel count, as [`Joiner::new`] checks them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub from_bar: u32,
    pub to_bar: u32,
    pub channels: usize,
}

/// The window being assembled: allocated once, each chunk's content copied in at its offset as
/// it arrives (in any order), so a chunk's buffer can be freed as soon as it is placed.
#[derive(Debug)]
pub struct Joiner {
    /// Per chunk: the bars, the offset and the length in frames.
    chunks: Vec<(Span, usize, usize)>,
    placed: Vec<bool>,
    out: Vec<Vec<f32>>,
    total: usize,
    tail: usize,
}

impl Joiner {
    pub(crate) fn channels(&self) -> usize {
        self.out.len()
    }

    /// Checks the chunks' bars against the window and allocates it, with `tail_frames` after the
    /// music. Each span carries its planned content frames. Errors (`Render`) when the window ends
    /// past [`MAX_BARS`] or cannot be allocated, there is no chunk, the first does not start at
    /// `window.from_bar`, two chunks leave a gap or overlap, a chunk is empty, the channel counts
    /// differ, the last does not end at `window.to_bar`, or the chunks' frames do not add up to
    /// the plan's `music_frames`.
    pub fn new(
        window: &PlannedWindow,
        spans: &[(Span, u64)],
        music_frames: u64,
        tail_frames: u64,
    ) -> Result<Self, RenderError> {
        // The planner refuses such a window; checked again before anything is allocated.
        if u64::from(window.to_bar) > MAX_BARS {
            return join_err(format!(
                "the window [{}, {}) ends past the render limit of {MAX_BARS} bars",
                window.from_bar, window.to_bar
            ));
        }
        let Some((first, _)) = spans.first() else {
            return join_err("no chunks".into());
        };
        if first.from_bar != window.from_bar {
            return join_err(format!(
                "chunk 0 starts at bar {}, the window at bar {}",
                first.from_bar, window.from_bar
            ));
        }
        let nch = first.channels;
        let mut at = window.from_bar;
        for (i, (c, _)) in spans.iter().enumerate() {
            if c.from_bar != at {
                return join_err(format!(
                    "chunk {i} starts at bar {}, the previous chunk ends at bar {at}",
                    c.from_bar
                ));
            }
            if c.to_bar <= c.from_bar {
                return join_err(format!(
                    "chunk {i} is empty ([{}, {}))",
                    c.from_bar, c.to_bar
                ));
            }
            if c.channels != nch {
                return join_err(format!(
                    "chunk {i} has {} channels, chunk 0 has {nch}",
                    c.channels
                ));
            }
            at = c.to_bar;
        }
        if at != window.to_bar {
            return join_err(format!(
                "the chunks end at bar {at}, the window at bar {}",
                window.to_bar
            ));
        }

        let mut chunks = Vec::with_capacity(spans.len());
        let mut total = 0usize;
        for (c, frames) in spans {
            let len = to_usize(*frames, "chunk frames")?;
            chunks.push((*c, total, len));
            total = total
                .checked_add(len)
                .ok_or_else(|| RenderError::Render("joined frame count overflows".into()))?;
        }
        if music_frames != total as u64 {
            return join_err(format!(
                "the chunks hold {total} frames, the window is {music_frames}"
            ));
        }
        let tail = to_usize(tail_frames, "tail frames")?;
        let frames = total
            .checked_add(tail)
            .ok_or_else(|| RenderError::Render("joined frame count overflows".into()))?;
        let mut out = Vec::with_capacity(nch);
        for _ in 0..nch {
            let mut ch: Vec<f32> = Vec::new();
            if ch.try_reserve_exact(frames).is_err() {
                return join_err(format!(
                    "cannot allocate the window ({nch} channels of {frames} frames)"
                ));
            }
            ch.resize(frames, 0.0);
            out.push(ch);
        }
        Ok(Joiner {
            placed: vec![false; chunks.len()],
            chunks,
            out,
            total,
            tail,
        })
    }

    /// [`Joiner::new`] for the chunks of `plans` (their planned `content_frames`), each of
    /// `channels` channels: the joiner the pipeline renders into.
    pub fn for_plans(
        window: &PlannedWindow,
        plans: &[ChunkWindow],
        channels: usize,
        music_frames: u64,
        tail_frames: u64,
    ) -> Result<Self, RenderError> {
        let spans: Vec<_> = plans
            .iter()
            .map(|p| {
                let span = Span {
                    from_bar: p.from_bar,
                    to_bar: p.to_bar,
                    channels,
                };
                (span, p.content_frames)
            })
            .collect();
        Joiner::new(window, &spans, music_frames, tail_frames)
    }

    /// Places chunk `index` from its whole rendered buffer: drops `plan.window_start_frame`
    /// frames from the head and everything after the content (a port of `contentOf`), then
    /// [`Joiner::place`]s the rest. Errors when `plan`'s bars are not the chunk's, a channel is
    /// too short for the plan (`chunk <index>: contentOf: ...`), or as `place` does.
    pub fn place_rendered(
        &mut self,
        index: usize,
        plan: &ChunkWindow,
        channels: &[Vec<f32>],
    ) -> Result<(), RenderError> {
        if let Some((span, _, _)) = self.chunks.get(index)
            && (span.from_bar, span.to_bar) != (plan.from_bar, plan.to_bar)
        {
            return join_err(format!(
                "chunk {index} is bars [{}, {}), its plan says [{}, {})",
                span.from_bar, span.to_bar, plan.from_bar, plan.to_bar
            ));
        }
        let content = kept(plan, channels).map_err(|e| match e {
            RenderError::Render(m) => RenderError::Render(format!("chunk {index}: {m}")),
            other => other,
        })?;
        if self.tail != 0 && index == self.chunks.len() - 1 {
            let start = kept_range(plan)?.end;
            let end = start
                .checked_add(self.tail)
                .ok_or_else(|| RenderError::Render("tail end overflows".into()))?;
            for (dst, src) in self.out.iter_mut().zip(channels) {
                match (dst.get_mut(self.total..), src.get(start..end)) {
                    (Some(dst), Some(src)) => dst.copy_from_slice(src),
                    _ => {
                        return join_err(format!(
                            "chunk {index} does not contain the planned tail"
                        ));
                    }
                }
            }
        }
        self.place(index, &content)
    }

    /// Copies chunk `index`'s content (one slice per channel) into the window. Errors when a
    /// channel has the wrong length or the chunk does not fit the window.
    pub fn place(&mut self, index: usize, channels: &[&[f32]]) -> Result<(), RenderError> {
        let Some(&(span, off, len)) = self.chunks.get(index) else {
            return join_err(format!("no chunk {index}"));
        };
        if channels.len() != span.channels {
            return join_err(format!(
                "chunk {index} has {} channels, the plan says {}",
                channels.len(),
                span.channels
            ));
        }
        for (ch, (dst, src)) in self.out.iter_mut().zip(channels).enumerate() {
            if src.len() != len {
                return join_err(format!(
                    "chunk {index} channel {ch} has {} frames, bars [{}, {}) are {len}",
                    src.len(),
                    span.from_bar,
                    span.to_bar
                ));
            }
            let Some(slot) = off.checked_add(len).and_then(|end| dst.get_mut(off..end)) else {
                return join_err(format!(
                    "chunk {index} frames [{off}, {off} + {len}) are outside the window's {}",
                    self.total
                ));
            };
            slot.copy_from_slice(src);
        }
        if let Some(p) = self.placed.get_mut(index) {
            *p = true;
        }
        Ok(())
    }

    /// The window; an error when a chunk was never placed.
    pub fn finish(self) -> Result<Vec<Vec<f32>>, RenderError> {
        if let Some(i) = self.placed.iter().position(|p| !p) {
            return join_err(format!("chunk {i} was not placed"));
        }
        Ok(self.out)
    }
}

fn join_err<T>(m: String) -> Result<T, RenderError> {
    Err(RenderError::Render(format!("join: {m}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::window_frames;
    use strudeland_host::Layout;

    fn plan(from_bar: u32, to_bar: u32, window_start_frame: u64) -> ChunkWindow {
        ChunkWindow {
            c0: from_bar,
            c1: to_bar,
            lookback_sec_used: 0.0,
            content_frames: u64::from(to_bar - from_bar),
            overlap_frames: None,
            window_start_frame,
            buffer_start_sec: 0.0,
            frames: 0,
            from_bar,
            to_bar,
            lookback_sec: 0.0,
        }
    }

    fn ramp(n: usize, base: f32) -> Vec<f32> {
        (0..n).map(|i| base + i as f32).collect()
    }

    /// One frame per bar, so offsets are easy to read (the frames [`plan`] gives a chunk).
    fn one_per_bar(from_bar: u32, to_bar: u32) -> u64 {
        u64::from(to_bar - from_bar)
    }

    fn kept_vecs(plan: &ChunkWindow, channels: &[Vec<f32>]) -> Vec<Vec<f32>> {
        kept(plan, channels)
            .unwrap()
            .into_iter()
            .map(<[f32]>::to_vec)
            .collect()
    }

    #[test]
    fn kept_drops_the_head_and_the_tail() {
        let c = kept_vecs(&plan(2, 5, 4), &[ramp(10, 0.0), ramp(10, 100.0)]);
        assert_eq!(c, vec![vec![4.0, 5.0, 6.0], vec![104.0, 105.0, 106.0]]);
        // Exactly long enough.
        let c = kept_vecs(&plan(0, 3, 2), &[ramp(5, 0.0)]);
        assert_eq!(c, vec![vec![2.0, 3.0, 4.0]]);
        // No lookback.
        let c = kept_vecs(&plan(0, 2, 0), &[ramp(2, 0.0)]);
        assert_eq!(c, vec![vec![0.0, 1.0]]);
    }

    #[test]
    fn kept_rejects_a_short_buffer() {
        let e = kept(&plan(0, 3, 2), &[ramp(5, 0.0), ramp(4, 0.0)]).unwrap_err();
        assert_eq!(
            e.to_string(),
            "render failed: contentOf: channel 1 has 4 frames, needs 5 (2 + 3)"
        );
        let e = kept(&plan(0, 1, u64::MAX), &[ramp(5, 0.0)]).unwrap_err();
        assert!(matches!(e, RenderError::Render(_)), "{e}");
    }

    #[test]
    fn explicit_frames_tile_the_window_and_locate_its_tail() {
        let mut plans = [plan(0, 1, 1), plan(1, 2, 2)];
        plans[0].content_frames = 3;
        plans[1].content_frames = 1;
        let w = window(0, 2);
        let mut j = Joiner::for_plans(&w, &plans, 1, 4, 2).unwrap();
        j.place_rendered(1, &plans[1], &[vec![0., 0., 4., 5., 6.]])
            .unwrap();
        j.place_rendered(0, &plans[0], &[vec![0., 1., 2., 3., 0.]])
            .unwrap();
        assert_eq!(j.finish().unwrap(), vec![vec![1., 2., 3., 4., 5., 6.]]);
        assert!(Joiner::for_plans(&w, &plans, 1, 5, 0).is_err());
    }

    #[test]
    fn place_rendered_keeps_the_content_of_each_chunk() {
        // Chunks [0, 2) and [2, 3) with 1 and 2 frames of lookback and a frame of tail each,
        // placed out of order.
        let plans = [plan(0, 2, 1), plan(2, 3, 2)];
        let mut j = Joiner::for_plans(&window(0, 3), &plans, 2, 3, 0).unwrap();
        j.place_rendered(1, &plans[1], &[ramp(4, 20.0), ramp(4, 120.0)])
            .unwrap();
        j.place_rendered(0, &plans[0], &[ramp(4, 10.0), ramp(4, 110.0)])
            .unwrap();
        assert_eq!(
            j.finish().unwrap(),
            vec![vec![11.0, 12.0, 22.0], vec![111.0, 112.0, 122.0]]
        );
        // A short buffer names the chunk; a plan for other bars is refused.
        let mut j = Joiner::for_plans(&window(0, 3), &plans, 2, 3, 0).unwrap();
        let e = j
            .place_rendered(1, &plans[1], &[ramp(4, 0.0), ramp(2, 0.0)])
            .unwrap_err();
        assert_eq!(
            e.to_string(),
            "render failed: chunk 1: contentOf: channel 1 has 2 frames, needs 3 (2 + 1)"
        );
        let e = j
            .place_rendered(0, &plans[1], &[ramp(4, 0.0), ramp(4, 0.0)])
            .unwrap_err();
        assert_eq!(
            e.to_string(),
            "render failed: join: chunk 0 is bars [0, 2), its plan says [2, 3)"
        );
    }

    /// A chunk's kept content `[from_bar, to_bar)`.
    #[derive(Debug, Clone, PartialEq)]
    struct Content {
        from_bar: u32,
        to_bar: u32,
        channels: Vec<Vec<f32>>,
    }

    /// `contents` placed in chunk order into a [`Joiner`], each planned at `frames(from, to)`
    /// content frames and the window at their sum.
    fn join(
        window: &PlannedWindow,
        frames: impl Fn(u32, u32) -> u64,
        contents: Vec<Content>,
    ) -> Result<Vec<Vec<f32>>, RenderError> {
        let spans: Vec<(Span, u64)> = contents
            .iter()
            .map(|c| {
                let span = Span {
                    from_bar: c.from_bar,
                    to_bar: c.to_bar,
                    channels: c.channels.len(),
                };
                (span, frames(c.from_bar, c.to_bar))
            })
            .collect();
        let music = spans.iter().map(|(_, n)| n).sum();
        let mut joiner = Joiner::new(window, &spans, music, 0)?;
        for (i, c) in contents.iter().enumerate() {
            let slices: Vec<&[f32]> = c.channels.iter().map(Vec::as_slice).collect();
            joiner.place(i, &slices)?;
        }
        joiner.finish()
    }

    fn content(from_bar: u32, to_bar: u32, nch: usize, base: f32) -> Content {
        let n = (to_bar - from_bar) as usize;
        Content {
            from_bar,
            to_bar,
            channels: (0..nch)
                .map(|c| ramp(n, base + 1000.0 * c as f32))
                .collect(),
        }
    }

    fn window(from_bar: u32, to_bar: u32) -> PlannedWindow {
        PlannedWindow {
            from_bar,
            to_bar,
            layout: Layout::Linear,
        }
    }

    #[test]
    fn join_concatenates_at_the_bar_offsets() {
        let out = join(
            &window(1, 6),
            one_per_bar,
            vec![
                content(1, 3, 2, 10.0),
                content(3, 4, 2, 20.0),
                content(4, 6, 2, 30.0),
            ],
        )
        .unwrap();
        assert_eq!(
            out,
            vec![
                vec![10.0, 11.0, 20.0, 30.0, 31.0],
                vec![1010.0, 1011.0, 1020.0, 1030.0, 1031.0]
            ]
        );
        // A single chunk is the window.
        let out = join(&window(0, 3), one_per_bar, vec![content(0, 3, 2, 0.0)]).unwrap();
        assert_eq!(out[0], vec![0.0, 1.0, 2.0]);
    }

    #[test]
    fn join_offsets_follow_the_absolute_rounding() {
        // cps 0.7: bars of 63000 frames; cps 0.3: 147000.00000000003, rounding per absolute bar.
        for cps in [0.7, 0.3, 4.0 / 3.0, 0.4375] {
            let w = window(0, 5);
            let contents: Vec<Content> = [(0, 2), (2, 3), (3, 5)]
                .into_iter()
                .enumerate()
                .map(|(k, (a, b))| Content {
                    from_bar: a,
                    to_bar: b,
                    channels: vec![vec![k as f32; window_frames(a, b, cps) as usize]; 2],
                })
                .collect();
            let out = join(&w, |a, b| window_frames(a, b, cps), contents).unwrap();
            let total = window_frames(0, 5, cps) as usize;
            assert_eq!(out[0].len(), total, "cps {cps}");
            let off1 = window_frames(0, 2, cps) as usize;
            let off2 = window_frames(0, 3, cps) as usize;
            assert_eq!(out[1][off1 - 1], 0.0, "cps {cps}");
            assert_eq!(out[1][off1], 1.0, "cps {cps}");
            assert_eq!(out[1][off2 - 1], 1.0, "cps {cps}");
            assert_eq!(out[1][off2], 2.0, "cps {cps}");
            assert_eq!(out[1][total - 1], 2.0, "cps {cps}");
        }
    }

    #[test]
    fn join_rejects_every_mismatch() {
        let cases: Vec<(&str, PlannedWindow, Vec<Content>, &str)> = vec![
            ("no chunks", window(0, 2), vec![], "join: no chunks"),
            (
                "wrong start",
                window(0, 4),
                vec![content(1, 4, 2, 0.0)],
                "join: chunk 0 starts at bar 1, the window at bar 0",
            ),
            (
                "gap",
                window(0, 4),
                vec![content(0, 1, 2, 0.0), content(2, 4, 2, 0.0)],
                "join: chunk 1 starts at bar 2, the previous chunk ends at bar 1",
            ),
            (
                "overlap",
                window(0, 4),
                vec![content(0, 2, 2, 0.0), content(1, 4, 2, 0.0)],
                "join: chunk 1 starts at bar 1, the previous chunk ends at bar 2",
            ),
            (
                "empty chunk",
                window(0, 2),
                vec![content(0, 0, 2, 0.0), content(0, 2, 2, 0.0)],
                "join: chunk 0 is empty ([0, 0))",
            ),
            (
                "wrong end",
                window(0, 4),
                vec![content(0, 2, 2, 0.0), content(2, 3, 2, 0.0)],
                "join: the chunks end at bar 3, the window at bar 4",
            ),
            (
                "past the end",
                window(0, 2),
                vec![content(0, 3, 2, 0.0)],
                "join: the chunks end at bar 3, the window at bar 2",
            ),
            (
                "channel mismatch",
                window(0, 2),
                vec![content(0, 1, 2, 0.0), content(1, 2, 1, 0.0)],
                "join: chunk 1 has 1 channels, chunk 0 has 2",
            ),
            (
                "one frame short",
                window(0, 3),
                vec![
                    content(0, 1, 2, 0.0),
                    Content {
                        from_bar: 1,
                        to_bar: 3,
                        channels: vec![vec![0.0; 1], vec![0.0; 2]],
                    },
                ],
                "join: chunk 1 channel 0 has 1 frames, bars [1, 3) are 2",
            ),
            (
                "one frame long",
                window(0, 2),
                vec![Content {
                    from_bar: 0,
                    to_bar: 2,
                    channels: vec![vec![0.0; 2], vec![0.0; 3]],
                }],
                "join: chunk 0 channel 1 has 3 frames, bars [0, 2) are 2",
            ),
        ];
        for (name, w, contents, want) in cases {
            let e = join(&w, one_per_bar, contents).unwrap_err();
            assert!(matches!(e, RenderError::Render(_)), "{name}");
            assert_eq!(e.to_string(), format!("render failed: {want}"), "{name}");
        }
    }

    #[test]
    fn joiner_places_chunks_in_any_order_and_needs_every_one() {
        let spans = [(1, 3), (3, 4), (4, 6)].map(|(from_bar, to_bar)| {
            let span = Span {
                from_bar,
                to_bar,
                channels: 2,
            };
            (span, one_per_bar(from_bar, to_bar))
        });
        let contents = [
            content(1, 3, 2, 10.0),
            content(3, 4, 2, 20.0),
            content(4, 6, 2, 30.0),
        ];
        let mut j = Joiner::new(&window(1, 6), &spans, 5, 0).unwrap();
        for i in [2, 0, 1] {
            let c = &contents[i];
            let slices: Vec<&[f32]> = c.channels.iter().map(Vec::as_slice).collect();
            j.place(i, &slices).unwrap();
        }
        assert_eq!(
            j.finish().unwrap(),
            join(&window(1, 6), one_per_bar, contents.to_vec()).unwrap()
        );

        let mut j = Joiner::new(&window(1, 6), &spans, 5, 0).unwrap();
        let c = &contents[0];
        j.place(0, &[&c.channels[0], &c.channels[1]]).unwrap();
        let e = j.place(1, &[&c.channels[0]]).unwrap_err();
        assert_eq!(
            e.to_string(),
            "render failed: join: chunk 1 has 1 channels, the plan says 2"
        );
        assert!(matches!(j.place(3, &[]), Err(RenderError::Render(_))));
        assert_eq!(
            j.finish().unwrap_err().to_string(),
            "render failed: join: chunk 1 was not placed"
        );
    }

    #[test]
    fn a_window_past_the_render_limit_is_an_error_not_an_allocation() {
        let to_bar = MAX_BARS as u32 + 1;
        // cps 0.5: about 8.8e9 frames, which would abort the process if allocated.
        let frames = window_frames(0, to_bar, 0.5);
        let span = Span {
            from_bar: 0,
            to_bar,
            channels: 2,
        };
        let e = Joiner::new(&window(0, to_bar), &[(span, frames)], frames, 0).unwrap_err();
        assert!(matches!(e, RenderError::Render(_)), "{e:?}");
        assert_eq!(
            e.to_string(),
            "render failed: join: the window [0, 100001) ends past the render limit of 100000 bars"
        );
        // A window that fits the bars but not the memory is an error too, never an abort.
        let frames = window_frames(0, 4, 1e-12);
        let span = Span {
            from_bar: 0,
            to_bar: 4,
            channels: 2,
        };
        let e = Joiner::new(&window(0, 4), &[(span, frames)], frames, 0).unwrap_err();
        assert!(
            e.to_string()
                .starts_with("render failed: join: cannot allocate the window"),
            "{e}"
        );
    }

    #[test]
    fn joiner_rejects_music_frames_that_differ_from_the_chunks() {
        let plans = [plan(0, 2, 0), plan(2, 3, 0)];
        for music in [2, 4] {
            let e = Joiner::for_plans(&window(0, 3), &plans, 2, music, 0).unwrap_err();
            assert_eq!(
                e.to_string(),
                format!("render failed: join: the chunks hold 3 frames, the window is {music}")
            );
        }
        assert!(Joiner::for_plans(&window(0, 3), &plans, 2, 3, 0).is_ok());
    }
}
