//! The app data dir and the one way its documents are loaded and saved.
//!
//! Every document is a JSON object `{ "schema": n, ...fields }` in the app data dir. A missing,
//! unreadable, corrupt or wrong-schema file gives the document's defaults and stays where it is;
//! the caller gets a problem sentence to log. Saves are pretty, end in `\n` and are atomic.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::ProjectError;
use crate::project::atomic_write;

/// The desktop app's identifier, which names its data dir.
pub(crate) const APP_IDENTIFIER: &str = "com.rudoi.strudeland";

/// The app data dir: `dirs::data_dir()/com.rudoi.strudeland`, the same dir as Tauri's
/// `app_data_dir()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppData {
    dir: PathBuf,
}

impl AppData {
    /// `dirs::data_dir()/com.rudoi.strudeland`. No I/O: nothing is created or canonicalized.
    pub fn locate() -> Result<AppData, ProjectError> {
        let base = dirs::data_dir().ok_or(ProjectError::NoDataDir)?;
        Ok(AppData::at(base.join(APP_IDENTIFIER)))
    }

    /// An explicit dir (tests, and any caller that already knows the dir).
    pub fn at(dir: impl Into<PathBuf>) -> AppData {
        AppData { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

/// A loaded document: the value, and what was wrong with the file when it gave the defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded<T> {
    pub value: T,
    pub problem: Option<String>,
}

impl<T> Loaded<T> {
    /// Logs the problem, if any, to stderr as `{what}: {problem}` and returns the value.
    pub fn log(self, what: &str) -> T {
        if let Some(problem) = self.problem {
            eprintln!("{what}: {problem}");
        }
        self.value
    }
}

impl<T: Default> Loaded<T> {
    fn fallback(problem: Option<String>) -> Loaded<T> {
        Loaded {
            value: T::default(),
            problem,
        }
    }
}

/// The on-disk shape of every document.
#[derive(Serialize, Deserialize)]
struct OnDisk<T> {
    schema: u32,
    #[serde(flatten)]
    value: T,
}

/// Loads `file` under the bad-file policy: a missing file gives the default with no problem; an
/// unreadable, unparsable or wrong-schema file gives the default with a problem. Never writes.
pub(crate) fn load<T: DeserializeOwned + Default>(file: &Path, schema: u32) -> Loaded<T> {
    let bytes = match fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Loaded::fallback(None),
        Err(e) => {
            return Loaded::fallback(Some(format!("cannot read {}: {e}", file.display())));
        }
    };
    match serde_json::from_slice::<OnDisk<T>>(&bytes) {
        Err(e) => Loaded::fallback(Some(format!("ignoring {}: {e}", file.display()))),
        Ok(on_disk) if on_disk.schema != schema => Loaded::fallback(Some(format!(
            "ignoring {}: schema must be {schema}",
            file.display()
        ))),
        Ok(on_disk) => Loaded {
            value: on_disk.value,
            problem: None,
        },
    }
}

/// Writes `value` to `file` as `{ "schema": schema, ... }`, pretty with a trailing `\n`,
/// atomically, creating the parent directory if needed.
pub(crate) fn save<T: Serialize>(file: &Path, schema: u32, value: &T) -> Result<(), ProjectError> {
    if let Some(parent) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| ProjectError::io(parent, e))?;
    }
    let on_disk = OnDisk { schema, value };
    let mut json =
        serde_json::to_string_pretty(&on_disk).expect("app data documents always serialise");
    json.push('\n');
    atomic_write(file, &json)
}

