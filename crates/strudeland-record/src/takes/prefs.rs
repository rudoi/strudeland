//! The saved input choice (`recording.json` in the app data dir; the app names the path).

use std::path::Path;

use serde::{Deserialize, Serialize};
use strudeland_project::atomic_write_bytes;

use crate::DeviceInfo;

const PREFS_SCHEMA: u32 = 1;

/// The saved input choice. Channel 0-based.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prefs {
    pub device_id: Option<String>,
    pub channel: u16,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrefsFile {
    schema: u32,
    device_id: Option<String>,
    channel: u16,
}

/// Parses `recording.json`; the error names what is wrong.
pub fn parse_prefs(bytes: &[u8]) -> Result<Prefs, String> {
    let file: PrefsFile = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if file.schema != PREFS_SCHEMA {
        return Err(format!("schema must be {PREFS_SCHEMA}"));
    }
    Ok(Prefs {
        device_id: file.device_id,
        channel: file.channel,
    })
}

/// The saved prefs, or the defaults when the file is missing or invalid (logged, left in place).
pub fn read_prefs(path: &Path) -> Prefs {
    match std::fs::read(path) {
        Ok(bytes) => parse_prefs(&bytes).unwrap_or_else(|e| {
            eprintln!("recording: ignoring {}: {e}", path.display());
            Prefs::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Prefs::default(),
        Err(e) => {
            eprintln!("recording: cannot read {}: {e}", path.display());
            Prefs::default()
        }
    }
}

pub fn write_prefs(path: &Path, prefs: &Prefs) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = PrefsFile {
        schema: PREFS_SCHEMA,
        device_id: prefs.device_id.clone(),
        channel: prefs.channel,
    };
    let mut json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    json.push('\n');
    atomic_write_bytes(path, json.as_bytes()).map_err(|e| e.to_string())
}

/// The saved device when present, else the default input (else the first); the saved channel
/// when the device has it, else 0. `None` without devices.
pub fn resolve_selection<'a>(
    devices: &'a [DeviceInfo],
    prefs: &Prefs,
) -> Option<(&'a DeviceInfo, u16)> {
    let saved = prefs
        .device_id
        .as_deref()
        .and_then(|id| devices.iter().find(|d| d.id == id));
    let device = saved
        .or_else(|| devices.iter().find(|d| d.is_default))
        .or_else(|| devices.first())?;
    Some((device, saved_channel(device, prefs)))
}

/// The saved channel when `device` has it (or reports no channels: an exclusive endpoint whose
/// config query was refused), else 0.
pub(super) fn saved_channel(device: &DeviceInfo, prefs: &Prefs) -> u16 {
    if device.channels == 0 || prefs.channel < device.channels {
        prefs.channel
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::takes::testing::FakePlatform;

    fn device(id: &str, name: &str, channels: u16, is_default: bool) -> DeviceInfo {
        FakePlatform::device(id, name, channels, is_default)
    }

    fn mic() -> DeviceInfo {
        FakePlatform::mic()
    }

    fn usb() -> DeviceInfo {
        FakePlatform::usb()
    }

    #[test]
    fn prefs_parse_errors_give_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("recording.json");
        assert_eq!(read_prefs(&path), Prefs::default());
        let cases: [(&str, &str); 6] = [
            ("{", "EOF while parsing an object at line 1 column 1"),
            (
                r#"{"schema":2,"deviceId":null,"channel":0}"#,
                "schema must be 1",
            ),
            (
                r#"{"deviceId":null,"channel":0}"#,
                "missing field `schema` at line 1 column 29",
            ),
            (
                r#"{"schema":1,"deviceId":3,"channel":0}"#,
                "invalid type: integer `3`, expected a string at line 1 column 24",
            ),
            (
                r#"{"schema":1,"deviceId":null,"channel":-1}"#,
                "invalid value: integer `-1`, expected u16 at line 1 column 40",
            ),
            (
                r#"{"schema":1,"deviceId":null,"channel":"0"}"#,
                "invalid type: string \"0\", expected u16 at line 1 column 41",
            ),
        ];
        for (text, message) in cases {
            assert_eq!(
                parse_prefs(text.as_bytes()),
                Err(message.to_string()),
                "{text}"
            );
            std::fs::write(&path, text).unwrap();
            assert_eq!(read_prefs(&path), Prefs::default(), "{text}");
            // The bad file is left in place.
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
        // A directory where the file should be.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(read_prefs(&path), Prefs::default());
    }

    #[test]
    fn prefs_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("data").join("recording.json");
        let prefs = Prefs {
            device_id: Some("coreaudio:abc".into()),
            channel: 3,
        };
        write_prefs(&path, &prefs).unwrap();
        assert_eq!(read_prefs(&path), prefs);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"schema\": 1,\n  \"deviceId\": \"coreaudio:abc\",\n  \"channel\": 3\n}\n"
        );
        assert_eq!(
            parse_prefs(br#"{"schema":1,"channel":2}"#),
            Ok(Prefs {
                device_id: None,
                channel: 2
            })
        );
    }

    #[test]
    fn selection_resolution() {
        let devices = [mic(), usb()];
        let saved = |id: Option<&str>, channel| Prefs {
            device_id: id.map(str::to_string),
            channel,
        };
        let pick =
            |prefs: &Prefs| resolve_selection(&devices, prefs).map(|(d, c)| (d.id.clone(), c));
        assert_eq!(pick(&Prefs::default()), Some(("fake:mic".into(), 0)));
        assert_eq!(
            pick(&saved(Some("fake:usb"), 3)),
            Some(("fake:usb".into(), 3))
        );
        // Channel clamp.
        assert_eq!(
            pick(&saved(Some("fake:usb"), 4)),
            Some(("fake:usb".into(), 0))
        );
        // Saved device absent: the default input, the channel checked against it.
        assert_eq!(
            pick(&saved(Some("fake:gone"), 3)),
            Some(("fake:mic".into(), 0))
        );
        // No default flagged: the first device.
        let plain = [device("x", "x", 2, false), device("y", "y", 2, false)];
        assert_eq!(
            resolve_selection(&plain, &saved(None, 1)).map(|(d, c)| (d.id.as_str(), c)),
            Some(("x", 1))
        );
        assert_eq!(resolve_selection(&[], &Prefs::default()), None);
    }

    #[test]
    fn saved_channel_cases() {
        let saved = |channel| Prefs {
            device_id: None,
            channel,
        };
        // An exclusive endpoint reports no channels: any saved channel is kept.
        let exclusive = device("x", "x", 0, false);
        assert_eq!(saved_channel(&exclusive, &saved(7)), 7);
        let usb = usb();
        assert_eq!(saved_channel(&usb, &saved(3)), 3);
        assert_eq!(saved_channel(&usb, &saved(4)), 0);
        assert_eq!(saved_channel(&usb, &saved(9)), 0);
    }
}
