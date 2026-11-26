//! The one error every fallible Tauri command rejects with: a [`CommandErrorKind`] from one flat
//! list and the message the user sees. Each webview adapter narrows the kind to the ones its
//! feature handles; any other kind is `other` there.
//!
//! A project error converts through one table ([`From<ProjectError>`]); the recording commands
//! use [`CommandError::for_memos`], which keeps a song's errors as `other`. There is no
//! `From<String>`: a string converts with an explicit [`CommandError::other`].

use std::fmt::{self, Display};

use serde::Serialize;
use strudeland_project::ProjectError;

const CHANGED_MESSAGE: &str = "the project changed";
const NO_PROJECT_MESSAGE: &str = "no project is open";

/// A command's rejection: `{ kind, message }` on the wire.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub kind: CommandErrorKind,
    pub message: String,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CommandErrorKind {
    ProjectChanged,
    NoProject,
    NotFound,
    Exists,
    InvalidName,
    Busy,
    PermissionDenied,
    NoDevice,
    DeviceLost,
    /// Only ever carried by a `recording://state` event.
    LimitReached,
    Stale,
    Output,
    BadBatch,
    Engine,
    Other,
}

/// What a project error names: a song or a memo. Only the name kinds have one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subject {
    Song,
    Memo,
}

/// The kind of a project error, and whether it is about a song or a memo. One `match` with no
/// `_` arm, so a new variant does not compile until it has a kind.
fn project_kind(e: &ProjectError) -> (CommandErrorKind, Option<Subject>) {
    use CommandErrorKind as K;
    match e {
        ProjectError::SongExists(_) => (K::Exists, Some(Subject::Song)),
        ProjectError::RecordingExists(_) => (K::Exists, Some(Subject::Memo)),
        ProjectError::SongNotFound(_) => (K::NotFound, Some(Subject::Song)),
        ProjectError::RecordingNotFound(_) => (K::NotFound, Some(Subject::Memo)),
        ProjectError::InvalidSongName(_) => (K::InvalidName, Some(Subject::Song)),
        ProjectError::InvalidRecordingName(_) => (K::InvalidName, Some(Subject::Memo)),
        ProjectError::NotAProject(_)
        | ProjectError::UnsupportedSchema { .. }
        | ProjectError::InvalidManifest(_)
        | ProjectError::TargetNotEmpty(_)
        | ProjectError::InvalidName(_)
        | ProjectError::PathOutsideProject(_)
        | ProjectError::Instrument { .. }
        | ProjectError::InvalidRenderName(_)
        | ProjectError::InvalidSidecar(_)
        | ProjectError::UnsupportedSidecarSchema { .. }
        | ProjectError::Io { .. } => (K::Other, None),
    }
}

impl CommandError {
    pub fn new(kind: CommandErrorKind, message: impl Into<String>) -> Self {
        CommandError {
            kind,
            message: message.into(),
        }
    }

    /// `.map_err(CommandError::other)`: the error's `Display` text, kind `other`.
    pub fn other(message: impl Display) -> Self {
        CommandError::new(CommandErrorKind::Other, message.to_string())
    }

    pub fn busy(message: impl Into<String>) -> Self {
        CommandError::new(CommandErrorKind::Busy, message)
    }

    /// "the project changed": the command's `root` is not the open project.
    pub fn project_changed() -> Self {
        CommandError::new(CommandErrorKind::ProjectChanged, CHANGED_MESSAGE)
    }

    /// "no project is open".
    pub fn no_project() -> Self {
        CommandError::new(CommandErrorKind::NoProject, NO_PROJECT_MESSAGE)
    }

    /// A recording command's project error: as `From`, except that a song's name error is
    /// `other` (memo-ui shows `other` as a notice and hides the memo name kinds).
    pub fn for_memos(e: ProjectError) -> Self {
        CommandError::for_memo_error(&e)
    }

    /// [`CommandError::for_memos`] by reference (the take lifecycle shares its memo errors).
    pub fn for_memo_error(e: &ProjectError) -> Self {
        let kind = match project_kind(e) {
            (_, Some(Subject::Song)) => CommandErrorKind::Other,
            (kind, _) => kind,
        };
        CommandError::new(kind, e.to_string())
    }
}

/// The message alone, so a `to_string()` of a command error is the text the user sees.
impl Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}

