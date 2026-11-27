# 001: pin today's texts, then add `strudeland_ctl::server`

## Goal

First, exact-text tests pin every reachable detail text of the Unix endpoint, written against
the **unchanged** app code and logged green before any ctl code exists. Then
`strudeland_ctl::server` gets the whole endpoint, behind the new `server` feature:

- the Unix lock, stale probe, bind, modes, Linux private parent, and peer-checked accept;
- the Windows first-instance and in-use decision, and the next-instance accept;
- `close` and `Drop`.

The moved bind tests and the new pins are byte-identical in both places, and new tests cover
the public API on every platform. The app's code is untouched in this stage: only its test
module gains the pins.

## Prerequisites

- The tree is code-identical to `9dbe6c5`: `git diff --name-only 9dbe6c5 HEAD` lists only
  `plans/`.
- `rustup target list --installed` includes `x86_64-pc-windows-msvc` and
  `x86_64-unknown-linux-gnu`, and `docker images` lists `rust:1.100.0-bookworm`. Both were
  present when this plan was written. If either is missing, STOP and ask: no other check
  covers that platform before CI.

## Files

- Edited:
  - `src-tauri/src/control.rs`, **inside `mod tests` only** (after `:797`);
  - `crates/strudeland-ctl/Cargo.toml`, `crates/strudeland-ctl/src/lib.rs`;
  - `AGENTS.md`, the `crates/strudeland-ctl/` bullet (`:179-184`).
- New:
  - `crates/strudeland-ctl/src/server/mod.rs`, `server/unix.rs`, `server/windows.rs`;
  - `$S/0131-literals.sh`, `$S/0131-same-tests.sh` (not committed).
- Untouched, besides the shared frozen list: everything in `src-tauri/` other than
  `control.rs`'s test module, `Cargo.lock` (expected, see the gate notes).

## Steps

1. **Pins on the unchanged code** (`control.rs` test module, all `#[cfg(unix)]`, using
   `short_tempdir` and `bind_socket`). Each expected text is composed from the same OS error,
   obtained in the test by repeating the failing call. That pins the composition without
   hard-coding an OS message.
   - `listen_reports_each_status` (`:1815`): replace `assert!(info.detail.is_some());` with
     `assert_eq!(info.detail, Some(format!("another strudeland is listening on {}", path.display())));`.
   - `an_uncreatable_parent_is_disabled` (`:1792`):
     - While `ro` is still `0500`, take `let e = fs::DirBuilder::new().recursive(true).mode(0o700).create(ro.join("a")).unwrap_err();`
       and assert `detail == format!("cannot create {}: {e}", ro.join("a").display())`, in
       place of the `starts_with` check.
     - For the second half, under a regular file, assert
       `Disabled(format!("cannot open {}: {e}", lock.display()))`, where
       `lock = file.join("c.sock.lock")` and `e` is `lock_socket`'s own `OpenOptions`
       (`read`, `write`, `create`, `truncate(false)`, `mode(0o600)`) opening `lock`.
   - New `an_unopenable_lock_is_disabled`: `fs::create_dir(lock_path(&path))`. Then
     `bind_socket(&path)` gives `Disabled(format!("cannot open {}: {e}", lock.display()))`,
     with `e` from the same `OpenOptions` on that directory. Assert `!path.exists()`.
   - New `an_unremovable_stale_socket_is_disabled`: make a stale socket (bind, drop), create
     `lock_path(&path)` with `fs::write`, `chmod 0500` the directory, then:
     - run `bind_socket`;
     - take `e = fs::remove_file(&path).unwrap_err()`;
     - restore `0700` **before** any assertion;
     - assert `Disabled(format!("cannot remove stale {}: {e}", path.display()))` and that
       the socket's inode is unchanged.
   - New `a_probe_error_is_disabled`: make a stale socket and `chmod 0o000` it. Take
     `e = StdUnixStream::connect(&path).unwrap_err()`. First assert the precondition:
     `e.kind()` is neither `ConnectionRefused` nor `NotFound`. Then assert
     `bind_socket` gives `Disabled(format!("{}: {e}", path.display()))` and the socket
     still exists.
   - New `an_unbindable_path_is_disabled`: `path = dir.path().join("s".repeat(110))`. Assert
     the precondition `path.as_os_str().len() > 108`. Take
     `e = StdUnixListener::bind(&path).unwrap_err()`, then assert
     `Disabled(format!("cannot listen on {}: {e}", path.display()))`.
   - Not reachable from a test, and left to the literal inventory (step 2): `cannot lock`
     (`:509`), the metadata arm of `{path}: {e}` (`:556`), the `chmod`/`set_nonblocking` arm
     (`:568`) and `from_std` (`:601`, which panics rather than fails outside a runtime).

   Run `L pins cargo test --locked -p strudeland-app control::tests::` with `N=001`. It must
   exit 0 **before step 3**. Read it, and record the six pinned and new test names. If a pin's
   precondition does not hold on macOS, STOP: the plan assumed it does.

