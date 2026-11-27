# 002: the app serves through `strudeland_ctl::server`

## Goal

`control.rs` binds with `server::bind`, accepts with `Endpoint::accept` in one
platform-independent loop, and closes with `Endpoint::close` on shutdown. The endpoint code and
the socket tests that 001 copied into ctl are deleted from the app. Afterwards `control.rs`
has no `cfg(unix)`, `cfg(windows)` or `target_os` outside test-only attributes. Every text,
stderr line, status, mode and wire byte is unchanged. This stage touches no ctl file.

## Prerequisites

- Stage 001 committed and green. Its `same-tests`, `literals` and `smoke-lines` logs are in
  `$S`.
- `$S/0131-001-smoke-lines.log` exists, or 001 reported the smoke as skipped. In that case
  this stage's smoke is a first look only, and the report says so.

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
   - `start` (`:713-749`): unchanged through `bridge.set_info(info)`. Then, when `listen` gave
     an endpoint: `let endpoint = Arc::new(endpoint);`, take a `Weak` with
     `Arc::downgrade`, spawn `serve(endpoint, …)` with `tauri::async_runtime::spawn`, and call
     `bridge.set_server(Server { endpoint: weak, task })`. The comment about entering the
     runtime (`:716-717`) stays.
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
   - New `#[cfg(unix)] close_server_removes_the_socket_and_frees_the_path`
     (`#[tokio::test]`): `listen` a path, then build a `Server` from that endpoint's `Weak`
     and a task from `tauri::async_runtime::spawn(serve(…))` that holds the `Arc`. After
     `bridge.close_server()`:
     - the socket file is gone, and `{path}.lock` still exists;
     - a second `listen` gives `Listening`;
     - a second `close_server()` does nothing.

     If `tauri::async_runtime::spawn` cannot run in a unit test, the task may come from
     `tokio::spawn`, but only if `Server.task` keeps its type. If neither works, drop this
     test and say so in the report. ctl's `close` tests still cover the removal.
   - Targeted: `cargo test --locked -p strudeland-app control::`.
5. **No cfg in app code.** `awk '/^mod tests/{exit} {print}' src-tauri/src/control.rs | grep -n 'cfg(' | grep -v test`
   prints nothing. Log it as `$S/0131-002-no-cfg.log`, with `echo "exit $?"`: grep's exit 1
   means nothing was found.
6. **Docs.** `AGENTS.md`, the `src-tauri/src/control.rs` bullet (`:190-194`): "the control
   socket listener; it bridges requests to the webview" becomes "the control socket server:
   it serves the endpoint `strudeland_ctl::server` binds, bridges requests to the webview".
   The rest of the bullet stays. Check that the ctl bullet 001 wrote still reads true.
   `README.md` does not change: the paths and permissions it states are the same.
7. **ipc.** `control_attach` and `control_reply` are unchanged, so `bun run ipc:build` must
   leave `bindings.ts` byte-identical (gate step 13).
8. **Smoke.** Repeat 001 step 9 exactly, with `N=002`. Then:
   - `diff "$S/0131-001-smoke-lines.log" "$S/0131-002-smoke-lines.log"` exits 0;
   - the `ping` and `modes` logs match 001's.

## Tests

- Deleted, and now in ctl since 001 with byte-identical bodies: the 8 bind tests, the 4 pins
  and `mode`.
- Deleted, its rule now held by type: `remove_socket_only_when_listening`.
- Adapted, every assertion unchanged: `listen_reports_each_status`,
  `a_connection_serves_a_screenshot`, `serve_on` and the four serve tests that use it
  (`:1882-1990`).
- New: `close_server_removes_the_socket_and_frees_the_path`.
- Unchanged and passing: every other `control::tests` test, including the wire-fixture tests.

## Validation

Run [the full gate](000-prompt.md#the-full-gate) with `N=002`: steps 1-11, 13 and 14. Step 12
does not apply: the app copies are gone. Its proof was 001's. Also, before committing:

```sh
git diff --name-only HEAD > "$S/0131-002-files.log"
git diff --exit-code HEAD -- crates/strudeland-ctl Cargo.lock > "$S/0131-002-ctl-untouched.log" 2>&1
echo "exit $?" >> "$S/0131-002-ctl-untouched.log"
git diff HEAD -- src-tauri/src/control.rs | grep '^[-+].*eprintln!' > "$S/0131-002-eprintln.log"
```

Run these before committing (`HEAD` is stage 001's commit).

- `files.log` lists only `AGENTS.md`, `src-tauri/Cargo.toml` and `src-tauri/src/control.rs`.
- `ctl-untouched.log` ends `exit 0`.
- `eprintln.log` holds exactly three `-` lines: the Unix and Windows `control: accept failed:
  {e}` and `control: cannot create next pipe instance: {e}`. It holds exactly their two `+`
  lines in the new `serve`, byte-identical apart from indentation. The `listening on` and
  `disabled` lines do not appear.
- `literals.log` shows the 002 column.
- `no-cfg.log` ends `exit 1`.
- `smoke-lines.log` is identical to 001's.

## STOP

- A `control::` test that needs a changed assertion, request or expected text.
- A needed change to any `crates/strudeland-ctl/` file (fix it in a new 001-style change only
  after reporting).
- A `cfg(unix)`, `cfg(windows)` or `target_os` that the app code cannot do without.
- Any smoke difference from 001.
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
