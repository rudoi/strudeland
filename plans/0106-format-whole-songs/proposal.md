# Format whole Strudel songs on save

## Goal

Make a saved song easy to scan and edit at the level of voices, sections, and effects. Give every
method in a pattern chain its own line, and keep the existing pulse layout for dense mini-notation.
Use the existing opt-in **Format on Save** control and apply the combined result in one undo step.

## Research

Strudel's coding guide uses leading dots for effects chains and demonstrates commenting out an
individual effect. Strudeland's own corpus mixes inline chains, partially wrapped chains, and
one-call-per-line chains. Its arrangement fixture also shows the intended shape of a `song` parts
object. [Strudel coding guide](https://strudel.cc/learn/code/) ·
[Late feedback fixture](../../tests/render/corpus/late-feedback.js) ·
[Song arrangement fixture](../../tests/render/corpus/song-linear.js)

Quote style carries meaning. The vendored mini plugin interprets double quotes and eligible untagged
backticks as mini-notation, and single quotes represent plain strings. The app's static metadata
parser also checks the spelling of selected strings. In a local probe with the repository's Prettier
3.9.10, both quote settings kept a short effects chain inline at width 100. With single quotes
preferred, Prettier changed `note("c4 e4")` to `note('c4 e4')`. With double quotes preferred, it
changed `.s('triangle')` to `.s("triangle")`. Stock Prettier therefore has two mismatches with the
requested behavior. A custom printer could handle these rules but would also need comment handling,
Strudel literal handling, and a way to recover localized editor edits. Acorn already exposes tokens,
comments, and parenthesis boundaries, which are sufficient for the proposed whitespace changes.
[Strudel mini-notation guide](https://strudel.cc/learn/mini-notation/) ·
[Vendored mini plugin](../../vendor/strudel/packages/transpiler/plugin-mini.mjs) ·
[Metadata parser](../../src/lib/project/meta.ts) ·
[Prettier options](https://prettier.io/docs/options.html) ·
[Acorn parser API](https://github.com/acornjs/acorn/tree/master/acorn#interface)

## Scope

- Format normal dot-call chains in voices, section parts, variables, and common callbacks. Keep the
  receiver and any voice label on the first line. Place every method suffix on a separate line,
  indented two spaces from the receiver's containing line, even when a chain fits inline. A base call
  followed by one method qualifies. For a bare receiver, require two method calls so standalone
  member calls such as `Math.round(x)` stay compact. The rule is syntactic and also covers
  user-defined methods and chains whose receiver is a callback parameter.
- Format the leading `const meta` header, its `sections` list, and literal parts objects in
  `song(meta, { … })`. Put each header field, section entry, and named part on its own line. Keep
  simple section objects compact with spaces around their contents. When a direct argument of
  `stack`, `seq`, or `cat` expands, give each argument its own row. Leave compact calls that
  contain no expanded chain in their current layout. Normalize spacing around the delimiters,
  commas, property colons, and assignments owned by these rules.
- Retain the existing rhythmic layout for eligible 16- and 32-step 4/4 mini and eligible slow
  sequences. Apply JavaScript layout in other meters and when a custom mini language is active.
  Broken JavaScript bypasses formatting and keeps the existing save behavior. A parseable but invalid
  header keeps its original text, suppresses mini formatting, and still permits layout in the body.
- Preserve spelling and order of names, numbers, punctuation, comments, and literals in the
  JavaScript pass. Keep blank lines between top-level voices. Preserve comment-only lines and skip
  layout around comments when a change would move a comment to a different line or alter its
  attachment. Keep tagged templates and interpolated templates opaque, including their substitutions.
- Outside this release are a general-purpose JavaScript style pass, complex mini-notation layout,
  format-as-you-type, a format command or CLI flag, import-time formatting, and per-rule preferences.
  Leave optional and computed receiver spines in their current layout for the first release.

### Example

Before:

```js
const meta={title:'Sketch',bpm:122,meter:'4/4'}
$: stack(s("bd ~ sd ~").bank('RolandTR909').gain(.8),note("c3 ~ e3 ~").s('triangle').lpf(800).gain(.25))
```

After:

```js
const meta = {
  title: 'Sketch',
  bpm: 122,
  meter: '4/4'
}
$: stack(
  s("bd ~ sd ~")
    .bank('RolandTR909')
    .gain(.8),
  note("c3 ~ e3 ~")
    .s('triangle')
    .lpf(800)
    .gain(.25)
)
```

The comma stays at the end of the first voice and the outer call's closing parenthesis aligns with
the `$:` line. Each nested chain uses its own receiver's resulting indentation. For an effect with
a multiline argument, the next dot returns to the same continuation column as the other effects.
A nested chain in a callback can start with `x => x` on the containing line, with its dots indented
one level from that line. Receivers wrapped in parentheses retain those parentheses.

## Architecture

Keep `formatSong(source)` as the pure coordinator returning sorted, nonoverlapping edits in
original UTF-16 coordinates. Put the JavaScript layout rules in a sibling editor module. Keep the
mini parser and rhythmic decisions in the existing formatter. Continue to call the coordinator
from the shared session write path, after it checks that the captured snapshot still matches the
editor. [Formatter](../../src/lib/editor/format-song.ts) ·
[Session](../../src/lib/project/session.ts) · [Previous formatting contract](../0103-format-on-save/design.md)

Apply JavaScript layout first so mini rows align to the line the literal will actually occupy.
Compose the two passes with CodeMirror's existing change representation or equivalent offset
mapping. Dispatch the result as a single isolated history event. Preserve the original snapshot
when the editor advances during an asynchronous save. Write and, for dirty Save and Play, evaluate
the exact formatted snapshot. Explicit Save and Vim `:w` can format a clean song. Clean Play
continues to bypass writing, and control socket saves retain their existing dirty-song rules.

## Technical design

1. Parse with the vendored transpiler's Acorn script settings. Collect tokens and comments. Use
   actual receiver boundaries, including grouping parentheses, and the leading dot token to identify
   gaps. Find each maximal direct-call spine once, then visit receiver and argument subexpressions
   for independent chains. Use syntax nodes to distinguish a method dot from a decimal, string,
   regex, or comment. Treat optional and computed calls as unsupported edges and skip the containing
   spine. Use actual delimiter tokens for objects, arrays, and argument lists.
2. Plan the surrounding container layout before planning inner indentation. Determine which
   containers expand from their syntax and child layout, then emit nonoverlapping gap edits
   top-down with the target indentation. This keeps nested patterns stable across saves. Replace
   only whitespace between adjacent tokens or in a comment-safe gap. Preserve the script's existing
   line endings outside literals, including for callers of the pure formatter. Leave a region
   unchanged when a delimiter or comment boundary cannot be assigned safely.
3. Reparse the JavaScript result and compare the full AST with only source positions removed. Verify
   raw token and comment spellings are unchanged. This catches changes to automatic semicolon
   insertion, expression grouping, and Strudel labels. If verification fails, keep the original
   JavaScript and continue with the existing independently verified mini pass.
4. Run the existing mini pass against the verified layout. Keep its metadata and meter gates,
   `mini-off` and `mini-on` ranges, per-literal mini AST comparisons, and exact-span masking for
   quote changes. Compose the edits without consuming unchanged source tokens. If a pass fails its
   checks, keep any independently verified edits. The formatter performs no song evaluation.
5. Update the status-bar tooltip, README, and embedded CLI skill to explain the broader behavior.
   Preserve the current preference default and persistence.

## Dependencies

Build on task #103 and existing Acorn, CodeMirror, and vendored mini APIs. Use the current frontend
save path and the existing test fakes. This proposal requires no new package, preference field,
native command, or vendored patch. Recommended workflow complexity is **medium**. Keep
singletonization and a light loss check. Decide during refinement whether a separate fracture
document would help.

## Risks and validation

- A quote or newline can change parsing. Cover real Strudel labels, automatic semicolon insertion,
  directives, regex and decimal tokens, parenthesized receivers, comments between effects, and
  opaque template contents. Check malformed JavaScript, invalid metadata, other meters, custom mini
  languages, and mini-disable directives. Test exact output, raw token preservation, and zero edits
  on a second pass. Extend the current mini tests to compare rational hap arcs for songs changed by
  both passes. [Formatter tests](../../tests/format-song.test.ts)
- Source offsets legitimately change. In focused transpiler or runtime tests, compare playback
  values, timing, and ordering while allowing source locations and derived IDs to move. Verify that
  widgets and highlights point at the formatted source after evaluation. Test selection, undo, and
  highlight mapping during Save, including dense mini on a newly indented method line.
  [Widget plugin](../../vendor/strudel/packages/transpiler/plugin-widgets.mjs) ·
  [Session tests](../../tests/session.test.ts)
- Batch edits can make synchronous saves sluggish. Measure already formatted and densely chained
  corpus songs and the live-pressure fixture locally before setting a latency budget. Keep normal
  paths to a bounded number of whole-file parses and reuse token lookups. Exercise exact disk bytes,
  watcher echoes, concurrent typing, stashed CLI saves, and write failures. Smoke-test Save, Vim
  `:w`, undo, and dirty Save and Play in the desktop app.

## Open questions

1. Start with the existing toggle applying both layout passes, or offer separate controls for
   rhythmic layout and JavaScript layout? The proposal uses one toggle, off by default.
2. Should everyday one-effect patterns such as `s("bd").gain(.8)` always split? The proposal says
   yes so a voice has a stable shape as effects are added.
3. How much container layout belongs in the first release? The proposal includes arrangement
   headers, named parts, and pattern-combining calls alongside chains so a typical song formats
   coherently. Arbitrary JavaScript objects and statements keep their current layout.
