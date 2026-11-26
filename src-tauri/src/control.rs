//! The local control endpoint, speaking line-delimited JSON-RPC (see `strudeland-ctl`).
//!
//! Rust does no music logic. `ping` and `screenshot` are answered here; every other known method is
//! forwarded to the webview as a `control://request` event and answered when the webview calls
//! `control_reply` with the same internal id, or with a timeout error when it does not.
//!
//! The bridge sits behind [`RequestEmitter`] and the native snapshot behind [`Snapshotter`], so
//! both are tested without an `AppHandle`; the listener setup is the free function
//! [`bind_socket`], tested on a tempdir path.

use std::collections::HashMap;
#[cfg(any(unix, test))]
use std::fs;
#[cfg(unix)]
use std::fs::{File, TryLockError};
#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use strudeland_ctl::{
    FrameError, LineReader, MAX_LINE_BYTES, Method, PathError, PingResult, Response, RpcError,
    ScreenshotResult, codes, encode_response, parse_request,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{
    self as async_io, AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt,
    BufReader,
};
#[cfg(unix)]
use tokio::net::UnixListener;
#[cfg(all(unix, test))]
use tokio::net::UnixStream;
#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::oneshot;

/// The event the webview listens to for requests; its payload is a [`BridgeRequest`].
pub const REQUEST_EVENT: &str = "control://request";

const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const MIN_TIMEOUT_MS: u64 = 1_000;
const MAX_TIMEOUT_MS: u64 = 120_000;

/// Whether this instance owns the control socket.
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum SocketStatus {
    Listening,
    InUse,
    Disabled,
}

/// What the webview learns on attach.
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ControlInfo {
    pub status: SocketStatus,
    pub path: Option<String>,
    pub detail: Option<String>,
}

impl ControlInfo {
    fn new(status: SocketStatus, path: Option<&Path>, detail: Option<String>) -> Self {
        Self {
            status,
            path: path.map(|p| p.to_string_lossy().into_owned()),
            detail,
        }
    }
}

/// The payload of `control://request`. `params` is forwarded as received; `timeout_ms` becomes
/// the effective (clamped) `timeoutMs`, so the webview can skip a request that waited longer.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BridgeRequest<'a> {
    pub id: u64,
    pub method: &'a str,
    pub params: &'a Value,
    pub timeout_ms: u64,
}

/// The argument of `control_reply`: `{ "result": … }` or `{ "error": { "code", "message" } }`.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ReplyPayload {
    Result(Value),
    Error(RpcError),
}

pub type Reply = Result<Value, RpcError>;

impl From<ReplyPayload> for Reply {
    fn from(p: ReplyPayload) -> Self {
        match p {
            ReplyPayload::Result(v) => Ok(v),
            ReplyPayload::Error(e) => Err(e),
        }
    }
}

/// Delivers a request to the webview.
pub trait RequestEmitter: Send + Sync + 'static {
    fn emit_request(&self, request: &BridgeRequest) -> Result<(), String>;
}

/// The outcome of a snapshot, delivered once.
pub type SnapshotReceiver = oneshot::Receiver<Result<ScreenshotResult, String>>;

/// Takes a PNG snapshot of the `main` webview's web content.
pub trait Snapshotter: Send + Sync + 'static {
    /// Starts a snapshot of the `main` webview's web content and writes a PNG to `path`.
    /// `Err` means it could not start (no window, unsupported OS). `Ok` yields a receiver that
    /// resolves with the outcome; a dropped sender means the snapshot was cancelled.
    fn snapshot(&self, path: PathBuf) -> Result<SnapshotReceiver, String>;
}

/// Pending webview requests and the socket's status.
pub struct Bridge {
    pending: Mutex<HashMap<u64, oneshot::Sender<Reply>>>,
    /// Internal ids, starting at 1; unrelated to the client's JSON-RPC id.
    next_id: AtomicU64,
    attached: AtomicBool,
    /// Set once by `start`; `disabled` until then.
    info: Mutex<ControlInfo>,
    /// The socket's lock file, held for the app's lifetime while listening.
    #[cfg(unix)]
    socket_lock: Mutex<Option<File>>,
    #[cfg(windows)]
    server: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl Default for Bridge {
    fn default() -> Self {
        Self::new()
    }
}

/// Locks `m`, recovering from poison: every critical section leaves the data consistent.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Bridge {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            attached: AtomicBool::new(false),
            info: Mutex::new(ControlInfo::new(SocketStatus::Disabled, None, None)),
            #[cfg(unix)]
            socket_lock: Mutex::new(None),
            #[cfg(windows)]
            server: Mutex::new(None),
        }
    }

    pub fn set_info(&self, info: ControlInfo) {
        *lock(&self.info) = info;
    }

    pub fn info(&self) -> ControlInfo {
        lock(&self.info).clone()
    }

    pub fn attached(&self) -> bool {
        self.attached.load(Ordering::SeqCst)
    }

    /// Marks the webview attached and fails every pending request: they belong to a previous page
    /// load (a reload or HMR). Returns the socket's status.
    pub fn attach(&self) -> ControlInfo {
        self.attached.store(true, Ordering::SeqCst);
        self.fail_pending();
        self.info()
    }

    /// Marks the webview detached (a new page load started) and fails every pending request, so
    /// requests answer `not ready` until the new page attaches.
    pub fn detach(&self) {
        self.attached.store(false, Ordering::SeqCst);
        self.fail_pending();
    }

    fn fail_pending(&self) {
        let pending = std::mem::take(&mut *lock(&self.pending));
        for (_, tx) in pending {
            let _ = tx.send(Err(RpcError::page_reloaded()));
        }
    }

    /// Resolves the pending request `id`. An unknown id is a late reply after a timeout.
    pub fn reply(&self, id: u64, payload: ReplyPayload) {
        let tx = lock(&self.pending).remove(&id);
        match tx {
            Some(tx) => {
                let _ = tx.send(payload.into());
            }
            None => eprintln!("control: ignoring a reply to unknown request {id}"),
        }
    }

    #[cfg(test)]
    fn pending_len(&self) -> usize {
        lock(&self.pending).len()
    }

    /// Forwards one request to the webview and waits up to `timeout` for its reply.
    pub async fn call(
        &self,
        emitter: &dyn RequestEmitter,
        method: Method,
        params: Value,
        timeout: Duration,
    ) -> Reply {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, tx);
        let request = BridgeRequest {
            id,
            method: method.as_str(),
            params: &params,
            timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        };
        if let Err(e) = emitter.emit_request(&request) {
            eprintln!("control: cannot emit {REQUEST_EVENT}: {e}");
            lock(&self.pending).remove(&id);
            return Err(RpcError::not_ready());
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => reply,
            // Only `attach`/`detach` drop senders, and they send first; keep its meaning if that
            // changes.
            Ok(Err(_)) => Err(RpcError::page_reloaded()),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(timeout_error(timeout))
            }
        }
    }
}

fn clamp_timeout(timeout_ms: Option<u64>) -> Duration {
    Duration::from_millis(
        timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS),
    )
}

