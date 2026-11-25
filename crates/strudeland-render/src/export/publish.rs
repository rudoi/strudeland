//! Publishing one mix file under a replace policy.

use std::path::Path;

use strudeland_project::{atomic_create_bytes, atomic_write_bytes, render_exists};

use super::Error;
use super::destination::Place;
use crate::ExportRole;

/// Whether a mix may replace existing files. A stem set never replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacePolicy {
    /// Replace whatever is there, without looking first (the CLI).
    Always,
    /// Replace only the files the approved preview said exist; create the others without
    /// clobbering a file that appeared since (the app). `approved` is the user's answer to the
    /// replacement prompt.
    CreateNew { approved: bool },
}

/// How one file is published, chosen by the policy and the preview snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    /// `ReplacePolicy::Always`: write over the path, never stat it.
    Always,
    /// `CreateNew` and the snapshot said the file exists: check it is still a regular file,
    /// then replace it.
    Replace,
    /// `CreateNew` and the snapshot said no file: create it, never replacing one.
    Create,
}

/// One mix file about to be published.
#[derive(Debug)]
pub struct PublishStep<'a> {
    /// The file's position in the role table.
    pub index: usize,
    pub role: ExportRole,
    pub path: &'a Path,
    pub(super) mode: Mode,
    pub(super) place: &'a Place,
}

/// Publishes one mix file atomically, by the destination and the mode.
pub fn publish_file(step: &PublishStep<'_>, bytes: &[u8]) -> Result<(), Error> {
    let path = step.path;
    let project_error = |source| Error::Project {
        at: path.to_owned(),
        source,
    };
    if let Place::ChosenFolder(folder) = step.place {
        check_chosen_folder(folder)?;
    }
    if step.mode == Mode::Replace {
        render_exists(path).map_err(project_error)?;
    }
    match (step.place, step.mode) {
        (Place::Project(project), Mode::Always | Mode::Replace) => {
            project.write_render(path, bytes)
        }
        (Place::Project(project), Mode::Create) => project.create_render(path, bytes),
        (Place::Explicit | Place::ChosenFolder(_), Mode::Always | Mode::Replace) => {
            atomic_write_bytes(path, bytes)
        }
        (Place::Explicit | Place::ChosenFolder(_), Mode::Create) => {
            atomic_create_bytes(path, bytes)
        }
    }
    .map_err(project_error)
}

/// The chosen folder must still be the directory the user picked: a directory whose canonical
/// form is the path itself.
pub fn check_chosen_folder(folder: &Path) -> Result<(), Error> {
    let changed = || Error::FolderChanged {
        path: folder.to_owned(),
    };
    if !folder.is_dir() {
        return Err(changed());
    }
    let canonical = folder
        .canonicalize()
        .map_err(|source| Error::FolderUnreadable {
            path: folder.to_owned(),
            source,
        })?;
    if canonical != folder {
        return Err(changed());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_folder_must_be_the_canonical_directory() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().canonicalize().unwrap();
        check_chosen_folder(&folder).unwrap();
        let missing = folder.join("missing");
        assert!(matches!(
            check_chosen_folder(&missing),
            Err(Error::FolderChanged { path }) if path == missing
        ));
        let file = folder.join("file");
        std::fs::write(&file, b"x").unwrap();
        assert!(matches!(
            check_chosen_folder(&file),
            Err(Error::FolderChanged { .. })
        ));
        #[cfg(unix)]
        {
            let link = folder.join("link");
            std::os::unix::fs::symlink(&folder, &link).unwrap();
            assert!(matches!(
                check_chosen_folder(&link),
                Err(Error::FolderChanged { path }) if path == link
            ));
        }
    }
}
