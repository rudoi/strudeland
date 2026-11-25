# 0115 loss check (light)

## Preserved

- **The research's decisions 1-13.** All carried, with decision 12 corrected.
- **The user's four answers** (Cmd-., Timeline, flash, both CLI quirks kept). They are recorded
  as approved changes 1-3 and as decisions 6-7. Follow-ups #141 and #142 are filed.
- **Refine, must fix 1-8:**
  1. stop stays unconditional outside a take;
  2. `wasPlaying` is "runtime started" on entry;
  3. `stop()` returns `done`;
  4. the audition gate is exact;
  5. the constructor dependencies and the `stopAuditions` side effects;
  6. the Timeline per-state spec, help precedence, `canPlayFrom`, and `play ending` and `stop
     looping` left out;
  7. the goal is limited to UI and control callers;
  8. the flash and `onStop` contracts.
- **Refine, nice to have 9-16:**
  - gesture and `App.svelte` untouched;
  - `beforeProjectChange`, `MemoList`, `StatusBar` and `PianoRoll` placement;
  - the handler seam;
  - teardown;
  - the missed tests;
  - the table cells;
  - the ambiguities.

## Modified (justified)

- **Decision 12 is reversed.** "Stop only when started" became "unconditional outside a take".
  The original rule would have changed behaviour: it would no longer cancel an in-flight
  autostart or silence auditions.

## Unintentionally dropped

- None.
