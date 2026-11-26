# 0120: one owner for the control methods' shapes, checked by golden wire examples

## Goal

`strudeland-ctl` owns the control protocol's method table, its types, its error codes and the
`events` limits. One set of committed **wire examples** (one JSON file per method) is the
cross-check every side reads:

- Rust round-trips the examples through each method's `Params` and `Result` types.
- The webview handler accepts every params example and refuses every invalid example with the
  exact code and message. Its real results have the examples' key shape.
- The CLI's and the app's tests take their fake replies from the examples.

The CLI calls every method through the typed `Rpc` table, not by `Method` plus an untyped
`Value`.

**Nothing on the wire changes.** Every request accepted today is accepted, and every refusal
keeps its code and message. Every CLI output, flag and exit code is unchanged, and `SKILL.md`'s
text is unchanged.

## Background

The [re-verification](0120-ctl-method-shapes/reverify.md) has the evidence, with file and line
at `3a189da`. In short:

- **The typed table has one caller.** `Rpc` (`methods.rs:90-94`) and its 16 marker impls are
  used only by `Client::call::<M>`, which only its own tests call. The CLI calls
  `Transport::call(Method, Value)` through its `call<T>` at 16 call sites (17 marker uses, since
  `eval_like` serves both `eval` and `play`) and repeats `Client::call`'s
  `"invalid {method} result: {e}"` text in its own `call<T>` (`run.rs:158-166`).
- **Shapes are cross-checked by regex, and only partly.** `tests/control-protocol.test.ts` scrapes
  the error codes from `protocol.rs`, so `protocol.rs:12` asks for one const per line. It scrapes
  the `LiveStats`, `BatchDecode`, `InvokeRtt` and `Levels` fields and the method names from
  `methods.rs`. Nothing checks the rest of `AppState`, any other result, or any params. The TS
  result interfaces (`protocol.ts:94-220`) and the handler's output can drift from the Rust
  types silently.
- **Example replies are written by hand in six places:**
  - `methods.rs:1027` `STATE_EXAMPLE`, plus partial older-app replies;
  - `run.rs:930` `state_json()` and `:1363` `events_json()`;
  - `e2e.rs:64-77` and `:143-153`;
  - `instrument_mcp.rs:217`.

  Each is a slightly different shape, and none is checked against the others.
- **The `events` limits are written in four places:** `methods.rs:473`, `handler.ts:159-160`,
  `cli.rs:74-75` and `SKILL.md:381`.
- **`methods.rs`'s module doc is misleading.** It calls the types "the contract", but the webview
  is the only validator. The Rust types are what a request the webview accepts decodes into.

## Scope

In:

1. **ctl owns the table and its constants** (stage 001, with the codes macro in 002):
   - `codes::ALL`: every code by name, complete by construction.
   - `EVENTS_LIMIT_DEFAULT` (2000) and `EVENTS_LIMIT_MAX` (100 000).
   - One `decode_result::<M>`, the only place the "invalid {method} result" text is built. Both
     `Client::call` and the CLI use it.
   - `Rpc`'s associated types gain the bounds the round-trip test needs.
   - The `methods.rs` module doc is corrected.
2. **Golden wire examples** under `crates/strudeland-ctl/tests/wire/` (stage 001):
   - one `<method>.json` per method (16 files), plus `errors.json` and `limits.json`;
   - the ctl integration test `tests/wire.rs`;
   - the `wire-fixtures` feature, which exposes the examples to other crates' tests;
   - the hand round-trips in `methods.rs` are pruned.
3. **TS readers** (stage 002):
   - `control-protocol.test.ts` reads the examples, and the regex scrapes go.
   - `control-handler.test.ts` runs every webview example through the real handler.
   - `handler.ts` exports its two limits, and a test pins them to `limits.json`.
4. **CLI typed calls** (stage 003):
   - `call::<M>` and `report::<M>` replace `call<T>`, `report<T>` and `params()`, at all 16 call sites.
   - `cli.rs`'s range uses `EVENTS_LIMIT_MAX`. A test pins the help text and `SKILL.md` to both
     limits.
   - `run.rs`, `e2e.rs` and `instrument_mcp.rs` take their replies from the examples.
