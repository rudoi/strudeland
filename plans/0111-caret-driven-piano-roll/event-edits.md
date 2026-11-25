# Source-backed piano roll event edits

The first drag can wrap the inspected pattern at its point of use. For example, moving a generated
C4 from the last beat of the first bar to the first beat of the second can produce

```js
(line.fast(2))._pianoEdits({ entries: [
  { from: ['3/4', '1'], pitch: 60, to: '1' }
] })
```

Fractions and other data strings use single-quoted JS literals because the Strudel transpiler
interprets double-quoted strings as mini. Parenthesize the **entire** receiver, including a
conditional or comma expression. For final-output edits use a trailing block comment
`/* @piano-edits {"entries":[{"from":["3/4","1"],"pitch":60,"to":"1"}]} */`. This JSON form
survives a labeled stack, `all()`, and an unlabeled final return. Escape comment terminators
in JSON strings. Read only the dedicated top-level trailing comment through Acorn. Songs
without the marker take the existing fast path. Reject malformed or duplicate declarations
with a source diagnostic. The editor updates the existing entry on successive gestures.

## Boundaries

- `src/lib/strudel/event-edits.ts` owns entry validation, exact fraction arithmetic, pitch
  decoding and encoding, identity, projection and the Pattern operation. Register the raw
  scalar method in desktop/render and host prebakes. Preserve `_steps`. A materialized edit
  makes the result time-dependent, so a constant-value optimization from the receiver would
  be incorrect. An auxiliary query uses a fresh painter list, while the main query registers
  painters in the caller's state once. Replacements retain live context including source
  locations, custom routing and schedule callbacks.
- `createRuntime` reads final-output entries from the original song and applies them in
  `repl.editPattern` to the fully composed logical pattern before playback clock conversion.
  Browser, desktop, page render and native host use this same code. Keep source passed to the
  transpiler intact for mini offsets and inspection capture. A failed evaluation never
  installs its entries. The raw method avoids lexical helper shadowing. Refuse a registration
  collision; never silently use a song-defined method of the same name.
- The inspection reply includes a typed `RollNote.event` identity in TypeScript and Rust.
  The display id remains ephemeral. `preparePhraseEvents(notes): RollNote[]` supplies the
  equivalent span/pitch/duplicate-count selector synchronously for proven authored phrases.
  Host inspection derives selectors from raw haps in the exact scope it observed.
  Prepare a local target for Expression, an identified lane for a proven section-local
  projection, and otherwise a written-song selector on the fully composed output. A source
  location alone does not prove timing equivalence through an outer transform.
- `planEventEdit(source, scope, note, gesture): readonly RollPatch[] | null` in
  `src/lib/piano-roll/event-source.ts` takes `{ kind: 'expression', expression: SourceSpan }`
  or `{ kind: 'song' }`. It consumes the ready selector, merges the gesture with the entry,
  emits checked UTF-16 patches and identifies the entry for reveal. Source integrity and one
  isolated undo transaction belong to the session. A failed direct codec falls through in the
  same call. Selection of a shared definition applies the wrap there. Editing a use wraps that
  use, and arrangement output is edited after transforms. The planner never evaluates the song
  on pointer release. Parse its output, preserve comments in the receiver, and update only
  generated entry data on repeat edits. Recognize the existing wrapper when targeting its receiver.

## Event and query semantics

An entry stores the original whole span, original projected MIDI pitch and sufficient evidence
to select one voice. Add sound, orbit, label, pitch representation, and differing serializable
controls when needed. A contributor discriminator must survive insertions and formatting. Use
verified token identity or relative musical atom identity, and retain raw offsets only for
navigation in the current revision. For a group of observationally identical haps record its
multiplicity and instance within that group. Validate multiplicity on the onset-aligned probe.
The host's enumeration index across a multi-bar query cannot identify a note. Projected
replacements carry their original entry identity in hap context so the second drag updates
the same record. Maintain original source identity separately from the destination span.

Query the inspected pattern at the original onset as well as over the inspected window. The
onset probe must find the same voice group, value shape and whole. Use exact rational Strudel
edges throughout. At runtime:

1. Query the underlying pattern normally. Remove the matched original wherever its part
   intersects the requested span, including continuation-only queries. Keep the relative
   order and values of other haps. Apply entries simultaneously against the input. An edit
   moving A onto B must not cause a second edit on B to consume A.
2. If the destination intersects the query, retrieve the original using a canonical
   onset-aligned query, even if the source is outside the query window. Share that result
   for all entries at the same origin in this query. Keep the incoming seed and controls.
   Obtain source CPS from its own meter at the original onset when it differs from the
   destination. Never run a schedule callback or resolve mutable state while probing.
