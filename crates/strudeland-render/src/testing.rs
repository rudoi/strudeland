//! Test support for this crate's tests and the CLI's: a pipeline that returns canned results
//! and records what it was asked to do, and summary fixtures. Compiled into non-test builds, so
//! it never unwraps or indexes.

use std::path::PathBuf;

use strudeland_host::{Layout, PlannedWindow};

use crate::export::{Error, Pipeline, PublishStep, publish_file};
use crate::{
    ExportFile, ExportPreset, Format, Progress, RenderError, RenderOptions, RenderOutput,
    StemExport, StemGrouping, StemTarget, Summary, Timings,
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