2. **The checking scripts** (in `$S`, not committed).
   - `0131-literals.sh N`. For each literal below, it counts the lines that contain it (fixed
     string) in the non-test part of each file: the lines before the first `^mod tests`
     line, found with `awk '/^mod tests/{exit} {print}'`, or the whole file if there is none.
     It prints a table and exits 1 on any count that differs from the expected one for stage
     `N`.

     | Literal (as in the source) | `control.rs` 001 | `control.rs` 002 | `server/unix.rs` | `server/windows.rs` |
     |---|---|---|---|---|
     | `"cannot open {}: {e}"` | 1 | 0 | 1 | 0 |
     | `"cannot lock {}: {e}"` | 1 | 0 | 1 | 0 |
     | `"cannot create {}: {e}"` | 1 | 0 | 1 | 0 |
     | `"{shown} exists and is not a socket"` | 1 | 0 | 1 | 0 |
     | `"cannot remove stale {shown}: {e}"` | 1 | 0 | 1 | 0 |
     | `"{shown}: {e}"` | 2 | 0 | 2 | 0 |
     | `"cannot listen on {shown}: {e}"` | 2 | 0 | 2 | 0 |
     | `"cannot listen on {}: {e}"` | 2 | 0 | 1 | 1 |
     | `"another strudeland is listening on {}"` | 1 | 0 | 1 | 0 |
     | `"control socket needs a private directory"` | 1 | 0 | 1 | 0 |
     | `"control: listening on {}"` | 1 | 1 | 0 | 0 |
     | `"control: disabled: {detail}"` | 1 | 1 | 0 | 0 |
     | `"control: disabled"` | 1 | 1 | 0 | 0 |
     | `"control: accept failed: {e}"` | 2 | 1 | 0 | 0 |
     | `"control: cannot create next pipe instance: {e}"` | 1 | 1 | 0 | 0 |

     The `control.rs` 001 column is `9dbe6c5`'s count. It was measured while writing this
     plan, and the script also checks it against `git show 9dbe6c5:src-tauri/src/control.rs`.
     The ctl columns are the same in both stages. If `cargo fmt` wraps a `format!` call, the
     literal still stays on one line: do not split a literal.
   - `0131-same-tests.sh` extracts each named item from `src-tauri/src/control.rs` and from
     `crates/strudeland-ctl/src/server/unix.rs`. An item runs from the line
     `^    fn NAME(` to the next line that is exactly `    }`, with
     `awk -v n=NAME 'index($0, "    fn " n "(") == 1 {p=1} p {print} p && $0 == "    }" {exit}'`.
     The script diffs each pair, prints each name with `same` or `differs`, and exits 1 if
     any name differs or is missing from either file. The names are:
     - `short_tempdir`, `mode`;
     - `binds_a_fresh_path_with_mode_0600`, `creates_a_missing_parent_dir_0700`,
       `keeps_the_mode_of_an_existing_parent_dir`, `a_stale_socket_is_removed_and_bound`,
       `a_live_socket_is_in_use_and_left_alone`,
       `a_held_lock_is_in_use_and_the_stale_socket_is_left_alone`,
       `a_regular_file_is_disabled_and_left_alone`, `an_uncreatable_parent_is_disabled`;
     - `an_unopenable_lock_is_disabled`, `an_unremovable_stale_socket_is_disabled`,
       `a_probe_error_is_disabled`, `an_unbindable_path_is_disabled`.

     The `#[test]`/`#[cfg(unix)]` lines above each `fn` are not compared. In ctl the whole
     file is Unix-only, so the `#[cfg(unix)]` may go.

