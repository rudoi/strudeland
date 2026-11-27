# 002: the app serves through `strudeland_ctl::server`

## Goal

`control.rs` binds with `server::bind`, accepts with `Endpoint::accept` in one
platform-independent loop, and closes with `Endpoint::close` on shutdown. The endpoint code and
the socket tests that 001 copied into ctl are deleted from the app. Afterwards `control.rs`
has no `cfg(unix)`, `cfg(windows)` or `target_os` outside test-only attributes. Every text,
stderr line, status, mode and wire byte is unchanged. This stage touches no ctl file.

## Prerequisites

- Stage 001 committed and green. Its `same-tests` and `literals` logs are in `$S`.

## Files

- Edited:
  - `src-tauri/src/control.rs`;
  - `src-tauri/Cargo.toml` (`:22`);
  - `AGENTS.md`, the `src-tauri/src/control.rs` bullet (`:190-194`).
- Untouched, besides the shared frozen list: all of `crates/strudeland-ctl/` (including
  `server/`, `Cargo.toml` and `lib.rs`), `src-tauri/src/lib.rs` (callers keep their
  signatures) and `Cargo.lock`.

## Steps

1. **Manifest.** `src-tauri/Cargo.toml:22`:
   `strudeland-ctl = { path = "../crates/strudeland-ctl", features = ["server"] }`. The
   dev-dependency line (`:45`) is unchanged, because features unify. src-tauri's tokio
   (`:31`) already has `net`, `sync`, `time` and `rt`.
2. **Delete from `control.rs`:**
   - the Unix `serve` (`:445-473`);
   - `BindOutcome`, `lock_path`, `lock_socket`, `bind_socket`, both `listen`s and
     `ensure_private_parent` (`:475-670`);
   - the Windows `serve` (`:672-711`);
   - `remove_socket` (`:751-758`);
   - `Bridge`'s `socket_lock` and `server` fields and their initializers (`:135-139`,
     `:160-163`).

   Then let clippy trim the now-unused imports (`:12-21`, `:38-42`). Imports that only tests
   use (`fs`, `MetadataExt`, `UnixStream`) become `#[cfg(test)]` or move into `mod tests`.
