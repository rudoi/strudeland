//! The loop splice: the continuous opening-plus-repeat PCM a loop preset is cut from, and its file
//! cuts.

use std::borrow::Cow;
use std::ops::Range;

use strudeland_host::PlannedWindow;

use crate::export::roles;
use crate::level::true_peak::TAPS_PER_PHASE;
use crate::level::{Measure, check_input, measure, true_peak_db};
use crate::{ExportPreset, ExportRole, LoopAudio, RenderError};

/// The PCM of one file a render will finish.
pub(crate) struct FileCut<'a> {
    pub role: ExportRole,
    pub channels: Cow<'a, [Vec<f32>]>,
    /// The authored bars the file covers.
    pub bars: Range<u32>,
    /// The loop in input-rate frames (exclusive end).
    pub loop_range: Option<Range<usize>>,
}

/// A loop preset's file cuts and the peak the shared gain must respect.
pub(crate) struct Splice {
    pub cuts: Vec<FileCut<'static>>,
    /// The measure of the whole opening-plus-repeat buffer.
    pub program: Measure,
    /// `program.true_peak_db` and the wrap peak, whichever is higher.
    pub peak_db: f64,
}

/// Splices the opening and the repeat into one continuous buffer, measures it, and cuts the
/// preset's files from it.
pub(crate) fn splice(
    channels: Vec<Vec<f32>>,
    music_frames: u64,
    window: PlannedWindow,
    sample_rate: u32,
    loop_audio: LoopAudio,
    preset: ExportPreset,
) -> Result<Splice, RenderError> {
    let LoopAudio {
        start_bar,
        start_frame,
        isolated_body,
    } = loop_audio;
    if !(window.from_bar..window.to_bar).contains(&start_bar) {
        return Err(render_error("loop start is outside the planned window"));
    }
    let start =
        usize::try_from(start_frame).map_err(|_| render_error("loop start is too large"))?;
    let end = usize::try_from(music_frames).map_err(|_| render_error("loop end is too large"))?;
    if end <= start {
        return Err(render_error("loop has no frames"));
    }
    let body_frames = end - start;
    let combined_frames = end
        .checked_add(body_frames)
        .ok_or_else(|| render_error("the combined loop is too large"))?;
    let repeat_range = end..combined_frames;
    let (mut opening, mut repeat) = match isolated_body {
        Some(body) if start > 0 => (channels, body),
        None if start == 0 => {
            let opening = channels
                .iter()
                .map(|ch| copy_opening(ch, end, combined_frames))
                .collect::<Result<Vec<_>, _>>()?;
            (opening, channels)
        }
        _ => return Err(render_error("the plan has inconsistent loop history")),
    };
    fold_tail(&mut repeat, body_frames)?;
    join_opening(&mut opening, &repeat, end, sample_rate)?;

    check_input(&opening)?;
    let program = measure(&opening, sample_rate);
    let wrap = wrap_peak(&repeat);
    let peak_db = program.true_peak_db.max(wrap);
    let cuts = match preset {
        ExportPreset::IntroLoop => {
            let &[intro, repeat_role] = roles(Some(ExportPreset::IntroLoop)) else {
                return Err(render_error("invalid loop export roles"));
            };
            for ch in &mut opening {
                ch.truncate(end);
            }
            vec![
                FileCut {
                    role: intro,
                    channels: Cow::Owned(opening),
                    bars: window.from_bar..window.to_bar,
                    loop_range: None,
                },
                FileCut {
                    role: repeat_role,
                    channels: Cow::Owned(repeat),
                    bars: start_bar..window.to_bar,
                    loop_range: Some(0..body_frames),
                },
            ]
        }
        ExportPreset::FoldedLoop | ExportPreset::TaggedLoop => {
            let &[role] = roles(Some(preset)) else {
                return Err(render_error("invalid loop export roles"));
            };
            drop(repeat);
            vec![FileCut {
                role,
                channels: Cow::Owned(opening),
                bars: window.from_bar..window.to_bar,
                loop_range: Some(repeat_range),
            }]
        }
        ExportPreset::OneShot => return Err(render_error("unexpected one-shot preset")),
    };
    Ok(Splice {
        cuts,
        program,
        peak_db,
    })
}