5. **App readers and docs** (stage 004):
   - `control.rs`'s screenshot and `project.open` refusal tests read the examples.
   - AGENTS.md gets the wire-example rule and a Layout line, and GLOSSARY.md the term.

Out:

- **Rust validating params before forwarding** (rejected below).
- **Any change to `handler.ts`'s validation, its order or its texts.** It stays the only params
  validator. Rust keeps only `screenshot` and the native absolute-path check for `project.open`,
  where they are.
- **Generated TS types or limits.** The TS interfaces stay hand-written. The examples check them
  from the outside.
- **Marker renames** (`StateCall`, `EvalCall` and the rest keep their names).
- **`output.rs`'s `base()`** and the CLI's other formatter fixtures: they are typed literals, so
  the compiler already checks them against the Rust types.
- **The CLI's other inline replies** (ping, stop, code get, the song commands, the screenshot echo
  in `e2e.rs`). They stay as they are. Only the six hand-written state and events replies and
  `instrument_mcp.rs`'s reply move to the examples.

## Technical design

### The module and its seam

The control protocol becomes one **deep module**: `strudeland-ctl`. Its **interface** is:

- the `Method` table, with one `Rpc` marker per method that names its `Params` and `Result`;
- `codes` and the two `events` limits;
- `decode_result`;
- under the `wire-fixtures` feature, the wire examples.

Every other module that speaks the protocol is an **adapter** at that seam:

- the CLI, which builds requests and parses results;
- `control.rs`, which answers `ping` and `screenshot` and forwards the rest;
- the webview handler, which validates and answers the rest.

The wire examples are the seam's test surface. All three adapters' tests cross it with the same
examples, so a shape changed in one place fails in each adapter that disagrees. Before, each
adapter kept its own copy of a reply.

