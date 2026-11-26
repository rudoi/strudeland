# 003: the CLI calls through the typed table

## Goal

Every CLI request goes through `call::<M: Rpc>` or `report::<M>`. The method, params type and
result type then come from one marker, and the invalid-result text comes from ctl's
`decode_result`.

The `--limit` range uses `EVENTS_LIMIT_MAX`. A test pins the help text and `SKILL.md` to both
limits.

The CLI tests that fake a state, events or `instrument.apply` reply read it from the wire
examples. Every output, flag and exit code stays as it is.

## Prerequisites

- Stage 002 committed.

## Files

- `crates/strudeland-cli/Cargo.toml`: under `[dev-dependencies]`,
  `strudeland-ctl = { path = "../strudeland-ctl", features = ["wire-fixtures"] }`.
- `crates/strudeland-cli/src/run.rs`: the typed `call`, `report` and `eval_like`; the 15 call sites in
  this file (12 `report`s, the one in `eval_like`, `code get` and `offline_project`); the `state_json` and `events_json` readers.
- `crates/strudeland-cli/src/run/instruments.rs`: the `instrument.apply` site and its imports.
- `crates/strudeland-cli/src/cli.rs`: the `--limit` range, the range test and the limits test.
- `crates/strudeland-cli/tests/e2e.rs`: the state and events replies.
- `crates/strudeland-cli/tests/instrument_mcp.rs`: the `instrument.apply` reply.

`SKILL.md` and the help text are not edited.

## Steps

1. **The typed core** in `run.rs`. Replace `call<T>` (`:158-166`), `params()` (`:168-170`) and
   `report<T>` (`:186-196`) with the signatures in the proposal's "CLI typed calls" section:

   ```rust
   fn call<M: Rpc>(transport: &mut dyn Transport, params: &M::Params)
       -> Result<(Value, M::Result), Failure> {
       let params = serde_json::to_value(params).expect("params serialize");
       let raw = transport.call(M::METHOD, params)?;
       let typed = ctl::decode_result::<M>(&raw)?;
       Ok((raw, typed))
   }
   ```

   `report::<M>` calls it, then `emit`. Keep `emit` and `stdout_only` as they are. Use
   `use strudeland_ctl as ctl;`, and write `ctl::Ping`, `ctl::Play` and `ctl::Stop` (and any
   marker you prefer) as paths, so they cannot be confused with `Command::Ping`,
   `Command::Play` and `Command::Stop`. Drop the `serde::de::DeserializeOwned` import if nothing
   uses it.
2. **The 16 call sites** (15 in `run.rs`, one in `instruments.rs`; 17 marker uses, because
   `eval_like` serves both `EvalCall` and `Play`):

   | Site | Becomes |
   |---|---|
   | `Command::Ping` | `report::<ctl::Ping>(…, &ctl::Empty {}, \|r\| …)`, keeping the `not_ready` capture |
   | `State` | `report::<ctl::StateCall>(…, &ctl::Empty {}, …)` |
   | `Stop` | `report::<ctl::Stop>(…, &ctl::Empty {}, …)` |
   | `Events` | `report::<ctl::EventsCall>(…, &EventsParams { … }, output::format_events)` |
   | `Screenshot` | `report::<ctl::ScreenshotCall>(…)` |
   | `song new` | `report::<ctl::SongNewCall>(…)` |
   | `song switch` | `report::<ctl::SongSwitchCall>(…)` |
   | `song rename` | `report::<ctl::SongRenameCall>(…)` |
   | `song delete` | `report::<ctl::SongDeleteCall>(…)` |
   | `Save` | `report::<ctl::SaveCall>(…)` |
   | `project open` | `report::<ctl::ProjectOpenCall>(…)` |
   | `code set` | `report::<ctl::CodeSet>(…)` |
   | `Eval` and `Play` | `eval_like::<ctl::EvalCall>(…)` and `eval_like::<ctl::Play>(…)`, with `eval_like<M: Rpc<Params = SongParams, Result = EvalOutcome>>` |
   | `code get` | `let (raw, r) = call::<ctl::CodeGet>(transport, &SongParams { … })?;` |
   | `offline_project` | `call::<ctl::StateCall>(transport, &ctl::Empty {})` |
   | `instruments.rs:115` | `let (_, result) = call::<InstrumentApply>(self.transport, &args)?;` |

   Remove `params` from `instruments.rs`'s `use super::{…}`.

   No `Method::X` remains in `run.rs` outside tests and `Transport` itself
   (`rg -n 'Method::' crates/strudeland-cli/src/run.rs crates/strudeland-cli/src/run/` shows only
   tests and the trait).
