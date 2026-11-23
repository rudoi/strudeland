# Implementation notes

[Proposal](proposal.md) · [Execution order](000-prompt.md)

## Delivered

- Section and meter metadata share validation between the editor, live runtime, browser planner, and headless host. Written bar numbers and local pattern cycles carry through both live and offline layouts.
- Live playback schedules the reference clock using the current arrangement and keeps authored coordinates for note seeds, event inspection, and visual queries. The timeline auditions contiguous ranges and shows changing beat grids. Clock handoffs preserve the playhead and the CPS of audio already scheduled.
- Exports measure marker edges and kept chunk spans from the same frame grid. The existing first-pass and isolated repeat effect behavior applies to the full authored range. Standard clip output and stems use the same timing.
- The original synthetic F/F/E fixture represents 35 quarter notes, or 17.5 seconds and 771,750 decoded WAV frames at 120 BPM and 44.1 kHz.

## Local quality passes

Reviewed the task diff from `cf25a0b`. The simplify pass consolidated arrangement validation, local CPS conversion, per-segment clock calculations, and Rust phase fallback. Two independent high-thoroughness reviews covered the arrangement, live transport, editor and memo UI, browser/host planner, and Rust engine and export pipeline.

Regression cases cover hot-edit clock continuity, tempo conflict rollback, local CPS queries, painter registration, stale timeline section order, half-frame rounding, onsets in the opening sample, duplicate meter markers, and shared-value gate lengths. Updated the render gates to consume mapped bar coordinates. Added the timing modules to reference provenance and regenerated engine references and the host bundle through the repository commands. Comparison with the base found only provenance changes in the 181 existing engine job JSON files.

## Validation

- Chromium timeline and export dialog smoke test with local desktop IPC mocks. Keyboard selection, full-range highlighting, meter readout, marked bar lines, and export destination preview passed.
- Browser and headless plan comparison covered the mixed-meter clip and all three loop presets. Existing batch corpus parity covered 34 fixtures.
- Native integration verified decoded markers for all loop presets and aligned stems. A 137 BPM run exercised a fractional-sample loop edge.
- Bun suite passed 3,341 tests with one skip. Frontend and render-page builds, Svelte and TypeScript checks, and Prettier lint passed.
- The CLI emitted an 859,950-frame opening and a 771,750-frame repeat. Its WAV repeat marker ran from frame zero through frame 771,749 inclusive.
- Six legacy Rust batch fixtures passed their calibrated audio gates. All three seam, stem, and bass mutations were caught.
- Compared 150,000 bar positions with a direct meter walk across linear, live, repeated clip, temporary loop, and jump layouts.
- The locked Rust workspace suite passed 2,164 tests and its doc tests. Clippy with warnings denied and Rust formatting passed.
