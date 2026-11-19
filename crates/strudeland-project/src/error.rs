use std::path::PathBuf;

/// Every error the project crate reports.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("{0} is not a strudeland project (no project.json)")]
    NotAProject(PathBuf),
    /// `found` is the JSON text of the `schema` value.
    #[error("unsupported project schema {found} (this app reads schema 1)")]
    UnsupportedSchema { found: String },
    /// A parse error, a missing or empty `name`, or a missing `schema`.
    #[error("invalid project.json: {0}")]
    InvalidManifest(String),
    #[error("{0} already contains files")]
    TargetNotEmpty(PathBuf),
    #[error("invalid project name: {0}")]
    InvalidName(String),
    #[error("invalid song name {0:?} (use letters, digits, space, - and _; 1 to 64 characters)")]
    InvalidSongName(String),
    #[error("song {0} already exists")]
    SongExists(String),
    #[error("song {0} not found")]
    SongNotFound(String),
    #[error("path {0:?} is outside the project's songs folder")]
    PathOutsideProject(String),
    #[error("invalid render name {0:?}")]
    InvalidRenderName(String),
    #[error("invalid memo name {0:?} (use letters, digits, space, - and _; 1 to 64 characters)")]
    InvalidRecordingName(String),
    /// The stem.
    #[error("memo {0} already exists")]
    RecordingExists(String),
    /// The stem.
    #[error("memo {0} not found")]
    RecordingNotFound(String),
    /// The text starts with `recordings/<stem>.json: `.
    #[error("invalid memo sidecar: {0}")]
    InvalidSidecar(String),
    /// `found` is the JSON text of the `schema` value.
    #[error("unsupported memo sidecar schema {found} (this app reads schema 1)")]
    UnsupportedSidecarSchema { found: String },
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl ProjectError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        ProjectError::Io {
            path: path.into(),
            source,
        }
    }
}