3. **`crates/strudeland-ctl/Cargo.toml`.**
   - `description`: "strudeland control socket: JSON-RPC protocol types, framing, a blocking
     client and the app's endpoint".
   - Under `[features]` (`:10-13`), add the following, with a comment in the `wire-fixtures`
     style:

     ```toml
     # The app's side of the control endpoint (`server`): bind, the lock and the stale probe,
     # verified accept. Only the app turns it on; the CLI never links it.
     server = ["dep:tokio", "tokio/net", "tokio/sync"]
     ```

   - `[dev-dependencies]` (`:20-21`): add
     `strudeland-ctl = { path = ".", features = ["server"] }`, with the comment "builds
     `server` and its tests under `cargo test -p strudeland-ctl`".
   - New `[target.'cfg(unix)'.dependencies]`:
     `tokio = { version = "1.54.1", features = ["net"], optional = true }`.
   - New `[target.'cfg(unix)'.dev-dependencies]`:
     `tokio = { version = "1.54.1", features = ["rt", "io-util"] }`.
   - The Windows tokio line (`:27`) is unchanged and stays non-optional: `windows_pipe` (the
     client) needs it. A probe crate showed that `dep:tokio` resolves with this split.
4. **`lib.rs`.** Add `#[cfg(feature = "server")] pub mod server;`. In the module doc, after
   "a blocking client", add: "and, behind the `server` feature, the endpoint the app listens
   on". No new re-export at the crate root: callers write `strudeland_ctl::server::…`.
5. **`server/mod.rs`.** The module doc says what the endpoint owns, platform by platform (see
   the proposal), and that `bind` must run inside a tokio runtime with IO enabled.
   - `Bind`, `AcceptError` with `closed()` and `pause()`, exactly as in the proposal's API.
     `pause()`: `Accept` gives `Some(100 ms)` on Unix and `None` on Windows. `NextInstance`
     gives `Some(100 ms)`. Name one `const ACCEPT_PAUSE: Duration = Duration::from_millis(100);`.
   - `#[cfg(unix)] mod unix; #[cfg(unix)] pub use unix::{Endpoint, Stream, bind};`, and the
     same for `windows`.
   - A `#[cfg(test)]` helper,
     `async fn answer_one_ping(stream: impl AsyncRead + AsyncWrite + Unpin)`. It reads one
     line, asserts `method == "ping"`, writes `{"jsonrpc":"2.0","id":<id>,"result":"ok"}\n`,
     then reads until EOF. This is the server half of
     `windows_pipe::tests::authenticated_local_round_trip` (`windows_pipe.rs:355-397`).
   - The `AcceptError` test (all platforms), `accept_error_pauses_and_closes_as_today`: each
     variant with `closed: false` and with `closed: true` gives the matching `closed()`, and
     `pause()` follows the per-platform rule above.
