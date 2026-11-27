# 0135 loss check

The one stage covers the scoped request and reverify's Decisions. The proposal's Out list keeps
everything that would change behaviour out of the stage on purpose: reverify §4's rows and
Decision 5.

| Plan item | Source | Stage |
|---|---|---|
| New `src-tauri/src/lifecycle.rs` with `on_reload(&QuitState, &Bridge, Option<&Takes>, Option<&PlaybackState>)` and `on_exit(Option<&PlaybackState>, Option<&Takes>, &Bridge)` | request, reverify Recommendation | 001 step 4 |
| `lib.rs` hooks call only lifecycle, in exactly today's order; filters and `try_state` skips kept | request, reverify §4 | 001 steps 4, 6; Validation table; gate 6 (lib.rs greps, per-call counts and per-function line order in `lifecycle.rs`) |
| Module docs: the order of both hooks and its reasons (from `lib.rs:37,115,119` and the callees' docs) | request | 001 step 4 |
| Decision 1 (reload does not cancel an export) in the docs, held by `on_reload`'s signature | Decisions | 001 step 4, test 3 note |
| Decision 2 (exit does not reset `QuitState` or release the permit) in the docs, held by `on_exit`'s signature; the reload half (detach keeps the permit) tested | Decisions, reverify §2 | 001 step 4, test 2 |
| Decision 3 (reload stops playback last) in the docs and pinned by the order test | Decisions | 001 step 4, test 5 |
| Decision 4 (watcher, piano roll host, instrument preview untouched) in the docs; none is a parameter | Decisions | 001 step 4 |
| Decision 5 (reload-then-close kills an export) | Decisions | out of scope: filed separately |
| Coverage tests over the real states, without an `AppHandle`: `QuitState::default`, `Bridge::new`, `PlaybackState::default`, `Takes` through the existing `TakesRig` | request | 001 step 5, tests 1 to 4 |
| Exit-order test only if the seam is at most about 20 lines (decided: a `#[cfg(test)]` step log, 18 lines; the reload order rides on the same seam) | request | 001 step 4 (STOP if over 20), test 5 |
| `control::shutdown` removed (it becomes unused); `close_server` made `pub`; `Manager` import dropped | request, proposal finding 1 | 001 step 2, gate 6 |
| Test-only helpers that the private state internals require (`QuitState`, `Bridge`) | proposal finding 2 | 001 steps 2, 3 |
| stderr lines identical; literal counts of every moved comment and every stderr text | request | gate 7, appendix |
| Transcript untouched (hash pinned) | request | 000 Rules, gate 6 |
| `bindings.ts` unchanged (`ipc:build` and an empty diff) | request | gate 5 |
| Gates: fmt, clippy `-D warnings`, `cargo test --locked -p strudeland-app`, `cargo test --locked --workspace` | request | gates 1 to 4 |
| Validation: list today's call order next to the new one | request | 001 Validation |
| STOP on any order or behaviour change, or a test that needs loosening | request | 000 STOP list |
| `AGENTS.md` names `lifecycle.rs` in the `src-tauri/` bullet | docs gate (pulled into the stage) | 001 step 7 |
