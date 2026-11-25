# 0113 loss check (light)

I compared the planning Q&A (decisions 1-10), the user's refine input and the refine review's
findings against `plans/0113-song-header-module.md` at a01b6cc.

## Preserved

- **Decisions 1-10.** Each appears in "Decisions from planning Q&A" and is carried into Scope and
  Technical design.
- **Rename of `expectedArrangement` (user, refine cycle 1).** It is in Scope item 5 and decision 10,
  and is the first commit.
- **Refine review, must fix 1-5:**
  - the order of errors for duplicate `loops` keys (Inside, steps 1-3, and two pinned rows in
    Risks);
  - the three forms of error text, including the corpus form without a position (Result shape);
  - the choke point for keeping `errorKind` off the wire, `evaluateGuarded`, with exact-key tests;
  - the `AGENTS.md` docs update and the commit order;
  - `meta` stays in the success result, with `EvaluatedSong.meta` kept.
- **Refine review, nice to have:**
  - `field` is required on `StructureError`, including in `withLoops`;
  - the corpus drops its `buildStructure` catch;
  - `setMetaValue` keeps two reads;
  - the hot-path wording;
  - the Goal wording;
  - `host:parity --batch`;
  - `PLANNER_FILES` stays unchanged.

## Modified (justified)

- **"Three commits" became four.** The rename now goes first, because it would otherwise touch
  `evaluate-options.ts` after that file was removed.
- **Regeneration happens in every commit, not once at the end.** The host-bundle staleness test
  must stay green in each commit.

## Intentionally dropped

- **A lighter internal path for `setMetaValue` and `headerRanges`.** It is not needed, because the
  cost is unchanged or lower.

## Unintentionally dropped

- None.
