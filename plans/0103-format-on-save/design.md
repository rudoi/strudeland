# Implementation contract

## Formatting

Implement `formatSong(source)` in `src/lib/editor/format-song.ts`. It returns a sorted list of
nonoverlapping edits with `from`, `to`, and `insert` in UTF-16 source coordinates. The
session owns applying edits. The formatter returns an empty list when parsing the song
or validating metadata fails, the song meter differs from 4/4, or another mini language
is installed. Unsupported literal shapes are skipped individually.

Read Acorn's comments using the same mini disable ranges as the transpiler. Leave plain
strings, metadata, tagged templates and their substitutions, interpolated templates,
escaped source, JavaScript keys, directives, and contexts where a replacement could
become a tag alone. Verify the resulting script parses and preserves its syntax tree
outside the intended literals. Compare the mini AST with only locations removed for
each replacement. Identify atom and rest spans from the vendored grammar, verify the
slices, then edit separator and delimiter spans only. Preserve existing bracketed bars
inside a bare slow sequence. Count 16 equal explicit steps for a row or 32 for two rows.
Use the document's existing line indentation plus two spaces for multiline content and
keep delimiters and comments outside the literal in their original positions.

## Session and preferences

The shared session write path captures the preference and save snapshot before its first
wait. Before settling a pending move, retain the project generation and song identity.
If either identity changes, abandon the write. If only the editor source changes during
the wait, write the originally captured source and keep the newer editor state untouched.
Otherwise apply the localized changes as one fully isolated history event and use the
resulting source for the native write, watcher echo checks, and evaluation. Return a
nullable saved source to distinguish an empty song from a failed save. Preserve the
existing write criteria for every caller, including save-only and Save and Play.
A write failure leaves the formatted buffer dirty and undoable.

Use the existing preference store and bridge. A defaulted `formatOnSave` field is compatible
with existing files. While hydration is pending, optimistically update the selected
control and queue each change. Persist complete ordered snapshots after loading the
untouched fields. Keep the default off. Surface write errors through the existing status.

## Decisions

Accept the vendored leaf-range end behavior only if note starts and visible highlighting
still behave correctly across rows. Track an existing leaf-range defect separately
unless it interferes with the feature. Keep Rust file writes byte-exact and retain control
API save outcomes. The embedded CLI skill documents the opt-in desktop behavior.