6. **`server/unix.rs`.**
   - Copy **verbatim** from `control.rs`, doc comments included: `BindOutcome` (`:475-483`,
     made private), `lock_path` (`:485-491`), `lock_socket` (`:493-511`), `bind_socket`
     (`:513-571`, made private), and `ensure_private_parent` (`:622-641`,
     `#[cfg(target_os = "linux")]`, calling `crate::path::check_private_dir`). Drop the
     per-item `#[cfg(unix)]`. Keep the imports they need.
   - `pub type Stream = tokio::net::UnixStream;`.
   - `#[derive(Debug)] pub struct Endpoint { listener: UnixListener, path: PathBuf, lock: Mutex<Option<File>> }`.
   - `pub fn bind(path: &Path) -> Bind`, as in the proposal ("Unix"), including the
     `cannot listen on {}: {e}` text and the removal on a `from_std` failure. Bind the lock as
     `lock` in that arm, so it drops after `remove_file`.
   - `impl Endpoint`:
     - `pub async fn accept(&self) -> Result<Stream, AcceptError>` is
       `self.accept_verified(verify_peer).await`;
     - `async fn accept_verified(&self, verify: impl Fn(&Stream) -> io::Result<()>)` loops:
       `listener.accept()`, `Err(source)` gives `Err(AcceptError::Accept { source, closed: false })`,
       a failed `verify` is `continue`, otherwise it returns the stream;
     - `fn verify_peer(stream: &Stream) -> io::Result<()>` calls `crate::path::check_peer`
       on Linux and returns `Ok(())` on other Unix, with no `return` and no unused-variable
       warnings on either side;
     - `pub fn close(&self)`: takes the lock out of the mutex (recovering from poison as the
       app's `lock` helper does). Only if it was `Some`, it runs
       `let _ = fs::remove_file(&self.path);` and then drops the lock;
     - `impl Drop for Endpoint { fn drop(&mut self) { self.close() } }`.
7. **`server/windows.rs`.**
   - `pub type Stream = NamedPipeServer;`.
   - `#[derive(Debug)] pub struct Endpoint { pipe: tokio::sync::Mutex<NamedPipeServer>, path: PathBuf }`.
   - `bind`: `windows_pipe::listen(path, true)`. On `Ok`, `Listening`. On `Err(e)`, the
     message is `format!("cannot listen on {}: {e}", path.display())`, and
     `windows_pipe::owned_existing(path)` picks `InUse(message)` or `Disabled(message)`
     (`control.rs:655-668`).
   - `accept` follows the proposal ("Windows"). Keep the comment "Keep at least one instance
     alive, and ensure a second client can connect." (`:691`).
   - `close` does nothing and says why in its doc. `Drop` calls it, for symmetry with Unix.
8. **ctl tests.**
   - `server/unix.rs` `#[cfg(test)] mod tests`:
     - the 14 items of step 2, byte-identical;
     - `bind_reports_each_outcome`, on a runtime from
       `tokio::runtime::Builder::new_current_thread().enable_all()`, entered: a fresh path gives
       `Listening`. A second `bind` while the first lives gives
       `InUse(format!("another strudeland is listening on {}", path.display()))`. A regular
       file gives `Disabled(format!("{} exists and is not a socket", path.display()))`, and the
       file is left alone;
     - `a_client_round_trips_over_the_endpoint`: bind, move the endpoint and the runtime into a
       thread that accepts and `answer_one_ping`s twice. On the main thread, two sequential
       `Client::connect(&path, 1 s)` + `call_raw("ping", json!({}))` each give `json!("ok")`.
       Join the thread, get the endpoint back, `close()` it, then assert the socket is gone,
       the lock file is still there, and `Client::connect` gives `CallError::NotRunning`;
     - `close_frees_the_path_and_twice_is_harmless`: after `close()`, a second `bind` gives
       `Listening`. A second `close()` on the first endpoint does not remove the new socket
       (assert that it exists);
     - `dropping_the_endpoint_closes_it`: `drop(endpoint)`, then the socket is gone and `bind`
       gives `Listening`;
     - `a_rejected_peer_is_skipped_silently`: call `accept_verified` with a verify that refuses
       the first stream and accepts the rest. Client one connects first and reads EOF. Client
       two connects and is the one returned (it gets the ping answer). No `Err` comes back;
     - Linux only (`#[cfg(target_os = "linux")]`):
       - `a_shared_parent_is_disabled`: a `0755` parent gives
         `Disabled(format!("{} must be an owned private directory (0700)", parent.display()))`,
         and neither the socket nor the lock file exists;
       - `a_path_without_a_parent_is_disabled`: `bind(Path::new("c.sock"))` gives
         `Disabled("control socket needs a private directory".into())`, and no `c.sock.lock`
         is created in the working directory;
       - `a_missing_grandparent_is_disabled`: `dir/a/b/c.sock` gives `Disabled(e.to_string())`,
         where `e` is `fs::DirBuilder::new().mode(0o700).create(dir/a/b)`'s error;
       - `a_new_private_parent_is_created_0700`: `dir/p/c.sock` gives `Listening`, with
         `dir/p` at `0700`;
       - `check_peer_accepts_a_same_user_pair` (`std::os::unix::net::UnixStream::pair`);
       - `check_peer_refuses_a_non_socket` (`File::open("/dev/null")`, `is_err`).

       `unexpected control socket peer` needs a foreign uid, which needs root, so it stays
       untested. Say so in the report.
   - `server/windows.rs` `#[cfg(test)] mod tests`, with the `endpoint()` name helper of
     `windows_pipe.rs:316-323` (a different prefix, `strudeland-server-test-`):
     - `a_second_client_connects_while_the_first_is_open`: bind. A thread accepts `a`, then
       `b`, then `answer_one_ping(a)` and `answer_one_ping(b)`. The main thread connects client
       A, then client B (only possible through the next instance), pings A, drops it, pings B,
       drops it, then joins;
     - `a_second_bind_is_in_use_with_the_listen_error`: while the first endpoint lives, take
       `e = windows_pipe::listen(&path, true).unwrap_err()`, then `bind(&path)` gives
       `InUse(format!("cannot listen on {}: {e}", path.display()))`;
     - `an_invalid_endpoint_is_disabled`: `\\.\pipe\bad\component` (local, so the in-use probe
       makes no network call) gives `Disabled(format!("cannot listen on {}: {e}", path.display()))`,
       with `e` from `windows_pipe::listen(&path, true)`;
     - `a_dropped_endpoint_is_not_running`: bind, drop, then `Client::connect` gives
       `CallError::NotRunning`.
   - Targeted runs: `cargo test --locked -p strudeland-ctl server::` (macOS),
     `DL server::` (Linux), `X x86_64-pc-windows-msvc --all-targets`.
9. **Smoke baseline** (the app is still `9dbe6c5`'s code). `/tmp` is used because macOS caps
   socket paths at 103 bytes and `$S` is longer, the same reason as `short_tempdir`:

   ```sh
   D=/tmp/sl0131; rm -rf "$D"; mkdir -m 0700 "$D"
   STRUDELAND_SOCKET="$D/ctl.sock" bun run tauri dev > "$S/0131-$N-app1.log" 2>&1   # run_in_background
   ```

   1. Wait (Monitor, an until-loop on `grep -q '^control: ' "$S/0131-$N-app1.log"`).
   2. `STRUDELAND_SOCKET="$D/ctl.sock" cargo run -q --locked -p strudeland-cli -- ping`,
      logged as `ping` (exit 0, `… is running`).
   3. `stat -f '%Sp %N' "$D"/*`, logged as `modes`: `srw-------` for `ctl.sock` and
      `-rw-------` for `ctl.sock.lock`.
   4. Start a second instance with
      `STRUDELAND_SOCKET="$D/ctl.sock" "$PWD/target/debug/strudeland-app" > "$S/0131-$N-app2.log" 2>&1`
      (background). Wait for its `control: ` line, then kill **that PID only**.
   5. Stop the first instance by its PID
      (`pgrep -f "$PWD/target/debug/strudeland-app"`; never a broad `pkill`: the user may
      have their own strudeland running), then the `bun` parent. A kill skips
      `RunEvent::Exit`, so `ctl.sock` stays stale.
   6. Start the first instance again, the same way, as `app3.log`. Its `control: ` line shows
      that the stale socket was taken over. Stop it the same way.
   7. `grep -h '^control: ' "$S/0131-$N-app1.log" "$S/0131-$N-app2.log" "$S/0131-$N-app3.log" > "$S/0131-$N-smoke-lines.log"`.

   The expected lines, in order:
   - `control: listening on /tmp/sl0131/ctl.sock`;
   - `control: disabled: another strudeland is listening on /tmp/sl0131/ctl.sock`;
   - `control: listening on /tmp/sl0131/ctl.sock`.

   Do not start playback or touch the UI. If port 1420 is taken or the app cannot start,
   report it and skip the smoke. It is a cross-check, not the proof.
10. **Docs.** `AGENTS.md`, the `crates/strudeland-ctl/` bullet (`:179-184`). After
    "blocking `Client`)", add: "and, behind its `server` feature (only the app turns it on),
    the endpoint the app listens on: `server::bind` (the lock, the stale probe and the modes
    on Unix, the first-instance pipe on Windows), and `Endpoint::accept` (the Linux peer
    check, the next pipe instance) and `close`". The `control.rs` bullet changes in 002, which
    makes it stale.

## Tests

- Strengthened on the unchanged app code: `listen_reports_each_status` (exact in-use text)
  and `an_uncreatable_parent_is_disabled` (exact `cannot create` and `cannot open`).
- New in the app: the four pins of step 1. Each also exists, byte-identical, in ctl.
- Copied into ctl, byte-identical: the 8 bind tests and 2 helpers (`control.rs:1672-1811`).
- New in ctl: step 5's `AcceptError` test, and step 8's Unix, Linux and Windows tests.

## Validation

Run [the full gate](000-prompt.md#the-full-gate) with `N=001`: steps 1-12 and 14. Step 13
does not apply. Also, before committing:

```sh
git diff -U0 9dbe6c5 -- src-tauri/src/control.rs | grep '^@@' \
  | awk '{split($3,a,/[+,]/); if (a[2] <= 797) bad=1} END {exit bad}' \
  > "$S/0131-001-app-tests-only.log"; echo "exit $?" >> "$S/0131-001-app-tests-only.log"
git diff --name-only 9dbe6c5 -- src-tauri > "$S/0131-001-tauri-files.log"
```

- `app-tests-only.log` ends `exit 0`: every hunk starts after `#[cfg(test)]` (`:797`), so
  the app's code is `9dbe6c5`'s.
- `tauri-files.log` lists only `src-tauri/src/control.rs`.
- `pins.log` (step 1) predates every `crates/strudeland-ctl/src/server/` file. Compare the
  log's first `date` line with the files' mtimes, and say so in the report.
- `same-tests.log` says `same` for all 14 names. `literals.log` shows every count as expected.

## STOP

- A pin that fails on the unchanged code, or whose precondition does not hold on macOS.
- A moved or pinned body that cannot stay byte-identical in ctl.
- A literal count off the table.
- An app hunk outside `mod tests`.
- Anything in the shared STOP list.

## Commit message

```
feat(ctl): add the control endpoint behind a server feature

strudeland-ctl gains the server side of the control socket: the Unix
lock, stale probe, bind and modes, the Linux private parent and peer
check on accept, and the Windows first-instance and next-instance
pipes, with an endpoint that removes its socket and then releases
its lock on close. The app does not use it yet. Its detail texts are
built exactly as the app builds them today, and new tests pin them
here and in the app.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
