# Audition typed notes while stopped

## Goal

With **Audition notes** enabled, type `g3` in a Strudel note pattern and hear a short G3 on that pattern’s instrument while the song is stopped. The editor and transport stay where they are. This works on unsaved text, including a song that has never been played.

## Scope

Place a labeled checkbox near Play and Stop in [Toolbar.svelte](../../src/components/Toolbar.svelte). Keep it unchecked by default and remember the preference across desktop launches. “Selected voice” means the pattern being edited, including separate voices in a stack or song section. The checkbox remains available during playback.

Start with explicit-octave pitch names in mini-notation, including accidentals and chords. Support `note("g3").s("supersaw")` and `s("sine*4").note("g3")`, including the `sound` alias. These cover the forms in the [project templates](../../crates/strudeland-project/src/template.rs). An unambiguous note pattern that omits the sound uses the existing triangle default.

The first pass previews fixed built-in pitched synths and a small set of literal voice settings, such as envelope and gain. Sample instruments, custom sounds, computed or alternating instruments, MIDI numbers, bare pitches, scale degrees, pitch transforms and full effects chains are outside the first pass. A sound the editor cannot resolve stays silent.

## Architecture

- [Editor setup](../../src/lib/editor/setup.ts) identifies the user edit and extracts the pitch and instrument from the current CodeMirror document.
- [App composition](../../src/lib/app.ts) owns pending audition work and checks the checkbox, active editor, transport, memo playback and recording state. [Runtime](../../src/lib/strudel/runtime.ts) supplies the transport lifecycle, including the interval between requesting playback and the scheduler starting.
- An audition player under [audio](../../src/lib/audio/) plays one bounded synth voice on the existing Web Audio graph. The [Superdough backend](../../src/lib/audio/superdough.ts) exposes that graph and owns context initialization. Preview uses Web Audio for both song engine settings.
- The [preferences store](../../src/lib/prefs/store.ts) and [native preferences](../../crates/strudeland-project/src/prefs.rs) carry the checkbox value. Older preference files load with audition disabled.

## Technical design

Use the CodeMirror JavaScript syntax tree to find a mini string in the edited pattern’s note argument, then walk that call’s receiver chain to resolve local sound literals. Use the vendored [mini AST](../../vendor/strudel/packages/mini/mini.mjs) for pitched atom spans and the [core note parser](../../vendor/strudel/packages/core/util.mjs) for pitch conversion, requiring an explicit octave and a MIDI value in range. Walk the main mini pattern separately from modifier arguments so typing an index or euclidean pulse count does not select a pitch. Accept fixed timing wrappers around a single sound, such as `sine*4`. Use reachable call structure to avoid borrowing a sound from another voice. Skip ambiguous chains, escaped or interpolated strings, comments and regions that disable mini syntax.

Use CodeMirror’s typing transaction annotation and changed ranges. After a short settling interval, preview a valid edited atom even if the editor already inserted the closing quote. A typed delimiter can confirm the pending atom immediately. Continuing the same atom replaces the pending candidate so `g-1` or a longer token does not trigger an intermediate preview on ordinary typing cadence. A delimiter after an already played note does not replay it. Paste, undo, deletion, cursor movement, reload, external edits and evaluation commands stay silent. A fresh typed edit elsewhere can schedule a new candidate. The syntax check should tolerate incomplete JavaScript outside the local pattern.

Reuse the built-in [Superdough synth triggers](../../vendor/strudel/packages/superdough/synth.mjs) through a preview gain node on the master graph. They provide a source handle and stop callback. Use existing audio bootstrap for registration, on a user gesture before delayed work needs the context. Pass only the resolved pitch, instrument and supported literal controls. Keep the gate, release and gain bounded and fade or disconnect the previous voice when replaced. Treat voice startup and cleanup as owned by the preview so disabling the checkbox, starting song or memo playback, recording, switching the song or project, or closing the editor invalidates pending work and cuts audible preview. Recheck after asynchronous resume or sound creation. The preview does not save, evaluate, install a pattern, or start a playback session.

## Dependencies and validation

Use the existing CodeMirror language extension, vendored Strudel parsers and audio backend. A local Bun probe confirmed that the editor syntax tree distinguishes the two template forms and comments and retains a string node on an unfinished call. The mini parser recognized nested pitches and modifiers separately, and Strudel converted `g3` to MIDI 55. The audio route still needs a desktop smoke test.

Cover typed completion and replacement, chord atoms, invalid notes, modifier arguments, distinct voice chains, unsupported instruments, non-typing transactions and incomplete editor text. Use a fake clock and audio seam for cancellation and stale callbacks. Check checkbox persistence and defaults from existing preference files. In the desktop app, verify first-use sound with playback stopped, a mid-pattern edit with multiple instruments, no preview during playback or recording, and preview cutoff when Play is pressed. Exercise both engine settings.

## Risks and open questions

The first pass favors predictable local voice resolution. A reusable instrument function or a sample-based line stays silent. Confirm whether sample instruments or project instrument helpers are needed for the first release, since they change how the preview voice is resolved and loaded.

The proposed typing pause makes `g3` audible without requiring a space or cursor movement. Refine the timing by ear with rapid entry and Vim insert mode. Confirm whether the toolbar placement and default-off preference fit the intended workflow.

Recommended workflow complexity is medium. Skip Fracture, verify the editor and audio contracts during Singletonize, and do a light Loss Check before sequencing.

## Refinement decisions

The first release uses the pattern at the edit site, with one fixed synth and explicit octave. Use approximately 120 ms to settle a typed pitch and a roughly 160 ms preview gate, with a short bounded release. Accept a delimiter as confirmation only when a candidate is pending. Reuse the existing shared bootstrap and master graph, including when the song is configured for Rust. A pending preview belongs to the active editor buffer and is invalidated on a buffer change, transport start request, or recording/memo activity. Skill-driven refinement confirmed these defaults for implementation.
