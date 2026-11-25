//! One export, shared by `strudeland render` and the desktop export: the request, where its
//! files go, the role table, the preview, render, the role check, the ordered publish of a mix
//! and the typed errors. Each caller keeps its adapter (flags or IPC, wording, progress).
//!
//! 1. [`check_length_for_preset`]: the loop-preset length rule, called by each adapter at its
//!    own point.
//! 2. [`prepare`] (the CLI: resolve only, primary role first, no stats) or
//!    [`prepare_previewed`] (the app: per role, resolve then stat, in table order; the
//!    [`Preview`]). Neither creates anything.
//! 3. [`Prepared::run`]: render and publish.

mod destination;
mod error;
pub(crate) mod fake;
mod pipeline;
mod publish;
mod request;

use std::cell::RefCell;
use std::path::{Path, PathBuf};

pub use destination::{Destination, Preview, PreviewFile};
pub use error::{Error, PartialWrite, RunError, WrittenPath};
pub use pipeline::{Pipeline, RealPipeline};
pub use publish::{PublishStep, ReplacePolicy, check_chosen_folder, publish_file};
pub use request::{ExportProblem, Kind, Request, check_length_for_preset, roles};

use destination::{MixNames, Place, stem_target};
use publish::Mode;
use strudeland_project::{InstrumentSnapshot, render_exists};

use crate::{
    BitDepth, EncoderOptions, ExportFile, ExportPreset, ExportRole, LoopPoints, Loudness,
    OutputOptions, Progress, RenderOptions, StemExport, StemGrouping, StemTarget, Summary,
};

/// One mix file's destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub role: ExportRole,
    pub path: PathBuf,
}

/// A request with its destination resolved. Nothing has been created.
#[derive(Debug)]
pub struct Prepared {
    request: Request,
    targets: Targets,
}

#[derive(Debug)]
enum Targets {
    Mix {
        files: Vec<Target>,
        place: Place,
    },
    Stems {
        grouping: StemGrouping,
        target: StemTarget,
    },
}

