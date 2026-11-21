# Task #106 • Whole-song formatting

Branch `llm/andrew/strudeland/0106-format-whole-songs`. Follow the
[proposal](proposal.md) and [binding design](design.md).

1. [Implement and test the formatter](010-formatter.md). The pure test file can be developed
   independently of the formatter module if both follow the design.
2. Rebase on fetched main and run the frontend build, test, typecheck, lint, and desktop smoke.
3. Run simplify and high code review, update docs, and publish for review.

Auto-advance stops on entry to Review Gate. Finalization retains this file, the proposal,
design, and loss check. Remove the temporary ordered stage file during Finalization.

## Delivery evidence

- Rebased against `9678d4c`. The frontend build, typecheck, and lint passed. The frontend
  suite passed 3,296 tests with the existing opt-in host bundle check skipped. All 11 CLI
  skill tests passed.
- All 168 checked-in render, engine micro, and scheduler fixtures returned zero edits on a
  second formatting pass. Local Bun measurements put the 95th percentile below 0.5 ms on
  the original fixtures and below 0.2 ms on formatted fixtures.
- The isolated macOS dev app saved and played a disposable synth song with grouped named parts,
  nested chains, and a 32-step gain pattern. The editor matched disk byte for byte. The app
  reported the expected synth haps and source locations. The native capture reproduced the
  gutter misalignment tracked in Keitora #39. Accessibility automation could not bind the
  isolated window. Automated editor tests exercise Vim `:w`, undo, and dirty Save and Play.
- Simplify removed work on empty passes. Independent high code review verified the final code,
  including the fixes for combiner anchors, compact section entries, and parenthesized containers.
  The README, contributor agent guide, and embedded CLI skill describe the expanded behavior.