fn render_error(message: &str) -> RenderError {
    RenderError::Render(message.into())
}

fn copy_opening(
    channel: &[f32],
    frames: usize,
    total_frames: usize,
) -> Result<Vec<f32>, RenderError> {
    let opening = channel
        .get(..frames)
        .ok_or_else(|| render_error("opening extends past the rendered PCM"))?;
    let mut copy = Vec::new();
    copy.try_reserve_exact(total_frames)
        .map_err(|_| render_error("cannot allocate the loop opening"))?;
    copy.extend_from_slice(opening);
    Ok(copy)
}

/// Bring the last few milliseconds of the opening to the same phase and effect
/// state as the repeat's end. The repeat stays independent of the opening.
fn join_opening(
    opening: &mut [Vec<f32>],
    repeat: &[Vec<f32>],
    opening_frames: usize,
    sample_rate: u32,
) -> Result<(), RenderError> {
    if opening.len() != repeat.len() {
        return Err(render_error("opening and repeat channel counts differ"));
    }
    for (first, periodic) in opening.iter_mut().zip(repeat) {
        if first.len() != opening_frames || first.len() < periodic.len() {
            return Err(render_error("invalid loop opening length"));
        }
        // 10 ms, limited to the latter half of even a very short loop.
        let blend_frames = (sample_rate as usize / 100).min(periodic.len() / 2);
        let fade = first.iter_mut().rev().zip(periodic.iter().rev());
        for (i, (dst, &src)) in fade.take(blend_frames).enumerate() {
            let x = 1.0 - i as f64 / blend_frames as f64;
            let weight = x * x * (3.0 - 2.0 * x);
            *dst = if i == 0 {
                src
            } else {
                let a = f64::from(*dst);
                (a + (f64::from(src) - a) * weight) as f32
            };
        }
        first
            .try_reserve_exact(periodic.len())
            .map_err(|_| render_error("cannot allocate the combined loop export"))?;
        first.extend_from_slice(periodic);
    }
    Ok(())
}

fn fold_tail(channels: &mut [Vec<f32>], body_frames: usize) -> Result<(), RenderError> {
    if body_frames == 0 {
        return Err(render_error("cannot fold into an empty loop"));
    }
    for channel in channels {
        let (body, tail) = channel
            .split_at_mut_checked(body_frames)
            .ok_or_else(|| render_error("loop extends past the rendered PCM"))?;
        for cycle in tail.chunks(body_frames) {
            for (sample, tail) in body.iter_mut().zip(cycle) {
                *sample += *tail;
            }
        }
        channel.truncate(body_frames);
    }
    Ok(())
}

/// Measure cyclic FIR input across the folded body's join. Each channel has at
/// least one frame. Cycling the edges covers loops shorter than a tap window.
fn wrap_peak(channels: &[Vec<f32>]) -> f64 {
    let mut edges = Vec::with_capacity(channels.len());
    for body in channels {
        let width = TAPS_PER_PHASE;
        let preceding = (body.len() - width % body.len()) % body.len();
        edges.push(
            body.iter()
                .skip(preceding)
                .chain(body.iter().cycle())
                .take(2 * width)
                .copied()
                .collect(),
        );
    }
    true_peak_db(&edges)
}

#[cfg(test)]
mod tests {
    use strudeland_host::Layout;

    use super::*;
    use crate::level::true_peak::PHASES;

    const RATE: u32 = 1000;
    /// One bar at cps 1.0.
    const BAR: usize = RATE as usize;
    const BARS: u32 = 4;
    const MUSIC: usize = BARS as usize * BAR;
    const TAIL: f32 = 0.05;

    fn window() -> PlannedWindow {
        PlannedWindow {
            from_bar: 0,
            to_bar: BARS,
            layout: Layout::Linear,
        }
    }

    /// The opening's bar `b` is `0.1 * (b + 1)` on the left and half that on the right; an
    /// isolated body's bars are `0.5 + 0.1 * b`. The tail is `TAIL` (halved on the right).
    fn bar_value(bar: usize, isolated: bool) -> f32 {
        if isolated {
            0.5 + 0.1 * bar as f32
        } else {
            0.1 * (bar + 1) as f32
        }
    }