/// Resolves the destination without touching a mix target (the CLI): the roles are resolved in
/// reverse table order, primary first, and stored in table order. A stem set's availability is
/// left to the render.
pub fn prepare(request: Request, destination: Destination) -> Result<Prepared, Error> {
    let targets = match &request.kind {
        Kind::Mix { preset, output } => {
            let names = MixNames::new(
                &request,
                *preset,
                output.encoder.format().ext(),
                destination,
            )?;
            let mut files = roles(*preset)
                .iter()
                .rev()
                .map(|&role| {
                    Ok(Target {
                        role,
                        path: names.path(role)?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            files.reverse();
            Targets::Mix {
                files,
                place: names.into_place(),
            }
        }
        Kind::Stems { grouping } => Targets::Stems {
            grouping: *grouping,
            target: stem_target(&request, destination)?,
        },
    };
    Ok(Prepared { request, targets })
}

/// Resolves the destination and previews it (the app): each mix role in table order is
/// resolved, then checked for an existing file, before the next; a stem set's directory must be
/// available.
pub fn prepare_previewed(
    request: Request,
    destination: Destination,
) -> Result<(Prepared, Preview), Error> {
    let (targets, preview) = match &request.kind {
        Kind::Mix { preset, output } => {
            let names = MixNames::new(
                &request,
                *preset,
                output.encoder.format().ext(),
                destination,
            )?;
            let mut files = Vec::new();
            let mut preview = Vec::new();
            for &role in roles(*preset) {
                let path = names.path(role)?;
                let exists = render_exists(&path).map_err(|source| Error::Project {
                    at: path.clone(),
                    source,
                })?;
                preview.push(PreviewFile {
                    role,
                    path: path.clone(),
                    exists,
                });
                files.push(Target { role, path });
            }
            (
                Targets::Mix {
                    files,
                    place: names.into_place(),
                },
                Preview::Mix(preview),
            )
        }
        Kind::Stems { grouping } => {
            let target = stem_target(&request, destination)?;
            target.check_available().map_err(Error::Render)?;
            let directory = target.path().to_owned();
            (
                Targets::Stems {
                    grouping: *grouping,
                    target,
                },
                Preview::Stems { directory },
            )
        }
    };
    Ok((Prepared { request, targets }, preview))
}

/// A step of an export, reported from the calling thread.
#[derive(Debug)]
pub enum Event<'a> {
    /// A pipeline step, forwarded unchanged.
    Pipeline(Progress),
    /// A mix only: once, after the render and the role check, before the first file.
    Publishing,
    /// A mix only: one file was published. Reported before the next file is published.
    /// `new_warnings` are the file's warnings not reported for an earlier file of this export.
    Written {
        file: &'a Published,
        new_warnings: &'a [String],
    },
}

/// A published mix file.
#[derive(Debug, Clone, PartialEq)]
pub struct Published {
    pub role: ExportRole,
    /// The resolved target (the app reports this).
    pub path: PathBuf,
    /// `path` made absolute, or `path` when that fails (the CLI prints this).
    pub absolute: PathBuf,
    pub summary: Summary,
    pub loop_points: Option<LoopPoints>,
}

/// A finished export. Built once per export and moved to the adapter, so the stem set's
/// summary stays inline.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Finished {
    Mix {
        preset: Option<ExportPreset>,
        files: Vec<Published>,
        /// Every file's warnings, first seen first, without repeats.
        warnings: Vec<String>,
    },
    Stems {
        grouping: StemGrouping,
        export: StemExport,
    },
}

impl Prepared {
    /// The mix files in table order; empty for a stem set.
    pub fn targets(&self) -> &[Target] {
        match &self.targets {
            Targets::Mix { files, .. } => files,
            Targets::Stems { .. } => &[],
        }
    }

    /// The stem set's directory; `None` for a mix.
    pub fn stem_directory(&self) -> Option<&Path> {
        match &self.targets {
            Targets::Mix { .. } => None,
            Targets::Stems { target, .. } => Some(target.path()),
        }
    }

    /// The mix preset; `None` for a plain mix and for a stem set.
    pub fn preset(&self) -> Option<ExportPreset> {
        match &self.request.kind {
            Kind::Mix { preset, .. } => *preset,
            Kind::Stems { .. } => None,
        }
    }

    /// Renders and publishes. A mix is checked against the policy and the preview before it
    /// renders, then its files are published in table order. A stem set ignores the policy
    /// and the preview: it never replaces.
    pub fn run(
        self,
        pipeline: &mut dyn Pipeline,
        code: &str,
        policy: ReplacePolicy,
        preview: Option<&Preview>,
        instruments: InstrumentSnapshot,
        progress: &dyn Fn(Event<'_>),
    ) -> Result<Finished, RunError> {
        let preset = self.preset();
        let Prepared { request, targets } = self;
        let output = match &request.kind {
            Kind::Mix { output, .. } => *output,
            Kind::Stems { .. } => OutputOptions {
                loudness: Loudness::Off,
                encoder: EncoderOptions::Wav {
                    bit_depth: BitDepth::F32,
                },
            },
        };
        let opts = RenderOptions {
            instruments,
            length: request.length.clone(),
            jobs: request.jobs,
            plan_timeout: request.plan_timeout,
            include_events: false,
            output,
        };
        match targets {
            Targets::Mix { files, place } => run_mix(
                pipeline,
                code,
                &opts,
                MixRun {
                    preset,
                    files,
                    place,
                    policy,
                    preview,
                },
                progress,
            ),
            Targets::Stems { grouping, target } => {
                run_stems(pipeline, code, &opts, grouping, &target, progress)
            }
        }
    }
}

struct MixRun<'a> {
    preset: Option<ExportPreset>,
    files: Vec<Target>,
    place: Place,
    policy: ReplacePolicy,
    preview: Option<&'a Preview>,
}

/// Each file's publish mode, from the policy and the approved preview. Nothing has rendered.
fn modes(
    files: &[Target],
    policy: ReplacePolicy,
    preview: Option<&Preview>,
) -> Result<Vec<Mode>, Error> {
    let ReplacePolicy::CreateNew { approved } = policy else {
        return Ok(vec![Mode::Always; files.len()]);
    };
    let snapshot = match preview {
        None => return Err(Error::PreviewRequired),
        Some(Preview::Mix(snapshot)) => snapshot,
        Some(Preview::Stems { .. }) => return Err(Error::RolesMismatch),
    };
    if !snapshot
        .iter()
        .map(|file| (file.role, &file.path))
        .eq(files.iter().map(|target| (target.role, &target.path)))
    {
        return Err(Error::RolesMismatch);
    }
    if !approved && snapshot.iter().any(|file| file.exists) {
        return Err(Error::ReplacementNotApproved);
    }
    Ok(snapshot
        .iter()
        .map(|file| {
            if file.exists {
                Mode::Replace
            } else {
                Mode::Create
            }
        })
        .collect())
}

fn run_mix(
    pipeline: &mut dyn Pipeline,
    code: &str,
    opts: &RenderOptions,
    run: MixRun<'_>,
    progress: &dyn Fn(Event<'_>),
) -> Result<Finished, RunError> {
    let MixRun {
        preset,
        files,
        place,
        policy,
        preview,
    } = run;
    let modes = modes(&files, policy, preview).map_err(RunError::Failed)?;
    let forward = |p| progress(Event::Pipeline(p));
    let rendered = match preset {
        None => pipeline.render(code, opts, &forward).map(|output| {
            vec![ExportFile {
                role: ExportRole::Single,
                output,
                loop_points: None,
            }]
        }),
        Some(preset) => pipeline.render_export(code, opts, preset, &forward),
    }
    .map_err(|e| RunError::Failed(Error::Render(e)))?;
    if !rendered
        .iter()
        .map(|file| file.role)
        .eq(files.iter().map(|target| target.role))
    {
        return Err(RunError::Failed(Error::RolesMismatch));
    }
    progress(Event::Publishing);
    let mut published: Vec<Published> = Vec::with_capacity(files.len());
    let mut seen: Vec<String> = Vec::new();
    for (index, ((file, target), mode)) in rendered.into_iter().zip(files).zip(modes).enumerate() {
        let step = PublishStep {
            index,
            role: target.role,
            path: &target.path,
            mode,
            place: &place,
        };
        if let Err(source) = pipeline.publish(&step, &file.output.bytes) {
            return Err(if published.is_empty() {
                RunError::Failed(source)
            } else {
                RunError::PartialWrite(PartialWrite {
                    written: published
                        .iter()
                        .map(|file| WrittenPath {
                            path: file.path.clone(),
                            absolute: file.absolute.clone(),
                        })
                        .collect(),
                    failed: Some(target.path),
                    source,
                })
            });
        }
        let mut new_warnings = Vec::new();
        for warning in &file.output.summary.warnings {
            if !seen.contains(warning) {
                seen.push(warning.clone());
                new_warnings.push(warning.clone());
            }
        }
        let absolute = std::path::absolute(&target.path).unwrap_or_else(|_| target.path.clone());
        published.push(Published {
            role: file.role,
            path: target.path,
            absolute,
            summary: file.output.summary,
            loop_points: file.loop_points,
        });
        if let Some(file) = published.last() {
            progress(Event::Written {
                file,
                new_warnings: &new_warnings,
            });
        }
    }
    Ok(Finished::Mix {
        preset,
        files: published,
        warnings: seen,
    })
}

fn run_stems(
    pipeline: &mut dyn Pipeline,
    code: &str,
    opts: &RenderOptions,
    grouping: StemGrouping,
    target: &StemTarget,
    progress: &dyn Fn(Event<'_>),
) -> Result<Finished, RunError> {
    let written = RefCell::new(Vec::new());
    let result = pipeline.render_stems(code, opts, grouping, target, &|event| {
        if let Progress::Written { path } = &event {
            written.borrow_mut().push(WrittenPath {
                path: path.clone(),
                absolute: path.clone(),
            });
        }
        progress(Event::Pipeline(event));
    });
    match result {
        Ok(export) => Ok(Finished::Stems { grouping, export }),
        Err(e) => {
            let written = written.into_inner();
            Err(if written.is_empty() {
                RunError::Failed(Error::Render(e))
            } else {
                RunError::PartialWrite(PartialWrite {
                    written,
                    failed: None,
                    source: Error::Render(e),
                })
            })
        }
    }
}

#[cfg(test)]
mod tests;
