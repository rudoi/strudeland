# Editor, preferences and audio components

Goal. Extract a typed pitch and its fixed source instrument, persist the checkbox, and play a bounded stoppable preview.

Prerequisite. Read the proposal and verification record. Editor parsing, preference persistence and audio rendering are independent.

Implement the CodeMirror transaction detector, the app preference and toolbar checkbox, and an isolated audition player over the existing Superdough graph. Test parsing and change attribution, startup and cancellation races, and preference load/write ordering. Test both song template forms.

Commit as `feat(editor): detect typed note audition candidates`, `feat(prefs): persist the note audition checkbox`, and `feat(audio): add a bounded synth preview`.
