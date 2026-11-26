//! Test support for this crate's tests and the CLI's: a pipeline that returns canned results
//! and records what it was asked to do, and summary fixtures. Compiled into non-test builds, so
//! it never unwraps or indexes.

use std::path::PathBuf;

use strudeland_host::{Layout, Meter, PlannedWindow, SAMPLE_RATE, Structure};

use crate::export::{Error, Pipeline, PublishStep, publish_file};
use crate::{
    ExportFile, ExportPreset, Format, Progress, RenderError, RenderOptions, RenderOutput,
    RenderedAudio, StemExport, StemGrouping, StemTarget, Summary, Timings,
};

#[derive(Debug, Default)]
pub struct FakePipeline {
    /// What `render` returns, once.
    pub render: Option<Result<RenderOutput, RenderError>>,
    /// What `render_export` returns, once.
    pub export: Option<Result<Vec<ExportFile>, RenderError>>,
    /// What `render_stems` returns, once.
    pub stems: Option<Result<StemExport, RenderError>>,
    /// Emitted by each render method before it returns.
    pub events: Vec<Progress>,
    /// The code and options of every render call.
    pub seen: Vec<(String, RenderOptions)>,
    /// The preset `render_export` was called with.
    pub preset: Option<ExportPreset>,
    /// The grouping `render_stems` was called with.
    pub grouping: Option<StemGrouping>,
    /// The directory of every `render_stems` call.
    pub stem_targets: Vec<PathBuf>,
    /// The paths of successful publishes.
    pub published: Vec<PathBuf>,
    /// `(n, error)`: the publish after `n` successful ones returns `error`, once.
    pub publish_error: Option<(usize, Error)>,
    /// `false`: a publish records only and touches no disk; `true`: it also publishes for real.
    pub real_publish: bool,
}

impl FakePipeline {
    fn emit(&self, progress: &dyn Fn(Progress)) {
        for event in &self.events {
            progress(event.clone());
        }
    }
}

fn missing<T>(result: Option<Result<T, RenderError>>) -> Result<T, RenderError> {
    result.unwrap_or_else(|| Err(RenderError::Render("fake pipeline: no result".into())))
}

impl Pipeline for FakePipeline {
    fn render(
        &mut self,
        code: &str,
        opts: &RenderOptions,
        progress: &dyn Fn(Progress),
    ) -> Result<RenderOutput, RenderError> {
        self.seen.push((code.to_owned(), opts.clone()));
        self.emit(progress);
        missing(self.render.take())
    }

    fn render_export(
        &mut self,
        code: &str,
        opts: &RenderOptions,
        preset: ExportPreset,
        progress: &dyn Fn(Progress),
    ) -> Result<Vec<ExportFile>, RenderError> {
        self.seen.push((code.to_owned(), opts.clone()));
        self.preset = Some(preset);
        self.emit(progress);
        missing(self.export.take())
    }

    fn render_stems(
        &mut self,
        code: &str,
        opts: &RenderOptions,
        grouping: StemGrouping,
        target: &StemTarget,
        progress: &dyn Fn(Progress),
    ) -> Result<StemExport, RenderError> {
        self.seen.push((code.to_owned(), opts.clone()));
        self.grouping = Some(grouping);
        self.stem_targets.push(target.path().to_owned());
        self.emit(progress);
        missing(self.stems.take())
    }

    fn publish(&mut self, step: &PublishStep<'_>, bytes: &[u8]) -> Result<(), Error> {
        if self
            .publish_error
            .as_ref()
            .is_some_and(|(before, _)| *before == self.published.len())
            && let Some((_, error)) = self.publish_error.take()
        {
            return Err(error);
        }
        if self.real_publish {
            publish_file(step, bytes)?;
        }
        self.published.push(step.path.to_owned());
        Ok(())
    }
}

/// A one-bar, 3-byte WAV summary with `warnings`.
pub fn summary(warnings: &[&str]) -> Summary {
    Summary {
        format: Some(Format::Wav),
        bytes: Some(3),
        window: PlannedWindow {
            from_bar: 0,
            to_bar: 1,
            layout: Layout::Linear,
        },
        frames: 0,
        seconds: 0.0,
        sample_rate: 44_100,
        chunks: 1,
        jobs: 1,
        loudness: None,
        timings: Timings::default(),
        warnings: warnings.iter().map(|w| w.to_string()).collect(),
    }
}

/// `bytes` with [`summary`]`(warnings)`.
pub fn output(bytes: &[u8], warnings: &[&str]) -> RenderOutput {
    RenderOutput {
        bytes: bytes.to_vec(),
        summary: summary(warnings),
    }
}

/// An empty song structure: 120 bpm in 4/4 (cps 0.5), no sections, one loop and zero bars.
pub fn structure() -> Structure {
    Structure {
        bpm: 120.0,
        meter: Meter { num: 4, den: 4 },
        beats_per_bar: 4.0,
        cps: 0.5,
        sections: vec![],
        loop_index: None,
        loop_range: None,
        meter_changes: None,
        loops: 1,
        total_bars: 0,
        intro_end: 0,
        end_of_song: None,
        clip_bars: 0,
        clip_seconds: 0.0,
    }
}

