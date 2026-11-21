# Task 103 delivery record

The [proposal](proposal.md), [implementation contract](design.md), and [loss check](loss-check.md)
describe Format on Save. The feature is delivered through [PR #16](https://github.com/rudoi/strudeland/pull/16).

- The formatter lays out supported 16- and 32-step 4/4 mini-notation. Localized edits preserve
  the saved snapshot, syntax and timing, editor selections, and undo.
- The preference is off by default and persists alongside Vim and note audition. The shared
  session save path writes the transaction's document. Save and Play evaluates that snapshot.
- Local verification passed the frozen Bun install, frontend build, typecheck, Prettier,
  3,279 Bun tests, Rust formatting, workspace Clippy, and 2,156 locked Rust workspace tests.
  The macOS smoke test verified two rows of 16 and identical timing and values for 16
  onsets, with playback continuing through a plain save.
- Independent review covered implicit semicolon boundaries, display widths, and saves
  waiting for a failed rename. Keitora task #104 tracks the vendored multiline leaf end offset.