impl From<ProjectError> for CommandError {
    fn from(e: ProjectError) -> Self {
        let (kind, _) = project_kind(&e);
        CommandError::new(kind, e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CommandErrorKind as K;
    use std::path::PathBuf;

    fn every_project_error() -> Vec<(ProjectError, K, &'static str)> {
        vec![
            (
                ProjectError::NotAProject(PathBuf::from("/p")),
                K::Other,
                "/p is not a strudeland project (no project.json)",
            ),
            (
                ProjectError::UnsupportedSchema { found: "9".into() },
                K::Other,
                "unsupported project schema 9 (this app reads schema 1)",
            ),
            (
                ProjectError::InvalidManifest("bad".into()),
                K::Other,
                "invalid project.json: bad",
            ),
            (
                ProjectError::TargetNotEmpty(PathBuf::from("/p")),
                K::Other,
                "/p already contains files",
            ),
            (
                ProjectError::InvalidName("x".into()),
                K::Other,
                "invalid project name: x",
            ),
            (
                ProjectError::InvalidSongName("a/b".into()),
                K::InvalidName,
                "invalid song name \"a/b\" (use letters, digits, space, - and _; 1 to 64 characters)",
            ),
            (
                ProjectError::SongExists("songs/x.js".into()),
                K::Exists,
                "song songs/x.js already exists",
            ),
            (
                ProjectError::SongNotFound("songs/x.js".into()),
                K::NotFound,
                "song songs/x.js not found",
            ),
            (
                ProjectError::PathOutsideProject("../x".into()),
                K::Other,
                "path \"../x\" is outside the project's songs folder",
            ),
            (
                ProjectError::Instrument {
                    id: "i".into(),
                    reason: "r".into(),
                },
                K::Other,
                "instrument \"i\": r",
            ),
            (
                ProjectError::InvalidRenderName("r".into()),
                K::Other,
                "invalid render name \"r\"",
            ),
            (
                ProjectError::InvalidRecordingName("a/b".into()),
                K::InvalidName,
                "invalid memo name \"a/b\" (use letters, digits, space, - and _; 1 to 64 characters)",
            ),
            (
                ProjectError::RecordingExists("take".into()),
                K::Exists,
                "memo take already exists",
            ),
            (
                ProjectError::RecordingNotFound("take".into()),
                K::NotFound,
                "memo take not found",
            ),
            (
                ProjectError::InvalidSidecar("recordings/take.json: x".into()),
                K::Other,
                "invalid memo sidecar: recordings/take.json: x",
            ),
            (
                ProjectError::UnsupportedSidecarSchema { found: "2".into() },
                K::Other,
                "unsupported memo sidecar schema 2 (this app reads schema 1)",
            ),
            (
                ProjectError::Io {
                    path: PathBuf::from("/p/x"),
                    source: std::io::Error::other("disk"),
                },
                K::Other,
                "/p/x: disk",
            ),
        ]
    }

    #[test]
    fn project_errors_map_through_one_table() {
        for (e, kind, message) in every_project_error() {
            assert_eq!(CommandError::from(e), CommandError::new(kind, message));
        }
    }

    #[test]
    fn memo_commands_keep_song_errors_as_other() {
        for (e, kind, message) in every_project_error() {
            let song = matches!(
                e,
                ProjectError::SongExists(_)
                    | ProjectError::SongNotFound(_)
                    | ProjectError::InvalidSongName(_)
            );
            let expected = if song { K::Other } else { kind };
            assert_eq!(
                CommandError::for_memos(e),
                CommandError::new(expected, message)
            );
        }
        let kind = |e| CommandError::for_memos(e).kind;
        assert_eq!(kind(ProjectError::RecordingExists("a".into())), K::Exists);
        assert_eq!(
            kind(ProjectError::RecordingNotFound("a".into())),
            K::NotFound
        );
        assert_eq!(
            kind(ProjectError::InvalidRecordingName("a/b".into())),
            K::InvalidName
        );
        assert_eq!(kind(ProjectError::SongExists("a".into())), K::Other);
    }

    #[test]
    fn every_kind_has_its_wire_name() {
        let err = CommandError::new(K::Exists, "song songs/x.js already exists");
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"kind":"exists","message":"song songs/x.js already exists"}"#
        );
        for (kind, wire) in [
            (K::ProjectChanged, "projectChanged"),
            (K::NoProject, "noProject"),
            (K::NotFound, "notFound"),
            (K::Exists, "exists"),
            (K::InvalidName, "invalidName"),
            (K::Busy, "busy"),
            (K::PermissionDenied, "permissionDenied"),
            (K::NoDevice, "noDevice"),
            (K::DeviceLost, "deviceLost"),
            (K::LimitReached, "limitReached"),
            (K::Stale, "stale"),
            (K::Output, "output"),
            (K::BadBatch, "badBatch"),
            (K::Engine, "engine"),
            (K::Other, "other"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), wire);
        }
    }

    #[test]
    fn display_is_the_message() {
        let err = CommandError::new(K::Busy, "a take is in progress");
        assert_eq!(err.to_string(), "a take is in progress");
        assert_eq!(CommandError::other("x").to_string(), "x");
        assert_eq!(CommandError::busy("b"), CommandError::new(K::Busy, "b"));
    }

    #[test]
    fn the_project_constructors_keep_their_texts() {
        assert_eq!(
            CommandError::project_changed(),
            CommandError::new(K::ProjectChanged, "the project changed")
        );
        assert_eq!(
            CommandError::no_project(),
            CommandError::new(K::NoProject, "no project is open")
        );
    }
}
