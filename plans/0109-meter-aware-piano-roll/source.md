# Source resolution and editing

[Approved proposal](proposal.md) · [Refinement audit](refinement-audit.md)

Related contracts: [Pattern inspection and arrangement](inspection.md), [Piano roll dock and note auditioning](dock.md).

## Selection and binding resolution

Take a snapshot of the active project, song, CodeMirror document revision and UTF-16 selection.
Add a dedicated selection and document listener in [setup.ts](../../src/lib/editor/setup.ts).
Also refresh the snapshot through the [session's `onShown` hook](../../src/lib/project/session.ts)
because the session swaps songs with `setState`. Keep focus changes into the roll from clearing
the active target.
Start with one nonempty selection. Resolve an exact expression, variable identifier, complete
pitch atom selection inside a mini literal, or recognized labeled pattern body. Trim trivia
and keep the smallest supported scope that contains the complete selection. Show the resolved
target and its source range before offering edits. An atom selection opens its enclosing pitched
pattern with those atoms selected. Arbitrary ranges spanning unrelated nodes and multiple editor
selections get a clear status. Caret-only inference can follow once selection boundaries are
proven.

Use Acorn with the same script options as the [transpiler](../../vendor/strudel/packages/transpiler/transpiler.mjs).
[format-song.ts](../../src/lib/editor/format-song.ts) already handles mini directives and conservative
literal detection. [note-audition.ts](../../src/lib/editor/note-audition.ts) distinguishes value
children from suffix arguments. Reuse those semantics where they have the same contract.
Classify candidates by lexical bindings and positively identified call positions. A JavaScript
string, a numeric atom, a property key and a note can have the same spelling. Confirm the
builtin pitch producer and the helper emitted by the transpiler's
[mini plugin](../../vendor/strudel/packages/transpiler/plugin-mini.mjs). Locally shadowed helpers,
dynamic scopes and method calls with uncertain receivers have no source edit capability.

Follow immutable local `const` aliases, respecting scope and shadowing, including a pattern
passed through known section/voice combinators. Retain the definition and the selected use as
separate spans. Use the distinctions in the [glossary](../../GLOSSARY.md) for source use sites
and written pattern occurrences. Collect direct and transitive lexical use sites, including
aliases, labels, and `song(meta, parts)` section references when provable. Mark escapes through
callbacks, computed properties, mutable aliases, object indirection and unclassified calls as
potentially affecting additional uses. Permit edits to an independently verified definition
with the uncertainty displayed. When the definition itself is uncertain, retain inspection and
omit write-back. Show
affected known uses before the first shared edit in a selection context, then let successive
gestures use that target until the context changes. When a selected expression transforms a
shared pattern, show the result of that expression. **Edit shared definition** takes the user to
the authored pitches in Expression and retains the selected occurrence as the meter anchor.

## Notation support and provenance

The examples below describe the first-release support boundary. Test the display and edit
decisions independently. A transformed expression can show editable source pitches in
Expression when the user selects the source operand, even when its generated result is
inspectable only.

| Strudel form | Expression / in-song inspection | Write-back |
| --- | --- | --- |
| `note("c4 ~ e4")`, MIDI numbers, static backticks, weighted atoms, simple slow sequences for multiple bars | Authored pitch, rests, timing and meter. Compare with Strudel query. | All five operations within the supported phrase grammar. |
| `[c4,e4]`, nested subdivision, `<c4 d4>`, `!`, `*`, `/`, Euclidean suffix, ranges, random choice | Use real mini timing and a bounded evaluation window. Distinguish group, alternate, repeat and generated instances. | Direct pitch replacement of a uniquely verified source leaf, including an individual note in an authored chord. All occurrences of a shared atom update together. Timing of simultaneous notes, arbitrary nesting, generated hits and individual alternate instances stays view-only. |
| `note(60)`, `note(seq('c4','e4'))`, `cat`, `stack`, `timeCat`, explicit pitches in constructors | Query the actual composition. Use AST source ranges even when the runtime has no locations. | Verified direct pitch literals and individually selected supported mini operands. General JS composition rhythm is view-only. |
| `const motif = note(...)`; `motif`; `motif.transpose(12)`; aliases, labels, section reuse | Show the expression or a verified use. Show direct and transformed uses separately when their identities are known. | Edit the shared authored target. Inspect transformed notes in context. Per-use timing or pitch inverse can be added only for an established operation. |
| `n("0 2 4").scale("C4:minor")`, `chord("C^7").voicing()`, arpeggiators | Query degrees, scale-produced pitches and voicings. Link generated events back to their source when known. | Generated pitches stay view-only. Degree- or chord-aware edits require explicit rules. |
| `struct`, `mask`, `fast`, `slow`, `rev`, `every`, `sometimes`, random or callback-generated values | Query finite audible-pattern projections with captured controls and seed. Indicate dynamic or unavailable context. | Editable source operands remain available in Expression. Generated event geometry or a sample of nondeterministic output supplies no write-back capability. |
| `freq`, interpolated text, escaped/normalized strings, tagged languages, stateful or external functions, drum and sample selectors | Inspect pitched output when the host can reproduce it. Report unavailable runtime inputs. Pure percussion has no piano pitch. | Continuous, ambiguous, unsupported and external source stays view-only. |

Bun probes against the pinned Strudel verified that `stack(motif, motif.transpose(12))` can share
one pitch source between two sounding events, `c4!2` reuses one atom, and Euclidean and speed
suffix arguments add their own locations. Applying `.gain("0.5 0.8")` put gain locations before
pitch locations. `.struct("x ~ x x")` produced multiple onsets from the same E4. A chord voicing
produced multiple pitches with one chord token. Quoted JS sequence arguments can emit no
locations, and a double-quoted mini literal with a JavaScript escaped newline produced shifted
locations. These observations establish test cases, including a literal intentionally chosen to
fool a pitch-looking-token heuristic.

The [mini parser](../../vendor/strudel/packages/mini/mini.mjs) distinguishes syntax and emits
positions, and [Hap.combineContext](../../vendor/strudel/packages/core/hap.mjs) merges
contributors. Use mini AST roles and verify exact raw UTF-16 source slices for edits.
A display location may link to a contributor such as a scale, speed or gain, or be unavailable
altogether. Context-only locations never grant an edit. Start with source forms whose raw and
parser coordinates can be checked directly. Keep rewrite support out of normalized and escaped
forms until a verified offset mapping exists.

## Writing changes

An edit intent contains the gesture, exact source revision, target capability, expected text at
all changed ranges, and source/use snapshot. Preview the gesture locally; commit once on pointer
release or completed keyboard action. Recheck project, active song, document and selection
generation, target and relevant pins immediately before dispatch. Invalidate old gestures even
when a switch or undo returns to byte-identical text. Follow
[the session's `applyInstrument`](../../src/lib/project/session.ts) for project checks, one range
transaction and isolated undo. Use the session's normal dirty and watcher behavior. Keep use
selection separate from the changed definition range. Map selection and focus through the edit
and update the roll from the resulting buffer. Undo and
redo update both views through their normal document listener.

- Pitch replacement patches one exact atom or numeric/string literal, preserving spelling style
  when practical, indentation and surrounding notation. Where one token generates several notes,
  explain which other displayed events will change too.
- For initial timing, resizing, addition and deletion, recognize closed monophonic bar phrases
  of direct pitches and rests, with simple per-bar weights. A move or resize preserves other
  onsets, durations and bar length. Rewrite the smallest eligible span with enough rest/weight
  capacity to represent the result, retaining the containing JS expression and sibling bars.
  Use the [transcriber's weighted notation](../../crates/strudeland-transcribe/src/notation.rs)
  as a reference for valid rest and slow-sequence encodings. Tied groups still need an edit rule.
  Encode exact ratios using bounded positive integer weights reduced by their common divisor.
  For example, moving a quarter-bar E4 from beat 3 to beat 4 in a 4/4 `c4 ~ e4 ~` can write
  `c4 ~@2 e4`. Delete leaves silence for the
  vacated interval. Deleting the final note can write a rest-only phrase, and insertion can split
  a rest. Reject unsupported overlap, out-of-phrase moves or disproportionate rewrites with an
  actionable reason.
- Before dispatch, reparse the candidate and check exact event spans, pitch, event counts,
  untouched intervals and unchanged enclosing JavaScript shape outside the intended replacement.
  Source-gesture validation uses the restricted mini grammar. Query comparable authored phrases
  in tests to catch parser-versus-runtime drift. Explicitly guard mixed-meter bars, tuplets and pattern
  weights whose effective duration is relative to sibling weights. A roll rectangle represents
  the pattern's logical interval. Playback `clip`, envelopes and sample tails can differ.
- If the prototype establishes a safe cross-bar move, preserve musical duration across meters
  and encode it using the destination bar's fractions. A quarter note moved from 4/4 to 7/8
  spans two eighth-note beats in either bar, so its duration changes from `1/4` to `2/7` of a
  cycle. Release scope depends on proving the exact rewrite and preservation of other notes.

## Interface and verification

Use a pure TypeScript module under `src/lib/piano-roll/`. The input is the current source and
one selected UTF-16 span. Return the eligible expression, any authored target, and known
shared-definition uses with arrangement section names when statically proven. An authored
target carries note intervals and source operations. The edit planner takes a note or bar
operation and returns precise replacement ranges for the original snapshot or an unavailable
result. The session owns revision checks and dispatch. This division keeps UI and process
state outside the grammar.

Cover direct literals, aliases and shadowing, exact token locations, chord-leaf pitch changes,
weighted monophonic rewrites, unchanged adjacent events, and rest-only phrases. The source path
reuses parsers and performs no whole-song evaluation.
