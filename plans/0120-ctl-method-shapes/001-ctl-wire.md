# 001: ctl constants, `decode_result` and the golden wire examples

## Goal

`strudeland-ctl` gains the following, all described in the proposal's
"`strudeland-ctl`: constants and `decode_result`", "The wire examples" and "The `wire-fixtures`
feature" sections:

- `codes::ALL`, as a hand list (the macro comes in 002);
- `EVENTS_LIMIT_DEFAULT` and `EVENTS_LIMIT_MAX`;
- `decode_result::<M>`, used by `Client::call`;
- the `Serialize + DeserializeOwned` bounds on `Rpc`;
- the corrected `methods.rs` module doc;
- the 18 wire examples, `tests/wire.rs` and the `wire-fixtures` feature.

The hand round-trips in `methods.rs` are deleted, because the examples cover them. No other
crate changes yet.

## Prerequisites

- On `llm/andrew/strudeland/0120-ctl-method-shapes`, at the plan commit.
- Read [the proposal](../0120-ctl-method-shapes.md) and [reverify.md](reverify.md).

## Files

- `crates/strudeland-ctl/Cargo.toml`: `[features] wire-fixtures = []`.
- `crates/strudeland-ctl/src/protocol.rs`: `codes::ALL`; delete `codes_match_the_error_table`.
- `crates/strudeland-ctl/src/methods.rs`: the limits, the `Rpc` bounds, the module doc, the
  `EventsParams.limit` doc, and the pruned tests.
- `crates/strudeland-ctl/src/client.rs`: `decode_result`, and `Client::call` using it.
- `crates/strudeland-ctl/src/lib.rs`: re-export `decode_result`; add
  `#[cfg(any(test, feature = "wire-fixtures"))] pub mod wire_fixtures;`; one line in the crate
  doc naming the examples.
- `crates/strudeland-ctl/src/wire_fixtures.rs` (new).
- `crates/strudeland-ctl/tests/wire.rs` (new).
- `crates/strudeland-ctl/tests/wire/*.json` (18 new files).

## Steps

1. **`codes::ALL`.** Leave every `pub const` and the comment at `protocol.rs:10-12` exactly as
   they are, because `tests/control-protocol.test.ts` still scrapes them until 002. After the
   last const, add:

   ```rust
   /// Every code by its Rust name, in declaration order.
   pub const ALL: &[(&str, i64)] = &[
       ("PARSE_ERROR", PARSE_ERROR),
       // … one row per const above, in order …
       ("BUSY", BUSY),
   ];
   ```

   There are 17 rows. The `ALL` line itself is not `i64 = N;`, so the TS regex ignores it.
   Delete `codes_match_the_error_table`: `tests/wire.rs` pins every value through
   `errors.json`.
2. **Limits.** Beside `EventsParams` in `methods.rs`, add `pub const EVENTS_LIMIT_DEFAULT: u32 =
   2000;` and `pub const EVENTS_LIMIT_MAX: u32 = 100_000;`, with the docs from the proposal.
   `EventsParams.limit`'s doc becomes "Absent means [`EVENTS_LIMIT_DEFAULT`]." `lib.rs`'s
   `pub use methods::*` exports them.
3. **`Rpc` bounds.** `type Params: Serialize + DeserializeOwned; type Result: Serialize +
   DeserializeOwned;`. Every type already derives both, so this compiles without other edits.