    fn stereo(left: Vec<f32>) -> Vec<Vec<f32>> {
        let right = left.iter().map(|x| x * 0.5).collect();
        vec![left, right]
    }

    fn bars(range: Range<usize>, isolated: bool, tail_frames: usize) -> Vec<Vec<f32>> {
        let mut left: Vec<f32> = range
            .flat_map(|bar| std::iter::repeat_n(bar_value(bar, isolated), BAR))
            .collect();
        left.extend(std::iter::repeat_n(TAIL, tail_frames));
        stereo(left)
    }

    /// A 4-bar window at 1000 Hz, cps 1.0, with a distinct constant block per bar and a distinct
    /// tail: `(channels, music_frames, loop_audio)`.
    fn fixture(
        start_bar: u32,
        isolated: bool,
        tail_frames: usize,
    ) -> (Vec<Vec<f32>>, u64, LoopAudio) {
        let start = start_bar as usize;
        let (channels, isolated_body) = if isolated {
            (
                bars(0..BARS as usize, false, 0),
                Some(bars(start..BARS as usize, true, tail_frames)),
            )
        } else {
            (bars(0..BARS as usize, false, tail_frames), None)
        };
        let loop_audio = LoopAudio {
            start_bar,
            start_frame: (start * BAR) as u64,
            isolated_body,
        };
        (channels, MUSIC as u64, loop_audio)
    }

    fn run(
        (channels, music, loop_audio): (Vec<Vec<f32>>, u64, LoopAudio),
        preset: ExportPreset,
    ) -> Result<Splice, RenderError> {
        splice(channels, music, window(), RATE, loop_audio, preset)
    }

    fn roles_of(spliced: &Splice) -> Vec<ExportRole> {
        spliced.cuts.iter().map(|cut| cut.role).collect()
    }

    /// The fold in the same order `fold_tail` sums.
    fn folded(channels: &[Vec<f32>], body: usize) -> Vec<Vec<f32>> {
        channels
            .iter()
            .map(|ch| {
                let mut out = ch[..body].to_vec();
                for cycle in ch[body..].chunks(body) {
                    for (sample, tail) in out.iter_mut().zip(cycle) {
                        *sample += *tail;
                    }
                }
                out
            })
            .collect()
    }

    #[test]
    fn folds_full_and_partial_repeats_into_a_body() {
        let mut channels = vec![vec![1., 2., 3., 4., 5., 6., 7.], vec![0.; 7]];
        fold_tail(&mut channels, 2).unwrap();
        assert_eq!(channels, [vec![16., 12.], vec![0.; 2]]);
        assert!(fold_tail(&mut channels, 0).is_err());
        assert!(fold_tail(&mut channels, 3).is_err());
    }

    #[test]
    fn the_opening_blend_matches_the_repeat_without_a_step_in_the_fade() {
        let mut opening = vec![vec![0.0; 30]];
        let repeat = vec![vec![1.0; 24]];
        join_opening(&mut opening, &repeat, 30, 1000).unwrap();
        let first = &opening[0];
        assert!(first[..20].iter().all(|&sample| sample == 0.0));
        assert!(first[19..30].windows(2).all(|pair| {
            let step = pair[1] - pair[0];
            step > 0.0 && step < 0.15
        }));
        assert_eq!(first[29], 1.0);
        assert_eq!(&first[30..], &repeat[0]);
    }

    #[test]
    fn measures_a_wrap_even_for_a_loop_shorter_than_the_filter() {
        let body = vec![vec![-0.9, 0.0]];
        let peak = wrap_peak(&body);
        assert!(peak.is_finite());
        assert!(peak > true_peak_db(&body));
    }