The webview stays the only params validator (the reverify's "Why Rust will not validate params
first"). So the examples describe what the webview accepts, within what Rust can decode. They do
not move validation.

### `strudeland-ctl`: constants and `decode_result`

```rust
// protocol.rs
pub mod codes {
    // Stage 001: the existing consts unchanged, plus a hand-written ALL (the TS regex still
    // reads the consts). Stage 002: one macro declares each const with its doc and builds ALL.
    /// Every code by its Rust name, in declaration order.
    pub const ALL: &[(&str, i64)] = &[("PARSE_ERROR", PARSE_ERROR), /* … */ ("BUSY", BUSY)];
}

// methods.rs, beside EventsParams
/// `events`' limit when `limit` is absent.
pub const EVENTS_LIMIT_DEFAULT: u32 = 2000;
/// The largest `limit` the webview accepts.
pub const EVENTS_LIMIT_MAX: u32 = 100_000;

pub trait Rpc {
    const METHOD: Method;
    type Params: Serialize + DeserializeOwned;
    type Result: Serialize + DeserializeOwned;
}

// client.rs, re-exported from lib.rs
/// `M`'s typed result from a raw `result`: `CallError::Protocol("invalid {method} result: {e}")`
/// when it does not match.
pub fn decode_result<M: Rpc>(result: &serde_json::Value) -> Result<M::Result, CallError>;
```

- **`decode_result` borrows the `Value`.** The CLI keeps the raw value for `--json`, so a
  borrowed decode saves a clone of up to 16 MiB of `code.get`. `Client::call` passes
  `&result`. serde_json reports the same error text for `&Value` and `Value`, because both
  deserializers share `Value::invalid_type` and serde's derive. The CLI already decodes
  `&Value` (`T::deserialize(&raw)`), so its text cannot change. A unit test pins one text.
- **The bounds on `Rpc` are not a wire change.** Every `Params` and `Result` type already derives
  both `Serialize` and `Deserialize`. The bounds let `tests/wire.rs` round-trip the examples
  through `M::Params` and `M::Result` without per-type where clauses.
- **`codes::ALL` is complete by construction from stage 002 on.** A `macro_rules!` takes
  `$(#[$doc:meta])* $NAME = $value;` rows and emits each `pub const` and `ALL`. That way a new
  code cannot be left out of `ALL`, and so out of `errors.json`. In stage 001 the consts stay as
  written (the regex test still reads them), and `ALL` is a hand list that `tests/wire.rs`
  checks against `errors.json`.
- **The module doc** of `methods.rs` says that the webview validates params (except `screenshot`,
  which the app checks), and that these types are what a request it accepts decodes into, and
  what a result parses into. `EventsParams.limit`'s doc names `EVENTS_LIMIT_DEFAULT`.

### The wire examples

`crates/strudeland-ctl/tests/wire/` holds 18 hand-written, prettier-formatted JSON files:

- `<method>.json` for each of the 16 methods, named by its wire name (`code.get.json`,
  `instrument.apply.json`);
- `errors.json`: `{ "PARSE_ERROR": -32700, … }`, every code by name;
- `limits.json`: `{ "eventsLimitDefault": 2000, "eventsLimitMax": 100000 }`.

A method file looks like this:

```json
{
  "method": "events",
  "answeredBy": "webview",
  "params": [
    { "name": "default", "value": {} },
    { "name": "bars", "value": { "bars": { "from": 2, "to": 4 }, "all": true, "limit": 10 } },
    { "name": "section", "value": { "section": "intro" } }
  ],
  "results": [
    { "name": "stale", "value": { "song": { "path": "songs/main.js", "title": "Main Theme" }, "…": "…" } },
    { "name": "truncated", "value": { "…": "…" } }
  ],
  "invalid": [
    { "name": "limit 0", "params": { "limit": 0 }, "code": -32602,
      "message": "invalid params: 'limit' must be an integer from 1 to 100000" }
  ]
}
```

These rules hold for every file:

- **`method`** equals the file name without `.json`.
- **`answeredBy`** is `"app"` for `ping` and `screenshot` (`Method::answered_by_app`), else
  `"webview"`.
- **`params`** is a list of requests that both sides accept:
  - Rust decodes each one into `M::Params`, and it serializes back equal (see "Numeric compare"
    below).
  - The webview does not refuse it with `INVALID_PARAMS`.

  So an example is never a value only one side takes: no `5.0` limit, no extra key, no `null`
  optional (the reverify lists these). The values aim at the handler test's harness, the
  project `/p` with songs `a`, `b` and `m`, so a webview check runs past song resolution. For
  example, `song.rename`'s example is `{ "song": "b", "name": "intro" }`.
- **`results`** is a list of named replies:
  - Rust decodes each one into `M::Result`, and it serializes back equal.
  - The values are the ones the CLI's tests print (`songs/main.js`, "Main Theme"), because those
    tests read the results.
- **`olderResults`** (optional) is a list of an older app's replies, with fields missing. Each
  must only decode, and every key it has must serialize back equal. Only `state` has one, holding
  `minimal`. Names are unique across `results` and `olderResults`.
- **`invalid`** is a list of state-independent shape refusals with their exact current texts.
  - Each entry is what the handler (or the app) returns for the params as sent. Absent and `null`
    params never get that far: `parse_request` (`frame.rs:147-149`) turns them into `{}`. So no
    entry has top-level `null` params; a non-object refusal uses `[]`, `"x"` or a number. The
    handler's own `null` case stays a handler-only hand test.
  - "State-independent" means the handler gives the same `{ code, message }` with a project open,
    with no project, and after an evaluate. Stage 002's test runs each entry in all three, so it
    is checked, not assumed.
  - A refusal whose text depends on the state stays a hand test where it is. Examples:
    `song.rename`'s `'name' must be a string`, which comes after song resolution, the `events`
    range errors, and `screenshot`'s directory errors.
  - The refusals are answered by whoever answers the method: the webview's are run by the TS
    handler test, and an app-answered method's (`screenshot`) by `control.rs`'s tests.
- **`invalidApp`** (optional) is a webview-answered method's refusals that Rust answers before
  forwarding, in the same entry shape. Only `project.open` has one: `{ "path": "relative" }` →
  `-32602` "invalid params: 'path' must be an absolute path". An app-answered method has none,
  since all its refusals are in `invalid`. The TS test does not read it, and `control.rs`'s test
  runs it. Names are unique across `invalid` and `invalidApp`.
- **`ping`** has no `invalid` entries, because Rust ignores its params.

**State.** `state.json` has a result and an older result:

- `full`: every nullable and optional field is non-null and present at least once:
  - `project`, `active`, `playing`, `evalError`, `metaError`, `outputError`;
  - `seed`;
  - `playhead`, with `songBar` and `section`;
  - `transport`, with `loopSection`, `loopThrough` and `armedSection`;
  - `engine: "rust"`, and `live` with its nested stats;
  - `levels`, with `peakDb: [null, 2.1]`, so the element type is both null and a number;
  - `vim: true`;
  - a song with `conflict: "changed"`.

  The values need not be a state the app can reach. `bpm` is written `120`, as the app sends it.
- `minimal` (in `olderResults`): the reply of an app from before `seed`. It has the twelve
  original fields and none of the later ones. Its values are `run.rs`'s `state_json()` today: project
  `demo` at `/Users/me/demo`, songs `main` (dirty) and `verse` (`conflict: "changed"`), and the
  rest of that function's values.

**Events.** `events.json` has two results:

- `stale`: `run.rs`'s `events_json()` values, with `stale: true` and `truncated: false`;
- `truncated`: `e2e.rs`'s values, with `stale: false`, `truncated: true`, title "Main", bars
  1-2 and location 2:5.

Each CLI test that prints a result reads the one it prints, so its expected output holds as
written.

**Numeric compare.** Rust writes an `f64` field back as `120.0`, while the app and the example
send `120`. serde_json's `Value` equality tells `PosInt(120)` from `Float(120.0)`, so
`tests/wire.rs` compares through a recursive `same(a, b)` instead:

- two numbers are equal when their `as_f64()` are equal;
- objects are equal by key set and value;
- arrays are equal element by element;
- everything else uses `==`.

Integers in the examples stay below 2^53.

### The `wire-fixtures` feature

```rust
// crates/strudeland-ctl/src/wire_fixtures.rs, #[cfg(any(test, feature = "wire-fixtures"))]
//! The golden wire examples (`tests/wire/`) for tests in other crates.
fn text(method: Method) -> &'static str;                 // exhaustive match over include_str!
pub fn result(method: Method, name: &str) -> Value;      // results, then olderResults; panics
                                                         // naming the method and name
pub fn invalid(method: Method) -> Vec<Invalid>;          // { name, params, code, message }
pub fn invalid_app(method: Method) -> Vec<Invalid>;      // invalidApp, empty when absent
```

- **The match over `Method` is exhaustive, and `include_str!` needs the file at compile time.**
  So a method without an example fails to compile wherever the feature is on.
- **Other crates turn it on only as a dev-dependency:**
  `strudeland-ctl = { path = "…", features = ["wire-fixtures"] }` under `[dev-dependencies]` of
  `strudeland-cli` and `strudeland-app`. This is the pattern `strudeland-transcribe`'s `eval`
  feature uses with the CLI. Under resolver 3 (resolver 2 semantics for features), a
  dev-dependency's features are not unified into builds that don't compile test targets
  (`cargo build`, `cargo install`, `app:bundle`), so the shipped CLI and app do not carry the
  examples. `cargo test --workspace` and `cargo clippy --all-targets` do unify them, which is
  what lets the tests read them.
- **The `any(test, …)` makes the module available to ctl's own unit tests** (`methods.rs`'s kept
  behaviour tests read `minimal` from it). ctl's integration test `tests/wire.rs` reads the
  directory with `std::fs` instead, so it also catches an extra file.

