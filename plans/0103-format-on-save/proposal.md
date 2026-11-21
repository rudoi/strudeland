# Format Strudel mini-notation on save

## Goal

Make dense patterns easy to scan in the editor and on disk. A 4/4 cycle with 32 equal slots
should read as two rows of 16 thirty-second-note slots, with visible groups of four and a wider
gap on the quarter-note boundary. Rests occupy the same visual positions as notes. Both
rows belong to the same cycle.

```js
$: note(`
  c4 ~  e4 ~   g4 ~  e4 ~     a4 ~  g4 ~   e4 ~  d4 ~
  c4 ~  e4 ~   g4 ~  a4 ~     g4 ~  e4 ~   d4 ~  c4 ~
`).s('piano')
```

The layout adds two spaces after slots 4 and 12 and four spaces after slot 8, after padding
each column to the widest token in that cycle. A 16-slot pattern fits
on one row with four groups of four, each group occupying one beat.

## Scope

- Add an app-wide **Format on Save** preference, off by default. Start with ordinary 4/4
  patterns written as 16 or 32 equal, explicit steps in a flat sequence. Include equivalent
  flat sequences inside the already-present bracketed bars of a bare slow sequence such as
  `<[bar one] [bar two]>`. Keep those brackets and the existing bar order. Apply this
  layout when all its bars are eligible, leaving mixed or complex sequences intact.
- A step is a bare mini atom or rest. Leave weighted steps, repeats, speed modifiers, ties,
  tuplets expressed through nesting, stacked or random alternatives, and compound or odd
  meters to a later design. Existing tokens, delimiters inside mini, and note lengths are
  preserved. A newline is presentation and contributes no musical timing.
- Keep the first release focused on pattern layout. Whole-song JavaScript formatting,
  configuration for grouping, a format-only command, and virtual grid views are outside this
  proposal.

## Architecture