    #[test]
    fn intro_loop_cuts_an_intro_and_a_loop() {
        let tail = 300;
        let (channels, _, _) = fixture(2, true, tail);
        let spliced = run(fixture(2, true, tail), ExportPreset::IntroLoop).unwrap();
        assert_eq!(roles_of(&spliced), [ExportRole::Intro, ExportRole::Loop]);
        let [intro, repeat] = &spliced.cuts[..] else {
            panic!("two cuts");
        };
        let body = 2 * BAR;

        assert_eq!(intro.bars, 0..4);
        assert_eq!(intro.loop_range, None);
        assert_eq!(repeat.bars, 2..4);
        assert_eq!(repeat.loop_range, Some(0..body));

        let expected_loop = folded(&bars(2..4, true, tail), body);
        assert_eq!(&repeat.channels[..], &expected_loop[..]);
        for (c, ch) in repeat.channels.iter().enumerate() {
            let scale = if c == 0 { 1.0 } else { 0.5 };
            assert_eq!(ch.len(), body);
            let bar = bar_value(2, true) * scale;
            assert!(ch[..tail].iter().all(|&x| x == bar + TAIL * scale));
            assert!(ch[tail..BAR].iter().all(|&x| x == bar));
        }

        let blend = RATE as usize / 100;
        for ((cut, original), periodic) in intro.channels.iter().zip(&channels).zip(&expected_loop)
        {
            assert_eq!(cut.len(), MUSIC);
            assert_eq!(cut[..MUSIC - blend], original[..MUSIC - blend]);
            let last = *periodic.last().unwrap();
            assert_eq!(cut[MUSIC - 1], last);
            let before = original[MUSIC - 1];
            assert!(
                cut[MUSIC - blend..MUSIC - 1]
                    .windows(2)
                    .all(|pair| { before < pair[0] && pair[0] < pair[1] && pair[1] <= last })
            );
        }
    }

    #[test]
    fn tagged_and_folded_cut_one_file_with_the_repeat_marked() {
        let tail = 300;
        let body = 2 * BAR;
        let mut channels = Vec::new();
        for preset in [ExportPreset::TaggedLoop, ExportPreset::FoldedLoop] {
            let spliced = run(fixture(2, true, tail), preset).unwrap();
            assert_eq!(roles_of(&spliced), roles(Some(preset)));
            let [cut] = &spliced.cuts[..] else {
                panic!("one cut");
            };
            assert_eq!(cut.bars, 0..4);
            assert_eq!(cut.loop_range, Some(MUSIC..MUSIC + body));

            let expected_loop = folded(&bars(2..4, true, tail), body);
            let mut opening = bars(0..4, false, 0);
            join_opening(&mut opening, &expected_loop, MUSIC, RATE).unwrap();
            assert_eq!(&cut.channels[..], &opening[..]);
            channels.push(cut.channels.clone().into_owned());
        }
        assert_eq!(channels[0], channels[1]);
    }

    #[test]
    fn a_loop_at_bar_zero_folds_a_long_tail_more_than_once() {
        let body = MUSIC;
        let tail = body * 5 / 2;
        let (mut channels, _, _) = fixture(0, false, tail);
        // A tail that varies, so each cycle's contribution is visible in the sum.
        for (c, ch) in channels.iter_mut().enumerate() {
            for (i, x) in ch[MUSIC..].iter_mut().enumerate() {
                *x = 0.001 * (i % 7) as f32 + 0.01 * c as f32;
            }
        }
        let input = channels.clone();
        let loop_audio = LoopAudio {
            start_bar: 0,
            start_frame: 0,
            isolated_body: None,
        };
        let spliced = splice(
            channels,
            MUSIC as u64,
            window(),
            RATE,
            loop_audio,
            ExportPreset::IntroLoop,
        )
        .unwrap();
        let [intro, repeat] = &spliced.cuts[..] else {
            panic!("two cuts");
        };

        let expected = folded(&input, body);
        assert_eq!(&repeat.channels[..], &expected[..]);
        for (c, ch) in repeat.channels.iter().enumerate() {
            // The music plus two full tail cycles and, for the first half, the partial third.
            let cycles = |i: usize| (1..4).filter(|k| k * body + i < input[c].len()).count();
            assert_eq!(cycles(0), 3);
            assert_eq!(cycles(body - 1), 2);
            for (i, &x) in ch.iter().enumerate().step_by(997) {
                let sum: f32 = (0..=cycles(i)).map(|k| input[c][k * body + i]).sum();
                assert!((x - sum).abs() < 1e-6, "frame {i}: {x} vs {sum}");
            }
        }

        let mut opening: Vec<Vec<f32>> = input.iter().map(|ch| ch[..MUSIC].to_vec()).collect();
        join_opening(&mut opening, &expected, MUSIC, RATE).unwrap();
        for ch in &mut opening {
            ch.truncate(MUSIC);
        }
        assert_eq!(&intro.channels[..], &opening[..]);
        let blend = RATE as usize / 100;
        for (cut, original) in intro.channels.iter().zip(&input) {
            assert_eq!(cut[..MUSIC - blend], original[..MUSIC - blend]);
        }
    }