### `tests/wire.rs` (ctl)

- **The file set** is `Method::ALL`'s wire names plus `errors.json` and `limits.json`, with
  nothing else and nothing missing. Each file's `method` equals its name, and its `answeredBy`
  is `app` exactly when `Method::answered_by_app` is true (`ping` and `screenshot`).
- **One dispatcher with no `_` arm:**
  `fn check(m: Method, ex: &Value) { match m { Method::Ping => check_rpc::<Ping>(m, ex), … } }`.
  `check_rpc::<M>` asserts `M::METHOD == m` (this replaces `markers_carry_their_method`). It then
  round-trips every `params` and `results` entry exactly and every `olderResults` entry one way
  (decodes, every key it has serializes back equal). It asserts that each `invalid` and
  `invalidApp` entry has the code `INVALID_PARAMS` and a message starting `invalid params: `,
  and that an app-answered method has no `invalidApp`. Names are unique within each list, across
  `results` and `olderResults`, and across `invalid` and `invalidApp`.
- **`errors.json`** equals `codes::ALL` as a map, and `ALL` has no duplicate name or value.
- **`limits.json`** equals `EVENTS_LIMIT_DEFAULT` and `EVENTS_LIMIT_MAX`.
- **`state`'s `full`** is checked for completeness. After the round trip, every `Option` field of
  `AppState`, `Playhead`, `TransportSummary` and `SongSummary` is `Some`, and
  `levels.peak_db` holds a `None` and a `Some`. The test reads the decoded struct, so a new
  `Option` field is caught when someone adds it to that list.

