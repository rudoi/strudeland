# 002: the TS tests read the wire examples

## Goal

The webview side is checked against the examples, not against regexes over Rust source:

- `control-protocol.test.ts` checks the error codes, the method names and the stats key sets
  against them.
- `control-handler.test.ts` runs the real handler on every webview example: accepted params,
  exact refusals, and its results' key shape. It also pins the handler's two `events` limits to
  `limits.json`.

The regex scrapes are deleted, so `codes` no longer needs one const per line: it becomes a macro
that builds `ALL`.

## Prerequisites

- Stage 001 committed: the examples exist and `tests/wire.rs` is green.

## Files

- `tests/helpers/wire-examples.ts` (new).
- `tests/control-protocol.test.ts`.
- `tests/control-handler.test.ts`.
- `src/lib/control/handler.ts`: `export` on `EVENTS_LIMIT` and `EVENTS_LIMIT_MAX` only.
- `crates/strudeland-ctl/src/protocol.rs`: the `codes` macro, and the regex comment removed.

## Steps

1. **`tests/helpers/wire-examples.ts`.**

   ```ts
   const DIR = new URL('../../crates/strudeland-ctl/tests/wire/', import.meta.url);
   export interface WireEntry { name: string; value: unknown; older?: boolean }
   export interface WireInvalid {
     name: string; params: unknown; code: number; message: string; answeredBy?: 'app' | 'webview';
   }
   export interface WireExample {
     method: MethodName; answeredBy: 'app' | 'webview';
     params: WireEntry[]; results: WireEntry[]; invalid: WireInvalid[];
   }
   /** Every `<method>.json`, read once, in sorted file order. */
   export function wireExamples(): WireExample[];
   export function wireExample(method: MethodName): WireExample;
   /** A named result's value; throws naming the method and name when it is missing. */
   export function wireResult(method: MethodName, name: string): any;
   export const wireErrors: Record<string, number>;
   export const wireLimits: { eventsLimitDefault: number; eventsLimitMax: number };
   ```

   It uses `readdirSync` and `readFileSync` from `node:fs`, and files other than `errors.json`
   and `limits.json` are method files. Nothing under `src/` imports it.
2. **`control-protocol.test.ts`:**
   - Delete the `protocol.rs` and `methods.rs` reads and the three regex blocks: `rustCodes`, the
     `fields()` helpers and `rustNames`.
   - **`ErrorCode`:** `expect(ErrorCode).toEqual(wireErrors)`; keep "codes are distinct"; delete
     the two "documented values" tests.
   - **`METHOD_NAMES`:** `[...METHOD_NAMES].sort()` equals the sorted `wireExamples()` methods,
     with no duplicates. Add one assertion: the methods whose example says
     `answeredBy: 'app'` are exactly `['ping', 'screenshot']`, the two that
     `ControlRequest['method']` excludes. The existing `@ts-expect-error` test stays.
   - **`LiveStats` and `Levels`:** keep each `satisfies Record<keyof T, true>` object. Compare its
     sorted keys with the sorted keys of
     `wireResult('state', 'full').live`, `.live.batchDecodeUs`, `.live.invokeRttMs` and
     `.levels`.
3. **`handler.ts`:** change `const EVENTS_LIMIT` and `const EVENTS_LIMIT_MAX` to `export const`.
   Change nothing else.
