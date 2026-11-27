# 0131 ctl server endpoint: step 0 re-verification

At 9dbe6c5 (origin/main). The claim was reviewed against 63b945d (v0.1.2). The endpoint code
has not moved since then. `git diff 63b945d HEAD -- src-tauri/src/control.rs` touches only the
test module (`@@ -800..1132`, the wire-fixture tests). `windows_pipe.rs` and `path.rs` are
unchanged. `client.rs` gained only `decode_result` (#35). Every cited line number still holds.

## 1. What lives where

### App crate (`src-tauri/src/control.rs`)

| Item | Lines | cfg | Notes |
|---|---|---|---|
| `SocketStatus` / `ControlInfo` (serialized to the webview, kebab/camel case) | 54-79 | all | app type, stays |
| `Bridge.socket_lock: Mutex<Option<File>>` / `Bridge.server: JoinHandle` | 135-139, 160-163 | unix / windows | transport state inside Bridge |
| `serve_connection` (framing, `handle_line`) | 417-443 | all | stays in the app |
| `serve` accept loop (unix) | 446-473 | unix | `check_peer` **only on Linux** (`:456`), so macOS does no peer check. A rejected peer is dropped silently (`continue`, no log). Accept error: `eprintln!("control: accept failed: {e}")` plus a 100 ms sleep |
| `BindOutcome { Listening(StdUnixListener, File), InUse, Disabled(String) }` | 475-483 | unix | `pub`, but only used in this file |
| `lock_path` (`{path}.lock`) | 485-491 | unix | |
| `lock_socket` (open 0600, `try_lock`, never removed) | 493-511 | unix | |
| `bind_socket` | 513-571 | unix | mkdir -p 0700 for a missing parent, take the lock, check the existing path (not a socket: Disabled; a live probe: InUse; refused or not found: stale, so remove it), bind, chmod 0600, set nonblocking |
| `listen` (unix) | 573-620 | unix + linux | `PathError` gives Disabled with path None. Linux runs `ensure_private_parent` first. Builds the tokio listener with `from_std` (needs the runtime entered) and composes the InUse detail text |
| `ensure_private_parent` | 622-641 | linux | creates only the last component 0700, then `ctl::path::check_private_dir` |
| `listen` (windows) | 643-670 | windows | `windows_pipe::listen(path, true)`. On error, `owned_existing` decides InUse or Disabled |
| `serve` (windows) | 672-711 | windows | JoinSet of clients. After each connect it creates the next instance with `listen(path, false)` |
| `start` | 713-749 | all | enters the tauri runtime, calls `listen(socket_path())`, logs, `set_info`, stores the lock (unix) or the JoinHandle (windows). The unix server task is detached (`drop(server)`) |
| `remove_socket` | 751-758 | unix | removes the socket only when status is Listening and the path is Some |
| `shutdown` | 760-772 | all | unix: remove the socket, then drop the lock. windows: abort the server task |
| module doc names `bind_socket` | 7-9 | | needs an edit after a move |

The Unix endpoint covers about 446-641 plus 751-772 (about 215 lines). With the Windows half,
443-772 is about 330 lines.

### ctl crate (`crates/strudeland-ctl`)

| Item | Location | Notes |
|---|---|---|
| `windows_pipe::listen(path, first) -> io::Result<NamedPipeServer>` | `windows_pipe.rs:120-153` | SDDL owner and DACL limited to the user SID, `reject_remote_clients`, `first_pipe_instance(first)`. No accept-time client check: the DACL is the verification |
| `windows_pipe::owned_existing(path) -> bool` | `:294-309` | in-use probe: open, then `check_owner`, retried 4 times on PIPE_BUSY |
| `check_owner`, `client_options`, `user_sid`, `validate_path`, `default_path` | `:42-189` | client-side owner check (`:162`) |
| `Pipe` (blocking client over tokio) | `:191-290` | `pub(crate)` |
| `path::check_private_dir` | `path.rs:91-108` | linux, `pub`. Used by `default_endpoint` (`:87`), the client (`client.rs:76`) and the app (`control.rs:640`) |
| `path::check_peer` (SO_PEERCRED uid == euid) | `path.rs:110-139` | linux, `pub`. Used by the client (`client.rs:97`) and the app (`control.rs:457`) |
| `SOCKET_FILE = "ctl.sock"`, length limits, `PathError` | `path.rs:14-33` | |
| `Client::connect` | `client.rs:73-104` | linux: private-dir check (NotFound gives NotRunning), then connect, then `check_peer` |
| deps | `Cargo.toml` | tokio is pulled in **only on windows** (`net, io-util, time, rt`), libc only on linux |

### Duplication between app and ctl

- No helper is duplicated outright. The split is by role: the app calls ctl's `check_peer` and
  `check_private_dir`, but owns the lock, the stale probe and the modes.
- Linux creates the parent twice: `ensure_private_parent` (single level, 0700, then checked) runs
  before `bind_socket`'s recursive 0700 mkdir, which is then a no-op.
- The Windows server is split the same way: the pipe primitives are in ctl, while the app owns the
  first or next instance loop (`control.rs:692`) and the InUse decision (`:661`).
- The Unix and Windows `listen` functions in the app both compose "cannot listen on {path}: …"
  separately.

## 2. Callers, threading, shutdown, errors

- Callers: `lib.rs:24` (`Bridge::new`), `:41` (`detach` on page load), `:60` (`control::start`
  in `setup`), `:108-109` (commands), `:123` (`control::shutdown` on `RunEvent::Exit`, after
  playback and the recorder). `snapshot.rs:11` imports only the trait types. Nothing else in
  `src-tauri` calls the endpoint.
- Other crates: `crates/strudeland-cli/tests/e2e.rs:645` calls `windows_pipe::listen(&socket, true)`
  directly for a fake Windows server. The unix e2e fakes use a raw `UnixListener::bind`
  (`e2e.rs:21-22`), and so do the ctl client tests (`client.rs` tests `:251`, `:397`, `:569`).
- Threading: no dedicated threads. Everything runs as tokio tasks on the tauri async runtime. One
  accept task, plus one `tokio::spawn` per connection (unix) or a `JoinSet` (windows). There is no
  idle timeout and no connection limit. Binding happens synchronously inside `start`, with the
  runtime entered, so the status is set before the webview can attach.
- Shutdown: unix removes the socket file (only if Listening), then releases the lock. The lock
  file is never removed. The unix accept task is never aborted; it dies with the runtime. Windows
  aborts the server JoinHandle. Nothing cleans up on a crash: the next start treats the socket as
  stale, because the lock holder is gone and the probe is refused.
- Error surfacing: stderr (`eprintln!`), the `ControlInfo` returned by `control_attach`, and the
  status bar notice (`StatusBar.svelte:89-93`). `start` never fails the app.

## 3. Tests

| Where | What | Platforms |
|---|---|---|
| `control.rs:1687-1811` (sync, unix) | fresh bind 0600; missing parent 0700 at each level; existing parent mode kept; stale removed and bound; live socket gives InUse and is left alone; held lock gives InUse, stale socket kept (same inode), lock name `c.sock.lock` 0600, lock never removed; regular file Disabled with exact text; uncreatable parent "cannot create" prefix | macOS, Ubuntu |
| `control.rs:1813-1834` | `listen` statuses: Listening path, second listen InUse with `detail.is_some()`, `PathError::NoHome` Disabled with path None | macOS, Ubuntu |
| `control.rs:1836-1857` | `remove_socket` only when Listening | macOS, Ubuntu |
| `control.rs:1208, 1859-1990` | real-socket `serve` tests (screenshot, in-order, parse error closes, oversize, at cap) through `listen` + `serve` | unix only |
| Windows server in the app | **none** (no `cfg(windows)` test of `listen`/`serve`) | |
| Linux `ensure_private_parent` and `check_peer` on accept | **none** in the app. `path.rs:252-275` covers `check_private_dir` only | |
| `windows_pipe.rs:311-398` | local endpoint validation; stalled peer timeout; authenticated round trip with a real `Client` and a second `listen(first)` refused | Windows |
| `client.rs` tests (`#[cfg(all(test, unix))]`) | client against hand-rolled `UnixListener` fakes, including a stale socket giving NotRunning | macOS, Ubuntu |
| `tests/wire.rs` | wire examples round trip | all |
| `e2e.rs` | CLI against fake servers; Windows uses `windows_pipe::listen` | all |

CI: `macos.yml` (macos-15) and `native.yml` (windows-2025, ubuntu-24.04) all run
`cargo test --workspace`, so ctl tests would run on all three.

What a move enables in ctl, without Tauri:

- A real `Client` ↔ server round trip on unix (today only a fake listener exists).
- In-use by lock and by live probe, stale removal, and a non-socket refused.
- Linux: the private-parent refusal (a 0755 parent) and the peer check (the Ubuntu runner can only
  test the accept path for the same uid; a foreign uid needs root, so cover the rejection with a
  `check_peer` unit test or skip it).
- Windows: the first-instance in-use decision (`owned_existing`) and the next-instance accept loop,
  which have no tests anywhere today.

## 4. Visible behaviour that must stay identical

| Text or behaviour | Built at | Pinned by |
|---|---|---|
| `control: listening on {path}` | `control.rs:722` | nothing |
| `control: disabled: {detail}` / `control: disabled` | `:726-727` | nothing |
| `control: accept failed: {e}` | `:468` (unix), `:683` (windows) | nothing |
| `control: cannot create next pipe instance: {e}` | `:695` | nothing |
| unix InUse detail `another strudeland is listening on {path}` | `:609` | nothing (only `detail.is_some()`, `:1826`) |
| windows InUse/Disabled detail `cannot listen on {path}: {e}` | `:666` | nothing |
| `cannot create {parent}: {e}` | `:527` | prefix (`:1802`) |
| `cannot open {lock}: {e}` / `cannot lock {lock}: {e}` | `:505`, `:509` | nothing |
| `{path} exists and is not a socket` | `:537` | exact (`:1783-1786`) |
| `cannot remove stale {path}: {e}` | `:550` | nothing |
| `{path}: {e}` (probe or metadata error) | `:553`, `:556` | nothing |
| `cannot listen on {path}: {e}` (bind, chmod, nonblock, from_std) | `:561`, `:568`, `:601` | nothing |
| `{dir} must be an owned private directory (0700)` | `path.rs:103` | `check_private_dir` is_err only |
| `control socket needs a private directory` | `control.rs:630` | nothing |
| `unexpected control socket peer` / `unexpected control pipe owner` | `path.rs:136`, `windows_pipe.rs:185` | nothing |
| `PathError` texts | `path.rs:26-32` | `path.rs:225-229`, `control.rs:1832` |
| UI `control socket in use` (title = path), `control disabled: {detail}` | `StatusBar.svelte:89-93` | `app.test.ts:484` (status plumbing only) |
| status strings `listening` / `in-use` / `disabled` | `SocketStatus` serde, `protocol.ts:73` | `control.rs:1635` test |
| socket path (macOS `~/Library/Application Support/com.rudoi.strudeland/ctl.sock`, Linux `$XDG_RUNTIME_DIR/com.rudoi.strudeland/ctl.sock`, Windows `\\.\pipe\com.rudoi.strudeland.<SID>.ctl`) | `path.rs` | `path.rs` tests, README:287-293 |
| modes: socket 0600, lock 0600, new parent 0700 (macOS: an existing parent keeps its mode; Linux: the parent must be 0700) | `control.rs:503,524,563,635` | `:1696,1707-1719,1768` |
| lock file `ctl.sock.lock`, never removed | `:487-491` | `:1767,1771` |
| second instance: InUse, socket untouched, app runs on with the status bar notice | `:530-534, 539-540` | `:1737-1772` |
| stale socket: removed and rebound only by the lock holder | `:529-552` | `:1722-1735` |
| peer rejection: Linux drops the connection silently, macOS has no check | `:456-459` | nothing |

## 5. Wire protocol

No change. Framing (`MAX_LINE_BYTES`, `LineReader`, `read_line_capped`), `handle_line`,
`serve_connection`, the method table and the wire examples are all outside the endpoint, and
the move would not touch them. `serve_connection` takes
`impl AsyncRead + AsyncWrite + Unpin`, so any verified stream type fits it.

## 6. New callers or copies since 63b945d

None in the endpoint. The only extra direct user of the server primitives is the CLI e2e
Windows fake (`e2e.rs:645`), which is the same as at 63b945d apart from the wire-fixture
changes (#35). No new copy of the lock, probe or permission logic exists.

## Recommendation: proceed, with this scope

Move the whole Unix endpoint into a new `strudeland_ctl::server` module, and fold the Windows
first-instance and in-use logic into the same interface:

- `bind(path) -> Bind::{Listening(Endpoint), InUse(detail), Disabled(detail)}`, plus
  `Endpoint::accept() -> verified stream` (unix: the Linux `check_peer` runs inside; windows: the
  next-instance swap runs inside) and `Endpoint::close()`, which removes the socket and then
  releases the lock.
- `lock_path`, `lock_socket`, `bind_socket`, `ensure_private_parent`, `remove_socket` and the
  socket tests at `control.rs:1672-1857` move into ctl. The app keeps `ControlInfo`/`SocketStatus`
  (mapping `Bind` to them), `Bridge`, `handle_line`, `serve_connection`, the spawn-per-connection
  loop, `start`/`shutdown` and the stderr logging.
- The app's real-socket `serve` tests (`:1208`, `:1859-1990`) stay and use the new `bind`/`accept`.
- Add ctl tests: a real `Client` round trip, in-use (lock and probe), stale, a non-socket refused,
  Linux private-parent refusal, plus Windows in-use and second-client tests.

Expected size: about -330 lines in the app and about +330 in ctl plus new tests. `Bridge`'s two
cfg fields collapse to one `Mutex<Option<Endpoint>>`.

### Decisions needed (recommended answer first)

1. **tokio in ctl on unix.** `accept` is async (tokio `UnixListener`), so ctl gains a tokio
   `net` dependency on unix, which the CLI already pulls in partly (`rt`). *Recommend:* put the
   server behind a `server` cargo feature (on in `src-tauri`, and in the CLI only as a dev
   dependency for e2e), so the CLI binary does not link the server code. On Windows, tokio
   remains a dependency either way.
2. **macOS peer check.** Today macOS does none (`control.rs:456`, `client.rs:96` are Linux-only).
   *Recommend:* keep it that way (identical behaviour). Record `getpeereid` as a possible
   follow-up, not part of this move.
3. **Silent peer rejection.** *Recommend:* keep it silent (no new log line). `accept` skips
   unverified streams internally.
4. **Where detail texts are composed.** *Recommend:* ctl builds every detail string, including
   "another strudeland is listening on {path}" and the Windows "cannot listen on {path}: {e}",
   byte-identical, and pins them with exact-text tests. The app only maps the variant to
   `SocketStatus` and keeps the `control: …` stderr lines verbatim.
5. **Windows InUse detail differs from unix.** *Recommend:* keep the difference (identical
   behaviour). Do not unify the texts in this change.
6. **Cleanup via Drop or an explicit close.** *Recommend:* an explicit `close()` called from
   `shutdown`, with `Drop` doing the same (remove the socket, then release the lock) as a safety
   net. Keep the order: remove the socket before releasing the lock. The lock file is still never
   removed. Unix keeps not aborting the accept task, or aborts it like Windows; either is
   invisible, so abort both for symmetry.
7. **`from_std` needing an entered runtime.** *Recommend:* `bind` is a sync function documented
   as "call inside a tokio runtime context", as `windows_pipe::owned_existing` already is. The
   `from_std` failure keeps the `cannot listen on {path}: {e}` text and removes the socket.
8. **Linux double parent creation.** *Recommend:* keep both steps (single-level create and check
   in `ensure_private_parent`, then the recursive no-op) so behaviour stays identical. Do not
   merge them in this change.
9. **`windows_pipe::listen` stays `pub`?** The CLI e2e test uses it as a fake server.
   *Recommend:* keep it `pub` (or switch that test to `server::bind`). Either works; switching
   exercises the real endpoint.

## Decisions (2026-11-26, recommended answers taken; none changes visible behaviour)

1. The server sits behind a ctl `server` cargo feature (tokio on Unix). The app turns it on; the CLI gets it only for its tests.
2. macOS keeps no peer check. Adding `getpeereid` is a separate follow-up.
3. Rejected peers stay silent.
4. ctl builds every detail text exactly as now, and exact-text tests pin them.
5. The Windows in-use text stays different from the Unix one.
6. An explicit `close()`, with `Drop` as a backstop, removes the socket and then releases the lock. The accept task is aborted on both platforms.
7. `bind` is a plain function that must be called inside the tokio runtime.
8. Linux keeps its two parent-folder steps.
9. `windows_pipe::listen` stays public for the CLI e2e test.
