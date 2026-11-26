# 004: the app's refusal tests read the wire examples; docs

## Goal

`control.rs`'s tests take the two refusals Rust answers from the wire examples:
`screenshot`'s shape errors and `project.open`'s native absolute-path check. Both are then
checked against the same texts the other sides read.

AGENTS.md names the wire examples as the protocol's cross-check, replacing the regex-era wording,
and gets a Layout line. GLOSSARY.md gains "Wire example".

## Prerequisites

- Stage 003 committed.

## Files

- `src-tauri/Cargo.toml`: under `[dev-dependencies]`,
  `strudeland-ctl = { path = "../crates/strudeland-ctl", features = ["wire-fixtures"] }`.
- `src-tauri/src/control.rs`: two tests.
- `AGENTS.md`: the control-socket rule and the Layout entry.
- `GLOSSARY.md`: the App boundary section.

## Steps

1. **`screenshot_rejects_invalid_params`** (`control.rs:1051`):
   - Its first six cases (`[]`, `"x"`, the unknown `size`, `path: 3`, `path: null`,
     `"shot.png"`) come from `wire_fixtures::invalid(Method::Screenshot)`, as
     `(entry.params, RpcError::new(entry.code, entry.message))`.
   - The four directory cases stay inline: `missing`, `d`, `root` and `under_file`. Their
     messages name tempdir paths, so they depend on the filesystem.
   - Assert that the example list is non-empty, and that every entry has
     `answered_by == "app"`.
   - The loop, the bridge (not attached) and the `snap.paths().is_empty()` assert are unchanged.
2. **`project_open_checks_native_absolute_paths_before_forwarding`** (`:967`):
   - The refusal comes from the `project.open` invalid entries whose `answered_by == "app"`
     (exactly one).
   - The response must equal `RpcError::new(entry.code, entry.message)`, which is now the whole
     error and not only the code, and nothing is emitted.
   - The forwarded absolute-path half is unchanged.
3. **One guard** in the same test module: every `invalid` entry of every method that has
   `answered_by == "app"` belongs to `screenshot` or `project.open`. A new app-answered refusal
   then needs a Rust test here.
4. **AGENTS.md, Rules.** Replace the control-socket bullet's first sentences (`:347-350`) with:

   > - Only `src/lib/control/api.ts` calls Tauri for the control socket. `strudeland-ctl` owns the
   >   method table (`Method`, one `Rpc` marker per method), the error codes and the `events`
   >   limits; the CLI calls only through `Rpc` markers. The wire examples in
   >   `crates/strudeland-ctl/tests/wire/` (one JSON file per method, plus `errors.json` and
   >   `limits.json`) are the cross-check: Rust round-trips them through the typed params and
   >   results, the webview handler accepts every params example, refuses every invalid one with
   >   its exact code and message, and produces their result shape, and the CLI and app tests
   >   take their fake replies from them. `protocol.ts`'s `ErrorCode`, `METHOD_NAMES` and
   >   `handler.ts`'s limits are checked against them. The webview stays the only params
   >   validator (Rust checks only `screenshot` and `project.open`'s absolute path). A change to
   >   a method's params, result or refusal texts updates its example in the same change; a
   >   change to the `state` result's fields updates `methods.rs`, `state.json`, `protocol.ts`,
   >   `handler.ts`, the CLI formatter and `SKILL.md` together.

   Keep the rest of the bullet. `rg -n 'cross-check|regex' AGENTS.md` must show no stale
   mention of reading Rust source from a test for the control protocol.
5. **AGENTS.md, Layout.** Extend the `crates/strudeland-ctl/` entry (`:158`) with:

   > `tests/wire/` the golden wire examples (hand-written, prettier-formatted), read by
   > `tests/wire.rs`, the TS control tests (`tests/helpers/wire-examples.ts`) and, through the
   > dev-only `wire-fixtures` feature, the CLI's and the app's tests.

6. **GLOSSARY.md, App boundary.** Add after "IPC bindings":

   > **Wire example**
   >
   > One committed JSON example of a control method (`crates/strudeland-ctl/tests/wire/`): params
   > both sides accept, named results, and the state-independent refusals with their exact code
   > and message. Rust, the webview handler, the CLI and the app test against the same examples.
   > _Avoid_: fixture (any test input), golden (the render harness's), schema (nothing is
   > generated from them)

7. **Feature leakage check:** `cargo tree -p strudeland-app -e normal,features -i strudeland-ctl`
   prints no `wire-fixtures`.

## Tests

- Moved: the six state-independent screenshot cases and the `project.open` native refusal now
  read the examples. The `project.open` assert is tightened from the code alone to the full
  error.
- New: the app-answered guard.
- Unchanged: every other `control.rs` test.

## Validation

- The gates in [000-prompt.md](000-prompt.md), logged as `0120-004-*`.
- The `cargo tree` check in step 7, and stage 003's for `strudeland-cli` again.
- A last check over the branch: `git diff main --stat -- crates/strudeland-cli/skill/SKILL.md
  src/lib/control/protocol.ts` is empty, and `git diff main -- src/lib/control/handler.ts`
  shows only the two `export`s.

## Commit message

```
test(app): read the control refusals Rust answers from the wire examples

The screenshot and project.open native refusal tests take their params
and exact errors from the ctl wire examples, and a guard keeps every
app-answered example covered by a Rust test. AGENTS.md names the wire
examples as the control protocol's cross-check and the glossary defines
them.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
