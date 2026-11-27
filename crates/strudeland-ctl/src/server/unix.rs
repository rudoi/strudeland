//! The Unix endpoint: the lock, the stale probe, the bind and the modes, and the
//! peer-checked accept.

use std::fs;
use std::fs::{File, TryLockError};
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tokio::net::UnixListener;

use super::{AcceptError, Bind};

/// A connected, verified client.
pub type Stream = tokio::net::UnixStream;

/// The bound socket and its held lock file. [`Endpoint::close`] (or dropping it) removes the
/// socket, then releases the lock; the lock file stays.
#[derive(Debug)]
pub struct Endpoint {
    listener: UnixListener,
    path: PathBuf,
    lock: Mutex<Option<File>>,
}

/// The result of [`bind_socket`]. `Listening` carries the held lock file: keep it as long as the
/// socket is served, and remove the socket before dropping it.
#[derive(Debug)]
enum BindOutcome {
    Listening(StdUnixListener, File),
    InUse,
    Disabled(String),
}

/// The lock file next to the socket: `ctl.sock` to `ctl.sock.lock`.
fn lock_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".lock");
    PathBuf::from(p)
}

/// Takes the exclusive lock on the socket's lock file (created `0600`, never removed). `None`
/// when another instance holds it.
fn lock_socket(path: &Path) -> Result<Option<File>, String> {
    let lock = lock_path(path);
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock)
        .map_err(|e| format!("cannot open {}: {e}", lock.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(format!("cannot lock {}: {e}", lock.display())),
    }
}

/// Binds the control socket at `path`: creates a missing parent dir `0700`, takes the lock on
/// `{path}.lock` (held by another instance: in use, the socket untouched), removes a stale
/// socket, leaves a live socket or a non-socket alone, and makes the new socket `0600` and
/// non-blocking.
fn bind_socket(path: &Path) -> BindOutcome {
    let shown = path.display();
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
        && !parent.exists()
        && let Err(e) = fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
    {
        return BindOutcome::Disabled(format!("cannot create {}: {e}", parent.display()));
    }
    // Only the lock holder probes, removes or binds, so two instances cannot both take the path.
    let lock = match lock_socket(path) {
        Ok(Some(lock)) => lock,
        Ok(None) => return BindOutcome::InUse,
        Err(detail) => return BindOutcome::Disabled(detail),
    };
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_socket() => {
            return BindOutcome::Disabled(format!("{shown} exists and is not a socket"));
        }
        Ok(_) => match StdUnixStream::connect(path) {
            Ok(_probe) => return BindOutcome::InUse,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                ) =>
            {
                if let Err(e) = fs::remove_file(path)
                    && e.kind() != io::ErrorKind::NotFound
                {
                    return BindOutcome::Disabled(format!("cannot remove stale {shown}: {e}"));
                }
            }
            Err(e) => return BindOutcome::Disabled(format!("{shown}: {e}")),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return BindOutcome::Disabled(format!("{shown}: {e}")),
    }
    let listener = match StdUnixListener::bind(path) {
        Ok(l) => l,
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => return BindOutcome::InUse,
        Err(e) => return BindOutcome::Disabled(format!("cannot listen on {shown}: {e}")),
    };
    let ready = fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .and_then(|()| listener.set_nonblocking(true));
    if let Err(e) = ready {
        drop(listener);
        let _ = fs::remove_file(path);
        return BindOutcome::Disabled(format!("cannot listen on {shown}: {e}"));
    }
    BindOutcome::Listening(listener, lock)
}

/// Binds the control socket at `path` and composes the detail of an outcome that does not
/// listen. Must be called inside a tokio runtime with IO enabled, for `UnixListener::from_std`.
pub fn bind(path: &Path) -> Bind {
    #[cfg(target_os = "linux")]
    if let Err(e) = ensure_private_parent(path) {
        return Bind::Disabled(e.to_string());
    }
    match bind_socket(path) {
        BindOutcome::Listening(std, lock) => match UnixListener::from_std(std) {
            Ok(listener) => Bind::Listening(Endpoint {
                listener,
                path: path.to_path_buf(),
                lock: Mutex::new(Some(lock)),
            }),
            Err(e) => {
                let _ = fs::remove_file(path);
                let detail = format!("cannot listen on {}: {e}", path.display());
                drop(lock);
                Bind::Disabled(detail)
            }
        },
        BindOutcome::InUse => Bind::InUse(format!(
            "another strudeland is listening on {}",
            path.display()
        )),
        BindOutcome::Disabled(detail) => Bind::Disabled(detail),
    }
}

