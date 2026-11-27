# 0135: one app lifecycle module

## Goal

`src-tauri/src/lib.rs` runs two lifecycle hooks inline: the main webview's reload
(`on_page_load`, `:38-49`) and the app exit (`RunEvent::Exit`, `:113-125`). Together they make
seven calls into four modules. Nothing tests them, and nothing records the asymmetries between
them (export, `QuitState`, the playback order).

After this change, a new `src-tauri/src/lifecycle.rs` holds `on_reload` and `on_exit` over
references to the managed states. Its module docs state the order of each hook, the reasons
behind it, and Decisions 1 to 4. `lib.rs` keeps the event filters and the state lookups, and
calls one function per hook. Tests build the real states without an `AppHandle`. They check
that each hook resets each state and that the calls run in the documented order.

**No behaviour change.** The calls, their order, the `main`/`Started` filter, the
"state not managed yet" skips, stderr and the IPC bindings all stay as they are.

## Background

[reverify.md](reverify.md) holds the evidence at `00feb37`. Its **Decisions** section is
binding. Re-checked at `00feb37` while planning, and every file:line in reverify §1 still holds.
Three findings correct or extend it:

1. **`Bridge::close_server` is private to `control`** (`control.rs:169`, a plain `fn`). A sibling
   module cannot call it, so reverify §3's "private, same crate" is wrong. It becomes `pub fn`,
   like the other `Bridge` methods (the struct sits in a private module, so this widens nothing
   outside the crate). Removing `control::shutdown` (`:562-566`) then leaves `Manager` unused in
   `control.rs:26`'s import. Its only use is `:565`.
2. **The test helpers the coverage tests need are private.** `QuitState`'s `set_ready`,
   `request`, `reply` and `lock` are private to `app_quit` (`app_quit.rs:26-72`). `Bridge` has
   no way to see whether a server is held. `src-tauri/src/recording/rig.rs` is `pub(super)`
   inside `recording`. The plan adds a small `#[cfg(test)] pub(crate)` helper block to
   `app_quit.rs` and to `control.rs`. For takes it uses the record crate's
   `strudeland_record::testing::TakesRig` directly. That type is always public, and
   `recording/rig.rs` wraps the same rig, so `recording/` stays untouched.
3. **An order seam fits in about 16 lines,** so the order test is in. A `#[cfg(test)]`
   thread-local step log takes about 9 lines, and one `step("…")` call follows each of the 7
   calls. In the app build `step` is an empty function. No production call changes or moves.

## Scope

In (one stage, [001-app-lifecycle-module.md](001-app-lifecycle-module.md)):

- New `src-tauri/src/lifecycle.rs`:
  - `on_reload(&QuitState, &Bridge, Option<&Takes>, Option<&PlaybackState>)`;
  - `on_exit(Option<&PlaybackState>, Option<&Takes>, &Bridge)`;
  - the module docs, the step log and the tests.
- `lib.rs`: `mod lifecycle;`, and the two hooks reduced to filters, lookups and one call each.
  The comment at `:37` stays. The two exit comments (`:115`, `:119`) move into `on_exit`
  verbatim.
- `control.rs`: `close_server` becomes `pub`. `shutdown` and its doc are deleted, and
  `Manager` leaves the import. A `#[cfg(test)]` helper block is added: `serve_idle`, `serving`.
- `app_quit.rs`: a `#[cfg(test)]` helper block (`asking`, `approved`, `view`).
- `AGENTS.md`: the `src-tauri/` bullet names `lifecycle.rs`.

Out:

- Any behaviour change, including all of reverify §4's "would change behaviour" rows:
  - cancelling an export on reload (Decision 1);
  - resetting `QuitState` or releasing the permit on exit (Decision 2);
  - aligning the reload order with the exit order (Decision 3);
  - stopping the watcher, the piano roll host or the instrument preview (Decision 4).
- The reload-then-close gap that kills an export (Decision 5, filed separately).
- `playback.rs`, `recording.rs`, `recording/rig.rs`, `recording/transcript.rs`,
  `export_audio.rs`, the record crate, every TS file, `bindings.ts`, `SKILL.md`.

## Design

```rust
// lifecycle.rs (shape; docs in 001)
pub fn on_reload(
    quit: &QuitState,
    bridge: &Bridge,
    takes: Option<&Takes>,
    playback: Option<&PlaybackState>,
) {
    quit.detach();                                  step("quit");
    bridge.detach();                                step("bridge");
    if let Some(t) = takes { t.on_reload();         step("takes"); }
    if let Some(p) = playback { playback::end_session(p); step("playback"); }
}

pub fn on_exit(playback: Option<&PlaybackState>, takes: Option<&Takes>, bridge: &Bridge) {
    // Audio stops first.
    if let Some(p) = playback { playback::end_session(p); step("playback"); }
    // Before the control socket, so a slow socket cannot delay the audio.
    if let Some(t) = takes { t.shutdown();          step("takes"); }
    bridge.close_server();                          step("bridge");
}
```

`lib.rs` looks every state up before the call: `try_state` for the recorder and playback, and
`state` for `QuitState` and the bridge, as today. Today the lookups are interleaved with the
calls. They are pure map reads with no side effects, so doing them first reorders no call. The
`state` lookups still panic if a state is missing, as today, and that cannot happen because
both states are managed before `build`.

## Risks

- **An order or behaviour slip.** Four guards cover it:
  - the order test;
  - a hook grep showing `lib.rs` makes no hook call of its own;
  - literal counts of the moved comments and every stderr text;
  - a listed review of the call order (001 Validation).
- **The step log reads as production code.** It is a `#[cfg(test)]` thread-local with a
  `#[cfg(not(test))]` empty twin, and it sits next to the calls it names. If it needs more than
  20 lines, the order test is dropped and the docs carry the order. That is a STOP-and-report
  case, not a silent workaround.
- **Thread-local state across tests.** Each test thread has its own log. The order test clears
  it with `take()` before each hook.

## Open questions

- Is the `step` marker acceptable in production function bodies, as a no-op in the app build?
  If not, drop the order test and keep only the coverage tests.
- Should the `AGENTS.md` line go in now, or at the docs gate? The plan puts it in the stage
  commit because it is one phrase.
