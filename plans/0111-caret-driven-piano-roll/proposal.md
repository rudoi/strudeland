# Caret-driven piano roll and editable song grids

## Goal

Open the piano roll and place the caret inside pitched material to work with it immediately.
Every note the roll presents accepts a drag. Moving a note previews the result during the drag and
updates the song source in the editor as soon as the gesture finishes. The editor scrolls to the
change and the affected notes visibly acknowledge the edit.

## Scope

- Follow one caret while stopped or playing, with explicit selection as an override. Keep the
  existing toggle, fitted pitch range, auditioning, and Expression and In song views from
  [the original roll](../0109-meter-aware-piano-roll/proposal.md).
- Every visible pitched note supports pitch and onset drags in either view. Preserve resize, add
  and delete for the existing simple phrases and expand them wherever the new write path supports
  the operation. The interaction is the same for authored, reused, transformed, and generated notes.
- Prefer a localized edit to the original notation when it can express the requested change in its
  intended scope. When that fails, write an explicit note edit into the song script around the
  relevant pattern or arrangement. The first drag does this automatically in one undo step.
- Use Luoyuan’s eight-bar grid as the direct-edit acceptance case. Focus the row under the caret,
  keep neighboring bars accessible, and show notes that continue across rows. Keep the verified
  glide behavior and retained suffix text when editing the original grid.
- Ship no new automatic save or transport restart behavior. A successful drag changes the buffer
  immediately and remains audible through the existing audition feature where it is enabled.
  Playback adopts the changes through the existing Re-evaluate flow. General direct cross-bar
  serialization is tracked in Keitora #110. A cross-bar drag that requires the fallback still
  commits in this task.

## Evidence from Luoyuan

The matching script was found in the local render fixture `target/underflow-profile/input/songs/original.js`. It declares a 120 BPM, 4/4 song with an eight-bar intro. The complete script stays in the local project. A synthetic regression fixture can exercise the same behavior with different pitches.

| Source stage | Observed behavior |
| --- | --- |
| `fluteGrid → fluteSteps` | Eight quoted rows, each trimmed, split on whitespace and checked for exactly 16 slots. The rows are flattened before interpreting holds. |
| `fluteSteps → fluteEvents` | Pitch starts a note. A following run of `_` extends that event across row boundaries. `~` starts silence. A leading `_` without a sounding onset becomes silence. |
| Pitch suffix | A decimal suffix becomes `pattack` in seconds. When a previous pitch exists, the consumer also computes `penv` from the pitch interval. The previous pitch persists through rests. The defaults are 0.09 seconds and a bend of 0.55 where the relevant glide inputs are absent. |
| `fluteEvents → fluteCall → intro` | `stepcat(...fluteEvents).slow(fluteGrid.length)` makes a slot exactly 1/16 of a cycle. The intro layers the authored line through a wrapper that adds a pitched sine voice and a noise voice. |

The existing host query over the intro returned ten orbit-12 sine onsets. The opening A5 begins at cycle zero and lasts 27 slots, ending at the G5 onset in bar 2. At 120 BPM the output duration is 3.375 seconds. The G5 with suffix `:2` lasts five slots, or 0.625 seconds, and the host reported its bend as −2 semitones. The suffix interpretation comes from the source helper. The CLI query summary does not expose `pattack`. Verify that control with direct queried values or engine jobs during implementation.

A pitch change can therefore intentionally change the current or next gliding note’s bend even when the other note’s pitch, duration, and source text stay fixed. The test oracle must verify that coupled effect. A hold at the start of bar 8 may belong to the D5 onset in bar 7.

A read-only probe against a temporary copy retuned the opening A5 to B5. The orbit-12 sine and noise events changed pitch at the same onset. The next gliding G5's bend changed from −2 to −4 semitones at the same onset and duration. The other queried events matched as a multiset between these paired runs.

## Architecture