fn timeout_error(timeout: Duration) -> RpcError {
    let secs = (timeout.as_millis() as u64).div_ceil(1000);
    RpcError::new(codes::TIMEOUT, format!("the app did not answer in {secs}s"))
}

fn invalid_params(detail: impl std::fmt::Display) -> RpcError {
    RpcError::new(codes::INVALID_PARAMS, format!("invalid params: {detail}"))
}

/// The default screenshot file: `strudeland-screenshot-<unix ms>.png` in the temp dir.
fn default_screenshot_path() -> PathBuf {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "strudeland-screenshot-{ms}-{}-{id}.png",
        std::process::id()
    ))
}

/// Validates the `screenshot` params (the one method whose params Rust checks): an object (null is
/// `{}`) with at most `path`, an absolute file path in an existing directory.
fn screenshot_path(params: &Value) -> Result<PathBuf, RpcError> {
    let map = match params {
        Value::Null => return Ok(default_screenshot_path()),
        Value::Object(map) => map,
        _ => return Err(invalid_params("expected an object")),
    };
    if let Some(key) = map.keys().find(|k| k.as_str() != "path") {
        return Err(invalid_params(format!("unknown parameter '{key}'")));
    }
    let Some(path) = map.get("path") else {
        return Ok(default_screenshot_path());
    };
    let Some(path) = path.as_str() else {
        return Err(invalid_params("'path' must be a string"));
    };
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(invalid_params(format!(
            "'path' must be absolute, got {}",
            path.display()
        )));
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return Err(invalid_params(match path.parent() {
            Some(parent) => format!("the directory {} does not exist", parent.display()),
            None => format!("{} is not a file path", path.display()),
        }));
    }
    if path.is_dir() {
        return Err(invalid_params(format!("{} is a directory", path.display())));
    }
    Ok(path)
}

/// Answers `screenshot` through the snapshotter, waiting up to `timeout` for the outcome.
async fn screenshot(
    snapshotter: &dyn Snapshotter,
    params: &Value,
    timeout: Duration,
) -> Result<Value, RpcError> {
    let path = screenshot_path(params)?;
    let failed = |m: String| RpcError::new(codes::SCREENSHOT_FAILED, m);
    let rx = snapshotter.snapshot(path).map_err(failed)?;
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(Ok(result))) => {
            Ok(serde_json::to_value(result).expect("ScreenshotResult serializes"))
        }
        Ok(Ok(Err(m))) => Err(failed(m)),
        Ok(Err(_)) => Err(failed("the snapshot was cancelled".into())),
        Err(_) => Err(timeout_error(timeout)),
    }
}

/// One request line to one response.
pub async fn handle_line(
    bridge: &Bridge,
    emitter: &dyn RequestEmitter,
    snapshotter: &dyn Snapshotter,
    line: &str,
) -> Response {
    let request = match parse_request(line) {
        Ok(r) => r,
        Err((id, e)) => return Response::err(id, e),
    };
    let id = request.id;
    let Some(method) = Method::parse(&request.method) else {
        return Response::err(
            id,
            RpcError::new(
                codes::METHOD_NOT_FOUND,
                format!("unknown method '{}'", request.method),
            ),
        );
    };
    if method == Method::Ping {
        let result = PingResult {
            version: env!("CARGO_PKG_VERSION").to_string(),
            attached: bridge.attached(),
        };
        return Response::ok(
            id,
            serde_json::to_value(result).expect("PingResult serializes"),
        );
    }
    let timeout = clamp_timeout(request.timeout_ms);
    if method == Method::Screenshot {
        // Needs the window, not the webview's control handler: never forwarded.
        return match screenshot(snapshotter, &request.params, timeout).await {
            Ok(result) => Response::ok(id, result),
            Err(e) => Response::err(id, e),
        };
    }
    if !bridge.attached() {
        return Response::err(id, RpcError::not_ready());
    }
    if method == Method::ProjectOpen
        && let Some(path) = request.params.get("path").and_then(Value::as_str)
        && !Path::new(path).is_absolute()
    {
        return Response::err(id, invalid_params("'path' must be an absolute path"));
    }
    match bridge.call(emitter, method, request.params, timeout).await {
        Ok(result) => Response::ok(id, result),
        Err(e) => Response::err(id, e),
    }
}

/// Reads one `\n`-terminated line of at most `max` bytes (newline included); `None` at EOF. The
/// async twin of `strudeland_ctl::read_message`, over the same `LineReader`.
async fn read_line_capped<R: AsyncBufRead + Unpin>(
    r: &mut R,
    max: usize,
) -> Result<Option<String>, FrameError> {
    let mut line = LineReader::new(max);
    loop {
        let (n, done) = line.feed(r.fill_buf().await?)?;
        r.consume(n);
        if done {
            return line.finish();
        }
    }
}

/// A parse error (`-32700` with a null id) closes the connection; other errors keep it open.
fn closes_connection(r: &Response) -> bool {
    r.id.is_none()
        && r.error
            .as_ref()
            .is_some_and(|e| e.code == codes::PARSE_ERROR)
}

/// Serves one connection: requests are handled one at a time, in order.
async fn serve_connection(
    stream: impl AsyncRead + AsyncWrite + Unpin,
    bridge: Arc<Bridge>,
    emitter: Arc<dyn RequestEmitter>,
    snapshotter: Arc<dyn Snapshotter>,
) {
    let (read, mut write) = async_io::split(stream);
    let mut reader = BufReader::new(read);
    loop {
        let response = match read_line_capped(&mut reader, MAX_LINE_BYTES).await {
            Ok(None) | Err(FrameError::Io(_)) => return,
            Ok(Some(line)) => {
                handle_line(&bridge, emitter.as_ref(), snapshotter.as_ref(), &line).await
            }
            Err(FrameError::TooLong) => Response::err(None, RpcError::too_large()),
            Err(e @ FrameError::NotUtf8) => Response::err(None, RpcError::parse(e)),
        };
        let mut out = encode_response(&response);
        out.push('\n');
        if write.write_all(out.as_bytes()).await.is_err() || write.flush().await.is_err() {
            return;
        }
        if closes_connection(&response) {
            return;
        }
    }
}

/// The accept loop: one task per connection, no idle timeout, no connection limit.
#[cfg(unix)]
async fn serve(
    listener: UnixListener,
    bridge: Arc<Bridge>,
    emitter: Arc<dyn RequestEmitter>,
    snapshotter: Arc<dyn Snapshotter>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                #[cfg(target_os = "linux")]
                if strudeland_ctl::path::check_peer(&stream).is_err() {
                    continue;
                }
                tokio::spawn(serve_connection(
                    stream,
                    Arc::clone(&bridge),
                    Arc::clone(&emitter),
                    Arc::clone(&snapshotter),
                ));
            }
            Err(e) => {
                eprintln!("control: accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// The result of [`bind_socket`]. `Listening` carries the held lock file: keep it as long as the
/// socket is served, and remove the socket before dropping it.
#[derive(Debug)]
#[cfg(unix)]
pub enum BindOutcome {
    Listening(StdUnixListener, File),
    InUse,
    Disabled(String),
}

/// The lock file next to the socket: `ctl.sock` to `ctl.sock.lock`.
#[cfg(unix)]
fn lock_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".lock");
    PathBuf::from(p)
}

