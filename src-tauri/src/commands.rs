use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::command_error::CommandError;

/// The pin file at the repo root, embedded at compile time.
const PINS_JSON: &str = include_str!("../../strudel-pins.json");

/// strudel-pins.json; `schema` is checked on its own first, so an old file gets a clear error.
#[derive(Deserialize)]
struct PinsFile {
    strudel: StrudelPin,
    npm: BTreeMap<String, PinEntry>,
}

/// The vendored upstream Strudel commit.
#[derive(Deserialize)]
#[allow(dead_code)] // `repo` is part of the file format, not of the app info.
struct StrudelPin {
    repo: String,
    commit: String,
    date: String,
}

#[derive(Deserialize)]
struct PinEntry {
    version: String,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct AppInfo {
    /// This app's version (Cargo package version).
    pub version: String,
    pub strudel: StrudelInfo,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct StrudelInfo {
    /// The vendored upstream commit (40 hex) and its commit date (ISO 8601).
    pub commit: String,
    pub date: String,
    /// npm package name -> pinned version for the kabelsalat packages.
    pub kabelsalat: BTreeMap<String, String>,
}

/// Parse a strudel-pins.json document (schema 2).
fn parse_pins(json: &str) -> Result<StrudelInfo, String> {
    #[derive(Deserialize)]
    struct Schema {
        schema: u32,
    }
    let Schema { schema } = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if schema != 2 {
        return Err(format!("unsupported strudel-pins.json schema {schema}"));
    }
    let file: PinsFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
    Ok(StrudelInfo {
        commit: file.strudel.commit,
        date: file.strudel.date,
        kabelsalat: file
            .npm
            .into_iter()
            .filter(|(name, _)| name.starts_with("@kabelsalat/"))
            .map(|(name, entry)| (name, entry.version))
            .collect(),
    })
}

fn build_app_info() -> Result<AppInfo, String> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        strudel: parse_pins(PINS_JSON)?,
    })
}

#[tauri::command]
pub fn app_info() -> Result<AppInfo, CommandError> {
    build_app_info().map_err(CommandError::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA_2: &str = r#"{
      "schema": 2,
      "strudel": {
        "repo": "https://codeberg.org/uzu/strudel",
        "commit": "0cb6341299229f739b5090be93528bb4f170d855",
        "date": "2026-11-08T02:32:27+01:00"
      },
      "npm": {
        "@kabelsalat/web": { "version": "0.4.1", "file": "dist/index.mjs", "sha256": "x" },
        "@kabelsalat/lib": { "version": "0.4.2", "file": "src/ugens.js", "sha256": "y" }
      }
    }"#;

    #[test]
    fn parses_schema_2() {
        let info = parse_pins(SCHEMA_2).unwrap();
        assert_eq!(info.commit, "0cb6341299229f739b5090be93528bb4f170d855");
        assert_eq!(info.date, "2026-11-08T02:32:27+01:00");
        assert_eq!(
            info.kabelsalat,
            BTreeMap::from([
                ("@kabelsalat/lib".to_string(), "0.4.2".to_string()),
                ("@kabelsalat/web".to_string(), "0.4.1".to_string()),
            ])
        );
    }

    #[test]
    fn embedded_pins_give_a_commit_and_both_kabelsalat_versions() {
        let info = build_app_info().expect("embedded pins must parse");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        let commit = &info.strudel.commit;
        assert_eq!(commit.len(), 40);
        assert!(
            commit
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert!(info.strudel.kabelsalat.contains_key("@kabelsalat/web"));
        assert!(info.strudel.kabelsalat.contains_key("@kabelsalat/lib"));
    }

    #[test]
    fn serializes_to_the_wire_shape() {
        let info = AppInfo {
            version: "0.1.0".into(),
            strudel: parse_pins(SCHEMA_2).unwrap(),
        };
        assert_eq!(
            serde_json::to_string(&info).unwrap(),
            r#"{"version":"0.1.0","strudel":{"commit":"0cb6341299229f739b5090be93528bb4f170d855","date":"2026-11-08T02:32:27+01:00","kabelsalat":{"@kabelsalat/lib":"0.4.2","@kabelsalat/web":"0.4.1"}}}"#
        );
    }

    #[test]
    fn rejects_schema_1() {
        let err = parse_pins(r#"{"schema":1,"packages":{}}"#).unwrap_err();
        assert!(err.contains("schema 1"), "{err}");
    }

    #[test]
    fn rejects_a_missing_commit() {
        let json = r#"{"schema":2,"strudel":{"repo":"r","date":"d"},"npm":{}}"#;
        let err = parse_pins(json).unwrap_err();
        assert!(err.contains("commit"), "{err}");
    }

    #[test]
    fn rejects_a_missing_npm_block() {
        let json = r#"{"schema":2,"strudel":{"repo":"r","commit":"c","date":"d"}}"#;
        assert!(parse_pins(json).unwrap_err().contains("npm"));
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(parse_pins("not json").is_err());
    }
}
