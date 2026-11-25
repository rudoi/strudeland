# 0113 re-verification (step 0)

Checked on `main` at 8312dd9 (2026-11-25), after the piano roll and instrument browser landed.
The review that raised this ran against 63b945d.

## Confirmed

- **The cross-field rules are worked out twice.** `readHeader` (`src/lib/project/meta.ts:266-288`)
  re-runs the cross-field rules to attach source positions: `meta.loop` against a loop section,
  `meterChanges` against the total bars, and `loops` against whether a loop exists.
  `buildStructure` (`src/lib/strudel/structure.ts:330-370`) runs the same rules again.
  - The literal `'meta.loop cannot be used with section loop: true'` appears in both files
    (`meta.ts:271`, `structure.ts:338`).
- **`staticStructure` relies on agreement nothing enforces.** It catches a `buildStructure`
  throw "that does not happen" (`src/lib/project/evaluate-options.ts:8-13`), relying on the two
  rule sets agreeing.
- **Callers repeat the same steps.** They call `parseMeta`, then `staticStructure` or
  `evaluateOptions`, then format the error themselves:
  - `session.ts:628-631`: `derive`.
  - `session.ts:1070-1088`: `evaluateSong`, which parses the same text a second time.
  - `render/job-plan.ts:89-113`.
  - `editor/format-song.ts:340-341`.
  - `scripts/bench/scheduler.ts:119-145`.
  - `render/corpus.ts:181-188`, which uses its own `buildStructure` try/catch.
- **The error kind is a text protocol.**
  - The `(line:col+1)` format is built in two places: `session.ts:630` and `job-plan.ts:93`.
  - A `meta: ` prefix is added in `session.ts:1672`, `job-plan.ts:93`, `corpus.ts:183,188` and
    `StatusBar.svelte:85`.
  - `recording/backing.ts:73-85` reads that prefix back to choose `kind: 'meta' | 'eval'`.

## Corrected from the review

- **The rules for single fields are already shared.** `meta.ts` imports `bpmProblem`,
  `meterProblem`, `sectionsProblem` and the rest from `structure.ts`. Only the cross-field rules
  are duplicated.
- **`buildStructure` must keep validating.** `song()` (`src/lib/strudel/song.ts:56`) calls it at
  evaluate time on the evaluated `meta` object, which has no source positions. Two entry points
  are legitimate:
  - the parsed header, with positions;
  - the evaluated object, without them.

  So the duplication to remove is the cross-field rules themselves, not one of the two entry
  points.
- **Some behaviour is specific to the parsed header.** These stay in `meta.ts`:
  - quote checks;
  - checking every occurrence of a duplicate key;
  - `headerRanges`.

## New since the review

- **Instrument pins are a third error source.** `evaluateBuffer` now reports instrument pin
  errors (`instrumentError`, `session.ts:1077`, `:1667`) alongside meta and eval errors.
  `backing.ts` files them under `eval` by default. A typed error kind should name all three.