/// Takes the exclusive lock on the socket's lock file (created `0600`, never removed). `None`
/// when another instance holds it.
#[cfg(unix)]
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
#[cfg(unix)]
pub fn bind_socket(path: &Path) -> BindOutcome {
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

/// Binds the socket and turns the outcome into a status, with the listener and its held lock
/// file. Needs a tokio runtime context, for `UnixListener::from_std`.
#[cfg(unix)]
fn listen(path: Result<PathBuf, PathError>) -> (ControlInfo, Option<(UnixListener, File)>) {
    let path = match path {
        Ok(p) => p,
        Err(e) => {
            return (
                ControlInfo::new(SocketStatus::Disabled, None, Some(e.to_string())),
                None,
            );
        }
    };
    #[cfg(target_os = "linux")]
    if let Err(e) = ensure_private_parent(&path) {
        return (
            ControlInfo::new(SocketStatus::Disabled, Some(&path), Some(e.to_string())),
            None,
        );
    }
    match bind_socket(&path) {
        BindOutcome::Listening(std, lock) => match UnixListener::from_std(std) {
            Ok(l) => (
                ControlInfo::new(SocketStatus::Listening, Some(&path), None),
                Some((l, lock)),
            ),
            Err(e) => {
                let _ = fs::remove_file(&path);
                let detail = format!("cannot listen on {}: {e}", path.display());
                (
                    ControlInfo::new(SocketStatus::Disabled, Some(&path), Some(detail)),
                    None,
                )
            }
        },
        BindOutcome::InUse => {
            let detail = format!("another strudeland is listening on {}", path.display());
            (
                ControlInfo::new(SocketStatus::InUse, Some(&path), Some(detail)),
                None,
            )
        }
        BindOutcome::Disabled(detail) => (
            ControlInfo::new(SocketStatus::Disabled, Some(&path), Some(detail)),
            None,
        ),
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
    strudeland_ctl::path::check_private_dir(parent)
}

#[cfg(windows)]
fn listen(path: Result<PathBuf, PathError>) -> (ControlInfo, Option<(NamedPipeServer, PathBuf)>) {
    use strudeland_ctl::windows_pipe;
    let path = match path {
        Ok(path) => path,
        Err(e) => {
            return (
                ControlInfo::new(SocketStatus::Disabled, None, Some(e.to_string())),
                None,
            );
        }
    };
    match windows_pipe::listen(&path, true) {
        Ok(pipe) => (
            ControlInfo::new(SocketStatus::Listening, Some(&path), None),
            Some((pipe, path)),
        ),
        Err(e) => {
            let status = if windows_pipe::owned_existing(&path) {
                SocketStatus::InUse
            } else {
                SocketStatus::Disabled
            };
            let message = format!("cannot listen on {}: {e}", path.display());
            (ControlInfo::new(status, Some(&path), Some(message)), None)
        }
    }
}

#[cfg(windows)]
async fn serve(
    (mut pipe, path): (NamedPipeServer, PathBuf),
    bridge: Arc<Bridge>,
    emitter: Arc<dyn RequestEmitter>,
    snapshotter: Arc<dyn Snapshotter>,
) {
    let mut clients = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            outcome = pipe.connect() => if let Err(e) = outcome {
                eprintln!("control: accept failed: {e}");
                if pipe.disconnect().is_err() {
                    return;
                }
                continue;
            },
            _ = clients.join_next(), if !clients.is_empty() => continue,
        }
        // Keep at least one instance alive, and ensure a second client can connect.
        let next = match strudeland_ctl::windows_pipe::listen(&path, false) {
            Ok(next) => next,
            Err(e) => {
                eprintln!("control: cannot create next pipe instance: {e}");
                if pipe.disconnect().is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let connected = std::mem::replace(&mut pipe, next);
        clients.spawn(serve_connection(
            connected,
            Arc::clone(&bridge),
            Arc::clone(&emitter),
            Arc::clone(&snapshotter),
        ));
    }
}

/// Starts the control socket. Never fails the app: every problem becomes the bridge's
/// [`ControlInfo`], logged to stderr.
pub fn start(app: &AppHandle, bridge: Arc<Bridge>) {
    // Enter the runtime so the listener is registered, and the status set, before the webview
    // can attach.
    let runtime = tauri::async_runtime::handle();
    let _guard = runtime.inner().enter();
    let (info, listener) = listen(strudeland_ctl::socket_path());
    match (&info.status, &info.detail) {
        (SocketStatus::Listening, _) => eprintln!(
            "control: listening on {}",
            info.path.as_deref().unwrap_or_default()
        ),
        (_, Some(detail)) => eprintln!("control: disabled: {detail}"),
        (_, None) => eprintln!("control: disabled"),
    }
    bridge.set_info(info);
    if let Some(listener) = listener {
        #[cfg(unix)]
        let listener = {
            let (listener, file) = listener;
            *lock(&bridge.socket_lock) = Some(file);
            listener
        };
        let emitter: Arc<dyn RequestEmitter> = Arc::new(TauriEmitter(app.clone()));
        let snapshotter: Arc<dyn Snapshotter> =
            Arc::new(crate::snapshot::TauriSnapshotter(app.clone()));
        let server =
            tauri::async_runtime::spawn(serve(listener, Arc::clone(&bridge), emitter, snapshotter));
        #[cfg(windows)]
        {
            *lock(&bridge.server) = Some(server);
        }
        #[cfg(unix)]
        drop(server);
    }
}

/// Removes the socket file if this instance created it. Returns whether it removed one.
#[cfg(unix)]
fn remove_socket(info: &ControlInfo) -> bool {
    match (&info.status, &info.path) {
        (SocketStatus::Listening, Some(path)) => fs::remove_file(path).is_ok(),
        _ => false,
    }
}

/// Removes the socket file on exit, when this instance was listening, then releases the lock.
pub fn shutdown(app: &AppHandle) {
    let bridge = app.state::<Arc<Bridge>>();
    #[cfg(unix)]
    {
        remove_socket(&bridge.info());
        lock(&bridge.socket_lock).take();
    }
    #[cfg(windows)]
    if let Some(server) = lock(&bridge.server).take() {
        server.abort();
    }
}

struct TauriEmitter(AppHandle);

impl RequestEmitter for TauriEmitter {
    fn emit_request(&self, request: &BridgeRequest) -> Result<(), String> {
        self.0
            .emit_to("main", REQUEST_EVENT, request)
            .map_err(|e| e.to_string())
    }
}

/// Called by the webview once it listens for requests. Fails requests left from a previous page
/// load and returns the socket's status.
#[tauri::command]
pub fn control_attach(bridge: State<'_, Arc<Bridge>>) -> ControlInfo {
    bridge.attach()
}

/// The webview's answer to the request `id`.
#[tauri::command]
pub fn control_reply(bridge: State<'_, Arc<Bridge>>, id: u64, reply: ReplyPayload) {
    bridge.reply(id, reply);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    use strudeland_ctl::wire_fixtures;

    type Respond = Box<dyn Fn(&str, &Value) -> ReplyPayload + Send + Sync>;

    /// Records each emitted request; optionally answers it at once through the bridge (the
    /// sender is registered before the emit, so a synchronous reply works).
    #[derive(Default)]
    struct FakeEmitter {
        requests: Mutex<Vec<(u64, String, Value)>>,
        timeouts: Mutex<Vec<u64>>,
        fail: bool,
        respond: Option<(Arc<Bridge>, Respond)>,
    }

    impl FakeEmitter {
        fn answering(
            bridge: &Arc<Bridge>,
            f: impl Fn(&str, &Value) -> ReplyPayload + Send + Sync + 'static,
        ) -> Self {
            Self {
                respond: Some((Arc::clone(bridge), Box::new(f))),
                ..Self::default()
            }
        }

        fn failing() -> Self {
            Self {
                fail: true,
                ..Self::default()
            }
        }

        fn requests(&self) -> Vec<(u64, String, Value)> {
            lock(&self.requests).clone()
        }
    }

    impl RequestEmitter for FakeEmitter {
        fn emit_request(&self, request: &BridgeRequest) -> Result<(), String> {
            lock(&self.requests).push((
                request.id,
                request.method.to_string(),
                request.params.clone(),
            ));
            lock(&self.timeouts).push(request.timeout_ms);
            if self.fail {
                return Err("no window".into());
            }
            if let Some((bridge, f)) = &self.respond {
                bridge.reply(request.id, f(request.method, request.params));
            }
            Ok(())
        }
    }

    /// How [`FakeSnapshotter`] answers.
    enum SnapAnswer {
        /// `800x600` at the requested path.
        Succeed,
        /// `snapshot` returns `Err`.
        Refuse(&'static str),
        /// The receiver yields `Err`.
        Fail(&'static str),
        /// The sender is dropped.
        Drop,
        /// The sender is kept and never used.
        Never,
    }

    /// Records each requested path and answers as configured.
    struct FakeSnapshotter {
        answer: SnapAnswer,
        paths: Mutex<Vec<PathBuf>>,
        held: Mutex<Vec<oneshot::Sender<Result<ScreenshotResult, String>>>>,
    }

    impl Default for FakeSnapshotter {
        fn default() -> Self {
            Self::new(SnapAnswer::Succeed)
        }
    }

    impl FakeSnapshotter {
        fn new(answer: SnapAnswer) -> Self {
            Self {
                answer,
                paths: Mutex::new(Vec::new()),
                held: Mutex::new(Vec::new()),
            }
        }

        fn paths(&self) -> Vec<PathBuf> {
            lock(&self.paths).clone()
        }
    }

    impl Snapshotter for FakeSnapshotter {
        fn snapshot(&self, path: PathBuf) -> Result<SnapshotReceiver, String> {
            lock(&self.paths).push(path.clone());
            let (tx, rx) = oneshot::channel();
            match self.answer {
                SnapAnswer::Succeed => {
                    let _ = tx.send(Ok(ScreenshotResult {
                        path: path.to_string_lossy().into_owned(),
                        width: 800,
                        height: 600,
                    }));
                }
                SnapAnswer::Refuse(m) => return Err(m.to_string()),
                SnapAnswer::Fail(m) => {
                    let _ = tx.send(Err(m.to_string()));
                }
                SnapAnswer::Drop => drop(tx),
                SnapAnswer::Never => lock(&self.held).push(tx),
            }
            Ok(rx)
        }
    }

    fn attached_bridge() -> Arc<Bridge> {
        let b = Arc::new(Bridge::new());
        b.attach();
        b
    }

    fn line(id: u64, method: &str, params: Value) -> String {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
    }

    fn err_code(r: &Response) -> i64 {
        r.error.as_ref().expect("an error response").code
    }

    /// The method's wire-example refusals that the app answers itself.
    fn app_answered(method: Method) -> Vec<wire_fixtures::Invalid> {
        wire_fixtures::invalid(method)
            .into_iter()
            .filter(|e| e.answered_by == "app")
            .collect()
    }

    #[test]
    fn every_app_answered_refusal_has_a_test_here() {
        for method in Method::ALL {
            let app = app_answered(method);
            assert!(
                app.is_empty() || matches!(method, Method::Screenshot | Method::ProjectOpen),
                "{method} has app-answered refusals with no control.rs test: {app:?}"
            );
        }
    }

    async fn wait_for_requests(emitter: &FakeEmitter, n: usize) {
        while emitter.requests().len() < n {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn a_reply_resolves_the_request() {
        let bridge = attached_bridge();
        let emitter = FakeEmitter::answering(&bridge, |method, params| {
            ReplyPayload::Result(json!({"method": method, "params": params}))
        });
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            &line(42, "code.get", json!({"song": "main"})),
        )
        .await;
        assert_eq!(r.id, Some(42));
        assert_eq!(
            r.result,
            Some(json!({"method": "code.get", "params": {"song": "main"}}))
        );
        assert_eq!(r.error, None);
        assert_eq!(bridge.pending_len(), 0);
        let reqs = emitter.requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].0, 1, "internal ids start at 1");
    }

    #[tokio::test]
    async fn project_open_checks_native_absolute_paths_before_forwarding() {
        let bridge = attached_bridge();
        let emitter =
            FakeEmitter::answering(&bridge, |_, params| ReplyPayload::Result(params.clone()));
        let snapshotter = FakeSnapshotter::default();
        let refusals = app_answered(Method::ProjectOpen);
        let [refusal] = refusals.as_slice() else {
            panic!("expected one app-answered project.open refusal: {refusals:?}");
        };
        let response = handle_line(
            &bridge,
            &emitter,
            &snapshotter,
            &line(1, "project.open", refusal.params.clone()),
        )
        .await;
        assert_eq!(
            response.error,
            Some(RpcError::new(refusal.code, refusal.message.clone())),
            "{}",
            refusal.name
        );
        assert!(emitter.requests().is_empty());
        let path = std::env::temp_dir().join("Music project");
        let response = handle_line(
            &bridge,
            &emitter,
            &snapshotter,
            &line(2, "project.open", json!({"path": path})),
        )
        .await;
        assert_eq!(response.result.unwrap()["path"], json!(path));
    }

    #[tokio::test]
    async fn screenshot_is_answered_without_the_webview() {
        let emitter = FakeEmitter::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        for bridge in [Arc::new(Bridge::new()), attached_bridge()] {
            let snap = FakeSnapshotter::default();
            let r = handle_line(
                &bridge,
                &emitter,
                &snap,
                &line(1, "screenshot", json!({"path": path})),
            )
            .await;
            assert_eq!(r.id, Some(1));
            assert_eq!(r.error, None);
            assert_eq!(
                r.result,
                Some(json!({"path": path, "width": 800, "height": 600}))
            );
            assert_eq!(snap.paths(), vec![path.clone()]);
        }
        assert!(emitter.requests().is_empty(), "nothing is emitted");
    }

    fn is_default_screenshot_path(p: &Path) -> bool {
        let name = p.file_name().unwrap().to_str().unwrap();
        p.parent() == Some(std::env::temp_dir().as_path())
            && name
                .strip_prefix("strudeland-screenshot-")
                .and_then(|n| n.strip_suffix(".png"))
                .is_some_and(|ms| {
                    !ms.is_empty() && ms.bytes().all(|b| b.is_ascii_digit() || b == b'-')
                })
    }

    #[tokio::test]
    async fn screenshot_without_a_path_writes_to_the_temp_dir() {
        let bridge = Bridge::new();
        let emitter = FakeEmitter::default();
        let snap = FakeSnapshotter::default();
        let with_params = line(1, "screenshot", json!({}));
        let null_params = line(2, "screenshot", Value::Null);
        let no_params = r#"{"jsonrpc":"2.0","id":3,"method":"screenshot"}"#.to_string();
        for l in [with_params, null_params, no_params] {
            let r = handle_line(&bridge, &emitter, &snap, &l).await;
            assert_eq!(r.error, None, "{l}");
        }
        let paths = snap.paths();
        assert_eq!(paths.len(), 3);
        for p in &paths {
            assert!(is_default_screenshot_path(p), "{}", p.display());
        }
        assert!(!is_default_screenshot_path(Path::new(
            "/tmp/strudeland-screenshot-.png"
        )));
    }

    #[tokio::test]
    async fn screenshot_rejects_invalid_params() {
        let bridge = Bridge::new();
        let emitter = FakeEmitter::default();
        let snap = FakeSnapshotter::default();
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().display().to_string();
        let missing = dir.path().join("missing");
        let root = dir.path().ancestors().last().unwrap().display().to_string();
        let examples = wire_fixtures::invalid(Method::Screenshot);
        assert!(
            !examples.is_empty(),
            "the screenshot example lists refusals"
        );
        for e in &examples {
            assert_eq!(e.answered_by, "app", "{}", e.name);
        }
        let from_examples = examples
            .into_iter()
            .map(|e| (e.params, RpcError::new(e.code, e.message)));
        let invalid = |m: String| RpcError::new(codes::INVALID_PARAMS, m);
        let inline = [
            (
                json!({"path": missing.join("shot.png")}),
                format!(
                    "invalid params: the directory {} does not exist",
                    missing.display()
                ),
            ),
            (
                json!({"path": d}),
                format!("invalid params: {d} is a directory"),
            ),
            (
                json!({"path": root}),
                format!("invalid params: {root} is not a file path"),
            ),
        ];
        fs::write(dir.path().join("file"), "").unwrap();
        let file = dir.path().join("file");
        let under_file = file.join("shot.png");
        let inline = inline.into_iter().chain([(
            json!({"path": under_file}),
            format!(
                "invalid params: the directory {} does not exist",
                file.display()
            ),
        )]);
        let cases = from_examples.chain(inline.map(|(params, m)| (params, invalid(m))));
        for (params, expected) in cases {
            let r = handle_line(
                &bridge,
                &emitter,
                &snap,
                &line(1, "screenshot", params.clone()),
            )
            .await;
            assert_eq!(r.error, Some(expected), "{params}");
        }
        assert!(snap.paths().is_empty(), "the snapshotter is not called");
    }

    #[tokio::test]
    async fn screenshot_failures_are_screenshot_failed() {
        let bridge = Bridge::new();
        let emitter = FakeEmitter::default();
        for (answer, message) in [
            (SnapAnswer::Refuse("no main window"), "no main window"),
            (
                SnapAnswer::Fail("the snapshot was empty"),
                "the snapshot was empty",
            ),
            (SnapAnswer::Drop, "the snapshot was cancelled"),
        ] {
            let snap = FakeSnapshotter::new(answer);
            let r = handle_line(&bridge, &emitter, &snap, &line(4, "screenshot", json!({}))).await;
            assert_eq!(r.id, Some(4));
            assert_eq!(
                r.error,
                Some(RpcError::new(codes::SCREENSHOT_FAILED, message))
            );
            assert_eq!(snap.paths().len(), 1);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_screenshot_that_never_answers_times_out() {
        let bridge = Bridge::new();
        let emitter = FakeEmitter::default();
        let snap = FakeSnapshotter::new(SnapAnswer::Never);
        let start = tokio::time::Instant::now();
        let r = handle_line(
            &bridge,
            &emitter,
            &snap,
            &json!({"jsonrpc":"2.0","id":5,"method":"screenshot","timeout_ms":2500}).to_string(),
        )
        .await;
        assert_eq!(start.elapsed(), Duration::from_millis(2500));
        assert_eq!(
            r.error,
            Some(RpcError::new(
                codes::TIMEOUT,
                "the app did not answer in 3s"
            ))
        );
        let start = tokio::time::Instant::now();
        let r = handle_line(&bridge, &emitter, &snap, &line(6, "screenshot", json!({}))).await;
        assert_eq!(
            start.elapsed(),
            Duration::from_secs(10),
            "the default timeout"
        );
        assert_eq!(err_code(&r), codes::TIMEOUT);
    }

    #[tokio::test]
    async fn other_methods_never_reach_the_snapshotter() {
        let bridge = attached_bridge();
        let emitter = FakeEmitter::answering(&bridge, |m, _| ReplyPayload::Result(json!(m)));
        let snap = FakeSnapshotter::new(SnapAnswer::Refuse("unused"));
        for method in ["ping", "state", "events", "eval"] {
            let r = handle_line(&bridge, &emitter, &snap, &line(1, method, json!({}))).await;
            assert_eq!(r.error, None, "{method}");
        }
        assert!(snap.paths().is_empty());
        assert_eq!(emitter.requests().len(), 3);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_connection_serves_a_screenshot() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let (info, listener) = listen(Ok(path.clone()));
        assert_eq!(info.status, SocketStatus::Listening);
        let (listener, lock) = listener.unwrap();
        let snap = Arc::new(FakeSnapshotter::default());
        let served: Arc<dyn Snapshotter> = Arc::clone(&snap) as Arc<dyn Snapshotter>;
        tokio::spawn(async move {
            let _lock = lock;
            serve(
                listener,
                Arc::new(Bridge::new()),
                Arc::new(FakeEmitter::default()),
                served,
            )
            .await
        });
        let mut stream = BufReader::new(UnixStream::connect(&path).await.unwrap());
        let shot = dir.path().join("s.png");
        stream
            .get_mut()
            .write_all(format!("{}\n", line(1, "screenshot", json!({"path": shot}))).as_bytes())
            .await
            .unwrap();
        let r = read_response(&mut stream).await.unwrap();
        assert_eq!(r["result"]["path"], json!(shot));
        assert_eq!(snap.paths(), vec![shot]);
    }

    #[tokio::test]
    async fn events_without_a_webview_is_not_ready() {
        let bridge = Arc::new(Bridge::new());
        let emitter = FakeEmitter::default();
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            &line(1, "events", json!({})),
        )
        .await;
        assert_eq!(err_code(&r), codes::NOT_READY);
        assert!(emitter.requests().is_empty());
    }

    #[tokio::test]
    async fn events_is_forwarded_to_the_webview() {
        let bridge = attached_bridge();
        let emitter = FakeEmitter::answering(&bridge, |m, p| {
            ReplyPayload::Result(json!({"method": m, "params": p}))
        });
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            &line(1, "events", json!({"section": "verse"})),
        )
        .await;
        assert_eq!(
            r.result,
            Some(json!({"method": "events", "params": {"section": "verse"}}))
        );
    }

    #[tokio::test]
    async fn internal_ids_are_not_the_client_ids() {
        let bridge = attached_bridge();
        let emitter = FakeEmitter::answering(&bridge, |_, _| ReplyPayload::Result(json!(null)));
        handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            &line(7, "state", json!({})),
        )
        .await;
        handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            &line(7, "state", json!({})),
        )
        .await;
        let ids: Vec<u64> = emitter.requests().iter().map(|r| r.0).collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[tokio::test]
    async fn result_and_error_pass_through_untouched() {
        let bridge = attached_bridge();
        let result = json!({"ok": false, "song": "songs/main.js", "error": "sound nope not found!",
                            "nested": [1, {"a": null}]});
        let r2 = result.clone();
        let emitter = FakeEmitter::answering(&bridge, move |_, _| ReplyPayload::Result(r2.clone()));
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            &line(5, "eval", json!({})),
        )
        .await;
        assert_eq!(r.result, Some(result));

        let emitter = FakeEmitter::answering(&bridge, |_, _| {
            ReplyPayload::Error(RpcError::new(
                codes::SONG_CHANGED,
                "the song changed since you read it",
            ))
        });
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            &line(8, "code.set", json!({"code": "x"})),
        )
        .await;
        assert_eq!(r.id, Some(8));
        assert_eq!(r.result, None);
        assert_eq!(
            r.error,
            Some(RpcError::new(
                codes::SONG_CHANGED,
                "the song changed since you read it"
            ))
        );
    }

    #[tokio::test]
    async fn a_null_result_stays_a_result() {
        let bridge = attached_bridge();
        let emitter = FakeEmitter::answering(&bridge, |_, _| ReplyPayload::Result(Value::Null));
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            &line(1, "stop", json!({})),
        )
        .await;
        assert_eq!(
            encode_response(&r),
            r#"{"jsonrpc":"2.0","id":1,"result":null}"#
        );
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_removes_the_pending_entry_and_a_late_reply_is_ignored() {
        let bridge = attached_bridge();
        let emitter = FakeEmitter::default();
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            &json!({"jsonrpc":"2.0","id":3,"method":"eval","timeout_ms":1500}).to_string(),
        )
        .await;
        assert_eq!(r.id, Some(3));
        assert_eq!(
            r.error,
            Some(RpcError::new(
                codes::TIMEOUT,
                "the app did not answer in 2s"
            ))
        );
        assert_eq!(bridge.pending_len(), 0);
        let internal = emitter.requests()[0].0;
        bridge.reply(internal, ReplyPayload::Result(json!("late")));
        assert_eq!(bridge.pending_len(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_ms_is_clamped_and_defaults_to_ten_seconds() {
        let bridge = attached_bridge();
        let emitter = FakeEmitter::default();
        for (timeout, secs) in [
            (Some(json!(0)), 1),
            (Some(json!(999)), 1),
            (None, 10),
            (Some(json!(Value::Null)), 10),
            (Some(json!(30_000)), 30),
            (Some(json!(500_000)), 120),
        ] {
            let mut req = json!({"jsonrpc":"2.0","id":1,"method":"state","params":{"x":1}});
            if let Some(t) = timeout {
                req["timeout_ms"] = t;
            }
            let start = tokio::time::Instant::now();
            let r = handle_line(
                &bridge,
                &emitter,
                &FakeSnapshotter::default(),
                &req.to_string(),
            )
            .await;
            assert_eq!(start.elapsed(), Duration::from_secs(secs));
            assert_eq!(
                r.error.unwrap().message,
                format!("the app did not answer in {secs}s")
            );
        }
        for (_, _, params) in emitter.requests() {
            assert_eq!(params, json!({"x": 1}), "timeout_ms is not in params");
        }
        assert_eq!(
            *lock(&emitter.timeouts),
            [1_000, 1_000, 10_000, 10_000, 30_000, 120_000],
            "the effective timeout is forwarded"
        );
    }

    #[test]
    fn clamp_timeout_bounds() {
        assert_eq!(clamp_timeout(None), Duration::from_secs(10));
        assert_eq!(clamp_timeout(Some(0)), Duration::from_secs(1));
        assert_eq!(clamp_timeout(Some(1_000)), Duration::from_secs(1));
        assert_eq!(clamp_timeout(Some(120_000)), Duration::from_secs(120));
        assert_eq!(clamp_timeout(Some(u64::MAX)), Duration::from_secs(120));
    }

    #[tokio::test]
    async fn not_attached_is_not_ready_without_an_emit() {
        let bridge = Bridge::new();
        let emitter = FakeEmitter::default();
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            &line(2, "state", json!({})),
        )
        .await;
        assert_eq!(r.id, Some(2));
        assert_eq!(
            r.error,
            Some(RpcError::new(codes::NOT_READY, "app not ready"))
        );
        assert!(emitter.requests().is_empty());
    }

    #[tokio::test]
    async fn attach_fails_pending_requests() {
        let bridge = attached_bridge();
        let emitter = Arc::new(FakeEmitter::default());
        let task = {
            let (b, e) = (Arc::clone(&bridge), Arc::clone(&emitter));
            tokio::spawn(async move {
                handle_line(
                    &b,
                    e.as_ref(),
                    &FakeSnapshotter::default(),
                    &line(4, "play", json!({})),
                )
                .await
            })
        };
        wait_for_requests(&emitter, 1).await;
        assert_eq!(bridge.pending_len(), 1);
        let info = bridge.attach();
        assert_eq!(info, bridge.info());
        let r = task.await.unwrap();
        assert_eq!(r.id, Some(4));
        assert_eq!(
            r.error,
            Some(RpcError::new(codes::NOT_READY, "the page reloaded"))
        );
        assert_eq!(bridge.pending_len(), 0);
        assert!(bridge.attached());
    }

    #[tokio::test]
    async fn detach_fails_pending_requests_and_later_ones_are_not_ready() {
        let bridge = attached_bridge();
        let emitter = Arc::new(FakeEmitter::default());
        let task = {
            let (b, e) = (Arc::clone(&bridge), Arc::clone(&emitter));
            tokio::spawn(async move {
                handle_line(
                    &b,
                    e.as_ref(),
                    &FakeSnapshotter::default(),
                    &line(4, "play", json!({})),
                )
                .await
            })
        };
        wait_for_requests(&emitter, 1).await;
        bridge.detach();
        let r = task.await.unwrap();
        assert_eq!(
            r.error,
            Some(RpcError::new(codes::NOT_READY, "the page reloaded"))
        );
        assert_eq!(bridge.pending_len(), 0);
        assert!(!bridge.attached());
        let r = handle_line(
            &bridge,
            emitter.as_ref(),
            &FakeSnapshotter::default(),
            &line(5, "state", json!({})),
        )
        .await;
        assert_eq!(
            r.error,
            Some(RpcError::new(codes::NOT_READY, "app not ready"))
        );
        assert_eq!(emitter.requests().len(), 1, "no emit while detached");
        bridge.attach();
        assert!(bridge.attached());
    }

    #[tokio::test]
    async fn emitter_error_is_not_ready_and_leaves_nothing_pending() {
        let bridge = attached_bridge();
        let emitter = FakeEmitter::failing();
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            &line(6, "state", json!({})),
        )
        .await;
        assert_eq!(
            r.error,
            Some(RpcError::new(codes::NOT_READY, "app not ready"))
        );
        assert_eq!(emitter.requests().len(), 1);
        assert_eq!(bridge.pending_len(), 0);
    }

    #[tokio::test]
    async fn unknown_method_is_method_not_found_without_an_emit() {
        let bridge = attached_bridge();
        let emitter = FakeEmitter::default();
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            r#"{"jsonrpc":"2.0","id":9,"method":"nope"}"#,
        )
        .await;
        assert_eq!(
            encode_response(&r),
            r#"{"jsonrpc":"2.0","id":9,"error":{"code":-32601,"message":"unknown method 'nope'"}}"#
        );
        assert!(emitter.requests().is_empty());
    }

    #[tokio::test]
    async fn ping_is_answered_in_rust_attached_or_not() {
        let emitter = FakeEmitter::failing();
        let version = env!("CARGO_PKG_VERSION");
        let bridge = Bridge::new();
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            &line(1, "ping", json!({})),
        )
        .await;
        assert_eq!(
            r.result,
            Some(json!({"version": version, "attached": false}))
        );
        bridge.attach();
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
        )
        .await;
        assert_eq!(
            r.result,
            Some(json!({"version": version, "attached": true}))
        );
        assert!(emitter.requests().is_empty());
    }

    #[tokio::test]
    async fn parse_and_invalid_request_errors() {
        let bridge = attached_bridge();
        let emitter = FakeEmitter::default();
        let r = handle_line(&bridge, &emitter, &FakeSnapshotter::default(), "not json").await;
        assert_eq!(
            encode_response(&r),
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error: expected ident at line 1 column 2"}}"#
        );
        assert!(closes_connection(&r));
        for bad in [
            r#"{"jsonrpc":"2.0","id":"1","method":"state"}"#,
            r#"{"jsonrpc":"2.0","id":null,"method":"state"}"#,
            r#"{"jsonrpc":"2.0","id":1.5,"method":"state"}"#,
            r#"{"jsonrpc":"2.0","method":"state"}"#,
        ] {
            let r = handle_line(&bridge, &emitter, &FakeSnapshotter::default(), bad).await;
            assert_eq!(
                (r.id, err_code(&r)),
                (None, codes::INVALID_REQUEST),
                "{bad}"
            );
            assert!(!closes_connection(&r));
        }
        let r = handle_line(
            &bridge,
            &emitter,
            &FakeSnapshotter::default(),
            r#"{"jsonrpc":"1.0","id":3,"method":"state"}"#,
        )
        .await;
        assert_eq!((r.id, err_code(&r)), (Some(3), codes::INVALID_REQUEST));
        assert!(emitter.requests().is_empty());
    }

    #[test]
    fn reply_payload_deserializes_both_shapes() {
        let p: ReplyPayload = serde_json::from_str(r#"{"result":{"wasPlaying":true}}"#).unwrap();
        assert_eq!(p, ReplyPayload::Result(json!({"wasPlaying": true})));
        let p: ReplyPayload =
            serde_json::from_str(r#"{"error":{"code":-32005,"message":"no song 'x'"}}"#).unwrap();
        assert_eq!(
            p,
            ReplyPayload::Error(RpcError::new(codes::NO_SUCH_SONG, "no song 'x'"))
        );
        let p: ReplyPayload = serde_json::from_str(r#"{"result":null}"#).unwrap();
        assert_eq!(p, ReplyPayload::Result(Value::Null));
        assert!(serde_json::from_str::<ReplyPayload>(r#"{"nope":1}"#).is_err());
    }

    #[test]
    fn bridge_request_and_info_serialize_for_the_webview() {
        let params = json!({"song": "main"});
        let req = BridgeRequest {
            id: 3,
            method: "code.get",
            params: &params,
            timeout_ms: 10_000,
        };
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            r#"{"id":3,"method":"code.get","params":{"song":"main"},"timeoutMs":10000}"#
        );
        let info = ControlInfo::new(SocketStatus::InUse, Some(Path::new("/s")), Some("x".into()));
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            json!({"status": "in-use", "path": "/s", "detail": "x"})
        );
        let info = Bridge::new().info();
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            json!({"status": "disabled", "path": null, "detail": null})
        );
        assert_eq!(
            serde_json::to_value(SocketStatus::Listening).unwrap(),
            json!("listening")
        );
    }

    #[test]
    fn unknown_reply_id_is_ignored() {
        let bridge = Bridge::new();
        bridge.reply(99, ReplyPayload::Result(json!(1)));
        assert_eq!(bridge.pending_len(), 0);
    }

    // --- socket ---

    #[cfg(unix)]
    fn short_tempdir() -> tempfile::TempDir {
        // macOS limits socket paths to 103 bytes; $TMPDIR is long, /tmp is not.
        tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .prefix("sl")
            .tempdir_in("/tmp")
            .unwrap()
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().mode() & 0o777
    }

    #[test]
    #[cfg(unix)]
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
    #[cfg(unix)]
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
    #[cfg(unix)]
    fn keeps_the_mode_of_an_existing_parent_dir() {
        let dir = short_tempdir();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let BindOutcome::Listening(_l, _lock) = bind_socket(&dir.path().join("c.sock")) else {
            panic!("expected Listening");
        };
        assert_eq!(mode(dir.path()), 0o755);
    }

    #[test]
    #[cfg(unix)]
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
    #[cfg(unix)]
    fn a_live_socket_is_in_use_and_left_alone() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let other = StdUnixListener::bind(&path).unwrap();
        assert!(matches!(bind_socket(&path), BindOutcome::InUse));
        StdUnixStream::connect(&path).unwrap();
        other.accept().unwrap();
    }

    #[test]
    #[cfg(unix)]
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
    #[cfg(unix)]
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
    #[cfg(unix)]
    fn an_uncreatable_parent_is_disabled() {
        let dir = short_tempdir();
        let ro = dir.path().join("ro");
        fs::create_dir(&ro).unwrap();
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o500)).unwrap();
        let outcome = bind_socket(&ro.join("a/c.sock"));
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o700)).unwrap();
        let BindOutcome::Disabled(detail) = outcome else {
            panic!("expected Disabled");
        };
        assert!(detail.starts_with("cannot create"), "{detail}");

        // A path under a regular file fails at the existence check instead.
        let file = dir.path().join("f");
        fs::write(&file, "").unwrap();
        assert!(matches!(
            bind_socket(&file.join("c.sock")),
            BindOutcome::Disabled(_)
        ));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn listen_reports_each_status() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let (info, listener) = listen(Ok(path.clone()));
        assert_eq!(info.status, SocketStatus::Listening);
        assert_eq!(info.path.as_deref(), Some(path.to_str().unwrap()));
        assert!(listener.is_some());

        // The first listener's lock is still held (same process, separate open file).
        let (info, listener) = listen(Ok(path.clone()));
        assert_eq!(info.status, SocketStatus::InUse);
        assert!(info.detail.is_some());
        assert!(listener.is_none());

        let (info, listener) = listen(Err(PathError::NoHome));
        assert_eq!(info.status, SocketStatus::Disabled);
        assert_eq!(info.path, None);
        assert_eq!(info.detail, Some(PathError::NoHome.to_string()));
        assert!(listener.is_none());
    }

    #[test]
    #[cfg(unix)]
    fn remove_socket_only_when_listening() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let _l = StdUnixListener::bind(&path).unwrap();
        for status in [SocketStatus::InUse, SocketStatus::Disabled] {
            assert!(!remove_socket(&ControlInfo::new(status, Some(&path), None)));
            assert!(path.exists());
        }
        assert!(!remove_socket(&ControlInfo::new(
            SocketStatus::Listening,
            None,
            None
        )));
        assert!(remove_socket(&ControlInfo::new(
            SocketStatus::Listening,
            Some(&path),
            None
        )));
        assert!(!path.exists());
    }

    #[cfg(unix)]
    async fn serve_on(path: &Path, bridge: Arc<Bridge>, emitter: Arc<dyn RequestEmitter>) {
        let (info, listener) = listen(Ok(path.to_path_buf()));
        assert_eq!(info.status, SocketStatus::Listening);
        let (listener, lock) = listener.unwrap();
        tokio::spawn(async move {
            let _lock = lock;
            serve(
                listener,
                bridge,
                emitter,
                Arc::new(FakeSnapshotter::default()),
            )
            .await
        });
    }

    #[cfg(unix)]
    async fn read_response<R: AsyncBufRead + Unpin>(r: &mut R) -> Option<Value> {
        let line = read_line_capped(r, 1 << 20).await.unwrap()?;
        Some(serde_json::from_str(&line).unwrap())
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_connection_serves_several_requests_in_order() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        let bridge = attached_bridge();
        let emitter = Arc::new(FakeEmitter::answering(&bridge, |method, _| {
            ReplyPayload::Result(json!(method))
        }));
        serve_on(&path, Arc::clone(&bridge), emitter).await;

        let (read, mut write) = UnixStream::connect(&path).await.unwrap().into_split();
        let mut reader = BufReader::new(read);
        let batch = [
            line(1, "state", json!({})),
            line(2, "ping", json!({})),
            r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#.to_string(),
            line(4, "stop", json!({})),
        ]
        .join("\n")
            + "\n";
        write.write_all(batch.as_bytes()).await.unwrap();
        let mut got = Vec::new();
        for _ in 0..4 {
            got.push(read_response(&mut reader).await.unwrap());
        }
        assert_eq!(got[0], json!({"jsonrpc":"2.0","id":1,"result":"state"}));
        assert_eq!(got[1]["id"], 2);
        assert_eq!(got[1]["result"]["attached"], true);
        assert_eq!(got[2]["id"], 3);
        assert_eq!(got[2]["error"]["code"], codes::METHOD_NOT_FOUND);
        assert_eq!(got[3], json!({"jsonrpc":"2.0","id":4,"result":"stop"}));

        // A second connection is served alongside.
        let mut other = UnixStream::connect(&path).await.unwrap();
        other
            .write_all(format!("{}\n", line(5, "ping", json!({}))).as_bytes())
            .await
            .unwrap();
        let mut other = BufReader::new(other);
        assert_eq!(read_response(&mut other).await.unwrap()["id"], 5);

        // The first connection is still open.
        write
            .write_all(format!("{}\n", line(6, "ping", json!({}))).as_bytes())
            .await
            .unwrap();
        assert_eq!(read_response(&mut reader).await.unwrap()["id"], 6);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_parse_error_closes_the_connection() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        serve_on(&path, attached_bridge(), Arc::new(FakeEmitter::default())).await;
        let (read, mut write) = UnixStream::connect(&path).await.unwrap().into_split();
        let mut reader = BufReader::new(read);
        write
            .write_all(format!("not json\n{}\n", line(1, "ping", json!({}))).as_bytes())
            .await
            .unwrap();
        let r = read_response(&mut reader).await.unwrap();
        assert_eq!(r["id"], Value::Null);
        assert_eq!(r["error"]["code"], codes::PARSE_ERROR);
        assert_eq!(read_response(&mut reader).await, None);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn an_oversize_line_closes_the_connection() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        serve_on(&path, attached_bridge(), Arc::new(FakeEmitter::default())).await;
        let (read, mut write) = UnixStream::connect(&path).await.unwrap().into_split();
        let mut reader = BufReader::new(read);
        tokio::spawn(async move {
            let mut big = vec![b'x'; MAX_LINE_BYTES];
            big.push(b'\n');
            // The server may close before reading it all.
            let _ = write.write_all(&big).await;
            let _ = write.write_all(b"{}\n").await;
        });
        let r = read_response(&mut reader).await.unwrap();
        assert_eq!(
            r,
            json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"request too large"}})
        );
        assert_eq!(read_response(&mut reader).await, None);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_line_at_the_cap_is_accepted() {
        let dir = short_tempdir();
        let path = dir.path().join("c.sock");
        serve_on(&path, attached_bridge(), Arc::new(FakeEmitter::default())).await;
        let (read, mut write) = UnixStream::connect(&path).await.unwrap().into_split();
        let mut reader = BufReader::new(read);
        let head = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{"pad":""#;
        let tail = r#""}}"#;
        let pad = MAX_LINE_BYTES - 1 - head.len() - tail.len();
        let req = format!("{head}{}{tail}\n", "p".repeat(pad));
        assert_eq!(req.len(), MAX_LINE_BYTES);
        tokio::spawn(async move { write.write_all(req.as_bytes()).await.unwrap() });
        let r = read_response(&mut reader).await.unwrap();
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["attached"], true);
    }

    #[tokio::test]
    async fn a_non_utf8_line_is_a_parse_error() {
        let mut r = BufReader::new(&b"\xff\xfe\n"[..]);
        assert!(matches!(
            read_line_capped(&mut r, 100).await,
            Err(FrameError::NotUtf8)
        ));
    }

    #[test]
    fn identifier_matches_tauri_conf() {
        let conf: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], strudeland_ctl::APP_IDENTIFIER);
    }
}