| Step | Contract | Existing seam |
| --- | --- | --- |
| Resolve | Locate the pattern, exact snapshot, authored or arranged scope, and optional source bindings | [Editor setup](../../src/lib/editor/setup.ts), [session](../../src/lib/project/session.ts), [source resolver](../../src/lib/piano-roll/source.ts) |
| Project | Return notes with exact logical spans and sufficient identity to select an edit. Preserve the written bar and meter anchor. | [Mini phrase codec](../../src/lib/piano-roll/phrase.ts), [host inspection](../../src/host/inspection.ts), [meter geometry](../../src/lib/piano-roll/grid.ts) |
| Plan | Given one completed gesture, return checked source patches, the spans to reveal, and the identities and positions of affected roll notes. Prefer direct notation, then use the saved event edit. | [Source planner](../../src/lib/piano-roll/source.ts) and a shared event-edit operation to design |
| Commit | Apply the plan to the same song and revision as a single CodeMirror transaction, map the caret, reveal the changed text, and return the result used for visual feedback | [Project session](../../src/lib/project/session.ts) and [editor visuals](../../src/lib/editor/visuals.ts) |
| Reconcile | Render the new note immediately, then reconcile it with source projection or refreshed inspection while retaining focus and feedback | [Piano roll](../../src/components/PianoRoll.svelte), [grid gestures](../../src/components/PianoRollGrid.svelte), [inspection queue](../../src/lib/piano-roll/inspection.ts) |

## Technical design

### Caret inference and target stability

Distinguish a single caret, explicit range, and multiple ranges in the snapshot. Use the existing
editor listener and cached syntax analysis so intra-pattern caret motion can retain the target,
scroll, mode, meter anchor, and inspection result. Keep exact explicit-selection resolution
first. Infer unambiguous pitched literals, recognized array rows, constructors, receiver chains,
and lexical aliases. Ignore unrelated automation arguments, comments, property keys, and blank
lines. Keep the last context briefly during incomplete edits and invalidate pending gestures as soon
as the document or edit scope changes.

The source/occurrence anchor is separate from editor scroll and selection revision. Scrolling the
editor to reveal an edit should leave focus where the user is working and keep the viewed pattern
and bar visible in the roll. A song switch, a different target, or a conflicting document mutation
cancels a pending gesture. Dock collapse suspends inspection and cancels pending feedback.

### Direct edits and scope

Try the original notation first when the caret or explicit selection is in the authored definition
or the user is editing a phrase expression whose effects are within scope. Resolve shared uses
using the current binding analysis. A drag at a particular use or arrangement occurrence should
default to that use or occurrence if a definition change would affect other music. A drag while
viewing the actual definition changes its shared notes, with affected uses visible. Preserve an
explicit route from a use to its definition. Neither route inserts a confirmation between the drag
and its result.

For grids, prove the Luoyuan interpretation using a narrowly recognized consumer and synthesize
tests with the same semantics. Keep a built-in construction as the documented authoring route if
a recognizer cannot prove equivalent legacy code. Map a retune to the pitch token. Map rhythm
gestures to a fixed number of cells while preserving quotes, comments, glide suffixes, unaffected
cells, and sister rows. Use the existing integer logical-cycle bars and exact fractions for
timing. Draw cross-row holds from their originating token. Validate complete event effects,
including the glide dependency on the previous onset. If a note edit is outside the direct
codec’s range, try the event-edit route in the same gesture.

### Source-backed edits for generated and arranged notes

Design a standard operation that wraps the relevant pattern and carries a compact, editable list
of note edits in the script. The first drag can add the wrapper and its first entry together.
Further gestures update an existing entry when they refer to the same edited note. Leave the
original expression in place and evaluate it normally. Attach edits at or after the transforms
that produced the displayed notes, so the gesture applies in the same musical coordinates shown
to the user. The operation should be available in the desktop, browser, offline render and
headless host from the same implementation. The review should verify pattern method metadata,
painter behavior, source locations and runtime scope registration.

- In Expression, wrap the smallest expression that produced the inspected note. Use a
  pattern-local scope, with a finite written window when the expression has no known period.
  In In song, prefer a section or voice value whose timing matches the displayed event. Fall back
  to a final-output edit keyed to the written song occurrence when the note has no trustworthy
  contributor. Resolve occurrence through the song layout so repeated transport segments and
  exports agree on which written note was changed. Free-running patterns use a declared logical
  cycle.