3. Construct a Hap with the new whole, its intersection with the requested destination
   window, updated pitch, original context, and stateful flag. Where source parts have
   internal gaps preserve those by shifting the source parts or querying the inverse-mapped
   destination fragment; filling the entire whole would sound muted time. Preserve exactly
   one onset, including point queries and a note crossing integer or meter boundaries.
   Translate MIDI through the actual active representation, undoing octave offset for the
   underlying `note` or `freq`. For expression primitives preserve scalar shape. Accept
   finite numeric primitive pitches only in a known pitched Expression scope.
4. Preserve the gate on pitch or onset changes. Strudel's `Hap.duration` reads an explicit
   numeric duration before the whole length and then multiplies by clip. Keep gate and
   structural span distinct in inspection. Resize only when their relationship is established.

In Expression use exact local coordinates. Restrict an unproved period to the specified local
occurrence. In song split global requests with `locate` and `pieceEnd` using the effective
live or supplied segments. Translate each piece to its written coordinates, then translate
the destination whole back by that occurrence's offset. During a cross-section move retrieve
the source on the written linear layout, with original source meter. Suppress the old note
on each visit to its written occurrence. The destination plays on visits to its own written
position, including when playback starts there without visiting the origin. A jump, loop wrap,
count-in or release must not create an onset halfway through a sustained note. Preserve layers
outside a `song()` placement on their own clock. With no arrangement the declared logical
cycle is absolute. A proven section-local edit that leaves that section must use final-output
scope to reach the destination.

## Fresh seeds, fragments and identity limits

For a stochastic pitch with a stable producer lane and onset, an explicit captured entry stores
that lane, origin span and bounded JSON control snapshot. Replace the selected lane occurrence
even when another run gives that lane a different pitch. Replay the captured controls at the
destination. A genuinely absent stochastic occurrence can be pinned at its destination when
the lane is isolated and replayable. Other voices and cycles continue to evaluate normally.
Continuous haps sample at their query midpoint. Capture one declared finite source window in
an isolated lane, split queries at its edges, suppress that lane inside it and emit the edited
discrete occurrence. Never extrapolate one sample to a whole cycle unless that whole cycle
is the declared source window. Include capture scope and controls in the entry.

Arbitrary JavaScript can create two haps with equal values, source spans, labels and timing,
different hidden callback closures, and order that changes on every query. Persisted JSON
cannot identify the same closure on reopen. A stateful Hap can carry a function that only
produces a pitch while advancing scheduler state. Inspection must not invoke such a function.
Use live object/context preservation where a distinct stable selector exists. An opaque tie
may use a demonstrated stable canonical order only when tests prove partition equivalence.
If observation cannot distinguish the lanes or preserve the callback, return a specific
inspection diagnostic for that material. A runtime mismatch after manual edits should surface
as a stale edit in inspection and leave existing output intact. Never suppress an entire stack
or duplicate a matched original on a guess. These specific classes are a refinement decision
if independently editable notes are required for arbitrary JavaScript.

## Proof and acceptance

A Bun probe with the vendored transpiler retained single-quoted scalar fields in the raw
method and left an unlabeled final expression as the return when followed by the directive.
A `stack` of two identical voices with whole `[3/4,5/4)` returned both onsets at a
zero-width `3/4` query, both continuations at `[1,5/4)`, and neither at `5/4`. That establishes
the probe for ordinary discrete haps. It does not establish arbitrary custom Pattern behavior.

- Compare haps as multisets and compare equal-time order where scheduling uses it. Cover a
  repeated/polyphonic local phrase, generated scalar and tagged pitches, frequency plus octave,
  stochastic lane, exact duplicates, different pan/orbit/callback, fragment-only queries,
  `duration` and `clip`, source locations and painters. Partition at every old and new edge.
- Verify normal render and query paths, whole-window and sliced scheduler queries, count-in,
  clip and linear repeats, jump, mixed meter and cross-section moves. Exercise labels, soloing,
  `all()`, top-level returns, local declarations, formatting, save/reopen and two edits to one
  entry. Check stale sources and malformed bounded data without modifying the editor.
- Bound inspection payload by actual encoded bytes as well as count, depth and span length.
  Share probes by onset and cap total query work under the existing host deadline. A budget
  failure is explicit. Check Rust deserialization and Tauri reserialization retain the full
  identity. Rebuild the host bundle and verify page/host job parity for saved event edits.
