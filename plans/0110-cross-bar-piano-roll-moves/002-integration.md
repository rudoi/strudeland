# Dock integration and delivery checks

Goal. Use one move interval in the preview, editor commit, and resulting projection.

Prerequisites. Exact move planning and the placement proof are available.

Implementation. Wire pointer and keyboard, fix the history anchor where needed, and apply
the placement and probe-context checks before writer selection. Record the source and UI
evidence in the implementation log. Document the editing limit where the piano roll is
described.

Validation. Run the canonical frontend checks, synthetic native desktop smoke and
mixed-meter host/render playback checks as applicable. Check no write before mouseup,
forward and reverse edits, one undo/redo, focus and retained anchors, fallback save/reopen,
and gated moves preserving source. Then apply the workflow simplify and high-review
skills, validate any fixes, and stage the PR for human review.

Commit. `feat(editor): wire meter-aware moves into the piano roll`