### TS readers

- **`tests/helpers/wire-examples.ts`** (new) reads `crates/strudeland-ctl/tests/wire/` with
  `readdirSync` and `readFileSync`. It exports:
  - `wireExamples(): WireExample[]`;
  - `wireResult(method, name)`;
  - `wireErrors` and `wireLimits`;
  - the `WireExample` type, which mirrors the schema above.

  Nothing in `src/` imports JSON from the crate, so the app bundle is unchanged.
- **`control-protocol.test.ts`:**
  - `ErrorCode` equals `wireErrors`, and the codes stay distinct.
  - `METHOD_NAMES`, sorted, equals the example methods, sorted, with no duplicates.
  - The `LiveStats`, `BatchDecode`, `InvokeRtt` and `Levels` key sets equal the keys of
    `state.full`'s `live`, `live.batchDecodeUs`, `live.invokeRttMs` and `levels`. The test
    keeps its `satisfies Record<keyof T, true>` objects, so TS also checks them at compile time.
  - All three regex scrapes go, and so does the `readFileSync` of `.rs` files. The two
    "documented values" tests go too, because `errors.json` pins every value.
- **`control-handler.test.ts`:**
  - **Accepted:** for each webview example's `params` entry, a fresh `setup()` (`/p` open) runs
    the method. It passes when the call resolves or fails with any code but `INVALID_PARAMS`.
  - **Refused:** for each `invalid` entry not answered by the app, `setup()`,
    `setup({ last: null })` and `evaluated()` each give exactly `{ code, message }`. The doc is
    unchanged, nothing stops, and the runtime is not queried.
    - This replaces the `invalid params` table (`:609-641`), the `events` `bad` table
      (`:975-1018`), "params are checked before the project" (`:643`) and "params are checked
      before anything was evaluated" (`:1020`).
    - It also replaces each per-method refusal that is state-independent (the song, save,
      project and instrument blocks). Every one of those cases moves into its method's
      `invalid` list with the same text, and the state-dependent ones stay.
  - **Shape:** `expectWireShape(actual, method, resultName)`, where both values are objects:
    - their key sets are equal, and each key recurses where both values are non-null;
    - arrays compare each actual element against the example's first element when both arrays
      are non-empty;
    - `events[].value` is a free map, so the check does not descend into it.

    One call goes into each webview method's existing happy-path test, against a non-empty
    result. `state`'s check runs in the audition-range test (`:1175`). Its harness starts
    playback (`stubRuntime({ audio: 'running', live: true })` and the evaluate), and its
    `setPosition` and `arm({ loop, through })` give non-null `playing`, `playhead` (with
    `songBar` and `section`) and `loopThrough`. `h.rt.patch({ seed: 7 })` is added so the
    optional `seed` key is present. `live` and `levels` are null there, so two more checks
    compare their nested shapes with `full`'s `live` and `levels` through the same diff: in "engine and live come through" (`:1199`) and in the levels test
    (`:1217`) after its first `setLevels`. The shape check skips a key where either side is
    `null`.
  - **Limits:** `handler.ts` exports `EVENTS_LIMIT` and `EVENTS_LIMIT_MAX`, and the test asserts
    both against `wireLimits`. "The largest limit is accepted" uses `wireLimits.eventsLimitMax`.