A pure formatter under [the editor library](../../src/lib/editor/) takes a song snapshot and
returns localized text edits. Acorn, already pinned by the app and used in
[Strudel's transpiler](../../vendor/strudel/packages/transpiler/transpiler.mjs), provides song
syntax and source ranges. Use its script settings from the transpiler, including ECMAScript
2022 and top-level await. The vendored [mini parser](../../vendor/strudel/packages/mini/mini.mjs)
provides musical structure. Reuse the existing [metadata parser](../../src/lib/project/meta.ts)
and [meter defaults](../../src/lib/project/evaluate-options.ts). The formatter runs without a
DOM, an audio engine, or execution of the song.

[The project session](../../src/lib/project/session.ts) prepares the save, applies edits to
the editor or stashed state, then hands the exact text to the existing native write API.
[App composition](../../src/lib/app.ts) supplies the live preference. An editor preference
control can reuse the pattern in [StatusBar.svelte](../../src/components/StatusBar.svelte).
The native layer continues writing the text supplied by the session.

## Technical design

1. Parse the complete snapshot. Consider static expression literals that the vendored
   [mini plugin](../../vendor/strudel/packages/transpiler/plugin-mini.mjs) treats as mini
   notation. Recognize double-quoted strings and untagged templates with one quasi and no
   substitutions. Exclude the metadata initializer, JS directives and literal keys, single
   quotes, escaped source, tagged templates, and `mini-off` regions, including nested and
   unclosed regions. Honor the transpiler's custom mini-language registration by skipping
   built-in mini formatting when a replacement language is active. A syntax or metadata
   error allows the usual save of the input. An ineligible mini literal does not prevent other
   eligible literals in the same file from formatting.
2. Use the parsed mini tree to recognize a root equal-weight sequence or a flat bracketed
   cycle under a bare top-level slow sequence. Use the original source ranges to recover
   atom spellings and separator spans. Parser ranges can include surrounding whitespace.
   Confirm the recovered slices correspond to the parser's atom values, and use the original
   spellings, including sample names, note octaves and rest spellings. Use spaces for columns,
   preserve surrounding JavaScript indentation, and make the layout idempotent. A single
   line can keep its quote style. Use ordinary untagged backticks where a double-quoted
   pattern needs physical line breaks. Reparse the edited JavaScript and check that each
   candidate still occupies the intended expression position. Reparse the mini body and
   require structural equality after removing source locations only. Preserve weights,
   operators, alignment, seeds and order in the comparison.
3. Prepare formatting at the common session save boundary, once per snapshot. This covers
   Mod-S, Vim `:w`, saves during Play and close/quit, deleted-file recovery, and desktop
   control/CLI saves. Preserve each entry point's existing rules for which songs need
   writing. Explicit editor Save can format a clean active song. Play on a clean song
   remains a pure evaluation. Save-and-evaluate uses the prepared snapshot that was
   actually written, including when the user types while the write is in flight.
4. Apply only separator and quote edits in a single isolated CodeMirror history transaction
   through the session's `apply`. Changes should map selections, note highlights, and
   other decorations through CodeMirror. Plain Save continues playback on the current
   evaluated snapshot. After waiting for a move, verify the original project and song
   identity before touching the buffer. If the source has advanced during that wait, save
   the originally captured snapshot without applying formatter edits to the newer buffer.
   If writing fails after formatting, leave the formatted buffer editable, dirty relative
   to disk, and undoable. Use the prepared string for the in-flight watcher echo check.
5. Add a defaulted `formatOnSave` boolean to [native preferences](../../crates/strudeland-project/src/prefs.rs)
   with the current schema. Ensure the [frontend preference store](../../src/lib/prefs/store.ts)
   merges startup hydration per field and serializes full preference snapshots so rapid
   changes preserve both Vim and formatting. Document the formatting effect on desktop CLI
   saves in the [embedded CLI skill](../../crates/strudeland-cli/skill/SKILL.md).

## Dependencies

The design uses the existing Acorn and mini parser, CodeMirror history, session write seam,
and preference storage. It covers the whole-object persistence and startup race relevant to
task #78. The policy for downgrading across preference schemas remains in that task. A future
native File > Save from task #96 should use the same editor save action. Task #101's note
audition is on a separate branch at the planning base. Recheck source-location behavior if
it lands before implementation.

Prettier's normal quote handling is unsafe for mixed Strudel source. The repository already
excludes songs from its `singleQuote` pass. Prettier supports custom embedded-language
printers, but Strudel's mini literals require special handling beyond ordinary tagged
templates. Use a targeted formatter for this scope. See
[Strudel's mini-notation guide](https://strudel.cc/learn/mini-notation/) and
[Prettier's plugin API](https://prettier.io/docs/plugins/).

## Risks and validation

- A local Bun probe through the vendored transpiler and mini runtime preserved all 96 ordered
  events, exact whole and part arcs, and a step count of 32 over three cycles after reflowing a
  flat 32-step sequence. Adding brackets around each quartet changed the step count to 8.
  Use rational event and step equality as regression tests. Include real notes and rests,
  both quote forms, existing slow sequences, two passes producing the same bytes, unsupported
  patterns, and malformed input.
- The mini location helper currently removes ASCII spaces from leaf ranges. A local probe
  found the final token on the first row can include the line break in its end offset.
  Verify playback highlighting and the reported start of tokens across rows, then check
  the range's end where it affects an editor action. A vendor correction, if needed, must
  have a focused test and follow the repository's patch and reference-regeneration rules.
- Extend the session tests for normal save and Save-and-Evaluate, active and stashed
  buffers, caret/selection and isolated undo, watcher echoes, pending moves, edits during
  save, and failed writes. Extend preference tests for loading old files, delayed startup
  hydration, and alternating toggles. Smoke-test the desktop preference and play a
  representative song before and after saving it.

## Open questions

- Does the proposed two-row layout match how you want to count a dense pattern? Four-slot
  groups and the wider quarter-note gap can be tuned in Refine.
- Should the first release also lay out another specific meter or a weighted pattern you
  already use? A representative song would help choose the next safe layout rule.

Recommended workflow complexity is **medium**. The formatter, save transaction and preferences
span multiple parts of the app. Keep singletonization and a light loss check. The proposal fits
in one document, so skip fracture after refinement.

## Automated refinement

The user authorized one automated refinement pass and advancement through PR creation to
the review gate. Keep the proposed layout and opt-in scope. Use one change per separator
or delimiter so CodeMirror can map individual tokens. Do not buffer formatting behind an
asynchronous parser. Isolate the session save behavior from any higher-level action that
might evaluate or display a document. The [implementation contract](design.md) specifies
the save snapshot and preference hydration rules.