4. **Module doc** (`methods.rs:1-6`). Write it as follows:

   > Every method of the control protocol, with its typed params and result.
   >
   > The webview validates params (`screenshot`'s are checked by the app). These types are what a
   > request the webview accepts decodes into and what a result parses into; the wire examples
   > in `tests/wire/` are the cross-check every side reads. Field names on the wire are
   > camelCase; absent optional request fields are omitted, while nullable result fields are
   > `null`.

5. **`decode_result`** in `client.rs`:

   ```rust
   /// `M`'s typed result from a raw `result`; `CallError::Protocol("invalid {method} result: {e}")`
   /// when it does not match. The one place that text is built.
   pub fn decode_result<M: Rpc>(result: &serde_json::Value) -> Result<M::Result, CallError> {
       <M::Result as serde::Deserialize>::deserialize(result)
           .map_err(|e| CallError::Protocol(format!("invalid {} result: {e}", M::METHOD.as_str())))
   }
   ```

   The fully qualified call names the trait, so nothing depends on which traits are in scope
   (`M::Result` is also `Serialize`). `Client::call` becomes
   `let result = self.call_raw(…)?; decode_result::<M>(&result)`.
   Re-export it from `lib.rs`: `pub use client::{CallError, Client, decode_result};`.
6. **The examples.** Write the 18 files under `crates/strudeland-ctl/tests/wire/` to the
   proposal's schema. The contents per file:
   - **`ping.json`:** `answeredBy: app`. Params `empty` `{}`. Result `attached`
     `{ "version": "0.1.0", "attached": true }`. `invalid: []`.
   - **`state.json`:**
     - params `empty` `{}`;
     - result `full`, as the proposal lists it;
     - result `minimal` (`"older": true`), with exactly `run.rs`'s `state_json()` values
       (`run.rs:930-941`);
     - invalid: `[]` → "invalid params: expected an object", and `{ "verbose": true }` →
       "invalid params: unknown parameter 'verbose'".
   - **`code.get.json`:**
     - params `{}`, `{ "song": "a" }` and `{ "song": "songs/b.js" }`;
     - result `example`
       `{ "song": "songs/main.js", "title": "Main Theme", "code": "$: s(\"bd\")\n", "dirty": false }`;
     - invalid:
       - `[]` → "expected an object";
       - `{ "song": "a", "foo": 1 }` → "unknown parameter 'foo'";
       - `{ "song": "" }` and `{ "song": 3 }` → "'song' must be a non-empty string".

     (Every message here and below starts with `invalid params: `.)
   - **`code.set.json`:**
     - params:
       - `full`: `{ "song": "a", "code": "$: s(\"bd sd\")\n", "eval": "never", "expect": "$: s(\"bd\")\n" }`;
       - `minimal`: `{ "code": "x" }`;
     - results:
       - `evaluated`: `methods.rs:1193`'s value;
       - `unevaluated`: `methods.rs:1214`'s value;
     - invalid:
       - `"x"` → "expected an object";
       - `{}` and `{ "code": 1 }` → "'code' must be a string";
       - `{ "code": "", "eval": "alwys" }` → `'eval' must be "auto", "always" or "never"`;
       - `{ "code": "", "expect": 3 }` → "'expect' must be a string".
   - **`eval.json`:**
     - params `{}` and `{ "song": "b" }`;
     - results `ok` and `failed`: `methods.rs:1239` and `:1228`'s values;
     - invalid: `[]` → "expected an object"; `{ "song": 3 }` → "'song' must be a non-empty
       string".
     - No `null` entry. `parse_request` (`frame.rs:147-149`) turns absent or `null` params into
       `{}` before anything answers, so `null` never reaches the handler over the wire.
       `control-handler.test.ts`'s `null` row stays a handler-only hand test (stage 002).
   - **`play.json`:** the same params and results as `eval.json`. Invalid: `{ "song": "" }` →
     "'song' must be a non-empty string".
   - **`stop.json`:** params `{}`. Result `{ "wasPlaying": true }`. Invalid: `{ "now": true }` →
     "unknown parameter 'now'"; `1` → "expected an object".
   - **`events.json`:**
     - params:
       - `default`: `{}`;
       - `bars`: `{ "bars": { "from": 2, "to": 4 }, "all": true, "limit": 10 }`;
       - `section`: `{ "section": "intro" }`;
     - results:
       - `stale`: exactly `run.rs:1363-1376`'s `events_json()`;
       - `truncated`: exactly `e2e.rs:144-153`'s reply;
     - invalid: the 16 rows of `control-handler.test.ts:975-1007`, one entry each, with the
       same params and the text `invalid params: <detail>`.
   - **`screenshot.json`:**
     - `answeredBy: app`;
     - params `{}` and `{ "path": "/tmp/a.png" }`;
     - result `{ "path": "/tmp/a.png", "width": 1600, "height": 1000 }`;
     - invalid, the state-independent rows of `control.rs:1060-1083`:
       - `[]` and `"x"` → "expected an object";
       - `{ "path": "/tmp/a.png", "size": 2 }` → "unknown parameter 'size'";
       - `{ "path": 3 }` and `{ "path": null }` → "'path' must be a string";
       - `{ "path": "shot.png" }` → "'path' must be absolute, got shot.png".
   - **`song.new.json`:**
     - params `{ "name": "intro" }`;
     - result `{ "song": { "path": "songs/a.js", "title": "a" } }`;
     - invalid: `{}` and `{ "name": 1 }` → "'name' must be a string";
       `{ "name": "x", "foo": 1 }` → "unknown parameter 'foo'".
   - **`song.switch.json`:**
     - params `{ "song": "b" }`;
     - result as `song.new`'s;
     - invalid: `{}` → "'song' is required"; `{ "song": "a", "x": 1 }` → "unknown parameter
       'x'".
   - **`song.rename.json`:**
     - params `{ "name": "intro" }` and `{ "song": "b", "name": "intro" }`;
     - result `methods.rs:886`'s value;
     - invalid: `{ "name": "x", "foo": 1 }` → "unknown parameter 'foo'"; `{ "song": "", "name":
       "x" }` → "'song' must be a non-empty string".
   - **`song.delete.json`:**
     - params `{}` and `{ "song": "b", "discard": true }`;
     - result `methods.rs:893`'s value;
     - invalid: `{ "x": 1 }` → "unknown parameter 'x'".
   - **`save.json`:**
     - params `{}`, `{ "song": "a" }` and `{ "all": true }`;
     - results `empty` `{ "saved": [] }` and `one` `{ "saved": [{ "path": "songs/a.js", "title": "a" }] }`;
     - invalid:
       - `{ "all": 1 }` → "'all' must be a boolean";
       - `{ "x": 1 }` → "unknown parameter 'x'";
       - `{ "song": "a", "all": true }` → "'song' and 'all' are mutually exclusive".
   - **`project.open.json`:**
     - params `{ "path": "/empty" }` and `{ "path": "/empty", "discard": true }`;
     - result `methods.rs:919`'s value;
     - invalid:
       - `{}` → "'path' must be a string";
       - `{ "path": "/p", "discard": 1 }` → "'discard' must be a boolean";
       - `{ "path": "/p", "x": 1 }` → "unknown parameter 'x'";
       - `{ "path": "relative" }`, with `"answeredBy": "app"` → "'path' must be an absolute
         path".
   - **`instrument.apply.json`:**
     - params `{ "root": "/p", "song": "songs/a.js", "id": "lead", "from": 8, "to": 18, "expect": "x" }`;
     - result `example`: `{ "song": "songs/main.js", "from": 8, "to": 48 }` (`instrument_mcp.rs`'s
       reply);
     - invalid:
       - `{ "id": "lead", "from": 0, "to": 1, "expect": "" }` → "'root' must be a string";
       - `{ "root": "/p", "id": "lead", "from": 5, "to": 5, "expect": "" }` and the same with
         `"from": -1` → "'from' and 'to' must be an increasing range of UTF-16 offsets";
       - the first params with `"x": 1` added → "unknown parameter 'x'".
   - **`errors.json`:** every `codes` const by name.
   - **`limits.json`:** `{ "eventsLimitDefault": 2000, "eventsLimitMax": 100000 }`.

   No entry anywhere has top-level `null` params, for the reason given under `eval.json`. A
   nested `null` (`screenshot`'s `{ "path": null }`) is sent as written.

   Copy each message from the code (`handler.ts`, `control.rs`), never from memory. Stage 002's
   TS test and stage 004's Rust test run every entry against the real handler. A wrong text fails
   there, but it costs less to get it right now. Run `bun run format`, then check that
   `git diff -- crates/strudeland-ctl/tests/wire/` shows no changed value: prettier keeps `1.0`
   as written but rewrites `1.50` as `1.5`.
7. **`wire_fixtures.rs`**, to the proposal's interface:

   ```rust
   pub struct Invalid { pub name: String, pub params: Value, pub code: i64, pub message: String,
                        /// The method's `answeredBy` unless the entry overrides it.
                        pub answered_by: String }
   ```

   `text(m)` is a `match` over every `Method` with no `_` arm, returning
   `include_str!("../tests/wire/<name>.json")`. `params` and `result` panic with
   `"no {kind} example '{name}' for {method}"` when the name is missing. Add a `#[cfg(test)]`
   unit test asserting that `params(Method::Events, "bars")["limit"] == 10` and that
   `invalid(Method::ProjectOpen)` has exactly one `answered_by == "app"` entry.
8. **`tests/wire.rs`**, to the proposal's "`tests/wire.rs` (ctl)" section:
   - `same(a, b)`, with its unit cases from the proposal's Risks section;
   - the file set test;
   - `check(m, ex)` with no `_` arm, and `check_rpc::<M>`;
   - the `errors.json` and `limits.json` tests;
   - the `state` `full` completeness test.

   Read the files with `std::fs` from `concat!(env!("CARGO_MANIFEST_DIR"), "/tests/wire")`, not
   through `wire_fixtures`.
9. **Prune `methods.rs`'s tests.**
   - Delete (their values are now examples, round-tripped by `tests/wire.rs`):
     `markers_carry_their_method`, `ping_result`, `STATE_EXAMPLE`, `state_result_example`,
     `state_with_engine_and_live_round_trips`, `state_with_levels_round_trips`,
     `state_result_with_a_seed_round_trips`, `state_result_with_a_conflict`,
     `state_with_playhead_and_transport`, `code_get_result_example`, `code_set_result_example`,
     `code_set_result_without_evaluate`, `eval_outcome_examples`, `code_set_params_full_example`,
     `code_set_params_omit_absent_optionals`, `song_params_omit_an_absent_song`,
     `song_delete_params_omit_absent_optionals`, `save_params_omit_absent_optionals`,
     `save_result`, `events_params_omit_absent_fields`, `events_result_round_trips`,
     `location_is_camel_case` and `screenshot_types`.
   - Trim to their refusals or behaviour (drop the `round_trip` calls, keep the `is_err` and
     default asserts):
     - `song_new_and_switch_params`, `song_rename_params`, `project_open_params`,
       `song_results` and `stop_result_is_camel_case`;
     - `project_open_result_uses_already_open_key` (keep the snake_case refusal);
     - `empty_params` (keep "unknown members are not rejected in Rust");
     - `playhead_and_transport_round_trip` (keep the default view and the lowercase enum
       asserts, and rename it `transport_view_defaults_to_song`).
   - Keep as they are: `method_names_round_trip`, `unknown_method_is_none`,
     `code_set_params_need_code`, `enums_are_lowercase`, `state_result_without_a_project`,
     `state_result_missing_a_field_is_an_error`, `event_before_the_first_bar_has_a_non_positive_bar`
     and `event_without_a_section_or_locations`.
   - `state_without_engine_and_live_defaults_them` and `state_without_the_new_fields_defaults_them`
     merge into one `older_state_defaults_the_new_fields`. It parses
     `wire_fixtures::result(Method::State, "minimal")` and keeps every default assert of both.
   - `example_event()` stays if a kept test uses it. `round_trip` goes if nothing uses it.
10. **`client.rs` test.** Add a test outside the unix-only module:
    `decode_result::<StateCall>(&json!({"project": null}))` is
    `Err(CallError::Protocol(m))`, where `m` is "invalid state result: missing field `songs`".
    If serde's wording differs, pin what it prints: this is the text the CLI shows today. Also
    `decode_result::<Stop>(&json!({"wasPlaying": true}))` is `Ok(StopResult { was_playing:
    true })`.

## Tests

- New: `tests/wire.rs`, the `wire_fixtures` unit test, and the `decode_result` test.
- Deleted or trimmed: as listed in step 9, plus `codes_match_the_error_table`.
- Unchanged and still passing: `control-protocol.test.ts`. Its regexes still find the 17 consts,
  the struct fields and the `Method::X => "…"` arms.

## Validation

- The gates in [000-prompt.md](000-prompt.md), logged as `0120-001-*`.
- `cargo test -p strudeland-ctl --test wire` lists every method. To confirm the dispatcher
  catches a mismatch, temporarily rename one example's `method` and see it fail, then revert.
- `cargo build -p strudeland-ctl` (no feature) builds without `wire_fixtures`.

## Commit message

```
test(ctl): golden wire examples for every control method

Add one JSON example per method (params, results, state-independent
refusals with their exact texts), plus the error codes and the events
limits, under crates/strudeland-ctl/tests/wire. A ctl test round-trips
them through each method's typed params and result; a dev-only
wire-fixtures feature exposes them to other crates' tests. ctl gains
codes::ALL, the events limit constants and decode_result, the one
place the invalid-result text is built. The hand round-trips the
examples replace are removed.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
