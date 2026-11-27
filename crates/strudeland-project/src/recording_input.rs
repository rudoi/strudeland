//! The saved input choice, one document of the app data dir (`recording.json`, `"schema": 1`),
//! read and saved by `strudeland-record`'s `Takes`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::app_data::{self, AppData, Loaded};
use crate::error::ProjectError;

/// The recording input file name inside the app data dir.
const RECORDING_INPUT_FILE: &str = "recording.json";

const RECORDING_INPUT_SCHEMA: u32 = 1;

/// The saved input choice. Channel 0-based. `channel` is required; `deviceId` may be absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingInput {
    pub device_id: Option<String>,
    pub channel: u16,
}

impl RecordingInput {
    /// The file's path in the app data dir.
    pub fn file(app: &AppData) -> PathBuf {
        app.file(RECORDING_INPUT_FILE)
    }

    /// Reads the saved input. A missing, unreadable, corrupt or wrong-schema file gives the
    /// defaults and stays in place, with a problem for every case but a missing file.
    pub fn load(app: &AppData) -> Loaded<RecordingInput> {
        app_data::load(&RecordingInput::file(app), RECORDING_INPUT_SCHEMA)
    }

    /// Writes the saved input atomically, creating the app data dir if needed.
    pub fn save(&self, app: &AppData) -> Result<(), ProjectError> {
        app_data::save(&RecordingInput::file(app), RECORDING_INPUT_SCHEMA, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_data::tempdir;
    use std::fs;

    const DEVICE_TEXT: &str =
        "{\n  \"schema\": 1,\n  \"deviceId\": \"coreaudio:abc\",\n  \"channel\": 3\n}\n";
    const NULL_TEXT: &str = "{\n  \"schema\": 1,\n  \"deviceId\": null,\n  \"channel\": 0\n}\n";

    #[test]
    fn today_text_loads_and_resaves_byte_identically() {
        let dir = tempdir();
        let app = AppData::at(dir.path().join("data"));
        let file = RecordingInput::file(&app);
        assert_eq!(file, dir.path().join("data").join("recording.json"));
        let device = RecordingInput {
            device_id: Some("coreaudio:abc".into()),
            channel: 3,
        };
        fs::create_dir_all(app.dir()).unwrap();
        for (input, text) in [
            (device, DEVICE_TEXT),
            (RecordingInput::default(), NULL_TEXT),
        ] {
            fs::write(&file, text).unwrap();
            let loaded = RecordingInput::load(&app);
            assert_eq!(loaded.problem, None, "{text}");
            assert_eq!(loaded.value, input, "{text}");
            fs::remove_file(&file).unwrap();
            loaded.value.save(&app).unwrap();
            assert_eq!(fs::read_to_string(&file).unwrap(), text);
        }
        // A save creates the app data dir.
        fs::remove_dir_all(app.dir()).unwrap();
        RecordingInput::default().save(&app).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), NULL_TEXT);
    }

    #[test]
    fn channel_is_required_and_device_is_optional() {
        let dir = tempdir();
        let app = AppData::at(dir.path());
        let file = RecordingInput::file(&app);
        fs::write(&file, r#"{"schema":1,"channel":2}"#).unwrap();
        assert_eq!(
            RecordingInput::load(&app),
            Loaded {
                value: RecordingInput {
                    device_id: None,
                    channel: 2
                },
                problem: None
            }
        );
        fs::write(&file, r#"{"schema":1,"deviceId":"x"}"#).unwrap();
        let loaded = RecordingInput::load(&app);
        assert_eq!(loaded.value, RecordingInput::default());
        assert!(loaded.problem.is_some());
    }

    #[test]
    fn bad_files_give_defaults_stay_in_place_and_are_reported() {
        let dir = tempdir();
        let app = AppData::at(dir.path());
        let file = RecordingInput::file(&app);
        // A missing file gives the defaults and no problem.
        assert_eq!(
            RecordingInput::load(&app),
            Loaded {
                value: RecordingInput::default(),
                problem: None
            }
        );
        let ignoring = format!("ignoring {}: ", file.display());
        let cases: [(&str, &str); 6] = [
            ("{", "EOF while parsing an object"),
            (r#"{"schema":2,"deviceId":null,"channel":0}"#, ""),
            (r#"{"deviceId":null,"channel":0}"#, "missing field `schema`"),
            (
                r#"{"schema":1,"deviceId":3,"channel":0}"#,
                "invalid type: integer `3`, expected a string",
            ),
            (
                r#"{"schema":1,"deviceId":null,"channel":-1}"#,
                "invalid value: integer `-1`, expected u16",
            ),
            (
                r#"{"schema":1,"deviceId":null,"channel":"0"}"#,
                "invalid type: string \"0\", expected u16",
            ),
        ];
        for (text, head) in cases {
            fs::write(&file, text).unwrap();
            let loaded = RecordingInput::load(&app);
            assert_eq!(loaded.value, RecordingInput::default(), "{text}");
            let problem = loaded.problem.expect(text);
            if head.is_empty() {
                assert_eq!(problem, format!("{ignoring}schema must be 1"));
            } else {
                assert!(problem.starts_with(&ignoring), "{problem}");
                assert!(problem.contains(head), "{problem}");
            }
            // The bad file is left in place.
            assert_eq!(fs::read_to_string(&file).unwrap(), text);
        }
        // A directory where the file should be.
        fs::remove_file(&file).unwrap();
        fs::create_dir(&file).unwrap();
        let loaded = RecordingInput::load(&app);
        assert_eq!(loaded.value, RecordingInput::default());
        let problem = loaded.problem.unwrap();
        assert!(
            problem.starts_with(&format!("cannot read {}: ", file.display())),
            "{problem}"
        );
    }
}