/// Joined audio for `channels` at `sample_rate`, as [`render_audio`](crate::render_audio) returns
/// it: one linear `0..1` window at cps 0.5 ([`structure`]), no chunks, events or plan errors, and
/// every frame musical. The summary is [`summary`]`(&[])` without a format or size.
pub fn audio(channels: Vec<Vec<f32>>, sample_rate: u32) -> RenderedAudio {
    let frames = channels.first().map_or(0, Vec::len) as u64;
    RenderedAudio {
        channels,
        music_frames: frames,
        cps: 0.5,
        structure: structure(),
        lookback_sec: 0.0,
        chunks: vec![],
        events: None,
        plan_errors: vec![],
        summary: Summary {
            format: None,
            bytes: None,
            frames,
            seconds: frames as f64 / f64::from(sample_rate),
            sample_rate,
            ..summary(&[])
        },
    }
}

/// Seeded stereo noise from an LCG: `frames` frames of the left channel, then the right, each
/// sample uniform in `-amp..amp`.
pub fn lcg_noise(seed: u32, frames: usize, amp: f32) -> Vec<Vec<f32>> {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (f64::from(state) / f64::from(u32::MAX) * 2.0 - 1.0) as f32 * amp
    };
    let l: Vec<f32> = (0..frames).map(|_| next()).collect();
    let r: Vec<f32> = (0..frames).map(|_| next()).collect();
    vec![l, r]
}

/// The test helper for a plan's frame counts (the pipeline reads them from the plan):
/// `round(to * sr / cps) - round(from * sr / cps)`: `Math.round((toBar * sampleRate) / cps)` minus
/// the same for `fromBar`. Operands are non-negative, so `f64::round` (ties away from zero) equals
/// JS `Math.round` (ties up). Multiply first, then divide, exactly as the TS does. The rounding is
/// applied to each bar's absolute frame, so the lengths of contiguous windows add up exactly.
/// Saturates at 0 when `to_bar` comes before `from_bar` (or `cps` is not a positive number).
pub fn window_frames(from_bar: u32, to_bar: u32, cps: f64) -> u64 {
    frame_of(to_bar, cps).saturating_sub(frame_of(from_bar, cps))
}

/// `Math.round((bar * sampleRate) / cps)` as a frame index (`as` saturates; NaN is 0).
fn frame_of(bar: u32, cps: f64) -> u64 {
    ((f64::from(bar) * f64::from(SAMPLE_RATE)) / cps).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The TS `windowFrames` evaluated by hand (Math.round of `bar * 44100 / cps`).
    #[test]
    fn window_frames_matches_the_ts_rounding() {
        // (from, to, cps, expected)
        let cases: [(u32, u32, f64, u64); 10] = [
            (0, 2, 0.5, 176_400),
            (0, 4, 0.5, 352_800),
            (3, 8, 0.5, 441_000),
            // cps = 0.5625: a bar is 78400 frames exactly.
            (0, 1, 0.5625, 78_400),
            (5, 7, 0.5625, 156_800),
            // cps = 4/3: a bar is 33075 frames.
            (0, 1, 4.0 / 3.0, 33_075),
            (1, 2, 4.0 / 3.0, 33_075),
            // cps = 0.4375: bar k at round(k * 100800) exactly; 3 / 0.4375 = 302400.
            (0, 3, 0.4375, 302_400),
            // cps = 0.7: bar 1 = 63000, bar 2 = 126000; 1/0.7 rounds, the sums telescope.
            (1, 3, 0.7, 126_000),
            (2, 2, 0.5, 0),
        ];
        for (from, to, cps, want) in cases {
            assert_eq!(
                window_frames(from, to, cps),
                want,
                "[{from}, {to}) at {cps}"
            );
        }
        // A non-whole bar length: cps = 0.3 gives 147000.00000000003 per bar.
        assert_eq!(frame_of(1, 0.3), 147_000);
        // A tie rounds up (JS Math.round), as f64::round does for positive numbers: cps = 88200
        // makes bar 1 frame 0.5.
        assert_eq!(frame_of(1, 88_200.0), 1);
        assert_eq!(frame_of(3, 88_200.0), 2);
        // Reversed, NaN and non-positive cps saturate instead of panicking.
        assert_eq!(window_frames(4, 2, 0.5), 0);
        assert_eq!(window_frames(0, 4, f64::NAN), 0);
        assert_eq!(window_frames(0, 4, -1.0), 0);
        // Telescoping: contiguous windows add up to the whole window at awkward tempos.
        for cps in [0.4375, 4.0 / 3.0, 0.7, 0.3, 0.5625, 0.123_456_789] {
            let parts: u64 = (0..37).map(|b| window_frames(b, b + 1, cps)).sum();
            assert_eq!(parts, window_frames(0, 37, cps), "cps {cps}");
        }
    }
}
