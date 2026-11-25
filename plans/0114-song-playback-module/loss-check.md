# 0114 loss check (light)

I compared the planning decisions, the user's answers and the refine findings against
`plans/0114-song-playback-module.md`.

## Preserved

- **The research's decisions 1-9.** All carried. Decision 9 grew into a shared transport model
  (decision 9 and scope item 1).
- **The user's answers.**
  - Decision 5 keeps the Timeline's arm.
  - Decision 7 fixes the armed name on the wire. It was widened to all three cases after a second
    answer.
- **Refine, must fix 1-10.** All carried:
  1. decision 7's three cases and the handler test rewrite;
  2. the module follows runtime stores and clears armed names from `pendingStart`;
  3. construction and wiring (`session.playback`, `BackingDeps`, controller and handler deps);
  4. `metaError` in `getHeader`;
  5. one generation counter;
  6. verb ordering contracts;
  7. loop names, the raw view, the derived `canArm`, `stopLooping`'s two behaviours, the `head`
     translation;
  8. `playFrom`'s guard paths;
  9. `metaTitle` and `titleFrom` on rename, `instruments()`, the `removed` rule;
  10. the transport model extracted rather than re-implemented.
- **Refine, nice to have 11-15.** All carried:
  - `AGENTS.md`;
  - the churn counts;
  - the missed test files;
  - duplicate names resolve to the first section, as today;
  - the extra readers.

## Modified (justified)

- **"The stub becomes a faithful model built from transport.ts's pure functions" became "extract
  the runtime's transport glue into a shared factory".** Building the stub from the pure functions
  alone would re-implement the glue, which the plan's own risk list forbids.

## Unintentionally dropped

- None.