    #[test]
    fn splice_errors_name_the_bad_plan() {
        let error = |input, preset| match run(input, preset) {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.to_string(),
        };
        let tagged = ExportPreset::TaggedLoop;

        let (channels, music, mut outside) = fixture(1, true, 0);
        outside.start_bar = BARS;
        assert_eq!(
            error((channels, music, outside), tagged),
            "render failed: loop start is outside the planned window"
        );

        let (channels, music, mut empty) = fixture(3, true, 0);
        empty.start_frame = music;
        assert_eq!(
            error((channels, music, empty), tagged),
            "render failed: loop has no frames"
        );

        let (channels, music, _) = fixture(0, false, 0);
        let isolated_at_zero = LoopAudio {
            start_bar: 0,
            start_frame: 0,
            isolated_body: Some(bars(0..4, true, 0)),
        };
        assert_eq!(
            error((channels, music, isolated_at_zero), tagged),
            "render failed: the plan has inconsistent loop history"
        );
        let (channels, music, mut missing) = fixture(2, true, 0);
        missing.isolated_body = None;
        assert_eq!(
            error((channels, music, missing), tagged),
            "render failed: the plan has inconsistent loop history"
        );

        let (channels, music, mut short) = fixture(2, true, 0);
        short.isolated_body = Some(bars(2..3, true, 0));
        assert_eq!(
            error((channels, music, short), tagged),
            "render failed: loop extends past the rendered PCM"
        );

        let (mut channels, music, loop_audio) = fixture(2, true, 0);
        channels[1][5] = f32::NAN;
        assert_eq!(
            error((channels, music, loop_audio), tagged),
            "render failed: the render contains a non-finite sample (frame 5, channel 1)"
        );

        let (channels, music, loop_audio) = fixture(2, true, 0);
        assert_eq!(
            error((channels, music, loop_audio), ExportPreset::OneShot),
            "render failed: unexpected one-shot preset"
        );
    }

    /// A body that is silent but for the twelve frames around its wrap, which carry the signs of
    /// the Annex 2 filter's phase-1 taps: the input the filter turns into its largest peak.
    fn stepped_wrap(frames: usize) -> Vec<f32> {
        let mut body = vec![0.0; frames];
        for m in 0..TAPS_PER_PHASE / 2 {
            body[frames - 6 + m] = 0.5f32.copysign(PHASES[1][11 - m] as f32);
            body[m] = 0.5f32.copysign(PHASES[1][5 - m] as f32);
        }
        body
    }

    #[test]
    fn the_peak_covers_the_wrap() {
        let body = 2 * BAR;
        let blend = RATE as usize / 100;
        let repeat = stepped_wrap(body);
        // The opening's last 10 ms oppose the repeat's tail, so the blend's residue softens the
        // handoff's copy of the wrap.
        let mut opening = vec![0.0f32; MUSIC];
        for k in 0..blend {
            opening[MUSIC - 1 - k] = -repeat[body - 1 - k];
        }
        let stepped = LoopAudio {
            start_bar: 2,
            start_frame: (2 * BAR) as u64,
            isolated_body: Some(vec![repeat.clone(); 2]),
        };
        let spliced = run(
            (vec![opening; 2], MUSIC as u64, stepped),
            ExportPreset::TaggedLoop,
        )
        .unwrap();
        assert!(spliced.peak_db > spliced.program.true_peak_db);
        assert_eq!(spliced.peak_db, wrap_peak(&[repeat]));

        let smooth = LoopAudio {
            start_bar: 2,
            start_frame: (2 * BAR) as u64,
            isolated_body: Some(vec![vec![0.5; body]; 2]),
        };
        let spliced = run(
            (vec![vec![0.5; MUSIC]; 2], MUSIC as u64, smooth),
            ExportPreset::TaggedLoop,
        )
        .unwrap();
        assert_eq!(spliced.peak_db, spliced.program.true_peak_db);
    }
}
