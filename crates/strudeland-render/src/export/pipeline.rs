//! The seam between an export and the render pipeline.

use super::Error;
use super::publish::{PublishStep, publish_file};
use crate::{
    ExportFile, ExportPreset, Progress, RenderError, RenderOptions, RenderOutput, StemExport,
    StemGrouping, StemTarget,
};

/// What an export needs from the render pipeline, so tests can inject a fake.
pub trait Pipeline {
    /// Plans, renders, levels and encodes one file.
    fn render(
        &mut self,
        code: &str,
        opts: &RenderOptions,
        progress: &dyn Fn(Progress),
    ) -> Result<RenderOutput, RenderError>;

    /// Plans, renders, levels and encodes every file of a preset.
    fn render_export(
        &mut self,
        code: &str,
        opts: &RenderOptions,
        preset: ExportPreset,
        progress: &dyn Fn(Progress),
    ) -> Result<Vec<ExportFile>, RenderError>;

    /// Plans, renders and writes a pre-master reference mix and its stems.
    fn render_stems(
        &mut self,
        code: &str,
        opts: &RenderOptions,
        grouping: StemGrouping,
        target: &StemTarget,
        progress: &dyn Fn(Progress),
    ) -> Result<StemExport, RenderError>;

    /// Publishes one mix file. The default is the real rule.
    fn publish(&mut self, step: &PublishStep<'_>, bytes: &[u8]) -> Result<(), Error> {
        publish_file(step, bytes)
    }
}

/// The real pipeline: [`crate::render`], [`crate::render_export`] and [`crate::stems::render_stems`].
#[derive(Debug, Clone, Copy, Default)]
pub struct RealPipeline;

impl Pipeline for RealPipeline {
    fn render(
        &mut self,
        code: &str,
        opts: &RenderOptions,
        progress: &dyn Fn(Progress),
    ) -> Result<RenderOutput, RenderError> {
        crate::render(code, opts, progress)
    }

    fn render_export(
        &mut self,
        code: &str,
        opts: &RenderOptions,
        preset: ExportPreset,
        progress: &dyn Fn(Progress),
    ) -> Result<Vec<ExportFile>, RenderError> {
        crate::render_export(code, opts, preset, progress)
    }

    fn render_stems(
        &mut self,
        code: &str,
        opts: &RenderOptions,
        grouping: StemGrouping,
        target: &StemTarget,
        progress: &dyn Fn(Progress),
    ) -> Result<StemExport, RenderError> {
        crate::stems::render_stems(code, opts, grouping, target, progress)
    }
}