#[cfg(target_os = "linux")]
fn ensure_private_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "control socket needs a private directory",
            )
        })?;
    // The XDG runtime directory is checked by endpoint discovery; creating only the last
    // component also prevents creating unchecked intermediate directories for an override.
    match fs::DirBuilder::new().mode(0o700).create(parent) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    crate::path::check_private_dir(parent)
}

/// Checks the connected peer: its uid must be ours on Linux.
#[cfg(target_os = "linux")]
fn verify_peer(stream: &Stream) -> io::Result<()> {
    crate::path::check_peer(stream)
}

/// No peer check on other Unix systems.
#[cfg(not(target_os = "linux"))]
fn verify_peer(_stream: &Stream) -> io::Result<()> {
    Ok(())
}

impl Endpoint {
    /// Accepts the next client whose peer passes the platform's check; a rejected peer is
    /// dropped without an error. Cancel-safe.
    pub async fn accept(&self) -> Result<Stream, AcceptError> {
        self.accept_verified(verify_peer).await
    }

    async fn accept_verified(
        &self,
        verify: impl Fn(&Stream) -> io::Result<()>,
    ) -> Result<Stream, AcceptError> {
        loop {
            let stream = match self.listener.accept().await {
                Ok((stream, _)) => stream,
                Err(source) => {
                    return Err(AcceptError::Accept {
                        source,
                        closed: false,
                    });
                }
            };
            if verify(&stream).is_err() {
                continue;
            }
            return Ok(stream);
        }
    }

    /// Removes the socket, then releases the lock. Only the first call does anything.
    pub fn close(&self) {
        let lock = self.lock.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(lock) = lock {
            let _ = fs::remove_file(&self.path);
            drop(lock);
        }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::answer_one_ping;
    use crate::{CallError, Client};
    use serde_json::json;
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn short_tempdir() -> tempfile::TempDir {
        // macOS limits socket paths to 103 bytes; $TMPDIR is long, /tmp is not.
        tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .prefix("sl")
            .tempdir_in("/tmp")
            .unwrap()
    }

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().mode() & 0o777
    }

