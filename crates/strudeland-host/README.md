# strudeland-host

The headless pattern host: evaluates a strudeland song in an embedded V8 and plans its engine
job (schema 2) exactly as the render page does in Chromium, without a browser. Both run the same
planner (`src/lib/render/job-plan.ts`, bundled into `js/host.js`), so the job text is byte-equal by
construction; `bun run host:parity` proves it on the render corpus. It is the planning half
of `strudeland render`. Its harness binary is invoked as `strudeland-host`.

Each request loads the committed bundle `js/host.js` into a fresh V8 isolate and context, with
deterministic natives installed first, so no global, `evalScope` write, transpiler-defined name or
module state leaks from one song to the next.

## Entry points

- The library: `Host::new()`, then `Host::plan_job(code, &RenderRequest)` (the job JSON plus its
  `info`), `Host::query(code, from, to, Layout)` (the events of a cycle window as JSON) and
  `Host::plan_render(code, &RenderPlanRequest)` (every chunk's job of a render in one call, below)
  and `Host::eval_script(source)` (a script in a fresh context, without the bundle). `Host` is
  `Send + Sync`; calls on one `Host` are served one at a time. Errors are `HostError`: `Meta`
  (`MetaFlag`: a `meta` failure of a render's length request that names the flag at fault),
  `Eval`, `Plan`, `Unsettled` (the song's promises never settled), `Timeout` and `Runtime`.
- The `strudeland-host` binary, which `render:check --planner host` and `host:parity` run:

  ```
  cargo run -p strudeland-host -- plan --song <file> --request <json|file> [--out <file>] [--timeout <secs>]
  cargo run -p strudeland-host -- plan-render --song <file> --request <json|file> [--out <file>]
  cargo run -p strudeland-host -- query --song <file> --from <c> --to <c> [--layout clip|linear]
  cargo run -p strudeland-host -- bench [--bars N] [--lengths FILE] [--profile] [--out-dir DIR] <song…>
  ```

  `plan` writes the job to `--out` or stdout and one line of `info` JSON on stderr. Exit 0 ok, 1
  host error (`strudeland-host: <kind>: <message>`), 2 bad input (the message and the usage).
- `plan-render` is the library's `plan_render`: see "Batch planning".
- `bench` (also `strudeland_host::bench`): plans each song three times on fresh hosts and reports
  medians, job size and hash, peak memory and the budget report (below).

`render:check --engine rust --planner host` plans with this crate's binary (built with cargo, or
`STRUDELAND_HOST_BIN`) instead of the render page and does not build the render page.

## Piano roll inspection

`Host::inspect` evaluates an editor snapshot in a fresh context with a required deadline.
The desktop passes the project instrument pins and either a selected expression's UTF-16 source
range or a written song window. Expression queries use local phase and can take the meter from
a written bar. The result carries pitches, rational whole and visible spans and verified source
contributors. The editor uses its separate authored phrase model to decide when it can write a
source edit.

Inspection uses whole-bar windows of up to 16 bars. The result limits the number of notes and
marks truncation. The desktop coalesces selection changes and ignores replies for old editor
snapshots.

## Batch planning

`plan_render` (`plan-render`) plans a whole render in one call, for `strudeland render`, instead of
one request (and one evaluation of the song) per chunk. The request is a `RenderPlanRequest`:
`{"length":{"kind":"clip"|"section"|"length"|"window",...},"mode":"mix","sampleRate":44100,
"preset"?,"includeEvents"?,"timeoutSec"?,"instruments"?}`. The optional `instruments` object maps
project pin IDs to validated instrument definitions. Requests without pins omit this field.
The bundle's `planRender` (`src/lib/render/job-plan.ts`):

1. resolves the length request against the song's `meta` (a clip with `loops`, a section, the
   first N bars, or an explicit window); one the song cannot satisfy is `HostError::MetaFlag` and
   names the CLI flag (`--length`, `--section`, `--loops`);
2. evaluates the song once and selects the schedule for its render window;
3. cuts that schedule per chunk and plans each chunk's engine job (the chunk windows, seam overlap
   and lookback are the same ones `renderChunkPlans` gives the harness).

The result is the structure, the window and chunk windows, the song's errors, and one job text per
chunk. The schedule is reused by default (`BATCH_REUSE_SCHEDULE = true`): each chunk's selection is
the full schedule cut at the chunk's end. Setting it to `false` makes each chunk query and select
`[0, end)` itself, as `plan_job` does (quadratic, equal by construction); `host:parity --batch`
found every corpus chunk job byte-equal with `true`, so the fallback is unused.

The optional `preset` field accepts `tagged-loop`, `intro-loop`, `folded-loop` or `one-shot`.
Loop presets take a plain clip request and set `export.loopStartBar` to the marked section's
start on the song timeline. The top-level chunks cover the one-time opening through the end
of the body. If the opening includes an intro, `export.tailFrames` is zero and `export.repeat`
provides its own `chunks` and `tailFrames` for the body in isolation. That repeat uses the
same selected events as the opening from `export.loopStartBar` to `window.toBar`.
For `folded-loop` and songs that
begin with the loop, the top-level body jobs and their `export.tailFrames` supply both passes.
The renderer folds the body's post-roll into the repeat. A one-shot accepts the usual length
request and uses the top-level `export.tailFrames`. Post-roll extends only the final job of
the pass it belongs to, without scheduling any later bars.

