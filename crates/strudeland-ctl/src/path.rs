//! Where the control socket lives. The app and every client compute it with this one helper.

use std::ffi::OsStr;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The app's bundle identifier; a `src-tauri` test checks it against `tauri.conf.json`.
pub const APP_IDENTIFIER: &str = "com.rudoi.strudeland";

/// The environment variable that overrides the socket path.
pub const SOCKET_ENV: &str = "STRUDELAND_SOCKET";

/// The socket's file name inside the app-data directory.
#[cfg(unix)]
pub(crate) const SOCKET_FILE: &str = "ctl.sock";

// The pathname must leave space for a NUL in sockaddr_un.
#[cfg(all(unix, not(target_os = "linux")))]
const MAX_SOCKET_PATH_BYTES: usize = 103;
#[cfg(target_os = "linux")]
const MAX_SOCKET_PATH_BYTES: usize = 107;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("HOME is not set; set {SOCKET_ENV} to choose the control socket path")]
    NoHome,
    #[cfg(unix)]
    #[error("socket path is {len} bytes; the platform allows at most {MAX_SOCKET_PATH_BYTES}")]
    TooLong { len: usize },
    #[error("{0}")]
    Endpoint(String),
}

/// The socket path from the process environment: `$STRUDELAND_SOCKET`, else the app-data dir.
pub fn socket_path() -> Result<PathBuf, PathError> {
    socket_path_with(None)
}

/// The socket path for an explicit `flag` (a client's `--socket`), else as [`socket_path`].
pub fn socket_path_with(flag: Option<&Path>) -> Result<PathBuf, PathError> {
    let env = std::env::var_os(SOCKET_ENV);
    let selected = flag.map(Path::as_os_str).or(env.as_deref());
    #[cfg(target_os = "linux")]
    let base = std::env::var_os("XDG_RUNTIME_DIR");
    #[cfg(not(target_os = "linux"))]
    let base = std::env::var_os("HOME");
    socket_path_from(selected, base.as_deref())
}

/// The endpoint for an override, or the platform default using the supplied runtime base.
pub fn socket_path_from(
    env_override: Option<&OsStr>,
    base: Option<&OsStr>,
) -> Result<PathBuf, PathError> {
    let path = env_override
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .map_or_else(|| default_endpoint(base), Ok)?;
    #[cfg(unix)]
    {
        let len = path.as_os_str().as_bytes().len();
        if len > MAX_SOCKET_PATH_BYTES {
            return Err(PathError::TooLong { len });
        }
    }
    #[cfg(windows)]
    crate::windows_pipe::validate_path(&path).map_err(|e| PathError::Endpoint(e.to_string()))?;
    Ok(path)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn default_endpoint(home: Option<&OsStr>) -> Result<PathBuf, PathError> {
    let home = home.filter(|s| !s.is_empty()).ok_or(PathError::NoHome)?;
    Ok(PathBuf::from(home)
        .join("Library/Application Support")
        .join(APP_IDENTIFIER)
        .join(SOCKET_FILE))
}

#[cfg(target_os = "linux")]
fn default_endpoint(runtime: Option<&OsStr>) -> Result<PathBuf, PathError> {
    let runtime = runtime.map(Path::new).filter(|p| p.is_absolute())
        .ok_or_else(|| PathError::Endpoint(format!(
            "XDG_RUNTIME_DIR must be an absolute private directory; set {SOCKET_ENV} to override"
        )))?;
    check_private_dir(runtime).map_err(|e| PathError::Endpoint(e.to_string()))?;
    Ok(runtime.join(APP_IDENTIFIER).join(SOCKET_FILE))
}

#[cfg(target_os = "linux")]
pub fn check_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path)?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    if meta.is_dir() && meta.uid() == uid && meta.mode() & 0o777 == 0o700 {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "{} must be an owned private directory (0700)",
                path.display()
            ),
        ))
    }
}

/// Authenticate the connected Unix peer independently of the pathname.
#[cfg(target_os = "linux")]
pub fn check_peer(socket: &impl std::os::fd::AsRawFd) -> std::io::Result<()> {
    let mut creds = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut len = size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: the descriptor is borrowed for this call and the kernel can write len bytes.
    let rc = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            creds.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: confirm the kernel supplied the whole structure before reading it.
    if len == size_of::<libc::ucred>() as libc::socklen_t
        && unsafe { creds.assume_init().uid == libc::geteuid() }
    {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "unexpected control socket peer",
        ))
    }
}

#[cfg(windows)]
fn default_endpoint(_: Option<&OsStr>) -> Result<PathBuf, PathError> {
    crate::windows_pipe::default_path().map_err(|e| PathError::Endpoint(e.to_string()))
}

#[cfg(all(test, target_os = "macos"))]
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
            "socket path is 120 bytes; the platform allows at most 103"
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

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    use std::fs::{Permissions, set_permissions};
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn runtime_directory_must_be_owned_private_and_real() {
        let temp = tempfile::Builder::new()
            .permissions(Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let path = temp.path();
        assert_eq!(
            socket_path_from(None, Some(path.as_os_str())).unwrap(),
            path.join(APP_IDENTIFIER).join(SOCKET_FILE)
        );
        let alias = path.join("alias");
        symlink(path, &alias).unwrap();
        assert!(check_private_dir(&alias).is_err());
        set_permissions(path, Permissions::from_mode(0o755)).unwrap();
        assert!(socket_path_from(None, Some(path.as_os_str())).is_err());
        assert!(socket_path_from(None, None).is_err());
    }

    #[test]
    fn linux_has_its_own_limit_and_honors_explicit_override() {
        let at_limit = format!("/{}", "w".repeat(106));
        assert_eq!(
            socket_path_from(Some(OsStr::new(&at_limit)), None),
            Ok(PathBuf::from(&at_limit))
        );
        let too_long = at_limit + "w";
        assert_eq!(
            socket_path_from(Some(OsStr::new(&too_long)), None),
            Err(PathError::TooLong { len: 108 })
        );
    }
}
