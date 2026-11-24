# Browser and editor

See the [proposal](proposal.md) and [native commands](storage-and-preview.md).

- Open a compact modal from an Instruments workspace button. Give the source
  switch and search consistent positions. Use the existing type and surface tokens.
  The left column lists sounds, the right shows the selected sound, controls and
  project actions. Stack columns on narrow windows. Search receives initial focus.
  Support Escape, visible focus, and focus return. Keep status messages near their actions.
- Preview uses defaults and distinguishes Global Library from Project Pins.
  Start audio preparation inside the button gesture. Stop when the browser closes,
  selection or project changes, or playback or recording begins. Retire late responses
  before a source can connect to the graph.
- Apply reads a complete selected expression and emits a single CodeMirror edit.
  The shared session takes an ID, and for remote calls a root, song, expected entire
  source, and offsets, checking each before editing the open buffer. Use zero-based
  UTF-16 offsets with an exclusive end. The visible tool shows what a user must select
  before the action is available.
- Validate expression bounds using Acorn as used by the existing editor formatter.
  Keep whitespace and surrounding text, and verify syntax before dispatch. Check
  that the name resolves through the song's pinned namespace. No write or automatic
  evaluation is part of this editor operation.
- Verify editor transaction, keyboard use, error recovery, and desktop layout with a
  capture. Use a fake graph for lifecycle tests.