3. **Add to `control.rs`** (`use strudeland_ctl::server::{self, AcceptError, Bind, Endpoint};`
   and `std::sync::Weak`):
   - `struct Server { endpoint: Weak<Endpoint>, task: tauri::async_runtime::JoinHandle<()> }`
     and the `Bridge` field `server: Mutex<Option<Server>>` (initially `None`). Its doc says
     what it holds: the accept task and a weak handle to close the endpoint, which that task
     owns.
   - `Bridge::set_server(&self, server: Server)` and `Bridge::close_server(&self)`:
     `close_server` takes the server and, if it was there, upgrades the `Weak` and calls
     `close()` (Unix: remove the socket, then release the lock), then `task.abort()`.
   - `fn listen(path: Result<PathBuf, PathError>) -> (ControlInfo, Option<Endpoint>)`:
     - `Err(e)` gives `ControlInfo::new(SocketStatus::Disabled, None, Some(e.to_string()))`,
       as today;
     - `Bind::Listening(endpoint)` gives `(Listening, Some(&path), None)` with
       `Some(endpoint)`;
     - `Bind::InUse(detail)` gives `(InUse, Some(&path), Some(detail))`;
     - `Bind::Disabled(detail)` gives `(Disabled, Some(&path), Some(detail))`.

     Its doc says it needs a tokio runtime context, as today's does (`:574`).
   - `async fn serve(endpoint: Arc<Endpoint>, bridge, emitter, snapshotter)`, doc "The accept
     loop: one task per connection, no idle timeout, no connection limit." (today's `:445`):

     ```rust
     let mut clients = tokio::task::JoinSet::new();
     loop {
         let accepted = tokio::select! {
             accepted = endpoint.accept() => accepted,
             _ = clients.join_next(), if !clients.is_empty() => continue,
         };
         match accepted {
             Ok(stream) => {
                 clients.spawn(serve_connection(stream, Arc::clone(&bridge), Arc::clone(&emitter), Arc::clone(&snapshotter)));
             }
             Err(error) => {
                 match &error {
                     AcceptError::Accept { source: e, .. } => eprintln!("control: accept failed: {e}"),
                     AcceptError::NextInstance { source: e, .. } => {
                         eprintln!("control: cannot create next pipe instance: {e}")
                     }
                 }
                 if error.closed() {
                     return;
                 }
                 if let Some(pause) = error.pause() {
                     tokio::time::sleep(pause).await;
                 }
             }
         }
     }
     ```

     The two `eprintln!` literals are byte-identical to `:468` and `:695`. Keep the binding
     named `e` so the literal inventory finds them.
   - `start` (`:713-749`) keeps its signature and doc. It builds the emitter and the
     snapshotter first (`TauriEmitter(app.clone())`, `TauriSnapshotter(app.clone())`, the two
     lines `:737-739` today; they are plain wrappers with no side effect, so building them
     when the bind fails is invisible), then calls
     `start_on(bridge, strudeland_ctl::socket_path(), emitter, snapshotter)`.
   - New `fn start_on(bridge: Arc<Bridge>, path: Result<PathBuf, PathError>, emitter: Arc<dyn RequestEmitter>, snapshotter: Arc<dyn Snapshotter>)`
     holds the rest of today's body, in today's order: enter the runtime (with the comment
     `:716-717`), `listen(path)`, the three-arm log `match` (moved, every literal
     byte-identical), `bridge.set_info(info)`. Then, when `listen` gave an endpoint:
     `let endpoint = Arc::new(endpoint);`, take a `Weak` with `Arc::downgrade`, spawn
     `serve(endpoint, …)` with `tauri::async_runtime::spawn`, and call
     `bridge.set_server(Server { endpoint: weak, task })`. Doc: "`start` without the
     `AppHandle`: binds `path`, logs the status, and serves it on the tauri runtime." It is
     the seam the start-path test drives.
   - `shutdown` (`:760-772`): `app.state::<Arc<Bridge>>().close_server();`. Doc: "Closes the
     endpoint on exit (Unix: removes the socket, then releases the lock) and stops the accept
     task."
   - The module doc (`:7-9`): "the listener setup is the free function [`bind_socket`], tested
     on a tempdir path" becomes "the endpoint is `strudeland_ctl::server`, which binds,
     verifies and closes it and is tested there".
4. **Tests in `control.rs`:**
   - Delete the items that 001 copied into ctl: the 14 names of 001 step 2, except
     `short_tempdir`, which the serve tests still use. `mode` goes. Delete
     `remove_socket_only_when_listening` (`:1836-1857`): only a `Listening` bind yields an
     `Endpoint`, so the rule holds by type, and ctl's `close` tests cover the removal.
   - `listen_reports_each_status`: rename the `listener` bindings to `endpoint`. Every
     assertion stays byte-identical, including 001's exact in-use text.
   - `a_connection_serves_a_screenshot` (`:1207-1237`) and `serve_on` (`:1859-1874`): call
     `listen(Ok(path))`, `assert_eq!(info.status, SocketStatus::Listening)`, then
     `tokio::spawn(serve(Arc::new(endpoint.unwrap()), …))`, without the `_lock` juggling.
     Every request, response and assertion stays byte-identical.
   - New `start_path_serves_and_closes_a_temp_endpoint`, a plain `#[test]` (not
     `#[tokio::test]`: `start_on` enters the tauri runtime itself, and the blocking client
     must not run on a runtime thread). The path is `short_tempdir().path().join("c.sock")` on
     Unix and `\\.\pipe\strudeland-app-test-{pid}` (`std::process::id()`) on Windows; the
     two `cfg`s sit inside `mod tests`. Then:
     - `start_on(Arc::clone(&bridge), Ok(path.clone()), Arc::new(FakeEmitter::default()), Arc::new(FakeSnapshotter::default()))`,
       and `bridge.info().status` is `Listening`;
     - a blocking `strudeland_ctl::Client::connect(&path, Duration::from_secs(1))` and
       `call_raw("ping", json!({}))` give a result whose `["attached"]` is `false` (the app
       answers `ping` itself; nothing attached);
     - drop the client and call `bridge.close_server()`;
     - Unix: the socket file is gone, `{path}.lock` still exists, and `Client::connect` gives
       `CallError::NotRunning`. Windows: `close` does nothing and the abort drops the pipe
       asynchronously, so `Client::connect` is retried every 10 ms for up to 2 s until it
       gives `CallError::NotRunning`;
     - a second `bridge.close_server()` is a no-op (no panic; on Unix the lock file is still
       there).

     It drives the real spawn, so no `JoinHandle` is built by hand. It does not start the
     app, touch the user's app data or kill a process.
   - Targeted: `cargo test --locked -p strudeland-app control::`.