- A source contributor or host note index alone cannot identify an evaluated note. Use the
  original whole span, pitch representation, voice, available contributor data and a validated
  discriminator for duplicate haps. Establish identity in an onset-aligned probe and test that
  it agrees across fragmented queries and repeated occurrences. A stochastic edit needs a
  defined retry policy under fresh seeds and controls. Prototype an occurrence-scoped captured
  event when live matching cannot preserve the demonstrated result. Its replacement scope and
  reproducible controls must be explicit in the written source. A first-drag fallback must
  neither change other voices nor silently guess which duplicate event to replace.
- Filter the original event where it overlaps the query. Query its origin when the edited span
  intersects a destination window that would otherwise never see it, then intersect the
  replacement with the requested window. Preserve controls, callbacks and source provenance
  when moving pitch and time. Test `note`, `freq`, numeric expression pitches, stateful or
  continuous output, and overlapping voices. Represent fragmentary observations as explicit
  editable occurrences when the original does not carry a discrete whole span. Verify whether
  duration and clip controls require a separate representation to keep the visual and audible
  lengths in sync.
- Compile and validate the operation before notes become interactive. Prepare the necessary
  identity and bounded data during inspection, so pointer release can update the editor without
  round-tripping through a second evaluation. A replacement must remain understandable after
  save, reopen, format, and re-evaluation. If prototype coverage exposes a class that cannot meet
  this contract, bring that class back to refinement and document the product choice before
  requesting implementation approval.

### Reveal and feedback

Use the grid’s existing pointer capture, live drag preview, and release handler. Keep release
atomic with one undo step for all changed notes and source ranges. Scroll the editor to the
resulting token, row, or event-edit entry in that transaction, centering an off-screen change
without moving focus out of the roll. For several changed spans, reveal the primary one and mark
the others where they are visible. Map feedback ranges through the committed changes.

Add a short outline pulse to the edited note or notes and a corresponding editor decoration on
the changed text. Use a static highlight with reduced motion. Clear feedback on expiration,
target invalidation, or dock close, and ensure it remains distinct from audition and playback
highlights. Key the visual response to the immediate edit result so it survives regenerated note
IDs and delayed host responses. If a transaction is cancelled or rejected because the buffer
changed, restore the current notes and explain the conflict without implying an edit succeeded.

## Dependencies and validation

- Extend [source and phrase tests](../../tests/piano-roll-source.test.ts),
  [geometry tests](../../tests/piano-roll-grid.test.ts),
  [host inspection tests](../../crates/strudeland-host/tests/inspection.rs) and the project session
  tests. Reuse the checked editor transaction and existing history isolation. Verify generated
  edit source is formatted, reparsed and evaluated on every supported path.
- Desktop acceptance starts with opening the dock once. Put the caret inside a mini note phrase
  and drag. Repeat for a Luoyuan row, an alias use, a generated expression with no direct source
  edit, and an In song event with ambiguous provenance. Every successful drag is visible in
  source immediately, scrolls to the new text, pulses the edited note, and undoes in one step.
  Repeat with the editor scrolled away, with keyboard note editing, and with reduced motion.
- Query exact before/after haps for Luoyuan, including glides, noise/sine layers, holds across
  rows, and edits unsupported by the row codec. Test overlap, repeats, mixed meter, random
  notes, look-ahead windows, query partitioning, exact moved durations, and inverse shifts across
  meter and section boundaries. Compare unaffected output and controls.
- Measure caret movement and pointer release with a long song, dense overlapping events and
  a slow inspection response. Verify cancellation and stale result handling on source changes,
  save/reload, switching songs, and collapse. Compare playback, host query, desktop and CLI
  rendering, and the render-page/headless job parity for the new source operation. Rebuild the
  host bundle and its inputs if the shared runtime changes.

## Risks and open questions

- Query-time identity across changing seeds, stateful and continuous signals, and indistinguishable
  overlays is the central risk. Prove the deterministic and captured-occurrence routes and decide
  how to expose stale matches after manual source changes.
- Scope needs to be obvious when navigating shared phrases. The proposed default is to edit a
  use where it appears, and affect all uses when working on the definition itself.
- A local edit wrapper is a song syntax change. Prototype the source form with actual Strudel
  transpilation and formatter coverage, including single-quoted plain strings and helper name
  shadowing. Use exact source validity and playback/render parity as exit criteria.
- Large cross-cutting work. Keep fracture, singletonize, loss check and sequencing. Resolve the
  fallback matcher, occurrence scope and feedback transaction contract before building UI to them.
