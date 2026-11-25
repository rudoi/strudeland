# 0113: one song header module

## Goal

Every module that reads a song header gets everything it needs from one call. Each rule about the
header's values lives in one place. No error kind travels as a text prefix.

This is a refactor. Nothing a user, the CLI or the control protocol sees changes, byte for byte.

## Background

The [re-verification](0113-song-header-module/reverify.md) records the evidence on `main` at
8312dd9. In summary:

- **The cross-field rules are written twice.** These are the rules that compare header fields with
  each other: `meta.loop` against a loop section, `meterChanges` against the total bars, and `loops`
  against whether a loop exists. `readHeader` writes them out to attach source positions, and
  `buildStructure` writes them again. One error message is duplicated word for word.
- **Six callers repeat the same steps.** They call `parseMeta`, then `staticStructure` or
  `evaluateOptions`, then build the `"msg (l:c)"` and `"meta: …"` text by hand.
- **`backing.ts` reads the `"meta: "` prefix back** to decide whether a failed evaluate was a header
  error.
- **The rules for single fields are already shared.** `meta.ts` imports `bpmProblem`,
  `meterProblem` and the others from `structure.ts`.
- **`buildStructure` must stay a validator.** `song()` calls it on the evaluated `meta` object,
  which has no source positions.

## Scope

In:

1. **One set of cross-field rules.** `buildStructure` is the only place they live. The reader of
   the parsed header translates a failing field into its source node.
2. **One song header module.** `src/lib/project/meta.ts` becomes
   `src/lib/project/song-header.ts`. It absorbs `evaluate-options.ts` and exposes
   `readSongHeader(code)`. All six callers move to it.
3. **A typed error kind for a failed evaluate.** The session's internal result names the kind:
   `meta`, `instruments` or `eval`. `backing.ts` reads the kind instead of the prefix. The control
   handler maps the result to the wire shape, which does not change.
4. **`GLOSSARY.md` terms:** song, song header, structure and section. They were added during
   planning.

Out:

- **Any change to visible text or positions.** That includes error messages, the
  `metaError`/`error` fields, and the CLI output.