The post-roll estimate uses each hap's actual duration, release, compressor pre-delay and bus
dependencies. Lowpass, highpass and bandpass filters in the voice or its FX chain can ring after
the amp envelope closes. The estimate allows at least half a second per ringing stage and scales
with Q and cutoff near DC or Nyquist. A static filter at either endpoint needs no extra time. It accounts for filter envelopes, LFOs and cascaded stages,
with a 30-second cap per voice. A generic modulator driving cutoff or Q uses that cap.
Vowel formants get one second and a phaser notch half a second. An active orbit DJ filter gets
another half-second after the effects returns.
The estimate allows the full room impulse response (1.5 times room size) and the first delay
tap, even at zero feedback, where the engine can skip the orbit delay send. Later repeats include the engine's 128-frame feedback latency until
the echo falls 60 dB below the first tap. Feedback tops out at 0.98 and delay time at one second.
The estimate subtracts the elapsed time before the musical cut. An empty schedule and silence
markers need zero post-roll. The scratch buffer limit also applies to the extended final job and the joined export.

### Planning timeout

A request may bound its planning: `plan --timeout <secs>` and `RenderPlanRequest.timeoutSec`
(a number greater than 0 and at most 86400; anything else is rejected). A watchdog thread calls V8's
`terminate_execution` on the isolate at the deadline and the request fails with `HostError::Timeout`
(`timeout: planning did not finish within N s`). No timeout is the default of the library and of
`plan`: a song with an infinite loop then hangs its request. `strudeland render` passes 120 s unless
`--plan-timeout` says otherwise. The timeout covers the bundle load too.

## Runtime: V8

The host embeds V8 through the raw `v8` crate (rusty_v8), pinned at `=152.2.0` (V8
`15.2.124.1-rusty`). Early planning measurements favored V8 over QuickJS-NG. The V8 backend
also helps the host match Chromium's JavaScript behavior, checked by the corpus parity harness
and the `Math` tests below.

### Performance budget

[`bench::BUDGET`](src/bench.rs) sets a 2 s limit for each full-song plan, 0.5 s for the median
plan, and 100 ms for the median per-request load (isolate, natives and bundle). The report flags
startup snapshot evaluation when the median load exceeds 100 ms or 25% of the median plan time.
The vendored [Fraction fast path](../../vendor/strudel/PATCHES.md) reduces planning time for dense
Strudel patterns. Re-run these checks when changing the bundle, the V8 pin or the planner.

```sh
cargo run --release -p strudeland-host -- bench --bars 160 --profile tests/render/corpus/late-feedback.js
cargo test --release -p strudeland-host --test perf_budget -- --ignored --nocapture
```

For timings over each song's full length, pass `--lengths FILE` with a JSON object mapping each
song stem to its bar count, for example `{"late-feedback": 160}`. Pass one or more song paths from
`tests/render/corpus/` or your own project. `--profile` splits the time into load, evaluate, query
and JSON. The process exit code covers execution errors, and the report's `budget` records misses.

The ignored [performance test](tests/perf_budget.rs) plans the widest corpus window three times
on fresh hosts and asserts that the median meets the 2 s per-song limit. The current fixture,
`late-feedback`, starts at bar 136 and queries the dense schedule through bar 160. Run the release
build on a quiet machine.

## Globals

Before `host.js` runs, the natives (`src/natives.rs`) install:

- `Math` overrides from the engine's `jsmath` (V8's `Math`, bit for bit): `sin`, `cos`,
  `tanh`, `exp`, `expm1`, `log1p`, `cosh`, `pow`; each keeps its name and `length`.
- `Math.random`, which throws (`Math.random is disabled in the strudeland host: patterns must be
  deterministic`). A song calling it does not plan headlessly.
- `Date.now()` and `performance.now()` returning 0, and the no-argument `new Date()` and `Date()`
  reading that frozen clock (the epoch); `new Date(...)` with arguments, `Date.parse` and
  `Date.UTC` work as in Chromium.
- `setTimeout`/`setInterval` returning increasing ids and never firing, `clearTimeout`/
  `clearInterval` doing nothing. After evaluate the host drains the microtask queue and reports a
  result that never settled as `HostError::Unsettled`.
- `console`: `log`, `info`, `debug`, `trace`, `dir` and the other standard methods (`assert`,
  `table`, `group`, `time`, `count`, …) are discarded; `warn` and `error` go to
  stderr only with `STRUDELAND_HOST_LOG=1` (the bundle's entry also captures them into
  `info.errors` during a plan).
- `__strudelandMark` (the phase marks of `bench --profile`).

and remove `window`, `document`, `navigator`, `fetch`, `AudioContext`, `localStorage`,
`requestAnimationFrame`, `queueMicrotask` and `structuredClone`.

