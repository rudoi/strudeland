//! Format-1 instrument definitions and portable project pins.
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::project::{ScopedDirectory, check_dir, device_basename, redirects};
use crate::{Project, ProjectError};

pub const INSTRUMENTS_DIR: &str = "instruments";
pub const MAX_INSTRUMENT_ENTRIES: usize = 128;
const APP_IDENTIFIER: &str = "com.rudoi.strudeland";
const LOCK_FILE: &str = ".strudeland.lock";
const MAX_FILE: usize = 64 * 1024;
const MAX_SNAPSHOT: usize = 2 * 1024 * 1024;
const MAX_STRING: usize = 1024;
const CONTROLS: LazyLock<Vec<&str>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../instrument-controls.json"))
        .expect("instrument controls JSON")
});

pub type InstrumentSnapshot = BTreeMap<String, InstrumentDefinition>;

/// Validated at deserialization. Field order and sorted parameter keys determine its fingerprint.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct InstrumentDefinition(Definition);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    schema: u32,
    parameters: BTreeMap<String, Value>,
    voices: Vec<Voice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Voice {
    name: String,
    chain: Vec<Control>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    control: String,
    value: Value,
}

fn identifier(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

fn scalar(value: &Value) -> bool {
    match value {
        Value::Number(n) => n.as_f64().is_some_and(f64::is_finite),
        Value::String(s) => s.len() <= MAX_STRING,
        Value::Bool(_) => true,
        _ => false,
    }
}

impl Definition {
    fn validate(&self) -> Result<(), String> {
        if self.schema != 1 {
            return Err(format!(
                "unsupported instrument schema {} (expected 1)",
                self.schema
            ));
        }
        if self.parameters.len() > 64 {
            return Err("at most 64 parameters are allowed".into());
        }
        for (key, value) in &self.parameters {
            if !identifier(key) || !scalar(value) {
                return Err(format!("invalid parameter {key:?} or its default"));
            }
        }
        if !(1..=32).contains(&self.voices.len()) {
            return Err("expected 1 to 32 voices".into());
        }
        let mut names = BTreeSet::new();
        for voice in &self.voices {
            if !identifier(&voice.name) || !names.insert(&voice.name) {
                return Err(format!("invalid or duplicate voice {:?}", voice.name));
            }
            if !(1..=64).contains(&voice.chain.len()) {
                return Err(format!("voice {:?}: expected 1 to 64 controls", voice.name));
            }
            for entry in &voice.chain {
                if !CONTROLS.contains(&entry.control.as_str()) {
                    return Err(format!(
                        "voice {:?}: unknown control {:?}",
                        voice.name, entry.control
                    ));
                }
                if scalar(&entry.value) {
                    continue;
                }
                if let Some(object) = entry.value.as_object()
                    && object.len() == 1
                    && let Some(key) = object.get("param").and_then(Value::as_str)
                {
                    if self.parameters.contains_key(key) {
                        continue;
                    }
                    return Err(format!(
                        "voice {:?}, control {:?}: unknown parameter {key:?}",
                        voice.name, entry.control
                    ));
                }
                return Err(format!(
                    "voice {:?}, control {:?}: expected scalar or parameter reference",
                    voice.name, entry.control
                ));
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for InstrumentDefinition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let definition = Definition::deserialize(deserializer)?;
        definition.validate().map_err(serde::de::Error::custom)?;
        Ok(Self(definition))
    }
}

fn invalid(id: &str, reason: impl Into<String>) -> ProjectError {
    ProjectError::Instrument {
        id: id.into(),
        reason: reason.into(),
    }
}

fn check_id(id: &str) -> Result<(), ProjectError> {
    if !identifier(id) || device_basename(id) {
        return Err(invalid(
            id,
            "expected a portable lowercase instrument identifier",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstrumentEntry {
    pub id: String,
    pub fingerprint: String,
}

impl InstrumentEntry {
    fn from_bytes(id: String, bytes: &[u8]) -> Self {
        Self {
            id,
            fingerprint: fingerprint(bytes),
        }
    }
}

fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl InstrumentDefinition {
    fn bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("validated JSON instrument definition")
    }

    fn snapshot_size(&self) -> usize {
        // JavaScript and serde_json can spell the same number differently (e.g. 1e20).
        // Count each number as one byte on both sides of the evaluation boundary.
        fn normalize_numbers(value: &mut Value) {
            match value {
                Value::Number(_) => *value = Value::from(0),
                Value::Array(items) => items.iter_mut().for_each(normalize_numbers),
                Value::Object(items) => items.values_mut().for_each(normalize_numbers),
                _ => {}
            }
        }
        let mut value = serde_json::to_value(self).expect("validated JSON instrument definition");
        normalize_numbers(&mut value);
        serde_json::to_vec(&value).expect("JSON value").len()
    }
}

/// Also check snapshots supplied through a JSON host request.
pub fn validate_instrument_snapshot(snapshot: &InstrumentSnapshot) -> Result<(), ProjectError> {
    if snapshot.len() > MAX_INSTRUMENT_ENTRIES {
        return Err(invalid(
            INSTRUMENTS_DIR,
            "at most 128 instruments are allowed",
        ));
    }
    let mut total = 0;
    for (id, definition) in snapshot {
        check_id(id)?;
        let size = definition.snapshot_size();
        total += size;
        if size > MAX_FILE || total > MAX_SNAPSHOT {
            return Err(invalid(id, "definition or snapshot exceeds size limit"));
        }
    }
    Ok(())
}

/// Includes invalid JSON filenames so the watcher can report an error (and its repair).
pub fn instrument_file_candidate(name: &OsStr) -> bool {
    let path = Path::new(name);
    !name.as_encoded_bytes().starts_with(b".")
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
}

fn open_store(path: &Path) -> Result<Option<ScopedDirectory>, ProjectError> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ProjectError::io(path, e)),
        Ok(_) => (),
    }
    check_dir(path, INSTRUMENTS_DIR, "").map_err(|e| match e {
        ProjectError::PathOutsideProject(_) => invalid(
            &path.display().to_string(),
            "instrument directory redirects outside its store",
        ),
        other => other,
    })?;
    checked_directory(path).map(Some)
}

fn checked_directory(path: &Path) -> Result<ScopedDirectory, ProjectError> {
    let dir = ScopedDirectory::open(path, "the instrument directory changed")?;
    dir.check_current()?;
    if dir.canonical_path() != path {
        return Err(invalid(
            &path.display().to_string(),
            "directory changed during open",
        ));
    }
    Ok(dir)
}

pub fn instrument_dir_entries(root: &Path) -> Result<Option<fs::ReadDir>, ProjectError> {
    let path = root.join(INSTRUMENTS_DIR);
    let Some(dir) = open_store(&path)? else {
        return Ok(None);
    };
    fs::read_dir(dir.path())
        .map(Some)
        .map_err(|e| ProjectError::io(path, e))
}

fn catalog(dir: &ScopedDirectory) -> Result<BTreeMap<String, u64>, ProjectError> {
    dir.check_current()?;
    let mut catalog = BTreeMap::new();
    for entry in fs::read_dir(dir.path()).map_err(|e| ProjectError::io(dir.path(), e))? {
        let entry = entry.map_err(|e| ProjectError::io(dir.path(), e))?;
        let name = entry.file_name();
        if !instrument_file_candidate(&name) {
            continue;
        }
        let path = entry.path();
        let id = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .ok_or_else(|| invalid(&path.display().to_string(), "expected <id>.json"))?;
        check_id(id)?;
        let meta = fs::symlink_metadata(&path).map_err(|e| ProjectError::io(&path, e))?;
        if redirects(&meta) || !meta.is_file() {
            return Err(invalid(id, "expected a regular instrument file"));
        }
        catalog.insert(id.to_owned(), meta.len());
        if catalog.len() > MAX_INSTRUMENT_ENTRIES {
            return Err(invalid(id, "at most 128 instruments are allowed"));
        }
    }
    dir.check_current()?;
    Ok(catalog)
}

// Resolve files through the held directory (Unix) or the non-delete-shared directory
// handle (Windows). Never follow a member symlink or open a special file for reading.
fn open_member(dir: &ScopedDirectory, name: &str, lock: bool) -> Result<File, ProjectError> {
    let path = dir.path().join(name);
    dir.check_current()?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let file = {
        use rustix::fs::{Mode, OFlags, openat};
        let access = if lock {
            OFlags::RDWR | OFlags::CREATE
        } else {
            OFlags::RDONLY
        };
        openat(
            dir.file(),
            name,
            access | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::from_bits_truncate(0o666),
        )
        .map(File::from)
        .map_err(io::Error::from)
    };
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        fs::OpenOptions::new()
            .read(true)
            .write(lock)
            .create(lock)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let file: io::Result<File> = Err(io::Error::from(io::ErrorKind::Unsupported));
    let file = file.map_err(|e| ProjectError::io(&path, e))?;
    let meta = file.metadata().map_err(|e| ProjectError::io(&path, e))?;
    if !meta.is_file() || redirects(&meta) {
        return Err(invalid(
            &path.display().to_string(),
            "expected a regular file",
        ));
    }
    Ok(file)
}

fn read_definition(dir: &ScopedDirectory, id: &str) -> Result<InstrumentDefinition, ProjectError> {
    let name = format!("{id}.json");
    let file = open_member(dir, &name, false)?;
    let mut bytes = Vec::new();
    file.take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| ProjectError::io(dir.path().join(&name), e))?;
    if bytes.len() > MAX_FILE {
        return Err(invalid(id, "definition exceeds 64 KiB"));
    }
    serde_json::from_slice(&bytes).map_err(|e| invalid(id, e.to_string()))
}

fn snapshot(dir: &ScopedDirectory) -> Result<InstrumentSnapshot, ProjectError> {
    let names = catalog(dir)?;
    check_sizes(&names)?;
    let mut defs = BTreeMap::new();
    for id in names.into_keys() {
        let definition = read_definition(dir, &id)?;
        defs.insert(id, definition);
    }
    validate_instrument_snapshot(&defs)?;
    dir.check_current()?;
    Ok(defs)
}

fn existing_pin_lock(dir: &ScopedDirectory) -> Result<Option<File>, ProjectError> {
    match open_member(dir, LOCK_FILE, false) {
        Ok(lock) => Ok(Some(lock)),
        Err(ProjectError::Io { path, source })
            if path == dir.path().join(LOCK_FILE) && source.kind() == io::ErrorKind::NotFound =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn read_pins(
    dir: &ScopedDirectory,
    mut read: impl FnMut() -> Result<InstrumentSnapshot, ProjectError>,
) -> Result<InstrumentSnapshot, ProjectError> {
    let lock = match existing_pin_lock(dir)? {
        Some(lock) => lock,
        None => {
            // A read-only checkout may never have had a writer. If the first writer
            // created its lock during our scan, discard the scan and read under the lock.
            let first = read();
            let Some(lock) = existing_pin_lock(dir)? else {
                return first;
            };
            lock
        }
    };
    lock.lock_shared()
        .map_err(|e| ProjectError::io(dir.path().join(LOCK_FILE), e))?;
    read()
}

fn check_sizes(names: &BTreeMap<String, u64>) -> Result<(), ProjectError> {
    let mut total = 0;
    for (id, &size) in names {
        if size > MAX_FILE as u64 {
            return Err(invalid(id, "definition exceeds 64 KiB"));
        }
        total += size;
        if total > MAX_SNAPSHOT as u64 {
            return Err(invalid(id, "instrument files exceed 2 MiB in total"));
        }
    }
    Ok(())
}

fn entries(snapshot: InstrumentSnapshot) -> Vec<InstrumentEntry> {
    snapshot
        .into_iter()
        .map(|(id, definition)| InstrumentEntry::from_bytes(id, &definition.bytes()))
        .collect()
}

pub struct InstrumentLibrary {
    path: PathBuf,
}

impl InstrumentLibrary {
    /// The default global directory is shared with Tauri's app data path.
    pub fn open_default() -> Result<Self, ProjectError> {
        let base = dirs::data_dir()
            .ok_or_else(|| invalid(INSTRUMENTS_DIR, "cannot locate user data directory"))?;
        fs::create_dir_all(&base).map_err(|e| ProjectError::io(&base, e))?;
        let base = base
            .canonicalize()
            .map_err(|e| ProjectError::io(&base, e))?;
        let base = checked_directory(&base)?;
        let app = checked_directory(&create_directory(&base, APP_IDENTIFIER)?)?;
        let path = create_directory(&app, INSTRUMENTS_DIR)?;
        open_store(&path)?;
        Ok(Self { path })
    }

    /// Use an explicitly selected library directory.
    pub fn at(path: PathBuf) -> Self {
        let normalized = path.parent().and_then(|parent| {
            let parent = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            };
            Some(parent.canonicalize().ok()?.join(path.file_name()?))
        });
        Self {
            path: normalized.unwrap_or(path),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> Result<Vec<InstrumentEntry>, ProjectError> {
        let Some(dir) = open_store(&self.path)? else {
            return Ok(Vec::new());
        };
        Ok(entries(snapshot(&dir)?))
    }

    fn selected(&self, id: &str) -> Result<InstrumentDefinition, ProjectError> {
        let missing = || invalid(id, "not found in the instrument library");
        let dir = open_store(&self.path)?.ok_or_else(missing)?;
        if !catalog(&dir)?.contains_key(id) {
            return Err(missing());
        }
        read_definition(&dir, id)
    }
}

fn create_directory(parent: &ScopedDirectory, name: &str) -> Result<PathBuf, ProjectError> {
    parent.check_current()?;
    let path = parent.path().join(name);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let created = rustix::fs::mkdirat(
        parent.file(),
        name,
        rustix::fs::Mode::from_bits_truncate(0o777),
    )
    .map_err(io::Error::from);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let created = fs::create_dir(&path);
    match created {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(ProjectError::io(path, e)),
    }
    parent.check_current()?;
    Ok(path)
}

impl Project {
    pub fn instrument_snapshot(&self) -> Result<InstrumentSnapshot, ProjectError> {
        let Some(dir) = open_store(&self.root().join(INSTRUMENTS_DIR))? else {
            return Ok(InstrumentSnapshot::new());
        };
        read_pins(&dir, || snapshot(&dir))
    }

    pub fn instrument_entries(&self) -> Result<Vec<InstrumentEntry>, ProjectError> {
        Ok(entries(self.instrument_snapshot()?))
    }

    pub fn pin_instrument(
        &self,
        library: &InstrumentLibrary,
        id: &str,
    ) -> Result<InstrumentEntry, ProjectError> {
        self.publish_instrument(library, id, None)
    }

    pub fn update_instrument(
        &self,
        library: &InstrumentLibrary,
        id: &str,
        expected: &str,
    ) -> Result<InstrumentEntry, ProjectError> {
        self.publish_instrument(library, id, Some(expected))
    }

    fn publish_instrument(
        &self,
        library: &InstrumentLibrary,
        id: &str,
        expected: Option<&str>,
    ) -> Result<InstrumentEntry, ProjectError> {
        check_id(id)?;
        let def = library.selected(id)?;
        let parent = checked_directory(self.root())?;
        let path = create_directory(&parent, INSTRUMENTS_DIR)?;
        let dir =
            open_store(&path)?.ok_or_else(|| invalid(id, "instrument directory disappeared"))?;
        let lock = open_member(&dir, LOCK_FILE, true)?;
        lock.lock()
            .map_err(|e| ProjectError::io(path.join(LOCK_FILE), e))?;
        let mut names = catalog(&dir)?;
        match (expected, names.contains_key(id)) {
            (None, true) => return Err(invalid(id, "already pinned")),
            (Some(_), false) => return Err(invalid(id, "not pinned")),
            (Some(expected), true) => {
                let current = fingerprint(&read_definition(&dir, id)?.bytes());
                if expected != current {
                    return Err(invalid(
                        id,
                        format!(
                            "expected fingerprint {expected}, current fingerprint is {current}"
                        ),
                    ));
                }
            }
            (None, false) if names.len() == MAX_INSTRUMENT_ENTRIES => {
                return Err(invalid(id, "at most 128 instruments are allowed"));
            }
            _ => (),
        }
        let bytes = def.bytes();
        names.insert(id.into(), bytes.len() as u64);
        check_sizes(&names)?;
        dir.publish(&format!("{id}.json"), &bytes, expected.is_some())?;
        Ok(InstrumentEntry::from_bytes(id.into(), &bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::{TempDir, tempdir};

    fn definition(cutoff: i64) -> Value {
        json!({
            "schema": 1, "parameters": {"cutoff": cutoff}, "voices": [
                {"name": "lead", "chain": [
                    {"control":"s","value":"sawtooth"},
                    {"control":"lpf","value":{"param":"cutoff"}}
                ]}
            ]
        })
    }

    fn setup() -> (TempDir, Project, InstrumentLibrary) {
        let tmp = tempdir().unwrap();
        let project = Project::create(&tmp.path().join("project"), "Example").unwrap();
        let library = InstrumentLibrary::at(tmp.path().join("library"));
        (tmp, project, library)
    }

    fn write_def(library: &InstrumentLibrary, id: &str, value: Value) {
        fs::create_dir_all(library.path()).unwrap();
        fs::write(library.path().join(format!("{id}.json")), value.to_string()).unwrap();
    }

    #[test]
    fn pins_keep_independent_definitions_and_only_update_explicitly() {
        let (tmp, project, library) = setup();
        assert!(library.entries().unwrap().is_empty());
        write_def(&library, "lead", definition(1200));
        let first = project.pin_instrument(&library, "lead").unwrap();
        let first_snapshot = project.instrument_snapshot().unwrap();
        assert_eq!(library.entries().unwrap(), [first.clone()]);
        assert_eq!(first.fingerprint.len(), 64);
        assert!(
            project
                .pin_instrument(&library, "lead")
                .unwrap_err()
                .to_string()
                .contains("already pinned")
        );
        write_def(&library, "lead", definition(900));
        let other = Project::create(&tmp.path().join("other"), "Other").unwrap();
        let second = other.pin_instrument(&library, "lead").unwrap();
        assert_ne!(first, second);
        assert_eq!(first_snapshot, project.instrument_snapshot().unwrap());
        assert!(
            project
                .update_instrument(&library, "lead", &second.fingerprint)
                .is_err()
        );
        assert_eq!(project.instrument_entries().unwrap(), [first.clone()]);
        assert_eq!(
            project
                .update_instrument(&library, "lead", &first.fingerprint)
                .unwrap(),
            second
        );
        fs::remove_dir_all(library.path()).unwrap();
        let reopened = Project::open(project.root()).unwrap();
        assert_eq!(
            reopened.instrument_snapshot().unwrap(),
            other.instrument_snapshot().unwrap()
        );
        assert_eq!(reopened.instrument_entries().unwrap(), [second]);
    }

    #[test]
    fn stable_fingerprints_ignore_whitespace_and_parameter_order() {
        let (_tmp, project, library) = setup();
        write_def(&library, "lead", definition(2));
        let p = library.path().join("lead.json");
        let mut d = definition(2);
        d["parameters"] = json!({"beta":1, "alpha":2});
        d["voices"][0]["chain"][1]["value"]["param"] = json!("beta");
        fs::write(&p, serde_json::to_string_pretty(&d).unwrap()).unwrap();
        let first = project.pin_instrument(&library, "lead").unwrap();
        let text = d
            .to_string()
            .replace(r#""alpha":2,"beta":1"#, r#""beta":1,"alpha":2"#);
        fs::write(&p, text).unwrap();
        assert_eq!(library.entries().unwrap(), [first]);
    }

    #[test]
    fn fractional_values_keep_the_same_fingerprint_after_publication() {
        let (_tmp, project, library) = setup();
        fs::create_dir(library.path()).unwrap();
        let source = definition(2)
            .to_string()
            .replace(r#""cutoff":2"#, r#""cutoff":995097304719337844e-19"#);
        fs::write(library.path().join("lead.json"), source).unwrap();
        let pin = project.pin_instrument(&library, "lead").unwrap();
        assert_eq!(project.instrument_entries().unwrap(), [pin]);
    }

    #[test]
    fn pinning_rejects_a_redirected_project_root_before_creating_directories() {
        let (tmp, project, library) = setup();
        write_def(&library, "lead", definition(1));
        let outside = tmp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::rename(project.root(), tmp.path().join("displaced")).unwrap();
        crate::test_symlink!(&outside, project.root()).unwrap();
        assert!(project.pin_instrument(&library, "lead").is_err());
        assert!(!outside.join(INSTRUMENTS_DIR).exists());
    }

    #[test]
    fn malformed_global_record_leaves_the_existing_pin_intact() {
        let (_tmp, project, library) = setup();
        write_def(&library, "lead", definition(1));
        let first = project.pin_instrument(&library, "lead").unwrap();
        let path = project.root().join("instruments/lead.json");
        let bytes = fs::read(&path).unwrap();
        for (pointer, value, diagnostic) in [
            ("/schema", json!(2), "schema"),
            ("/parameters/cutoff", json!({}), "cutoff"),
            (
                "/parameters/cutoff",
                json!("a".repeat(MAX_STRING + 1)),
                "cutoff",
            ),
            ("/voices/0/chain/0/control", json!("typo"), "typo"),
            ("/voices/0/chain/1/value/param", json!("missing"), "missing"),
            (
                "/voices/0/chain/1/value",
                json!({"param":"cutoff","other":1}),
                "lpf",
            ),
            ("/voices/0/chain", json!([]), "lead"),
            ("/voices", json!([]), "voices"),
        ] {
            let mut d = definition(1);
            *d.pointer_mut(pointer).unwrap() = value;
            write_def(&library, "lead", d);
            let err = project
                .update_instrument(&library, "lead", &first.fingerprint)
                .unwrap_err()
                .to_string();
            assert!(err.contains("lead") && err.contains(diagnostic), "{err}");
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        let mut kit = definition(1);
        let voice = kit["voices"][0].clone();
        kit["voices"].as_array_mut().unwrap().push(voice);
        assert!(serde_json::from_value::<InstrumentDefinition>(kit.clone()).is_err());
        kit["voices"][1]["name"] = json!("bass");
        assert!(serde_json::from_value::<InstrumentDefinition>(kit).is_ok());
    }

    #[test]
    fn legacy_folder_and_listing_rules() {
        let (_tmp, project, library) = setup();
        let dir = project.root().join(INSTRUMENTS_DIR);
        fs::remove_dir_all(&dir).unwrap();
        assert!(project.instrument_snapshot().unwrap().is_empty());
        assert!(instrument_dir_entries(project.root()).unwrap().is_none());
        write_def(&library, "z", definition(1));
        write_def(&library, "a", definition(1));
        project.pin_instrument(&library, "z").unwrap();
        project.pin_instrument(&library, "a").unwrap();
        fs::write(dir.join(".copy.json"), "").unwrap();
        fs::write(dir.join("notes.txt"), "").unwrap();
        assert_eq!(
            project
                .instrument_snapshot()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["a", "z"]
        );
        for id in ["Lead", "a/b", "com1", "nul", "_", "é", "", "../outside"] {
            assert!(project.pin_instrument(&library, id).is_err(), "{id:?}");
        }
        // Reject this spelling on case-sensitive disks too, before it can reach another OS.
        fs::write(dir.join("BASS.JSON"), definition(1).to_string()).unwrap();
        assert!(
            project
                .instrument_entries()
                .unwrap_err()
                .to_string()
                .contains("BASS.JSON")
        );
    }

    #[test]
    fn a_snapshot_retries_under_a_shared_lock_when_the_first_writer_arrives() {
        let (_tmp, project, library) = setup();
        let path = project.root().join(INSTRUMENTS_DIR);
        for id in ["a", "b"] {
            fs::write(path.join(format!("{id}.json")), definition(100).to_string()).unwrap();
            write_def(&library, id, definition(200));
        }
        let pins = project.instrument_entries().unwrap();
        assert!(!path.join(LOCK_FILE).exists());
        let dir = open_store(&path).unwrap().unwrap();
        let mut captures = 0;
        let captured = read_pins(&dir, || {
            captures += 1;
            if captures == 1 {
                let a = read_definition(&dir, "a")?;
                for pin in &pins {
                    project.update_instrument(&library, &pin.id, &pin.fingerprint)?;
                }
                let b = read_definition(&dir, "b")?;
                assert_ne!(a, b);
                return Ok(BTreeMap::from([("a".into(), a), ("b".into(), b)]));
            }
            let writer = open_member(&dir, LOCK_FILE, true)?;
            assert!(matches!(
                writer.try_lock(),
                Err(fs::TryLockError::WouldBlock)
            ));
            snapshot(&dir)
        })
        .unwrap();
        assert_eq!(captures, 2);
        assert_eq!(captured["a"], captured["b"]);
        assert_eq!(captured, project.instrument_snapshot().unwrap());
    }

    #[test]
    fn simultaneous_updates_from_one_revision_only_publish_once() {
        use std::sync::Barrier;
        let (_tmp, project, library) = setup();
        write_def(&library, "lead", definition(100));
        let first = project.pin_instrument(&library, "lead").unwrap();
        write_def(&library, "lead", definition(200));
        let ready = Barrier::new(2);
        let update = || {
            ready.wait();
            project.update_instrument(&library, "lead", &first.fingerprint)
        };
        let (a, b) = std::thread::scope(|s| {
            let a = s.spawn(update);
            (update(), a.join().unwrap())
        });
        let (ok, err) = if a.is_ok() { (a, b) } else { (b, a) };
        assert!(err.unwrap_err().to_string().contains("current fingerprint"));
        assert_eq!(project.instrument_entries().unwrap(), [ok.unwrap()]);
    }

    #[test]
    fn snapshot_quota_is_independent_of_number_formatting() {
        let record = |value: Value| {
            let chain = vec![json!({"control":"gain","value":value}); 64];
            let voices: Vec<_> = (0..32)
                .map(|i| json!({"name":format!("voice{i}"),"chain":chain}))
                .collect();
            let data = json!({"schema":1,"parameters":{},"voices":voices});
            serde_json::from_value::<InstrumentDefinition>(data).unwrap()
        };
        let large_numbers = record(json!(1e20));
        assert!(large_numbers.bytes().len() > MAX_FILE);
        assert_eq!(
            large_numbers.snapshot_size(),
            record(json!(0)).snapshot_size()
        );
        let mut snapshot = BTreeMap::from([("pad".into(), large_numbers)]);
        validate_instrument_snapshot(&snapshot).unwrap();
        snapshot.insert("pad".into(), record(json!("long literal")));
        assert!(validate_instrument_snapshot(&snapshot).is_err());
    }

    #[test]
    fn input_and_snapshot_limits_are_enforced() {
        let (_tmp, project, library) = setup();
        write_def(&library, "lead", definition(1));
        let dir = project.root().join(INSTRUMENTS_DIR);
        let padding = vec![b' '; MAX_FILE];
        fs::write(
            dir.join("too_large.json"),
            [padding.as_slice(), b" "].concat(),
        )
        .unwrap();
        assert!(
            project
                .instrument_snapshot()
                .unwrap_err()
                .to_string()
                .contains("64 KiB")
        );
        fs::remove_file(dir.join("too_large.json")).unwrap();
        for i in 0..=MAX_SNAPSHOT / MAX_FILE {
            fs::write(dir.join(format!("pad_{i}.json")), &padding).unwrap();
        }
        assert!(
            project
                .instrument_snapshot()
                .unwrap_err()
                .to_string()
                .contains("2 MiB")
        );
        for i in 0..MAX_INSTRUMENT_ENTRIES {
            fs::write(dir.join(format!("entry_{i}.json")), "{}").unwrap();
        }
        assert!(
            project
                .instrument_snapshot()
                .unwrap_err()
                .to_string()
                .contains("128")
        );
    }

    #[test]
    fn directory_and_member_links_are_refused() {
        let (tmp, project, library) = setup();
        write_def(&library, "lead", definition(1));
        let dir = project.root().join(INSTRUMENTS_DIR);
        let other = tmp.path().join("outside");
        fs::create_dir(&other).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        crate::test_symlink!(&other, &dir).unwrap();
        assert!(project.instrument_snapshot().is_err());
        assert!(project.pin_instrument(&library, "lead").is_err());
        assert_eq!(fs::read_dir(&other).unwrap().count(), 0);
        #[cfg(windows)]
        fs::remove_dir(&dir).unwrap();
        #[cfg(not(windows))]
        fs::remove_file(&dir).unwrap();
        fs::create_dir(&dir).unwrap();
        let file = library.path().join("lead.json");
        let bytes = fs::read(&file).unwrap();
        crate::test_symlink!(&file, dir.join("lead.json")).unwrap();
        assert!(project.instrument_entries().is_err());
        assert!(project.pin_instrument(&library, "lead").is_err());
        fs::remove_file(dir.join("lead.json")).unwrap();
        // A competing lock entry must never redirect a write outside the project.
        fs::remove_file(dir.join(LOCK_FILE)).unwrap();
        crate::test_symlink!(&file, dir.join(LOCK_FILE)).unwrap();
        assert!(project.pin_instrument(&library, "lead").is_err());
        assert_eq!(fs::read(&file).unwrap(), bytes);
    }

    #[test]
    fn default_directory_matches_desktop_identifier() {
        let config: Value =
            serde_json::from_str(include_str!("../../../src-tauri/tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], APP_IDENTIFIER);
        assert!(config["app"]["appDirectoriesOverride"].is_null());
    }
}
