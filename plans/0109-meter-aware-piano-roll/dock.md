# Piano roll dock and note auditioning

[Approved proposal](proposal.md) · [Refinement audit](refinement-audit.md)

Related contracts: [Source resolution and editing](source.md), [Pattern inspection and arrangement](inspection.md).

## UI and accessibility

Put a labeled **Piano Roll** disclosure in [Toolbar.svelte](../../src/components/Toolbar.svelte),
near the editor controls, with `aria-expanded` and `aria-controls`. Use a dock below the editor
and above the arrangement timeline in [App.svelte](../../src/App.svelte). Use the existing
[app preferences](../../src/lib/prefs/store.ts) to remember dock visibility across launches,
defaulting to collapsed when the preference is absent. Keep the user's choice while switching
songs and keep the toggle reachable at minimum supported window width. Collapse returns focus
to the toggle when the panel had focus.
Use existing Svelte primitives/styles. Provide clickable source links, keyboard selection and
edit commands, zoom/scroll as needed, visible focus, and announced editable/read-only, loading,
empty and stale states. Keep the selection gesture responsive on the editor side.

## Note auditioning

Audition a note or piano key on deliberate click or keyboard command. **Audition notes**
also previews a newly added note or a committed pitch change. Timing moves, resizing, source
selection and dock changes are silent. Distinguish a click from the start of a drag. Audition
is unavailable during song or memo playback, recording, or a transition into playback.

Reuse the bounded [note audition player](../../src/lib/audio/note-audition.ts) and the
[app audio lifecycle](../../src/lib/app.ts). Call `runtime.prepareAudition()` from the roll
gesture even when the song uses the Rust engine. The existing
[window gesture handler](../../src/App.svelte) routes non-editor gestures to the song backend,
so give roll auditions an explicit route to the preview graph. Share preview ownership with
typed-note auditions and preserve the typed queue's debounce behavior.

Use the pattern's verified built-in synth when available. For a sample, custom instrument or
ambiguous voice, use a labeled stock triangle preview. The current player accepts integer MIDI
pitches from 0 through 127 and checks whether the current registered trigger still matches
its captured built-in. Verify that the fallback can use the stock triangle when a song has
replaced that registry entry. Frequency and fractional pitches remain visible in inspection.
Cancel or invalidate pending and audible previews on collapse, song change, playback request,
memo playback or recording. Recheck eligibility after async preparation and guard highlight
cleanup against superseding previews. **Preview expression** remains silent.

## Interface and verification

The dock consumes an active editor snapshot, selection and transaction callbacks through the
app/session. A single controller owns inspection epochs and audition eligibility. The visual
surface receives the projected notes and emits user intents. The app owns the preview graph
and preference writes. Explicit audition stays usable with automatic previews disabled.

Exercise collapsed startup and persistence, min-size layout, keyboard operation, the
Expression to In song transition, explicit draft previews, shared edit navigation, voice
fallback and stale result suppression. Confirm by desktop smoke test when the environment
can run the app.