### CLI typed calls

```rust
/// Calls `M` with typed params; the raw result (for `--json`) and its typed form.
fn call<M: Rpc>(transport: &mut dyn Transport, params: &M::Params) -> Result<(Value, M::Result), Failure>;
/// [`call`], then [`emit`] with the human text from `format`.
fn report<M: Rpc>(
    transport: &mut dyn Transport, env: &mut Env, json: bool,
    params: &M::Params, format: impl FnOnce(&M::Result) -> Report,
) -> Result<i32, Failure>;
fn eval_like<M: Rpc<Params = SongParams, Result = EvalOutcome>>(
    transport: &mut dyn Transport, env: &mut Env, json: bool, song: &Option<String>,
) -> Result<i32, Failure>;
```

- **`Transport::call(Method, Value) -> Result<Value, CallError>` stays** as the raw core. It is
  object-safe, so `&mut dyn Transport` and the `NoTransport` and `Fake` fakes are unchanged.
  `call::<M>` serializes the params with `.expect("params serialize")` (as `params()` did),
  calls `transport.call(M::METHOD, …)`, and decodes through `decode_result::<M>(&raw)`.
- **All 16 call sites use it** (17 marker uses):
  - `report::<ctl::Ping>`, `StateCall`, `Stop`, `EventsCall`, `ScreenshotCall`, `SongNewCall`,
    `SongSwitchCall`, `SongRenameCall`, `SongDeleteCall`, `SaveCall`, `ProjectOpenCall` and
    `CodeSet` (12 sites);
  - `eval_like::<EvalCall>` and `eval_like::<ctl::Play>`;
  - `call::<CodeGet>`;
  - `offline_project`'s `call::<StateCall>`;
  - `instruments.rs:115`'s `call::<InstrumentApply>`.

  `Ping`, `State` and `Stop` pass `&Empty {}`, which serializes to `{}`, the bytes `json!({})`
  sent. Write the markers whose names clash with `Command::Play`, `Command::Stop` and
  `Command::Ping` as `ctl::` paths (`use strudeland_ctl as ctl;`).
- **`params()` and the untyped `call<T>` and `report<T>` are deleted.**

### Event limits

- **The ctl consts are the source.** `cli.rs`'s `--limit` parser becomes
  `clap::value_parser!(u32).range(1..=i64::from(EVENTS_LIMIT_MAX))`. Line 168, `render
  --length`'s `100000`, is unrelated and stays.
- **The help text and `SKILL.md` keep their literals,** because a doc comment cannot take a
  const. A CLI test pins them:
  - the `events --limit` help (from `Cli::command()`) contains `1..={EVENTS_LIMIT_MAX}` and
    `default {EVENTS_LIMIT_DEFAULT}`;
  - `SKILL.md`'s `` `--limit N` `` line contains `1 to {EVENTS_LIMIT_MAX}` and
    `default {EVENTS_LIMIT_DEFAULT}`.
- **`cli.rs`'s range test** (`:1184-1187`) uses `EVENTS_LIMIT_MAX` and `EVENTS_LIMIT_MAX + 1`.
- **`handler.ts` keeps its own consts,** exported and pinned to `limits.json` by a test. The
  bundle does not import JSON from a crate.

## What stays

- **Every wire byte:** the requests the CLI sends, each method's accepted params, each refusal's
  code and message, and the order of the checks. That includes `song.rename` and `song.delete`
  resolving the song before their other checks, and NOT_READY before the `project.open` check.