3. **The limits in `cli.rs`.**
   - Change `:75` to `value_parser = clap::value_parser!(u32).range(1..=i64::from(strudeland_ctl::EVENTS_LIMIT_MAX))`.
     Leave `:74`'s doc comment text as it is, and leave `:168` (`render --length`) alone.
   - The range test (`:1184-1187`) uses `EVENTS_LIMIT_MAX.to_string()` and
     `(EVENTS_LIMIT_MAX + 1).to_string()` in place of `"100000"` and `"100001"`. Keep `"0"`,
     `"-1"` and `"x"`.
   - Add `events_limit_docs_name_the_ctl_limits`:
     - Find `Cli::command()`'s `events` subcommand and its `limit` argument. Its
       `get_help().unwrap().to_string()` contains `format!("1..={EVENTS_LIMIT_MAX}")` and
       `format!("default {EVENTS_LIMIT_DEFAULT}")`.
     - `crate::skill::SKILL`'s line containing `` `--limit N` `` contains
       `format!("1 to {EVENTS_LIMIT_MAX}")` and `format!("default {EVENTS_LIMIT_DEFAULT}")`.

     Put it in `cli.rs`'s test module, or in `skill.rs`'s if `SKILL` is private there.
4. **The fixture-fed CLI tests:**
   - **`run.rs`:**
     - `state_json()` becomes
       `strudeland_ctl::wire_fixtures::result(Method::State, "minimal")`.
     - `events_json()` becomes `…result(Method::Events, "stale")`.
     - The callers that patch `["project"]` (`:2006`, `:2379`, `:2393`) keep working unchanged.
   - **`e2e.rs`:**
     - `state_against_a_fake_server`'s inline reply (`:71-77`) becomes `result(Method::State,
       "minimal")`. Its expected stdout becomes exactly `state_human_and_json`'s:

       ```
       "project: demo (/Users/me/demo)\nsongs:\n  ● main   \"Main Theme\"   active, playing\n    verse  \"Verse\"        conflict: changed\ntempo: 120 bpm 4/4\naudio: running\nplaying: Main Theme\n"
       ```

     - `events_against_a_fake_server`'s `result` (`:144-153`) becomes `result(Method::Events,
       "truncated")`. Its expected stdout and stderr (`:158-165`) are unchanged.
   - **`instrument_mcp.rs`:** the reply at `:217` becomes `result(Method::InstrumentApply,
     "example")`. The assert at `:225-228` compares against the same value; either keep the
     literal or reuse the fixture value.
   - **Imports in `e2e.rs` and `instrument_mcp.rs`.** Their readers sit in `#[cfg(unix)]` tests.
     An ungated `use strudeland_ctl::{Method, wire_fixtures};` is unused on Windows and fails
     `cargo clippy --all-targets -- -D warnings` there (`.github/workflows/native.yml`). Gate the
     import with `#[cfg(unix)]`, as both files already do for their other unix-only imports, or
     import inside the tests. macOS clippy cannot catch this: check it by reading.

   Run each of these tests before step 4 and after, and confirm that the asserted strings are the
   ones written above. That is the check that the example values match what the tests print.
5. **Feature leakage check:**
   `cargo tree -p strudeland-cli -e normal,features -i strudeland-ctl` prints no
   `wire-fixtures`. With the dev edges included (`-e all`) it does. As a fallback that does not
   depend on reading the inverted tree,
   `cargo tree -p strudeland-cli -e normal -f '{p} {f}' | grep wire-fixtures` prints nothing.

## Tests

- Moved: `state_json`, `events_json`, the two `e2e.rs` replies and the `instrument_mcp.rs` reply
  now read the examples. Their assertions are unchanged, except `e2e.rs`'s state stdout, which
  is written above.
- New: `events_limit_docs_name_the_ctl_limits`.
- Unchanged and passing:
  - every `run.rs` test that asserts `f.calls`: the typed `Empty {}` and params types serialize
    to the same `Value`s;
  - `malformed_result_is_an_app_error` ("error: protocol error: invalid state result…");
  - the `Fake` and `NoTransport` transports.

## Validation

- The gates in [000-prompt.md](000-prompt.md), logged as `0120-003-*`.
- The `cargo tree` check in step 5.
- `git diff --stat crates/strudeland-cli/skill/SKILL.md` is empty.

## Commit message

```
refactor(cli): call every control method through its typed Rpc marker

The CLI's call and report helpers take a method marker, so each
request's params and result types come from the ctl table and the
invalid-result text from ctl's decode_result. The events --limit range
uses ctl's limit, and a test pins the help text and the skill to both
limits. The CLI tests take their state, events and instrument apply
replies from the ctl wire examples. Output, flags and exit codes are
unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
