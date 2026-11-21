//! App preferences, stored in the app data dir with `"schema": 1`.
//!
//! Later preferences are added as fields with defaults, with no schema bump.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::ProjectError;
use crate::project::atomic_write;

/// The prefs file name inside the app data dir.
pub const PREFS_FILE: &str = "prefs.json";

const PREFS_SCHEMA: u32 = 1;

/// The app preferences. Unknown keys in the file are ignored; missing keys take their default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// Vim keybindings in the editor.
    pub vim: bool,
    /// Preview typed notes while song playback is stopped.
    pub audition_notes: bool,
    /// Format supported Strudel patterns when saving a song.
    pub format_on_save: bool,
}

/// The on-disk shape.
#[derive(Serialize, Deserialize)]
struct PrefsFile {
    schema: u32,
    #[serde(flatten)]
    prefs: Prefs,
}

impl Prefs {
    /// Reads the prefs. Best effort: a missing, unreadable, corrupt or wrong-schema file gives
    /// the defaults.
    pub fn load(file: &Path) -> Prefs {
        fs::read_to_string(file)
            .ok()
            .and_then(|text| serde_json::from_str::<PrefsFile>(&text).ok())
            .filter(|f| f.schema == PREFS_SCHEMA)
            .map(|f| f.prefs)
            .unwrap_or_default()
    }

    /// Writes the prefs atomically, creating the parent directory if needed.
    pub fn save(&self, file: &Path) -> Result<(), ProjectError> {
        if let Some(parent) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| ProjectError::io(parent, e))?;
        }
        let on_disk = PrefsFile {
            schema: PREFS_SCHEMA,
            prefs: self.clone(),
        };
        let mut json = serde_json::to_string_pretty(&on_disk).expect("the prefs always serialise");
        json.push('\n');
        atomic_write(file, &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempdir();
        assert_eq!(Prefs::load(&dir.path().join(PREFS_FILE)), Prefs::default());
        assert!(!Prefs::default().vim);
        assert!(!Prefs::default().audition_notes);
        assert!(!Prefs::default().format_on_save);
    }

    #[test]
    fn corrupt_file_gives_defaults() {
        let dir = tempdir();
        let file = dir.path().join(PREFS_FILE);
        fs::write(&file, "{ not json").unwrap();
        assert_eq!(Prefs::load(&file), Prefs::default());
    }

    #[test]
    fn unreadable_file_gives_defaults() {
        let dir = tempdir();
        // A directory where the file should be cannot be read as text.
        let file = dir.path().join(PREFS_FILE);
        fs::create_dir(&file).unwrap();
        assert_eq!(Prefs::load(&file), Prefs::default());
    }

    #[test]
    fn wrong_schema_gives_defaults() {
        let dir = tempdir();
        let file = dir.path().join(PREFS_FILE);
        fs::write(&file, r#"{ "schema": 2, "vim": true }"#).unwrap();
        assert_eq!(Prefs::load(&file), Prefs::default());
        fs::write(&file, r#"{ "vim": true }"#).unwrap();
        assert_eq!(Prefs::load(&file), Prefs::default());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let dir = tempdir();
        let file = dir.path().join(PREFS_FILE);
        fs::write(&file, r#"{ "schema": 1, "vim": true, "future": [1, 2] }"#).unwrap();
        assert!(Prefs::load(&file).vim);
    }

    #[test]
    fn missing_keys_default() {
        let dir = tempdir();
        let file = dir.path().join(PREFS_FILE);
        fs::write(&file, r#"{ "schema": 1 }"#).unwrap();
        assert_eq!(Prefs::load(&file), Prefs::default());

        fs::write(&file, r#"{ "schema": 1, "vim": true }"#).unwrap();
        let prefs = Prefs::load(&file);
        assert!(prefs.vim);
        assert!(!prefs.audition_notes);
        assert!(!prefs.format_on_save);

        fs::write(
            &file,
            r#"{ "schema": 1, "vim": true, "auditionNotes": true }"#,
        )
        .unwrap();
        let prefs = Prefs::load(&file);
        assert!(prefs.vim);
        assert!(prefs.audition_notes);
        assert!(!prefs.format_on_save);
    }

    #[test]
    fn save_creates_the_parent_dir() {
        let dir = tempdir();
        let file = dir.path().join("a/b").join(PREFS_FILE);
        Prefs::default().save(&file).unwrap();
        assert!(file.is_file());
    }

    #[test]
    fn round_trip_writes_pretty_json_with_a_newline() {
        let dir = tempdir();
        let file = dir.path().join(PREFS_FILE);
        let prefs = Prefs {
            vim: true,
            audition_notes: true,
            format_on_save: true,
        };
        prefs.save(&file).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\"schema\": 1"));
        assert!(text.contains("\"vim\": true"));
        assert!(text.contains("\"auditionNotes\": true"));
        assert!(text.contains("\"formatOnSave\": true"));
        assert_eq!(Prefs::load(&file), prefs);
        Prefs::default().save(&file).unwrap();
        assert_eq!(Prefs::load(&file), Prefs::default());
    }

    #[test]
    fn save_error_is_returned() {
        let dir = tempdir();
        // The parent "directory" is a file, so it cannot be created.
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "x").unwrap();
        let err = Prefs::default()
            .save(&blocker.join(PREFS_FILE))
            .unwrap_err();
        assert!(matches!(err, ProjectError::Io { .. }));
        // The target itself is a directory, so the rename fails.
        let file = dir.path().join("target");
        fs::create_dir(&file).unwrap();
        assert!(Prefs::default().save(&file).is_err());
    }
}
