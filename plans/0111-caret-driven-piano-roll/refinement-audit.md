# Refinement audit

The user’s feedback makes editing the primary interaction. “Evaluated expression · view only”
is evidence of a missing edit route. Generated and in-song notes need a source write path that
works on the same first drag as a note with a known literal.

## Code references and decisions

| Finding | Effect on the design |
| --- | --- |
| [`pianoRollSnapshot`](../../src/lib/project/session.ts) returns a null range for a single caret. [`locateTarget`](../../src/lib/piano-roll/source.ts) also rejects an empty range. | Update both boundaries. Keep explicit selection validation and test caret positions lexically inside and outside the pitch value. |
| [`PianoRoll.svelte`](../../src/components/PianoRoll.svelte) derives `editTarget` from an authored phrase in Expression. Shared phrases can be displayed before that gate is satisfied. [`PianoRollGrid.svelte`](../../src/components/PianoRollGrid.svelte) additionally requires capability flags to start a gesture. | Supply a ready edit plan context for all displayed notes in both views, including an event-based path where there is no writable token. Avoid publishing an interactive note until the information needed to commit it is ready. |
| The grid already captures a pointer, renders a prospective note, then invokes `onedit` at release. | Define immediate as live roll preview during movement and a single synchronous editor edit on drop. No additional Apply control or selection step. Keep keyboard edits on the same commit path. |
| [`editPianoRoll`](../../src/lib/project/session.ts) checks the snapshot and isolates history. [`revealPianoRoll`](../../src/lib/project/session.ts) selects the text and focuses the editor. | Keep the guarded edit path. Add a mapped scroll effect to that transaction, leaving editor selection and roll focus intact. Explicit navigation may still choose source and focus it. |
| [`visuals.ts`](../../src/lib/editor/visuals.ts) decorates an audition token. [`inspectHaps`](../../src/host/inspection.ts) uses a host query index as the note ID and can return fragments or multiple contributors. | Handle edit feedback independently from audition. Use the commit's new spans and destination notes as feedback anchors and form a durable event identity for write-back. |
| [`song`](../../src/lib/strudel/song.ts) queries section-local patterns, then shifts and clips them to occurrences. [`queryMetered`](../../src/lib/strudel/metered-pattern.ts) queries in logical time and splits at meter changes. | Apply event changes in the matching logical time domain, before reference-clock mapping. Test repeated written sections, cross-meter boundaries, and final-output fallback for unattributed song events. |
| [Browser/render prebake](../../src/lib/strudel/prebake.ts) and [host prebake](../../src/host/prebake.ts) register scope separately. | Any edit operation written into the script needs common semantics in both, and parity in the normal render entry points. |

## Probes and limits

- Ran the existing headless host against the local Luoyuan fixture and a temporary retuned copy.
  The paired query result changed only the two layers of the edited onset and the next sine
  glide’s bend. The original fixture was read only. A regression test should fix its seed and
  inspect complete hap controls, including `pattack`, which is absent from the query summary.
- Ran an in-memory probe using the vendored `Pattern`, `Hap` and `TimeSpan`. A note was moved from
  `[3/4, 5/4)` to `[5/4, 7/4)`, absent from its original query interval, and split across two
  destination queries with only one onset. The replacement kept a sample control and context
  while changing pitch. This verifies the query-at-origin technique for one deterministic
  event. The probe does not establish stable selection across arbitrary patterns.
- Artifact checks should reproduce the entire user loop in a desktop smoke and prove edit
  semantics in partitioned scheduler and render queries. In particular, test generated pitches
  driven by randomness or controls, exact duplicates, stateful output, frequency-valued pitches,
  and events with duration/clip overrides. Decide how an occurrence-scoped captured edit will
  represent any output that cannot be matched reliably at runtime.

## Requirements carried forward

The editor script remains the source of truth. Caret targeting, shared definition navigation,
the collapsible panel, note auditioning, adaptive pitch range, mixed meter, and stale-snapshot
protection still belong in the plan. Luoyuan remains the direct source acceptance case.
The earlier proposal relied on a separate conversion preview to unlock some notes. The new
requirement makes the drag itself commit a small, visible, undoable source change. It also asks
the editor to reveal the change and the roll to visibly acknowledge the edited notes.

The proposed scope rule keeps direct edits to the shared definition global and makes a drag on a
particular use local when changing the definition would also move other occurrences. This is a
change from the previous default to offer editing the shared definition first. Keep a visible
control to open that definition and show affected uses. Confirm this scope rule during refinement.