- **Exposing the error kind on the control wire.** That is follow-up task #137, blocked by this one.
- **The other review candidates.** The song playback module (#114) builds on this task.
- **Renaming `EvaluateOptions.expectedArrangement`.** See Open questions.
- **The default structure built inline** in `runtime.ts:555` and `recording/controller.ts:396`
  (`buildStructure({})`). These read no header.

## Architecture

```mermaid
flowchart TB
  subgraph callers
    S[session.ts]
    J[render/job-plan.ts]
    F[editor/format-song.ts]
    W[project/meta-write.ts]
    C[render/corpus.ts]
    B[scripts/bench/scheduler.ts]
  end
  callers --> H["song-header.ts<br/>readSongHeader(code)<br/>headerRanges(code)"]
  H --> R["internal: find and parse the header,<br/>quote checks, every-occurrence checks"]
  R --> ST["structure.ts<br/>buildStructure: per-field and cross-field rules<br/>StructureError {field}"]
  SONG["song.ts: song() at evaluate time"] --> ST
```

Two entry points, one rule set:

- **The parsed path:** `readSongHeader` works on text, with positions.
- **The evaluated path:** `song()` works on the evaluated object, without positions.

Both end in `buildStructure`.

`format-script.ts` and `meta-write.ts` keep using `headerRanges` for minimal-diff edits.

## Technical design

### `structure.ts`

- **`StructureError` gains `field`.** It names the `meta` key at fault: `bpm`, `meter`, `sections`,
  `loop`, `meterChanges` or `loops`.
  - Each `check(...)` call in `buildStructure` passes its key.
  - The thrown `'meta.loop cannot be used with section loop: true'` passes `loop`.
- **Messages are unchanged.** `song()` still prefixes them with `song: `.

### `song-header.ts`

The renamed `meta.ts`.

**Public interface:**

- `readSongHeader(code): SongHeader`.
- `headerRanges(code)`, unchanged.
- `LINE_BREAK`, which `meta-write.ts` uses.
- The types `Meta`, `MetaValue`, `MetaError` and `SongHeader`.

The result carries the formatted error texts, so no caller builds them.

**No longer public:** `parseMeta`, `staticStructure`, `evaluateOptions` and `EMPTY_STRUCTURE`. They
become internal or disappear.

**Result shape:**

- On success: `{ ok: true, meta, title, structure, options }`. `options` is what
  `runtime.evaluate` takes today from `evaluateOptions`.
- On error: `{ ok: false, error, structure }`.
  - `error` is `{ message, line, column, pos }`, plus the two formatted texts: `"msg (l:c)"`, and
    `"meta: msg (l:c)"` as the planner, the session and the corpus show it today.
  - `structure` is the default structure.
  - There are no options, so a caller cannot evaluate a song whose header is broken.
  - There is no title, as today, so callers fall back to the file name.
- **A header that fails to parse** (an acorn syntax error) stays `ok: true` with an empty `meta`,
  as today. Evaluating the song reports it.

**Inside:**

- `readHeader` keeps the checks that need source positions or apply only to the parsed header:
  - finding the header;
  - literal-only values;
  - quote checks;
  - per-field checks on every occurrence of a key;
  - range checks on earlier duplicate `loops` keys.
- **It drops its cross-field block.** Once the per-field checks pass, it calls `buildStructure`
  once. A `StructureError` becomes a `MetaError` at the last occurrence of `error.field`. The error
  order is preserved: per-field checks run first, as today, and `buildStructure` checks loop, then
  meter changes, then loops, the same order `readHeader` uses.
- **The structure from that call is the result's `structure`.** That removes the second
  `buildStructure` call and `staticStructure`'s catch.

### Callers

| Caller | Today | After |
|---|---|---|
| `session.ts` `derive` | `parseMeta`, `staticStructure`, its own `(l:c)` text | `readSongHeader`: title, structure, error text |
| `session.ts` `evaluateSong` | `parseMeta` again, then `evaluateOptions` | `readSongHeader`: `options`, title |
| `session.ts` `evaluateBuffer` | `"meta: " + metaError`, `error: string` only | the kind plus the same text (below) |
| `session.ts:315` title only | `parseMeta(code).meta.title` | `readSongHeader(code)` title |
| `render/job-plan.ts` | `parsedMeta`, `evaluateOptions`, `staticStructure` fallback | `readSongHeader`; a `PlanJobError('meta', …)` with the same text |
| `editor/format-song.ts` | `parseMeta`, `staticStructure(meta).meter` | `readSongHeader`: structure meter, `ok` |
| `project/meta-write.ts` | `parseMeta(...).error`, `.meta` | `readSongHeader` |
| `render/corpus.ts` | `parseMeta`, plus its own `buildStructure` try/catch | `readSongHeader` |
| `scripts/bench/scheduler.ts` | `parseMeta`, `staticStructure`, `evaluateOptions` | `readSongHeader` |

`job-plan.ts` is the riskiest caller to get wrong. The engine jobs it produces must stay
byte-identical.

### The error kind

- **Inside the session.** `evaluateBuffer` returns the wire `EvalOutcome` fields plus
  `errorKind: 'meta' | 'instruments' | 'eval' | null`.
  - The session interface types this as a session-level result that extends `EvalOutcome`.
  - The precedence is the one `evaluateBuffer` uses today: a meta error, then an instrument pin
    error, then the runtime's `evalError`, then `'evaluate failed'`.
- **`backing.ts`** maps `errorKind === 'meta'` to its `meta` outcome. Instruments and eval stay
  `eval`, as today. `META_PREFIX` goes.
- **The control handler** builds the wire `EvalOutcome` explicitly: `evaluate` and the `state`
  result's `evaluated` field. `errorKind` never reaches the socket.

### Tests (replace, don't layer)

- **New: `tests/song-header.test.ts`.** One table of header texts. Each row asserts the full result:
  `ok`, structure (or its key fields), options, and the exact error text and position. The
  `parseMeta` and `headerRanges` cases from `tests/meta.test.ts` move here.
- **New: a pairing test.** It runs each cross-field error through both entry points: the header text
  through `readSongHeader`, and the evaluated object through `buildStructure`. It asserts that the
  messages match, which enforces the "both paths agree" claim.
- **Deleted:** `tests/evaluate-options.test.ts` and `tests/meta.test.ts`. Their cases are folded
  into the table above.
- **`tests/structure.test.ts`** keeps the cases on the evaluated object. It gains assertions on
  `StructureError.field`.
- **Updated** for the new import and result shape: `tests/meta-write.test.ts`,
  `render-corpus.test.ts`, `render-job-plan.test.ts`, `runtime-backends.test.ts`, the session and
  backing tests, and `control-handler.test.ts` (no `errorKind` on the wire).
- **`recording-backing.test.ts`** asserts the outcome kind from `errorKind`, including an
  instrument-pin failure.

### Regeneration (AGENTS.md)

- `job-plan.ts` is a planner file, and `meta.ts` and `evaluate-options.ts` are bundled into the
  headless host. Run `bun run host:build`, `bun run engine:refs --write` and `bun run host:parity`.
  - The provenance hashes change.
  - The jobs must come out byte-identical. Any change to job bytes is a bug.
- **Full gate:** `bun test`, `bun run check`, `bun run lint`, `bun run build`, and
  `cargo test --workspace` (the refs staleness test).

## Dependencies

- None open.
- **This task blocks:** #114 (song playback module, which arms sections by name against one trusted
  structure) and #137 (the error kind on the wire).