/// A fresh temporary dir for this crate's tests.
#[cfg(test)]
pub(crate) fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const SCHEMA: u32 = 1;

    #[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    struct Doc {
        flag: bool,
        name: Option<String>,
    }

    fn doc() -> Doc {
        Doc {
            flag: true,
            name: Some("n".into()),
        }
    }

    fn ignoring(file: &Path) -> String {
        format!("ignoring {}: ", file.display())
    }

    #[test]
    fn missing_gives_the_default_and_no_problem() {
        let tmp = tempdir();
        let loaded = load::<Doc>(&tmp.path().join("doc.json"), SCHEMA);
        assert_eq!(loaded, Loaded::fallback(None));
    }

    #[test]
    fn corrupt_gives_the_default_with_a_problem_and_stays_in_place() {
        let tmp = tempdir();
        let file = tmp.path().join("doc.json");
        for text in ["", "{", "[]"] {
            fs::write(&file, text).unwrap();
            let loaded = load::<Doc>(&file, SCHEMA);
            assert_eq!(loaded.value, Doc::default(), "{text:?}");
            let problem = loaded.problem.expect(text);
            assert!(problem.starts_with(&ignoring(&file)), "{problem}");
            assert_eq!(fs::read(&file).unwrap(), text.as_bytes());
        }
    }

    #[test]
    fn invalid_utf8_is_a_parse_problem() {
        let tmp = tempdir();
        let file = tmp.path().join("doc.json");
        fs::write(&file, b"{\"schema\":1,\"name\":\"\xff\"}").unwrap();
        let loaded = load::<Doc>(&file, SCHEMA);
        assert_eq!(loaded.value, Doc::default());
        assert!(loaded.problem.unwrap().starts_with(&ignoring(&file)));
    }

    #[test]
    fn unreadable_gives_the_default_with_a_problem() {
        let tmp = tempdir();
        let file = tmp.path().join("doc.json");
        fs::create_dir(&file).unwrap();
        let loaded = load::<Doc>(&file, SCHEMA);
        assert_eq!(loaded.value, Doc::default());
        let problem = loaded.problem.unwrap();
        assert!(
            problem.starts_with(&format!("cannot read {}: ", file.display())),
            "{problem}"
        );
    }

    #[test]
    fn wrong_schema_gives_the_default() {
        let tmp = tempdir();
        let file = tmp.path().join("doc.json");
        for schema in ["2", "0", "\"1\""] {
            fs::write(
                &file,
                format!(r#"{{"schema":{schema},"flag":true,"name":"n"}}"#),
            )
            .unwrap();
            let loaded = load::<Doc>(&file, SCHEMA);
            assert_eq!(loaded.value, Doc::default(), "{schema}");
            let problem = loaded.problem.unwrap();
            assert!(problem.starts_with(&ignoring(&file)), "{problem}");
            if schema == "2" {
                assert_eq!(
                    problem,
                    format!("ignoring {}: schema must be 1", file.display())
                );
            }
        }
        fs::write(&file, r#"{"flag":true,"name":"n"}"#).unwrap();
        let loaded = load::<Doc>(&file, SCHEMA);
        assert_eq!(loaded.value, Doc::default());
        assert!(loaded.problem.unwrap().starts_with(&ignoring(&file)));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let tmp = tempdir();
        let file = tmp.path().join("doc.json");
        fs::write(
            &file,
            r#"{"schema":1,"flag":true,"name":"n","future":[1,2]}"#,
        )
        .unwrap();
        assert_eq!(
            load::<Doc>(&file, SCHEMA),
            Loaded {
                value: doc(),
                problem: None
            }
        );
    }

    #[test]
    fn save_creates_the_parent() {
        let tmp = tempdir();
        let file = tmp.path().join("a/b/doc.json");
        save(&file, SCHEMA, &doc()).unwrap();
        assert_eq!(load::<Doc>(&file, SCHEMA).value, doc());
    }

    #[test]
    fn save_is_atomic_and_overwrites() {
        let tmp = tempdir();
        let file = tmp.path().join("doc.json");
        save(&file, SCHEMA, &Doc::default()).unwrap();
        save(&file, SCHEMA, &doc()).unwrap();
        let names: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["doc.json"]);
        assert_eq!(load::<Doc>(&file, SCHEMA).value, doc());
    }

    #[test]
    fn save_errors_are_returned() {
        let tmp = tempdir();
        // The parent "directory" is a file, so it cannot be created.
        let blocker = tmp.path().join("blocker");
        fs::write(&blocker, "x").unwrap();
        let err = save(&blocker.join("doc.json"), SCHEMA, &doc()).unwrap_err();
        assert!(matches!(err, ProjectError::Io { .. }));
        // The target itself is a directory, so the rename fails.
        let target = tmp.path().join("target");
        fs::create_dir(&target).unwrap();
        assert!(save(&target, SCHEMA, &doc()).is_err());
    }

    #[test]
    fn save_writes_exact_bytes() {
        let tmp = tempdir();
        let file = tmp.path().join("doc.json");
        save(&file, SCHEMA, &doc()).unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\n  \"schema\": 1,\n  \"flag\": true,\n  \"name\": \"n\"\n}\n"
        );
        save(&file, SCHEMA, &Doc::default()).unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\n  \"schema\": 1,\n  \"flag\": false,\n  \"name\": null\n}\n"
        );
    }

    #[test]
    fn log_returns_the_value_with_or_without_a_problem() {
        let clean = Loaded {
            value: 1,
            problem: None,
        };
        assert_eq!(clean.log("test"), 1);
        let bad = Loaded {
            value: 2,
            problem: Some("ignoring x: bad".to_string()),
        };
        assert_eq!(bad.log("test"), 2);
    }

    #[test]
    fn at_and_file_join_the_dir() {
        let app = AppData::at("/x/data");
        assert_eq!(app.dir(), Path::new("/x/data"));
        assert_eq!(app.file("doc.json"), Path::new("/x/data/doc.json"));
    }

    #[test]
    fn locate_is_the_data_dir_and_the_identifier() {
        assert_eq!(
            AppData::locate().unwrap().dir(),
            dirs::data_dir().unwrap().join(APP_IDENTIFIER)
        );
    }

    #[test]
    fn default_directory_matches_desktop_identifier() {
        let config: Value =
            serde_json::from_str(include_str!("../../../src-tauri/tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], APP_IDENTIFIER);
        assert!(config["app"]["appDirectoriesOverride"].is_null());
    }
}