- **Every CLI output, flag, help text, exit code and `SKILL.md` sentence.**
- **`Transport`, `SocketTransport` and the fakes; `Client::call_raw`; `BridgeRequest`'s raw
  params.**
- **`handler.ts`'s validation code,** apart from the `export` on two consts.
- **The behaviour tests in `methods.rs`:**
  - defaults when fields are missing (now reading `minimal`);
  - nulls serialize as null, and an absent `seed` is omitted;
  - refusals: a missing field, snake_case keys and wrong enum case;
  - a non-positive event bar;
  - an event without a section or locations;
  - `Method::parse`;
  - `TransportSummary::default()`.

## Rejected

- **Rust deserializes params first, with `deny_unknown_fields`** (the issue's suggestion). This
  is a wire change on every axis the reverify lists: every text, the first error reported,
  floats and large integers newly refused, `null` and `""` newly accepted, the codes for
  `song.rename` and `song.delete`, and NOT_READY turning into INVALID_PARAMS. The webview stays
  the single validator.
- **Replacing `output.rs`'s `base()` with an example.** It is a typed `AppState` literal, so the
  compiler already checks it against the Rust type. An example would add a second copy and
  check nothing new.
- **Generating the TS limits (or types) from Rust.** That needs a generator or a JSON import into
  the bundle for two numbers. A pinned test gives the same guarantee.
- **Renaming the markers** (`StateCall` → `State`). It is churn with no checking gain, and it
  would clash with `Command::State` in `run.rs`.

## Tests: replace, don't layer

A test of deleted or moved code moves to its replacement with the same assertions. Never keep
both.

- **`methods.rs`:** every hand round-trip is deleted, because `tests/wire.rs` covers it from the
  examples. Stage 001 lists them. `STATE_EXAMPLE` goes too.
- **`protocol.rs`:** `codes_match_the_error_table` is deleted, because `errors.json` pins every
  value and the old test covered only 13 of them. The comment at `:10-12` goes in stage 002.
- **`client.rs`:** `wrong_result_shape_is_protocol` stays. A non-unix unit test pins
  `decode_result`'s text.
- **TS:** see "TS readers" above. The regex scrapes and the case tables go, and the examples
  replace them.
- **CLI:**
  - `state_json()` and `events_json()` become one-line readers of `minimal` and `stale`.
  - `e2e.rs`'s state reply reads `minimal`, so its expected text becomes the one
    `state_human_and_json` asserts. Its events reply reads `truncated`, and its expected text is
    unchanged.
  - `instrument_mcp.rs`'s reply reads `instrument.apply`'s `example`.
  - The limits test is new.
- **`control.rs`:**
  - `screenshot_rejects_invalid_params` takes its state-independent cases from
    `screenshot.json` and keeps the directory cases inline.
  - `project_open_checks_native_absolute_paths_before_forwarding` takes its refusal from
    `project.open.json`'s `invalidApp` entry, with the exact message (it checked only the code
    before).

## Dependencies

- **After #119** (merged as `3a189da`). It touches no IPC binding, and `ipc:build` is not needed
  (no command, event or kind changes).
- **Not needed:** `host:build`, `engine:refs`, `engine:vectors`, `render:vectors`, goldens and
  `host:parity`. No `host.inputs.json` file is touched: `handler.ts` and `protocol.ts` are not
  host inputs.

## Risks

- **`120` against `120.0`.** A plain `Value` compare fails on every `f64` field the app sends as
  an integer (`bpm`, `cps` and `beat` in examples). `same()` compares numbers as `f64`. Its own
  unit cases are: `120` equals `120.0`, `1` does not equal `2`, `{}` does not equal `{"a":1}`,
  and `[1]` does not equal `[1,1]`.
