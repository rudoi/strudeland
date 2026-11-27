# 0131: the control endpoint moves into strudeland-ctl

## Goal

The server side of the control socket gets one home: a new `strudeland_ctl::server` module,
behind a `server` cargo feature. It owns everything between a socket path and a verified
stream:

- **Unix:** the `{path}.lock` lock, the stale probe, the bind, the modes (a missing parent
  `0700`, the socket `0600`), the Linux private-parent step, and accept with the Linux peer
  check. A rejected peer is dropped silently.
- **Windows:** the first-instance create, the in-use decision (`owned_existing`) and the
  next-instance swap after each connect.

The app (`src-tauri/src/control.rs`) keeps what is app-specific: `ControlInfo`/`SocketStatus`
(the webview's view of the endpoint), `Bridge`, `handle_line`, `serve_connection`, the
connection loop, `start`/`shutdown` and every `control: …` stderr line.

**No visible change.** Every detail text, stderr line, status string, socket path, file mode,
lock file name, wire byte and CLI output stays byte-identical. Decision 6 below allows one
internal difference that nothing can observe: on Unix, shutdown now aborts the accept task, as
it already does on Windows.

## Background

[reverify.md](reverify.md) holds the evidence at `9dbe6c5`, and its **Decisions** section is
binding. In short:

- The Unix endpoint is `control.rs:446-641` plus `:751-772`. The Windows half is `:643-711`.
  `Bridge` carries two cfg-split transport fields (`:135-139`, `:160-163`).
- ctl already holds the primitives: `windows_pipe::listen` / `owned_existing`
  (`windows_pipe.rs:120-153`, `:294-309`), and `path::check_private_dir` / `check_peer`
  (`path.rs:91-139`). The app owns the policy that strings them together.
- About half the detail texts are pinned by no test (reverify §4). The Windows server and the
  Linux private-parent and peer paths have no test anywhere (§3).
- `src-tauri` cannot be type-checked for Windows or Linux on the Mac: its C dependencies
  (`ogg_next_sys`, from `strudeland-render`) fail to cross-compile. This plan's session checked
  that `cargo clippy --target x86_64-pc-windows-msvc -p strudeland-app` fails in a build
  script. `strudeland-ctl` cross-checks cleanly for `x86_64-pc-windows-msvc` and
  `x86_64-unknown-linux-gnu`, which are both installed. Its tests also run on Linux in Docker
  (`rust:1.100.0-bookworm`, run as uid 501, offline against the host registry): 69 + 6 tests
  passed on `9dbe6c5`.

That last point shapes the design. **All platform-specific code goes into ctl**, where it can
be linted for every target and run on Linux before CI. After the change, `control.rs` has no
`cfg(unix)`, `cfg(windows)` or `target_os` outside its tests.

## Scope

In:

1. `crates/strudeland-ctl/src/server/` (`mod.rs`, `unix.rs`, `windows.rs`), compiled only with
   the new `server` feature. Unix gets an optional tokio (`net`). The feature also turns on
   tokio `sync`, which the Windows endpoint's mutex needs.
2. A ctl self dev-dependency with `features = ["server"]`, so `cargo test -p strudeland-ctl`
   and `--all-targets` clippy build the server and its tests. A probe crate showed that the
   self dev-dependency resolves, that a test behind the feature runs, and that it cross-checks
   for Windows.
3. The Unix bind tests move from `control.rs:1672-1811` into ctl. Their bodies stay
   byte-identical, because the private helpers keep their names (`BindOutcome`,
   `bind_socket`, `lock_socket`, `lock_path`).
4. New exact-text tests for every reachable detail text, written first against the
   **unchanged** app code, then copied into ctl.
5. New ctl tests:
   - a real `Client` round trip, Unix and Windows;
   - two sequential clients (Unix), and a second client while the first is still connected
     (Windows: the next instance);
   - in use by lock and by live probe, and a stale socket removed;
   - a non-socket left alone;
   - `close` and `Drop`;
   - a rejected peer dropped without an error;
   - Linux: private parent, no parent, `mkdir` error, and `check_peer` on a same-user pair and
     on a non-socket;
   - Windows: in use, and an invalid endpoint disabled;
   - `AcceptError`'s pause and closed rules.
6. The app maps `server::Bind` to `ControlInfo`, keeps one platform-independent connection
   loop, and closes the endpoint on shutdown. The old endpoint code and its tests are deleted.
7. `AGENTS.md` (the ctl and `control.rs` layout bullets), ctl's `Cargo.toml` description and
   `lib.rs` doc, and `control.rs`'s module doc.

Out:

- Any text, mode, path or status change, including unifying the Windows and Unix in-use texts
  (Decision 5).
- A macOS peer check (`getpeereid`, Decision 2). It is a follow-up, not this change.
- Merging the two Linux parent steps (Decision 8).
- Switching the CLI e2e Windows fake (`e2e.rs:645`) to `server::bind`. `windows_pipe::listen`
  stays `pub` (Decision 9). The CLI manifest is unchanged: it has no test that needs `server`.
- `serve_connection`, `handle_line`, framing, the method table and the wire examples
  (reverify §5).
- `lib.rs`: `start` and `shutdown` keep their signatures (`lib.rs:60`, `:123`).

## Architecture

```
strudeland-ctl (feature "server")            src-tauri control.rs
──────────────────────────────────           ─────────────────────────────────────────
server::bind(&Path) -> Bind                  listen(Result<PathBuf, PathError>)
  Unix:    [linux] ensure_private_parent        -> (ControlInfo, Option<Endpoint>)
           bind_socket (lock, probe, stale,       PathError     -> Disabled, path None
             bind, 0600, nonblocking)             Bind::*       -> status + Some(path)
           UnixListener::from_std                                 + the detail as given
           InUse text composed here
  Windows: windows_pipe::listen(path, true)  start: log the status line (unchanged),
           owned_existing -> InUse/Disabled      set_info, Arc<Endpoint> into serve,
                                                 Weak<Endpoint> + JoinHandle into Bridge
Endpoint::accept(&self)
  -> Result<Stream, AcceptError>             serve(endpoint, ..): JoinSet of
  Unix:    accept, [linux] check_peer          serve_connection; on AcceptError log
           (a failure is skipped silently)     the same line as today, stop if
  Windows: connect, listen(path, false),       closed(), sleep pause()
           swap
Endpoint::close(&self), Drop -> close        shutdown -> Bridge::close_server:
  Unix:    remove socket, then drop lock       upgrade Weak, close(), abort task
  Windows: nothing (the pipe closes on drop)
```

## Technical design

### The ctl API (`server/mod.rs`)

```rust
/// What binding the control endpoint gave.
pub enum Bind {
    /// This process owns the endpoint.
    Listening(Endpoint),
    /// Another strudeland owns it; the detail names it.
    InUse(String),
    /// The endpoint cannot be used; the detail says why.
    Disabled(String),
}

#[derive(Debug)]
pub enum AcceptError {
    /// Unix `accept` or Windows `connect` failed.
    Accept { source: io::Error, closed: bool },
    /// Windows only: the next pipe instance could not be created, so the client that just
    /// connected was dropped. Never returned on Unix.
    NextInstance { source: io::Error, closed: bool },
}

impl AcceptError {
    /// The endpoint accepts no more. Windows: resetting the pipe after the failure failed too.
    /// Always false on Unix.
    pub fn closed(&self) -> bool;
    /// How long the caller waits before the next `accept`. Today's rule: 100 ms after a Unix
    /// accept failure and after a next-instance failure, none after a Windows connect failure.
    pub fn pause(&self) -> Option<Duration>;
}

#[cfg(unix)]    pub use unix::{Endpoint, Stream, bind};     // Stream = tokio::net::UnixStream
#[cfg(windows)] pub use windows::{Endpoint, Stream, bind};  // Stream = NamedPipeServer
```

- `bind(path: &Path) -> Bind` is a plain function. It must be called inside a tokio runtime
  with IO enabled (Decision 7): `UnixListener::from_std` and `ServerOptions` panic without
  one, as they do today.
- `Endpoint::accept(&self) -> Result<Stream, AcceptError>` is cancel-safe. The app polls it in
  a `select!` against `JoinSet::join_next`, as the Windows loop does today. On Unix its only
  await is `UnixListener::accept`. On Windows it locks a `tokio::sync::Mutex<NamedPipeServer>`
  and awaits `connect`. Both are cancel-safe, and everything after the await is synchronous.
- `Endpoint::close(&self)` is idempotent. On Unix it takes the lock `File` out of a
  `std::sync::Mutex<Option<File>>` and, only when it was still there, removes the socket file
  and then drops the lock. On Windows it does nothing: the pipe instance closes when the
  endpoint drops. `Drop` calls `close` (Decision 6).
- The text that each stderr line puts after `control: ` is still built in the app. The app
  matches the `AcceptError` variant and keeps its `eprintln!` literals byte-identical.
  `NextInstance` exists on every platform, so that match needs no cfg.

### Unix (`server/unix.rs`)

The private helpers move verbatim from `control.rs`: `lock_path` (`:485-491`),
`lock_socket` (`:493-511`), `BindOutcome` (`:475-483`), `bind_socket` (`:513-571`) and
`ensure_private_parent` (`:622-641`). `bind` is `listen`'s body (`:573-620`) without the
`PathError` arm and without `ControlInfo`:

- Linux `ensure_private_parent` error: `Disabled(e.to_string())`;
- `Listening(std, lock)`: `from_std`. On success, an `Endpoint { listener, path, lock }`.
  On failure, remove the socket, compose `cannot listen on {}: {e}`, and let `lock` drop
  after the removal, as today;
- `InUse`: `InUse(format!("another strudeland is listening on {}", path.display()))`;
- `Disabled(d)`: `Disabled(d)`.

`accept` calls a private `accept_verified(&self, verify)` with the platform's check: Linux
`crate::path::check_peer`, other Unix always `Ok`. A stream that fails `verify` is dropped
and the loop accepts again. There is no error and no log (Decision 3). The seam lets a
macOS test prove that a rejected peer is skipped silently, which no test can show for a real
foreign uid without root.

### Windows (`server/windows.rs`)

- `bind` is the Windows `listen` (`control.rs:643-670`) without `PathError`.
  `windows_pipe::listen(path, true)`. On error, `owned_existing(path)` picks `InUse` or
  `Disabled`, both with `cannot listen on {}: {e}`.
- `accept` is the body of today's Windows `serve` loop (`:679-704`) for one client. On a
  `connect` error, `disconnect`. On a `listen(path, false)` error, `disconnect`. In both cases
  `closed = disconnect().is_err()`. On success, `std::mem::replace` swaps in the next
  instance and the connected one is returned.

### The app

- `Bridge` gets one field, `server: Mutex<Option<Server>>`, with
  `struct Server { endpoint: Weak<Endpoint>, task: tauri::async_runtime::JoinHandle<()> }`.
  The serve task owns the only strong `Arc`. Today the server future owns the pipe. With the
  `Weak`, the endpoint still drops when that future ends, which is when the Windows loop
  returns after a `closed` error, and the pipe vanishes exactly then, as today.
- `listen(path: Result<PathBuf, PathError>) -> (ControlInfo, Option<Endpoint>)` is one
  function for every platform. The `PathError` arm is unchanged (`Disabled`, path `None`,
  `e.to_string()`).
- `serve(endpoint: Arc<Endpoint>, bridge, emitter, snapshotter)` is one loop: a `JoinSet` of
  `serve_connection`, reaped in the `select!`. On `Err(e)` it does three things in order:
  - logs `control: accept failed: {e}` or `control: cannot create next pipe instance: {e}`,
    the same literals as today;
  - returns if `e.closed()`;
  - sleeps `e.pause()`.

  Unix connections move from detached `tokio::spawn` into the `JoinSet`. They used to outlive
  the accept task, which never ended, until the runtime died. Now they end when shutdown
  aborts the task. That is invisible: shutdown runs on `RunEvent::Exit`.
- `start` is unchanged up to `set_info`. It then wraps the endpoint in an `Arc`, spawns
  `serve`, and stores the `Weak` and the handle. `shutdown` calls `Bridge::close_server`, which
  upgrades the `Weak` and calls `close()` (Unix: remove the socket, then release the lock), and
  then aborts the task.
- `remove_socket` (`:751-758`) goes away. Only a `Listening` bind yields an `Endpoint`, so
  "remove only when listening" now holds by type.

### Texts and where they are built

| Text | Today | After |
|---|---|---|
| `cannot create {parent}: {e}`, `cannot open {lock}: {e}`, `cannot lock {lock}: {e}`, `{path} exists and is not a socket`, `cannot remove stale {path}: {e}`, `{path}: {e}` ×2, `cannot listen on {path}: {e}` ×3 | `control.rs` `bind_socket`, `listen` | `server/unix.rs` |
| `another strudeland is listening on {path}` | `control.rs:609` | `server/unix.rs` `bind` |
| `control socket needs a private directory`, and the Linux `mkdir` and `check_private_dir` errors passed through `to_string()` | `control.rs:622-641` | `server/unix.rs` |
| Windows `cannot listen on {path}: {e}` | `control.rs:666` | `server/windows.rs` `bind` |
| `control: listening on {}`, `control: disabled: {detail}`, `control: disabled` | `control.rs:722-727` | unchanged |
| `control: accept failed: {e}` (Unix and Windows copies), `control: cannot create next pipe instance: {e}` | `control.rs:468`, `:683`, `:695` | `control.rs` `serve`, one copy each |
| `PathError`, `check_private_dir`, `check_peer`, `check_owner` texts | `path.rs`, `windows_pipe.rs` | unchanged, untouched |

Stage 001 builds a literal inventory with expected counts, and both stages check it.

## Stages

1. [001-ctl-server-module.md](001-ctl-server-module.md). First, pin today's texts with
   exact-text tests on the unchanged app code. Then add `strudeland_ctl::server` with the
   copied helpers, the moved and pinned tests byte-identical, and the new tests. The app code
   is unchanged: only its test module gains the pins.
2. [002-app-uses-server.md](002-app-uses-server.md). The app switches to `server::bind`,
   `Endpoint::accept` and `close`. The endpoint code and the tests now in ctl are deleted, the
   serve tests are adapted, and the docs are updated.

**Why two stages.** 001 is the proof stage. The same pin tests run against the old code
(`control.rs`, still live) and the new code (ctl) in one commit. A script shows that the moved
test bodies are byte-identical in both files. The ctl module is linted for all three
platforms, and its tests run on macOS and on Linux in Docker, before the app depends on it.
002 is the switch. Its diff is the app plus deletions, and it touches no ctl file, so any
behaviour change in 002 can only come from that diff. One combined stage would lose the point
where old and new code are checked side by side, and the bisection point. Splitting the pins
into a third stage would add a full gate run and nothing else: 001 runs the pins and logs them
before it writes any ctl code.

The stages are sequential: 002 uses the API that 001 fixes.

## Dependencies

- tokio `1.54.1` (already in `Cargo.lock` under `strudeland-ctl`). The probe found no
  lockfile change. Any `Cargo.lock` diff is reviewed in the stage report.
- Docker with `rust:1.100.0-bookworm` (present) for the Linux test run. This is newer than
  CI's `1.94.1`. It is good enough to execute the Linux-only tests, and CI then runs them on
  the pinned toolchain.

## Risks

- **Windows code runs only in CI.** It is linted locally for `x86_64-pc-windows-msvc`, but no
  Windows test executes before the PR. Mitigation: the Windows code is small and copied from
  today's loop, and the pins compare against errors taken from the same call. A CI failure
  goes to the STOP list.
- **Permission-based pins behave differently as root.** CI and Docker (`--user`) run
  unprivileged. A pin whose precondition cannot be produced on some platform is gated to the
  platforms where it can, and the report says so. Its expected composition is never changed.
- **Feature unification.** `cargo test --workspace` turns `server` on everywhere, so a missing
  `#[cfg(feature = "server")]` would only show in a CLI-alone build. Gates: `clippy -p
  strudeland-ctl --lib` (feature off) for each target, and a `cargo tree` check that the CLI
  does not enable `server`.
- **The `Weak` handoff on Unix.** If `close` ran after the task had dropped the endpoint, it
  would do nothing. That cannot happen: the Unix accept loop never returns, so only the
  aborted future drops the `Arc`, and `close_server` upgrades before it aborts.

## Open questions

1. Joining Unix connections into the `JoinSet` means shutdown aborts them with the accept task.
   That is invisible at exit, and Decision 6 asks for the abort, but it is a change in task
   ownership. The plan takes it. Say if detached Unix connections must stay. They would need
   a `cfg(unix)` in the app that cannot be linted locally.
2. On Windows the pipe is now `disconnect`ed inside `accept` before the app logs the failure
   (today: log, then disconnect). The difference is a few microseconds of ordering, with
   nothing observable.
3. File the `getpeereid` follow-up (Decision 2) as a task now, or when this lands?
