//! Why an export failed. [`Error`] has no `Display`: each adapter words it.

use std::io;
use std::path::PathBuf;

use strudeland_project::ProjectError;

use crate::RenderError;
use crate::output::io_error;

#[derive(Debug)]
pub enum Error {
    /// The pipeline or a resolver failed.
    Render(RenderError),
    /// A project rule or a write failed. `at` is the path the CLI names for an error that has
    /// none of its own.
    Project { at: PathBuf, source: ProjectError },
    /// The chosen folder is no longer the directory the user picked.
    FolderChanged { path: PathBuf },
    /// The chosen folder could not be resolved.
    FolderUnreadable { path: PathBuf, source: io::Error },
    /// The song's file name has no UTF-8 stem.
    InvalidSongName,
    /// The rendered files, or the preview, do not match the destinations' roles and paths.
    RolesMismatch,
    /// The preview said a file exists and the replacement was not approved.
    ReplacementNotApproved,
    /// `ReplacePolicy::CreateNew` needs the preview the user approved.
    PreviewRequired,
}

/// An export that failed before anything was published, or after some files were.
#[derive(Debug)]
pub enum RunError {
    Failed(Error),
    PartialWrite(PartialWrite),
}

/// At least one file was published before `source` stopped the export.
#[derive(Debug)]
pub struct PartialWrite {
    /// Never empty.
    pub written: Vec<WrittenPath>,
    /// The mix file that failed (its target path); `None` for a stem set.
    pub failed: Option<PathBuf>,
    pub source: Error,
}

/// A published file: the path as resolved, and as the CLI prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenPath {
    pub path: PathBuf,
    pub absolute: PathBuf,
}

/// The pipeline's wording, which the CLI prints.
impl From<Error> for RenderError {
    fn from(error: Error) -> Self {
        match error {
            Error::Render(e) => e,
            Error::Project { at, source } => io_error(&at, source),
            Error::FolderChanged { path } => RenderError::Io {
                path,
                source: io::Error::other("the chosen output folder changed"),
            },
            Error::FolderUnreadable { path, source } => RenderError::Io { path, source },
            Error::InvalidSongName => RenderError::Render("invalid song filename".into()),
            Error::RolesMismatch => {
                RenderError::Render("the rendered files do not match the destinations".into())
            }
            Error::ReplacementNotApproved => {
                RenderError::Render("replacement requires confirmation".into())
            }
            Error::PreviewRequired => RenderError::Render("the export has no preview".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_word_as_the_pipeline_did() {
        let cases: Vec<(Error, &str)> = vec![
            (
                Error::Render(RenderError::Encode("boom".into())),
                "encode failed: boom",
            ),
            (
                Error::Project {
                    at: "/p/renders".into(),
                    source: ProjectError::InvalidRenderName("!!".into()),
                },
                "/p/renders: invalid render name \"!!\"",
            ),
            (
                Error::Project {
                    at: "/p/renders".into(),
                    source: ProjectError::Io {
                        path: "/p/renders/a.wav".into(),
                        source: io::Error::other("denied"),
                    },
                },
                "/p/renders/a.wav: denied",
            ),
            (
                Error::FolderChanged {
                    path: "/out".into(),
                },
                "/out: the chosen output folder changed",
            ),
            (
                Error::FolderUnreadable {
                    path: "/out".into(),
                    source: io::Error::other("gone"),
                },
                "/out: gone",
            ),
            (
                Error::InvalidSongName,
                "render failed: invalid song filename",
            ),
            (
                Error::RolesMismatch,
                "render failed: the rendered files do not match the destinations",
            ),
            (
                Error::ReplacementNotApproved,
                "render failed: replacement requires confirmation",
            ),
            (
                Error::PreviewRequired,
                "render failed: the export has no preview",
            ),
        ];
        for (error, text) in cases {
            let label = format!("{error:?}");
            let e = RenderError::from(error);
            assert_eq!(e.to_string(), text, "{label}");
        }
        assert!(matches!(
            RenderError::from(Error::Render(RenderError::Usage {
                flag: "--out",
                message: "m".into()
            })),
            RenderError::Usage { flag: "--out", .. }
        ));
    }
}