- **The numeric leniency also hides an int/float type change.** A `u32` field turned `f64`
  still writes `10` back as `10.0`, and `same()` calls that equal. Keep the integer params
  (`limit`, `bars.from`/`to`, `instrument.apply`'s `from`/`to`) as `u32` in Rust, so their
  examples keep round-tripping through an integer type; changing one of those types is a wire
  change, reviewed as one, not something the examples catch.
- **Example values against the CLI's human output.** The CLI tests assert exact text, so the
  examples take the values those tests print. Where two tests print different replies,
  `events.json` holds both (`stale`, `truncated`). `e2e.rs`'s state test is the one expectation
  that changes: it now prints `minimal`. That is a test input, not behaviour. Stage 003 runs
  each moved test before and after, and the outputs must match the strings written in the stage
  file.
- **Feature leakage.** If `wire-fixtures` reached a normal dependency, the examples would ship in
  the binary. It is only ever a dev-dependency feature. Stages 003 and 004 check
  `cargo tree -p strudeland-cli -e normal,features -i strudeland-ctl` (and the same for
  `strudeland-app`), whose output must not contain `wire-fixtures`. As a fallback,
  `cargo tree -p strudeland-cli -e normal -f '{p} {f}' | grep wire-fixtures` (and the same for
  `strudeland-app`) prints nothing. The CLI binary that
  `e2e.rs` runs is built with the test feature set and carries the examples as dead data, which
  is harmless.
- **An example that only one side accepts.** If the webview accepted a params example that serde
  refused, or the other way round, it would fail one side's test. That failure is the point:
  pick a value inside both. The reverify's list of divergent inputs is the guide.
- **"State-independent" drifting.** If a later edit moves a shape check after song resolution, its
  example starts failing under `setup({ last: null })`. Move that case back to a hand test. Do
  not reorder the handler.
- **Prettier owns the JSON layout.** The examples are hand-written source, not generated, so
  they are not prettier-ignored. Run `bun run format` after editing them.

## Complexity

Medium: about 30 files, 18 of them new JSON.

- ctl: `protocol.rs`, `methods.rs`, `client.rs`, `lib.rs`, `Cargo.toml`, the new
  `wire_fixtures.rs`, `tests/wire.rs` and the 18 examples.
- TS: `handler.ts` (two `export`s), the new `tests/helpers/wire-examples.ts`, and the two tests.
- CLI: `run.rs`, `run/instruments.rs`, `cli.rs`, `Cargo.toml`, `e2e.rs` and
  `instrument_mcp.rs`.
- App: `control.rs` and `Cargo.toml`.
- Docs: AGENTS.md and GLOSSARY.md.

The stages are sequential, and each one leaves `bun test` and `cargo test --workspace` green. The
plan is one document, so there are no fracture or loss-check stages.

## Decisions

1. **The webview stays the only params validator.** The examples describe what it accepts,
   intersected with what Rust decodes.
2. **One example file per method,** with named params, results and invalid entries, plus
   `errors.json` and `limits.json`. They are hand-written, committed and prettier-formatted.
3. **`decode_result::<M>(&Value)`** borrows. The brief said `Value`. Borrowing keeps the CLI's
   raw value without a clone, and serde gives the same text either way.
4. **`Rpc`'s associated types are bounded `Serialize + DeserializeOwned`,** so the round trip
   needs no per-type where clauses.
5. **`codes::ALL` comes from a macro in stage 002,** so it is complete by construction. Stage 001
   uses a hand list because the TS regex still reads the plain consts until stage 002 deletes it.
6. **Older-app results go in `olderResults`** and are checked as decode plus a subset round
   trip. Every `results` entry must round-trip exactly.
7. **The invalid entries run in three handler states.** That makes "state-independent" a checked
   property.
8. **A webview method's app-answered refusals go in `invalidApp`.** It is used only for
   `project.open`'s native absolute-path check.
9. **The params examples target the handler harness** (`/p`; songs `a`, `b`, `m`). The result
   examples carry the CLI's values.
10. **`e2e.rs`'s state test reads `minimal`,** and its expected text changes. `events.json` keeps
    two results, so neither events test's expected text changes.
11. **The help text and `SKILL.md` keep their literal limits,** pinned by a CLI test. `handler.ts`
    keeps its consts, pinned by a TS test.
12. **GLOSSARY.md gains "Wire example"** in the App boundary section (stage 004).
