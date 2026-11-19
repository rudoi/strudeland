//! Where the control socket lives. The app and every client compute it with this one helper.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The app's bundle identifier; a `src-tauri` test checks it against `tauri.conf.json`.
pub const APP_IDENTIFIER: &str = "com.rudoi.strudeland";

/// The environment variable that overrides the socket path.
pub const SOCKET_ENV: &str = "STRUDELAND_SOCKET";

/// The socket's file name inside the app-data directory.
pub(crate) const SOCKET_FILE: &str = "ctl.sock";

/// The longest socket path macOS accepts (`sun_path` is 104 bytes, NUL included).
pub(crate) const MAX_SOCKET_PATH_BYTES: usize = 103;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("HOME is not set; set {SOCKET_ENV} to choose the control socket path")]
    NoHome,
    #[error("socket path is {len} bytes; macOS allows at most {MAX_SOCKET_PATH_BYTES}")]
    TooLong { len: usize },
}

/// The socket path from the process environment: `$STRUDELAND_SOCKET`, else the app-data dir.
pub fn socket_path() -> Result<PathBuf, PathError> {
    socket_path_with(None)
}

/// The socket path for an explicit `flag` (a client's `--socket`), else as [`socket_path`].
pub fn socket_path_with(flag: Option<&Path>) -> Result<PathBuf, PathError> {
    let env = std::env::var_os(SOCKET_ENV);
    socket_path_from(
        flag.map(Path::as_os_str).or(env.as_deref()),
        std::env::var_os("HOME").as_deref(),
    )
}

/// The socket path for a given override and `HOME`. A non-empty override is used as given (not
/// canonicalized); otherwise `$HOME/Library/Application Support/<id>/ctl.sock`. Either way the
/// result must fit in [`MAX_SOCKET_PATH_BYTES`].
pub fn socket_path_from(
    env_override: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Result<PathBuf, PathError> {
    let path = match env_override.filter(|s| !s.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => {
            let home = home.filter(|s| !s.is_empty()).ok_or(PathError::NoHome)?;
            PathBuf::from(home)
                .join("Library/Application Support")
                .join(APP_IDENTIFIER)
                .join(SOCKET_FILE)
        }
    };
    let len = path.as_os_str().as_bytes().len();
    if len > MAX_SOCKET_PATH_BYTES {
        return Err(PathError::TooLong { len });
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn os(s: &str) -> OsString {
        OsString::from(s)
    }

    #[test]
    fn default_is_under_home() {
        let p = socket_path_from(None, Some(&os("/Users/me"))).unwrap();
        assert_eq!(
            p,
            Path::new("/Users/me/Library/Application Support/com.rudoi.strudeland/ctl.sock")
        );
    }

    #[test]
    fn override_wins_and_is_used_as_given() {
        let p = socket_path_from(Some(&os("./rel/../x.sock")), Some(&os("/Users/me"))).unwrap();
        assert_eq!(p, Path::new("./rel/../x.sock"));
        // No HOME needed with an override.
        let p = socket_path_from(Some(&os("/tmp/s.sock")), None).unwrap();
        assert_eq!(p, Path::new("/tmp/s.sock"));
    }

    #[test]
    fn empty_override_is_ignored() {
        let p = socket_path_from(Some(&os("")), Some(&os("/h"))).unwrap();
        assert_eq!(
            p,
            Path::new("/h/Library/Application Support/com.rudoi.strudeland/ctl.sock")
        );
    }

    #[test]
    fn home_unset_or_empty_is_no_home() {
        assert_eq!(socket_path_from(None, None), Err(PathError::NoHome));
        assert_eq!(
            socket_path_from(None, Some(&os(""))),
            Err(PathError::NoHome)
        );
        assert_eq!(
            socket_path_from(Some(&os("")), Some(&os(""))),
            Err(PathError::NoHome)
        );
    }

    #[test]
    fn override_length_limit() {
        let at = format!("/{}", "a".repeat(102));
        assert_eq!(at.len(), 103);
        assert_eq!(
            socket_path_from(Some(&os(&at)), None).unwrap(),
            PathBuf::from(&at)
        );
        let over = format!("/{}", "a".repeat(103));
        assert_eq!(
            socket_path_from(Some(&os(&over)), None),
            Err(PathError::TooLong { len: 104 })
        );
    }

    #[test]
    fn default_length_limit() {
        // The suffix after HOME is fixed; size HOME so the result is exactly 103, then 104.
        let suffix = "/Library/Application Support/com.rudoi.strudeland/ctl.sock".len();
        let home_at = format!("/{}", "h".repeat(103 - suffix - 1));
        let p = socket_path_from(None, Some(&os(&home_at))).unwrap();
        assert_eq!(p.as_os_str().len(), 103);
        let home_over = format!("{home_at}h");
        assert_eq!(
            socket_path_from(None, Some(&os(&home_over))),
            Err(PathError::TooLong { len: 104 })
        );
    }

    #[test]
    fn too_long_message() {
        assert_eq!(
            PathError::TooLong { len: 120 }.to_string(),
            "socket path is 120 bytes; macOS allows at most 103"
        );
    }

    #[test]
    fn socket_path_reads_the_environment() {
        // Only checks it agrees with the pure function; the environment is not mutated.
        assert_eq!(
            socket_path(),
            socket_path_from(
                std::env::var_os(SOCKET_ENV).as_deref(),
                std::env::var_os("HOME").as_deref()
            )
        );
    }

    #[test]
    fn a_flag_wins_over_the_environment() {
        let flag = Path::new("/tmp/flag.sock");
        assert_eq!(socket_path_with(Some(flag)), Ok(flag.to_path_buf()));
        assert_eq!(socket_path_with(None), socket_path());
    }
}
