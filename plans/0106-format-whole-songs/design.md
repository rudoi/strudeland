# Whole-song formatter contract

## Layout

Keep the existing opt-in preference. Use two spaces beyond the line where the receiver actually
starts in the output. A call followed by a normal dot method qualifies even with one suffix.
Otherwise require two direct dot methods on the receiver spine. Align all suffixes to the same
column and retain trailing punctuation. Parentheses create a boundary. An optional or computed
edge keeps that spine as written. Independently nested arguments may still qualify.

Format fields of the supported leading metadata header, entries in its sections array, and fields
of a literal parts object passed to `song(meta, …)`. Use one entry per line and compact simple
section objects, retaining rows in multiline entries with comments. Preserve grouping parentheses
around these containers. Expand direct calls to `stack`, `seq`, and `cat` when an argument
contains a qualifying chain that can expand or already spans multiple lines. Align items one level
beyond the opener's line and the closing delimiter with the opener. This decision depends on the
syntax and still holds after a chain is already split.
Leave ordinary helper calls compact around their arguments unless another rule applies inside them.

Apply group and chain edits only at whitespace gaps between known tokens. Preserve blank lines
when reindenting existing line breaks. Leave comment-crossing gaps in place and preserve comments
and raw literals byte for byte. Treat tagged and interpolated templates as opaque. Match the
supported leading header using the existing metadata range reader, and leave an invalid header
alone while visiting other statements. Use the first line ending outside literal tokens for
inserted JavaScript line breaks, with LF as the fallback.

## Integration

Reuse the Acorn script grammar, collect ordered tokens, and use grouping-aware nodes to delimit
receiver spines and list items. Collect intended gaps with earlier source anchors, then resolve
each anchor's indentation from the emitted prefix. This handles a nested receiver or a second
same-line receiver that moves after an earlier break. Verify the complete JavaScript AST outside
positions and verify exact raw token and comment spelling. A failed JavaScript pass can still
yield verified mini edits.

Run a mini pass first to establish its line breaks, then plan JavaScript layout and run mini
again for final indentation. A compact 16-step backtick literal can otherwise pull a later
receiver onto an earlier line and cause a second-save indentation change. If the final mini pass
changes text, check the combined layout again. Allow a bounded number of reconciliation passes
and fall back to the independently verified mini result if the combined result fails to stabilize.
Use CodeMirror's change composition with LF as its explicit split character so embedded CRLF
stays in the original code-unit coordinate system. Return only original-coordinate, localized edits.
Retain the existing mini timing verification, metadata and language gates, and mini comment ranges.

Keep the session path, preference default, exact native bytes, and editor history behavior
specified by [task #103](../0103-format-on-save/design.md). Tests must cover live and stashed saves,
one-step undo of both kinds of edit, dirty Save and Play, and editor range mapping. Runtime smoke
should exercise an opted-in save with a nested chain and an eligible mini pattern.

## Refinement

The user confirmed two-space chain indentation and authorized automatic refinement. The bundled
layout, opt-in control, and single-effect behavior follow the proposal. Independent review identified
the output-indent and cross-pass stability cases above. No native or transport behavior needs a new
interface.

## Delivery status

Implemented and validated in [PR #17](https://github.com/rudoi/strudeland/pull/17).
The [delivery record](000-prompt.md) lists the checks and desktop smoke evidence.