4. **`control-handler.test.ts`**, to the proposal's "TS readers" section:
   - **A `describe('wire examples')` block:**
     - **accepted:** one test per webview method's `params` entry. Run it in a fresh `setup()`,
       and pass if `h.handle` resolves, or rejects with a `ControlError` whose code is not
       `ErrorCode.INVALID_PARAMS`. Its value is catching a params example that the webview
       wrongly refuses: Rust's round trip alone would let one through.
     - **refused:** one test per `invalid` entry whose effective `answeredBy` is `webview`. Run
       it in `setup()`, `setup({ last: null })` and `evaluated()`. Each must give exactly
       `{ code, message }` (via `h.fail`), leave `h.doc()` as before the call, and leave
       `h.rt.stops()` and `h.rt.queries()` unchanged.
     - **limits:** `EVENTS_LIMIT === wireLimits.eventsLimitDefault` and
       `EVENTS_LIMIT_MAX === wireLimits.eventsLimitMax`.
   - **Delete what the block replaces:**
     - the `describe('invalid params')` table and the "params are checked before the project"
       test. Keep "an empty code string is allowed". Keep the table's `null` row (`eval`,
       `null` → "expected an object") as a handler-only hand test: `parse_request` turns `null`
       params into `{}` (`frame.rs:147-149`), so it has no wire example.
     - `events`' `bad` table and "params are checked before anything was evaluated". "The
       largest limit is accepted" stays, with `{ limit: wireLimits.eventsLimitMax }`.
     - the state-independent asserts inside the per-method params tests, each of which now has
       an `invalid` entry with the same text:
       - `song.new`'s `{}`, `{ name: 1 }` and `{ name: 'x', foo: 1 }`;
       - `song.switch`'s "song is required" and `{ song: 'a', x: 1 }`;
       - `song.rename`'s `{ name: 'x', foo: 1 }`;
       - `song.delete`'s `{ x: 1 }`;
       - `save`'s `{ all: 1 }`, `{ x: 1 }` and the mutually-exclusive test;
       - `project.open`'s whole "params: missing path, wrong types, unknown key" test.

       Keep the state-dependent asserts in those tests: `song.rename`'s `{ song: 'b' }` and
       `{ song: 'b', name: 3 }`; `song.delete`'s `{ discard: 'yes' }` (it resolves first, so
       with no project it is `NO_PROJECT`); `save`'s unknown song and no-project cases. Delete a
       test that is left empty.
     - `instrument.apply`: the range refusal (`from: from + 3`) depends on a pin and stays.
   - **`expectWireShape(actual, method, resultName)`** as a local helper, to the proposal's rule.
     The `events[].value` skip is by key path. The helper's own cases:
     - `{ a: 1 }` against `{ a: 2 }` passes;
     - `{ a: 1, b: 1 }` against `{ a: 1 }` fails;
     - `{ a: null }` against `{ a: { x: 1 } }` passes.

     Add one call to an existing happy-path test of each webview method:

     | Method | Result | Test |
     |---|---|---|
     | `state` | `full` | `:1175`, with `h.rt.patch({ seed: 7 })` before the call, so `seed` is present |
     | `state` | `full` | `:1199` ("engine and live come through"), for `live`'s nested shape |
     | `state` | `full` | `:1217` (levels), after the first `setLevels`, for `levels`' shape |
     | `code.get` | `example` | `:178` |
     | `code.set` | `evaluated` | an auto-eval test where `evaluated` is non-null (`:413`) |
     | `eval` and `play` | `ok` | `:522` |
     | `stop` | the only result | `:596` |
     | `events` | `stale` | `:773` |
     | `song.new` | the only result | `:1266` |
     | `song.switch` | the only result | `:1364` |
     | `song.rename` | the only result | `:1412` |
     | `song.delete` | the only result | `:1516` |
     | `save` | `one` | `:1594` |
     | `project.open` | the only result | `:1693` |
     | `instrument.apply` | `example` | `:1796` |

     In `:1175` the harness has started playback (`stubRuntime({ audio: 'running', live: true
     })` and the evaluate), so `playing` and `playhead` (with `songBar` and `section`) are
     non-null. `loopThrough` comes from the test's `arm({ loop: 'intro', through: 'body' })`,
     not from the seed patch. `live` and `levels` are null there, hence the two extra rows. The
     check skips a key where either side is `null`, so `evalError`, `metaError`,
     `outputError` and `conflict` are checked by key presence only; their non-null values are
     checked by the Rust round trip.

     If a test's result lacks a key the example has (or the reverse), that is a real drift
     between the handler and Rust: report it. Do not edit the example to match.
5. **The `codes` macro** in `protocol.rs`. Replace the consts and the hand `ALL` with:

   ```rust
   macro_rules! codes {
       ($($(#[$doc:meta])* $name:ident = $value:expr;)*) => {
           $($(#[$doc])* pub const $name: i64 = $value;)*
           /// Every code by its Rust name, in declaration order.
           pub const ALL: &[(&str, i64)] = &[$((stringify!($name), $name)),*];
       };
   }
   ```

   Define and invoke it inside `pub mod codes`, so `ALL` stays at `codes::ALL`, with the 17
   rows and their doc comments unchanged. The
   module doc becomes "The error codes of the control protocol. `src/lib/control/protocol.ts`
   mirrors them; the wire examples' `errors.json` is the cross-check." The "keep one
   `pub const` per line" sentence goes. `codes::X` paths and values are unchanged, so
   `tests/wire.rs` stays green. From here `errors.json`'s values are pinned against
   `codes::ALL` only, which the macro builds from the same rows as the consts. That is
   acceptable: there is no second Rust list left to disagree.

## Tests

- New: the `wire examples` block, the `expectWireShape` cases and the limits pin.
- Replaced: the regex tests, the two invalid tables, the two "checked before" tests and the moved
  per-method asserts. Every assertion they made is now an example entry with the same text.
- Kept: every state-dependent refusal, every behaviour test, the `null` params row as a
  handler-only test, and the `replyTo` test (it checks the reply envelope, not params).

## Validation

- The gates in [000-prompt.md](000-prompt.md), logged as `0120-002-*`.
- Count the refused tests the block runs against the `invalid` entries left after removing
  `answeredBy: app` entries. They must match (`bun test tests/control-handler.test.ts` lists
  them).
- `rg -n 'matchAll|readFileSync' tests/control-protocol.test.ts` finds no `.rs` read.

## Commit message

```
test(control): check the webview handler against the wire examples

The protocol test reads the error codes, method names and stats key
sets from the ctl wire examples instead of scraping the Rust source.
The handler test runs every example: each params example is accepted,
each invalid one is refused with its exact code and message with or
without a project, and real results have the examples' key shape. The
handler's events limits are pinned to the examples. codes::ALL is now
built by the same macro that declares the codes.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