5. **No cfg in app code.** `awk '/^mod tests/{exit} {print}' src-tauri/src/control.rs | grep -n 'cfg(' | grep -vE 'cfg\((all\([^)]*, )?test\)'`
   prints nothing. Only `cfg(test)` and `cfg(all(…, test))` are allowed, exactly; a
   `cfg(not(test))` or a `cfg(any(unix, test))` is reported. Log it as
   `$S/0131-002-no-cfg.log`, with `echo "exit $?"`: grep's exit 1 means nothing was found.
6. **Docs.** `AGENTS.md`, the `src-tauri/src/control.rs` bullet (`:190-194`): "the control
   socket listener; it bridges requests to the webview" becomes "the control socket server:
   it serves the endpoint `strudeland_ctl::server` binds, bridges requests to the webview".
   The rest of the bullet stays. In the ctl bullet 001 wrote, "behind its `server` feature"
   becomes "behind its `server` feature (on in the app only)", now that the app turns it on.
   Check that the rest of that bullet still reads true.
   `README.md` does not change: the paths and permissions it states are the same.
7. **ipc.** `control_attach` and `control_reply` are unchanged, so `bun run ipc:build` must
   leave `bindings.ts` byte-identical (gate step 13).

## Tests

- Deleted, and now in ctl since 001 with byte-identical bodies: the 8 bind tests, the 4 pins
  and `mode`.
- Deleted, its rule now held by type: `remove_socket_only_when_listening`.
- Adapted, every assertion unchanged: `listen_reports_each_status`,
  `a_connection_serves_a_screenshot`, `serve_on` and the four serve tests that use it
  (`:1882-1990`).
- New: `start_path_serves_and_closes_a_temp_endpoint`.
- Unchanged and passing: every other `control::tests` test, including the wire-fixture tests.

## Validation

Run [the full gate](000-prompt.md#the-full-gate) with `N=002`: steps 1-11, 13 and 14. Step 12
does not apply: the app copies are gone. Its proof was 001's. Also, before committing:

```sh
git diff --name-only HEAD > "$S/0131-002-files.log"
git diff --exit-code HEAD -- crates/strudeland-ctl Cargo.lock > "$S/0131-002-ctl-untouched.log" 2>&1
echo "exit $?" >> "$S/0131-002-ctl-untouched.log"
for side in - +; do
  git diff HEAD -- src-tauri/src/control.rs | grep "^$side" | grep -v "^$side$side" \
    | grep -o '"control: [^"]*"' | sort > "$S/0131-002-eprintln$side.log"
done
```

Run these before committing (`HEAD` is stage 001's commit).

- `files.log` lists only `AGENTS.md`, `src-tauri/Cargo.toml` and `src-tauri/src/control.rs`.
- `ctl-untouched.log` ends `exit 0`.
- `eprintln-.log` and `eprintln+.log` hold the `control: ` literals the diff removes and adds,
  quotes included. The `+` list is the `-` list with one `"control: accept failed: {e}"`
  fewer: the Unix and Windows copies become the one in the new `serve`, beside the one
  `"control: cannot create next pipe instance: {e}"`. The `listening on` and `disabled`
  literals appear on both sides or on neither, since they move from `start` into `start_on`;
  each literal is byte-identical.
- `literals.log` shows the 002 column.
- `no-cfg.log` ends `exit 1`.

## STOP

- A `control::` test that needs a changed assertion, request or expected text.
- A needed change to any `crates/strudeland-ctl/` file (fix it in a new 001-style change only
  after reporting).
- A `cfg(unix)`, `cfg(windows)` or `target_os` that the app code cannot do without.
- Anything in the shared STOP list.

## Commit message

```
refactor(app): serve the control socket through strudeland-ctl

The app binds, accepts and closes the control endpoint through
strudeland_ctl::server and keeps only the webview bridge, the
connection loop and its log lines, so control.rs has no
platform-specific code left. Every status, detail text, log line,
file mode and wire byte is unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