The bundle (`src/host/`) replaces `@strudel/webaudio` and `@strudel/codemirror`: superdough's
eval-time functions (`samples`, `aliasBank`, `registerSound`, …) are no-ops and fetch nothing (sample
sounds stay unsupported in the engine), and the editor's value-affecting names (`slider`,
`markcss`, `scope`, `_scope`, `_pianoroll`, `pianoroll`, `pitchwheel`, …) are shims that return
what the real functions return. `@strudel/draw` is bundled for real. `tests/host-shims.test.ts`
compares the shims with the real functions and `tests/host-scope.test.ts` fails on an app-scope
name that is neither real, shimmed, stubbed nor listed in `src/host/not-reachable.ts`.

## Math

Every own property of `Math` is classified exactly once (`src/math.rs`; `tests/math.rs` checks the
classification and compares every function the host runs with the Chromium reference
`crates/strudeland-engine/tests/refs/v8-math.json`):

- Overridden by `jsmath`: `sin`, `cos`, `tanh`, `exp`, `expm1`, `log1p`, `cosh`, `pow`.
- Thrown: `random`.
- Constants (bit patterns equal `std::f64::consts`): `E`, `LN10`, `LN2`, `LOG10E`, `LOG2E`, `PI`,
  `SQRT1_2`, `SQRT2`.
- Exact by the spec (not compared): `abs`, `ceil`, `floor`, `trunc`, `sign`, `sqrt`, `fround`,
  `f16round`, `clz32`, `imul`, `max`, `min`, `round`, `sumPrecise` (`f16round` and `sumPrecise`
  may be absent).
- Verified native (the embedded V8 matches Chromium bit for bit on every reference point): `tan`,
  `asin`, `acos`, `atan`, `atan2`, `sinh`, `asinh`, `acosh`, `atanh`, `log`, `log2`, `log10`,
  `cbrt`, `hypot`.

The reference points match for the overrides and the `**` operator (2051 points over five
groups). See [tests/math.rs](tests/math.rs) for the comparisons and
[tests/string_number.rs](tests/string_number.rs) for the ignored comparison of `String(number)`
and `JSON.stringify` of numbers against Chromium and the engine's `js` port.

## Known differences

- Keyboard state: with no `window`, core's `getCurrentKeyboardState()` returns `undefined` where
  the page has `{}`; songs that read keyboard input are not offline-renderable.
- `info` errors are not compared byte for byte. The page captures `strudel.log` DOM events (core's
  `logger`: `[query] error: …`, `[core] Modulation type … not found`, …) and `window` errors
  (unhandled rejections, page errors). The host has no DOM, but the logger also writes each
  message through `console.log`; the host captures the failures from there with the page's rule
  (`src/lib/render/logged-errors.ts`), so they are in its `info.errors` with the same text. It
  also captures `console.warn`/`console.error` and thrown errors. Page-only: `window` errors, and
  logger messages that are failures only by their `type` argument, which `console.log` does not
  carry (`scale`'s `[tonal] Invalid value format …`). `render:check --planner host` reports what
  the host captured.
- `getDrawContext`, `getAudioContext` and similar getters that return something a song then uses
  are not available: such a song fails with an `Eval` error.
- The planning timeout is opt-in (`--timeout`, `timeoutSec`): without one a song with an infinite
  loop hangs its request.
- `plan_render`'s chunk jobs are byte-equal to `plan_job` of each chunk's request (`host:parity
  --batch`), but `info` is not part of the comparison, as for `plan`. A song the page cannot
  schedule must fail `plan_render` too.

## The bundle

`js/host.js` (about 1.0 MiB, unminified IIFE) is built by `bun run host:build`
(`scripts/host/build.ts`, Bun's bundler) from `src/host/main.ts` and embedded with
`include_str!`, so `cargo build` never needs Bun. It is committed and never hand-edited, together
with `js/host.inputs.json` (the sha256 of every file the bundler read and a combined hash); a
`bun test` staleness test (`tests/host-bundle.test.ts`) fails when a listed file changes until the
bundle is rebuilt.

### After a Strudel sync

The sync prints the reminder. Then:

```
bun run host:build
bun run host:parity
cargo test -p strudeland-host
```

and commit the rebuilt `js/host.js` and `js/host.inputs.json`. The same applies after an edit of
any file listed in `host.inputs.json`. Re-run the performance checks above when the bundle changes
much.

## Building

The first build of the `v8` crate downloads a prebuilt static V8 library
(`librusty_v8_release_<target>.a.gz`, about 40 MB) and caches it in `~/.cargo/.rusty_v8`; the
first link is slow. Offline builds point `RUSTY_V8_ARCHIVE` at a downloaded archive, or
`RUSTY_V8_MIRROR` at a mirror of the release assets. The hardened macOS app and its bundled CLI
enable the host's `jitless` feature to run V8 without a JIT entitlement. Standalone CLI source builds use V8's
default mode. The engine crate does not depend on this crate or on V8
(`tests/licence.rs` checks `Cargo.lock`).

## Licences

The crate is AGPL-3.0-or-later. The `v8` crate is MIT; V8 is BSD-3-Clause and is linked with ICU,
Abseil, simdutf, libc++ and other libraries. Their licence texts are in `LICENSE-v8`.