    #[test]
    fn binds_a_fresh_path_with_mode_0600() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let BindOutcome::Listening(_l, _lock) = bind_socket(&path) else {
            panic!("expected Listening");
        };
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_socket());
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn creates_a_missing_parent_dir_0700() {
        let dir = short_tempdir();
        let path = dir.path().join("a/b/c.sock");
        let BindOutcome::Listening(_l, _lock) = bind_socket(&path) else {
            panic!("expected Listening");
        };
        assert_eq!(mode(&dir.path().join("a")), 0o700);
        assert_eq!(mode(&dir.path().join("a/b")), 0o700);
    }

    #[test]
    fn keeps_the_mode_of_an_existing_parent_dir() {
        let dir = short_tempdir();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let BindOutcome::Listening(_l, _lock) = bind_socket(&dir.path().join("c.sock")) else {
            panic!("expected Listening");
        };
        assert_eq!(mode(dir.path()), 0o755);
    }

    #[test]
    fn a_stale_socket_is_removed_and_bound() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        drop(StdUnixListener::bind(&path).unwrap());
        assert!(path.exists(), "dropping a listener leaves the file");
        let BindOutcome::Listening(l, _lock) = bind_socket(&path) else {
            panic!("expected Listening");
        };
        StdUnixStream::connect(&path).unwrap();
        l.set_nonblocking(false).unwrap();
        l.accept().unwrap();
    }

    #[test]
    fn a_live_socket_is_in_use_and_left_alone() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let other = StdUnixListener::bind(&path).unwrap();
        assert!(matches!(bind_socket(&path), BindOutcome::InUse));
        StdUnixStream::connect(&path).unwrap();
        other.accept().unwrap();
    }

    #[test]
    fn a_held_lock_is_in_use_and_the_stale_socket_is_left_alone() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        drop(StdUnixListener::bind(&path).unwrap()); // stale: nothing accepts
        let BindOutcome::Listening(l, held) = bind_socket(&path) else {
            panic!("expected Listening");
        };
        drop(l);
        assert!(path.exists());
        let before = fs::symlink_metadata(&path).unwrap().ino();
        assert!(matches!(bind_socket(&path), BindOutcome::InUse));
        assert_eq!(
            fs::symlink_metadata(&path).unwrap().ino(),
            before,
            "the lock holder's socket is not removed"
        );
        let lock = lock_path(&path);
        assert_eq!(lock, dir.path().join("c.sock.lock"));
        assert_eq!(mode(&lock), 0o600);
        drop(held);
        assert!(matches!(bind_socket(&path), BindOutcome::Listening(_, _)));
        assert!(lock.exists(), "the lock file is never removed");
    }

    #[test]
    fn a_regular_file_is_disabled_and_left_alone() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        fs::write(&path, "keep me").unwrap();
        let BindOutcome::Disabled(detail) = bind_socket(&path) else {
            panic!("expected Disabled");
        };
        assert_eq!(
            detail,
            format!("{} exists and is not a socket", path.display())
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");
    }

    #[test]
    fn an_uncreatable_parent_is_disabled() {
        let dir = short_tempdir();
        let ro = dir.path().join("ro");
        fs::create_dir(&ro).unwrap();
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o500)).unwrap();
        let outcome = bind_socket(&ro.join("a/c.sock"));
        let e = fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(ro.join("a"))
            .unwrap_err();
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o700)).unwrap();
        let BindOutcome::Disabled(detail) = outcome else {
            panic!("expected Disabled");
        };
        assert_eq!(
            detail,
            format!("cannot create {}: {e}", ro.join("a").display())
        );

        // A path under a regular file skips the create and fails at the lock instead.
        let file = dir.path().join("f");
        fs::write(&file, "").unwrap();
        let lock = file.join("c.sock.lock");
        let e = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lock)
            .unwrap_err();
        let BindOutcome::Disabled(detail) = bind_socket(&file.join("c.sock")) else {
            panic!("expected Disabled");
        };
        assert_eq!(detail, format!("cannot open {}: {e}", lock.display()));
    }

    #[test]
    fn an_unopenable_lock_is_disabled() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let lock = lock_path(&path);
        fs::create_dir(&lock).unwrap();
        let e = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lock)
            .unwrap_err();
        let BindOutcome::Disabled(detail) = bind_socket(&path) else {
            panic!("expected Disabled");
        };
        assert_eq!(detail, format!("cannot open {}: {e}", lock.display()));
        assert!(!path.exists());
    }

    #[test]
    fn an_unremovable_stale_socket_is_disabled() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        drop(StdUnixListener::bind(&path).unwrap()); // stale: nothing accepts
        fs::write(lock_path(&path), "").unwrap();
        let before = fs::symlink_metadata(&path).unwrap().ino();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let outcome = bind_socket(&path);
        let e = fs::remove_file(&path).unwrap_err();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let BindOutcome::Disabled(detail) = outcome else {
            panic!("expected Disabled");
        };
        assert_eq!(
            detail,
            format!("cannot remove stale {}: {e}", path.display())
        );
        assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), before);
    }

    #[test]
    fn a_probe_error_is_disabled() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        drop(StdUnixListener::bind(&path).unwrap()); // stale: nothing accepts
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let e = StdUnixStream::connect(&path).unwrap_err();
        assert!(
            !matches!(
                e.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ),
            "{e}"
        );
        let BindOutcome::Disabled(detail) = bind_socket(&path) else {
            panic!("expected Disabled");
        };
        assert_eq!(detail, format!("{}: {e}", path.display()));
        assert!(path.exists());
    }

    #[test]
    fn an_unbindable_path_is_disabled() {
        let dir = short_tempdir();
        let path = dir.path().join("s".repeat(110));
        assert!(path.as_os_str().len() > 108);
        let e = StdUnixListener::bind(&path).unwrap_err();
        let BindOutcome::Disabled(detail) = bind_socket(&path) else {
            panic!("expected Disabled");
        };
        assert_eq!(detail, format!("cannot listen on {}: {e}", path.display()));
    }

    #[test]
    fn bind_reports_each_outcome() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let rt = runtime();
        let _guard = rt.enter();
        let Bind::Listening(_endpoint) = bind(&path) else {
            panic!("expected Listening");
        };
        let Bind::InUse(detail) = bind(&path) else {
            panic!("expected InUse");
        };
        assert_eq!(
            detail,
            format!("another strudeland is listening on {}", path.display())
        );

        let file = dir.path().join("f.sock");
        fs::write(&file, "keep me").unwrap();
        let Bind::Disabled(detail) = bind(&file) else {
            panic!("expected Disabled");
        };
        assert_eq!(
            detail,
            format!("{} exists and is not a socket", file.display())
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep me");
    }

    #[test]
    fn a_client_round_trips_over_the_endpoint() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let rt = runtime();
        let endpoint = {
            let _guard = rt.enter();
            let Bind::Listening(endpoint) = bind(&path) else {
                panic!("expected Listening");
            };
            endpoint
        };
        let server = std::thread::spawn(move || {
            rt.block_on(async {
                for _ in 0..2 {
                    answer_one_ping(endpoint.accept().await.unwrap()).await;
                }
            });
            (rt, endpoint)
        });
        for _ in 0..2 {
            let mut client = Client::connect(&path, Duration::from_secs(1)).unwrap();
            assert_eq!(client.call_raw("ping", json!({})).unwrap(), json!("ok"));
        }
        let (_rt, endpoint) = server.join().unwrap();
        endpoint.close();
        assert!(!path.exists());
        assert!(lock_path(&path).exists(), "the lock file is never removed");
        assert!(matches!(
            Client::connect(&path, Duration::from_secs(1)),
            Err(CallError::NotRunning { .. })
        ));
    }

    #[test]
    fn close_frees_the_path_and_twice_is_harmless() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let rt = runtime();
        let _guard = rt.enter();
        let Bind::Listening(first) = bind(&path) else {
            panic!("expected Listening");
        };
        first.close();
        assert!(!path.exists());
        let Bind::Listening(_second) = bind(&path) else {
            panic!("expected Listening");
        };
        first.close();
        assert!(path.exists(), "a second close leaves the new socket alone");
    }

    #[test]
    fn dropping_the_endpoint_closes_it() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let rt = runtime();
        let _guard = rt.enter();
        let Bind::Listening(endpoint) = bind(&path) else {
            panic!("expected Listening");
        };
        drop(endpoint);
        assert!(!path.exists());
        assert!(matches!(bind(&path), Bind::Listening(_)));
    }

    #[test]
    fn a_rejected_peer_is_skipped_silently() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let rt = runtime();
        let _guard = rt.enter();
        let Bind::Listening(endpoint) = bind(&path) else {
            panic!("expected Listening");
        };
        let mut one = StdUnixStream::connect(&path).unwrap();
        one.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut two = Client::connect(&path, Duration::from_secs(1)).unwrap();
        let calls = AtomicUsize::new(0);
        let stream = rt
            .block_on(endpoint.accept_verified(|_| {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(io::Error::other("refused"))
                } else {
                    Ok(())
                }
            }))
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(one.read(&mut [0]).unwrap(), 0, "the refused peer reads EOF");
        let client = std::thread::spawn(move || {
            assert_eq!(two.call_raw("ping", json!({})).unwrap(), json!("ok"));
        });
        rt.block_on(answer_one_ping(stream));
        client.join().unwrap();
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_shared_parent_is_disabled() {
        let dir = short_tempdir();
        let parent = dir.path().join("p");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
        let path = parent.join("c.sock");
        let Bind::Disabled(detail) = bind(&path) else {
            panic!("expected Disabled");
        };
        assert_eq!(
            detail,
            format!(
                "{} must be an owned private directory (0700)",
                parent.display()
            )
        );
        assert!(!path.exists());
        assert!(!lock_path(&path).exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_path_without_a_parent_is_disabled() {
        let Bind::Disabled(detail) = bind(Path::new("c.sock")) else {
            panic!("expected Disabled");
        };
        assert_eq!(detail, "control socket needs a private directory");
        assert!(!Path::new("c.sock.lock").exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_missing_grandparent_is_disabled() {
        let dir = short_tempdir();
        let e = fs::DirBuilder::new()
            .mode(0o700)
            .create(dir.path().join("a/b"))
            .unwrap_err();
        let Bind::Disabled(detail) = bind(&dir.path().join("a/b/c.sock")) else {
            panic!("expected Disabled");
        };
        assert_eq!(detail, e.to_string());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_new_private_parent_is_created_0700() {
        let dir = short_tempdir();
        let rt = runtime();
        let _guard = rt.enter();
        let Bind::Listening(_endpoint) = bind(&dir.path().join("p/c.sock")) else {
            panic!("expected Listening");
        };
        assert_eq!(mode(&dir.path().join("p")), 0o700);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn check_peer_accepts_a_same_user_pair() {
        let (a, _b) = StdUnixStream::pair().unwrap();
        crate::path::check_peer(&a).unwrap();
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn check_peer_refuses_a_non_socket() {
        let file = File::open("/dev/null").unwrap();
        assert!(crate::path::check_peer(&file).is_err());
    }
}
