//! A pipeline that returns canned results and records what it was asked to do. Compiled into
//! non-test builds for the CLI's tests, so it never unwraps or indexes.

use std::path::PathBuf;

use super::Error;
use super::pipeline::Pipeline;
use super::publish::{PublishStep, publish_file};
use crate::{
    ExportFile, ExportPreset, Progress, RenderError, RenderOptions, RenderOutput, StemExport,
    StemGrouping, StemTarget,
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
    /// The directory of every `render_stems` call.
    pub stem_targets: Vec<PathBuf>,
    /// The paths of successful publishes.
    pub published: Vec<PathBuf>,
    /// Returned by the publish after `publishes_before_error` successful ones.
    pub publish_error: Option<Error>,
    pub publishes_before_error: usize,
    /// `false`: a publish records only and touches no disk; `true`: it also calls
    /// [`publish_file`].
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
        _grouping: StemGrouping,
        target: &StemTarget,
        progress: &dyn Fn(Progress),
    ) -> Result<StemExport, RenderError> {
        self.seen.push((code.to_owned(), opts.clone()));
        self.stem_targets.push(target.path().to_owned());
        self.emit(progress);
        missing(self.stems.take())
    }

    fn publish(&mut self, step: &PublishStep<'_>, bytes: &[u8]) -> Result<(), Error> {
        if self.published.len() == self.publishes_before_error
            && let Some(error) = self.publish_error.take()
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
