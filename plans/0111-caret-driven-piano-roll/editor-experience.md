# Caret, gestures, and editor feedback

## 1. Resolve a stable working context

Expose a single CodeMirror caret as a zero-width span in `PianoRollSnapshot.selection`. Keep a nonempty selection authoritative and return no range for multiple selections. Pass either single range to `resolveRollTarget`. Use song path, source text, resolved expression span, mode, and chosen written occurrence to identify inspection work. Keep this identity independent of the editor selection revision. Caret motion within a resolved expression retains the inspection, anchor, page, and shared-definition controls. The authored grid target can change its focused row independently of inspection identity. A different expression, document change, song switch, or collapse invalidates pending gestures. Bind generated draft approval to the song and code. A successful roll-authored transition may carry approval forward to its exact resulting code.

## 2. Project notes ready to edit

Build the displayed grid and commit context together. Expression mode uses the existing phrase projector for writable mini notation and authored rows. Other expressions and In song use host notes with prepared event-edit identity. Keep the expression or written-song scope with each prepared context. Give every displayed pitched note pitch and onset gestures, and expose add, delete, and resize where their planners support them. An alias use can link to the shared definition. Use the actual definition to edit all its uses. Within a single use or written occurrence, send the gesture through the occurrence planner when a direct token edit would affect other music.

Keep pointer movement as a prospective note in the existing grid. At gesture start, keep the prepared context identity for release. Resolve the completed gesture through `planRollEdit` for source-safe authored edits and `planEventEdit` when direct notation cannot express the scoped change. Prepare this fallback during inspection or from eligible local notes so drop can commit synchronously. Use the same callback for keyboard edits. Restore the displayed notes and give concise feedback when current source rejects the gesture.

## 3. Commit and reveal one change

Have the session verify the active song, source text, expected source spans, and working context before dispatch. Combine all patches into the existing history-isolated transaction. Map the editor selection through changes and use a CodeMirror scroll effect for the primary post-edit source range. Use a separate transaction effect and decoration field for the changed ranges. Keep focus with the roll. Explicit source navigation continues through `revealPianoRoll`, where choosing text and focusing the editor are intentional.

Return the committed source and mapped feedback ranges. Map the target across roll-authored source transitions to keep the roll and its selected page present. Show the destination note immediately, using a predicted note after a generated edit and reconciling against the next matching host response. Retain edit feedback while inspection is pending. Anchor the roll pulse to destination pitch and exact start/end, including all affected notes when available. Use a short outline, with a static treatment under reduced motion. Expire it and the source decoration on a timer or clear them on scope invalidation and collapse. Keep edit feedback distinct from audition and playback.

## 4. Verify at the integration seams

Add dependency-light tests for stable target and inspection keys, readiness and gesture cancellation, and destination feedback through host note ID changes. Extend project-session coverage for collapsed and multiple selections, stale edit rejection, post-edit selection mapping, scroll and decoration effects, and undo/redo isolation. Run a desktop smoke with direct mini notation, an authored row, a shared use, generated Expression, and In song. Check immediate source write, original roll focus, scroll to an off-screen change, reduced motion, one-step undo, and keyboard commit. Exercise caret motion and note dragging with a delayed inspection response and while switching songs or closing the dock.

## Integration points

[single-range snapshots](../../src/lib/project/session.ts) and
[`resolveRollTarget`](../../src/lib/piano-roll/source.ts) continue to use half-open UTF-16 spans.
A collapsed span invokes caret inference. A grid target returns the producer's expression span,
the zero-based focused row when one exists, and its finite number of bars. Pass that producer
span to the direct planner; the authored grid's actual code patch may be elsewhere in the file.

For fallback, use `planEventEdit` and `preparePhraseEvents` from the
[event edit contract](event-edits.md). A new expression wrapper can be built by two insertions
around the receiver. Reveal the suffix containing the entry. An existing entry or final-song
directive normally uses one patch. The session maps these original positions to the final buffer.
