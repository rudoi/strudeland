//! App preferences, one document of the app data dir (`prefs.json`, `"schema": 1`).
//!
//! Later preferences are added as fields with defaults, with no schema bump.

use serde::{Deserialize, Serialize};

use crate::app_data::{self, AppData, Loaded};
use crate::error::ProjectError;

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
    /// Show the piano roll dock when an editor is open.
    pub piano_roll_open: bool,
}

impl Prefs {
    /// Reads the prefs. A missing, unreadable, corrupt or wrong-schema file gives the defaults,
    /// with a problem for every case but a missing file.
    pub fn load(app: &AppData) -> Loaded<Prefs> {
        app_data::load(&app.file(PREFS_FILE), PREFS_SCHEMA)
    }

    /// Writes the prefs atomically, creating the app data dir if needed.
    pub fn save(&self, app: &AppData) -> Result<(), ProjectError> {
        app_data::save(&app.file(PREFS_FILE), PREFS_SCHEMA, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    const ALL_TRUE_TEXT: &str = "{\n  \"schema\": 1,\n  \"vim\": true,\n  \"auditionNotes\": true,\n  \"formatOnSave\": true,\n  \"pianoRollOpen\": true\n}\n";
    const DEFAULT_TEXT: &str = "{\n  \"schema\": 1,\n  \"vim\": false,\n  \"auditionNotes\": false,\n  \"formatOnSave\": false,\n  \"pianoRollOpen\": false\n}\n";

    #[test]
    fn saves_exactly_todays_bytes_and_loads_them_back() {
        let dir = tempdir();
        let app = AppData::at(dir.path());
        let file = dir.path().join(PREFS_FILE);
        let all_true = Prefs {
            vim: true,
            audition_notes: true,
            format_on_save: true,
            piano_roll_open: true,
        };
        for (prefs, text) in [(all_true, ALL_TRUE_TEXT), (Prefs::default(), DEFAULT_TEXT)] {
            prefs.save(&app).unwrap();
            assert_eq!(fs::read_to_string(&file).unwrap(), text);
            fs::remove_file(&file).unwrap();
            fs::write(&file, text).unwrap();
            assert_eq!(Prefs::load(&app).value, prefs);
        }
    }

    #[test]
    fn defaults_are_off() {
        assert!(!Prefs::default().vim);
        assert!(!Prefs::default().audition_notes);
        assert!(!Prefs::default().format_on_save);
        assert!(!Prefs::default().piano_roll_open);
    }

    #[test]
    fn missing_keys_default() {
        let dir = tempdir();
        let app = AppData::at(dir.path());
        let file = dir.path().join(PREFS_FILE);
        fs::write(&file, r#"{ "schema": 1 }"#).unwrap();
        assert_eq!(Prefs::load(&app).value, Prefs::default());

        fs::write(&file, r#"{ "schema": 1, "vim": true }"#).unwrap();
        let prefs = Prefs::load(&app).value;
        assert!(prefs.vim);
        assert!(!prefs.audition_notes);
        assert!(!prefs.format_on_save);

        fs::write(
            &file,
            r#"{ "schema": 1, "vim": true, "auditionNotes": true }"#,
        )
        .unwrap();
        let prefs = Prefs::load(&app).value;
        assert!(prefs.vim);
        assert!(prefs.audition_notes);
        assert!(!prefs.format_on_save);
    }
}