## Risks

- **Error order or position drift.** The parsed path now gets its cross-field errors from
  `buildStructure`. A header with several problems must still report the same first one, at the
  same node. The table tests pin first-error cases with combined faults, and the pairing test pins
  the messages.
- **The duplicate `loops` key semantics.** Earlier occurrences are only range-checked; the last one
  is checked against the loop. `buildStructure` sees only the last value, so the
  earlier-occurrence check stays in the reader. Pin it with a test.
- **Planner byte drift.** Guarded by `host:parity` and the engine refs.
- **Hot path.** `derive` runs on every keystroke. The new path calls `buildStructure` once instead
  of twice, so it costs no more.

## Open questions

- **"Arrangement" means two things.** `EvaluateOptions.expectedArrangement` (`runtime.ts`) holds
  the structure, but `GLOSSARY.md` uses "arrangement" for the written order of patterns. Should it
  be renamed? That is out of scope here; if yes, file a follow-up.
- **Complexity is medium.** About 12 source and test files, one subsystem (the frontend), and the
  design decisions settled in Q&A. Recommendation:
  - skip Fracture, so the plan stays one document;
  - keep Singletonize as a short pass;
  - run a light Loss check.

## Decisions from planning Q&A

1. **Scope:** cross-field rules, the song header module and the typed error kind, in three commits
   in that order.
2. **Visible behaviour:** every message, position and wire text stays byte-identical.
3. **Regeneration:** `host:build`, `engine:refs --write` and `host:parity` are accepted. A change
   to job bytes is a bug.
4. **Name:** the term is **song header**, recorded in `GLOSSARY.md`.
5. **Rule owner:** `buildStructure` owns the cross-field rules. `StructureError.field` lets the
   parsed path attach each error to its node.
6. **Module:** `meta.ts` becomes `song-header.ts` and absorbs `evaluate-options.ts`.
   `readSongHeader` replaces `parseMeta` and its companions.
7. **Result shape:** success or error. On error there are no options, and the module owns the error
   texts.
8. **Error kind:** internal only. The wire is unchanged; #137 tracks exposing it.
9. **Tests:** replace, don't layer. One table test, and a pairing test for both entry points.
