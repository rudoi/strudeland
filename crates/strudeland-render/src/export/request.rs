//! The export request, the role table and the loop-preset length rule.

use std::path::PathBuf;
use std::time::Duration;

use super::Error;
use crate::output::{export_suffix, render_suffix};
use crate::{ExportPreset, ExportRole, LengthRequest, OutputOptions, StemGrouping};

/// The files an export writes, in publish order. `None` is one `Single` file rendered with
/// [`crate::render`], not [`crate::render_export`] with `OneShot`.
pub fn roles(preset: Option<ExportPreset>) -> &'static [ExportRole] {
    match preset {
        None | Some(ExportPreset::OneShot | ExportPreset::TaggedLoop) => &[ExportRole::Single],
        Some(ExportPreset::FoldedLoop) => &[ExportRole::Loop],
        Some(ExportPreset::IntroLoop) => &[ExportRole::Intro, ExportRole::Loop],
    }
}

/// A rule an export request breaks. Each adapter words it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportProblem {
    /// A loop preset selects its own bars, so it takes only the plain clip.
    LoopPresetLength,
}

/// `Err` iff `preset` is a loop preset (not `OneShot`) and `length` is not `Clip { loops: None }`.
pub fn check_length_for_preset(
    length: &LengthRequest,
    preset: Option<ExportPreset>,
) -> Result<(), ExportProblem> {
    let loop_preset = preset.is_some_and(|preset| preset != ExportPreset::OneShot);
    if loop_preset && !matches!(length, LengthRequest::Clip { loops: None }) {
        return Err(ExportProblem::LoopPresetLength);
    }
    Ok(())
}

/// What to export.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// The song file. It may not exist (an unsaved editor buffer).
    pub song: PathBuf,
    pub length: LengthRequest,
    pub kind: Kind,
    pub jobs: Option<usize>,
    pub plan_timeout: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// Finished files, one per role of the preset.
    Mix {
        preset: Option<ExportPreset>,
        output: OutputOptions,
    },
    /// A pre-master set. Its output is fixed: loudness off, 32-bit float WAV.
    Stems { grouping: StemGrouping },
}

/// The file-name suffix of one role: the length's for a plain render, the preset's otherwise.
pub(super) fn suffix(
    length: &LengthRequest,
    preset: Option<ExportPreset>,
    role: ExportRole,
) -> Result<Option<String>, Error> {
    match preset {
        None => Ok(render_suffix(length)),
        Some(preset) => export_suffix(length, preset, role)
            .map(Some)
            .map_err(Error::Render),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strudeland_host::Layout;

    const PRESETS: [Option<ExportPreset>; 5] = [
        None,
        Some(ExportPreset::OneShot),
        Some(ExportPreset::TaggedLoop),
        Some(ExportPreset::FoldedLoop),
        Some(ExportPreset::IntroLoop),
    ];

    #[test]
    fn roles_table() {
        use ExportRole::*;
        let table: Vec<_> = PRESETS.iter().map(|&p| (p, roles(p))).collect();
        assert_eq!(
            table,
            [
                (None, &[Single][..]),
                (Some(ExportPreset::OneShot), &[Single][..]),
                (Some(ExportPreset::TaggedLoop), &[Single][..]),
                (Some(ExportPreset::FoldedLoop), &[Loop][..]),
                (Some(ExportPreset::IntroLoop), &[Intro, Loop][..]),
            ]
        );
    }

    #[test]
    fn loop_presets_take_only_a_plain_clip() {
        let lengths = [
            LengthRequest::Clip { loops: None },
            LengthRequest::Clip { loops: Some(2) },
            LengthRequest::Section {
                name: "body".into(),
            },
            LengthRequest::Length { bars: 4 },
            LengthRequest::Window {
                from_bar: 0,
                to_bar: 4,
                layout: Layout::Clip,
            },
        ];
        for preset in PRESETS {
            let loop_preset = matches!(
                preset,
                Some(ExportPreset::TaggedLoop | ExportPreset::FoldedLoop | ExportPreset::IntroLoop)
            );
            for length in &lengths {
                let plain = *length == LengthRequest::Clip { loops: None };
                let want = if loop_preset && !plain {
                    Err(ExportProblem::LoopPresetLength)
                } else {
                    Ok(())
                };
                assert_eq!(
                    check_length_for_preset(length, preset),
                    want,
                    "{preset:?} {length:?}"
                );
            }
        }
    }

    #[test]
    fn suffixes_follow_the_role_table() {
        let length = LengthRequest::Clip { loops: None };
        for preset in PRESETS.into_iter().flatten() {
            for role in [ExportRole::Intro, ExportRole::Loop, ExportRole::Single] {
                let result = export_suffix(&length, preset, role);
                if roles(Some(preset)).contains(&role) {
                    assert!(result.is_ok(), "{preset:?} {role:?}");
                } else {
                    assert_eq!(
                        result.unwrap_err().to_string(),
                        "render failed: invalid output role for export preset",
                        "{preset:?} {role:?}"
                    );
                }
            }
        }
    }
}
