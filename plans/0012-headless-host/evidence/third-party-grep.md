# Third-party randomness and clock grep

Measured 2026-11-12 over the committed bundle `crates/strudeland-host/js/host.js` (1 074 056
bytes, 142 input files; the bundle as rebuilt by `bun run host:build` in the bench commit, which
added only the profile marks of `src/host/main.ts`).

Method: case-sensitive fixed-string greps (`grep -nF`) for `Math.random`, `Date.now`, `new Date`,
`performance.now`, `crypto.`, `getRandomValues`, `Intl.`, `toLocale`. For each hit the enclosing
module is the nearest `// <path>` module comment above the line. Reachability was read from the
call sites. A second, empirical check backs the `Math.random` rows: in the host `Math.random`
throws, so any call on a planning path would have failed the plan, and all 26 benchmark songs and all
35 corpus files planned in the bench.

| Pattern | Module | Line in `host.js` | Reachable from `planJob`/`query` | Action |
|---|---|---|---|---|
| `Math.random` | `node_modules/source-map/lib/quick-sort.js` (`randomIntInRange`, the quicksort pivot) | 1827 | no: only `SourceMapConsumer` sorts mappings; the transpiler calls `escodegen.generate(ast)` without a `sourceMap` option, so no consumer is ever built | none |
| `Math.random` | `node_modules/@tonaljs/array/dist/index.js` (`shuffle(arr, rnd = Math.random)`, a default parameter) | 5345 | no: tonal's `shuffle` is not called by `@strudel/tonal` (no `shuffle` in `vendor/strudel/packages/tonal/`); Strudel's own `shuffle` (core `signal.mjs`, `randrun`) is seeded and unrelated | none |
| `Math.random` | `node_modules/@tonaljs/collection/dist/index.js` (the same `shuffle` default) | 5406 | no: as above | none |
| `Math.random` | `vendor/strudel/packages/core/neocyclist.mjs` (`NeoCyclist` constructor, `neocyclist.mjs:29`) | 23490 | no: expected. `NeoCyclist` is built only when `sync && typeof SharedWorker != "undefined"`; the host has no `SharedWorker`, and the scheduler is never started | none (decision 5: it would throw) |
| `Date.now` | `vendor/strudel/packages/core/neocyclist.mjs` (same line) | 23490 | no: as above | none |
| `Date.now` | `vendor/strudel/packages/core/util.mjs` (`getUnixTimeSeconds`, `util.mjs:459`) | 20279 | no: expected (core's clock). Used only as `ClockCollator`'s default `getTargetClockTime`, built by `NeoCyclist` | none (returns 0 in the host) |
| `performance.now` | `vendor/strudel/packages/core/util.mjs` (`getPerformanceTimeSeconds`, `util.mjs:455`) | 20276 | no: expected (core's clock). Exported, read by the scheduler only | none (returns 0 in the host) |
| `performance.now` | `vendor/strudel/packages/core/logger.mjs` (`logger`, `logger.mjs:15`) | 19330 | yes: expected. `logger` runs on evaluate and pattern errors; it debounces repeated identical messages by time and then `console.log`s them and dispatches a DOM event when a `document` exists | none: with the clock at 0, a repeated identical message is always debounced, but the logger's output never reaches the job (`console.log` is discarded; `info.errors` takes `console.warn`/`console.error` only, decision 12) |

`new Date`, `crypto.`, `getRandomValues`, `Intl.` and `toLocale`: no hits.

Result: no reachable hit beyond the three expected modules (`NeoCyclist`'s `Math.random`, core's
clock in `util.mjs`, the logger's `performance.now`). The three extra `Math.random` hits are in
third-party code that the host never calls.
