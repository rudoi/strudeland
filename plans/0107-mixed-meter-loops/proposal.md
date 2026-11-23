# Section-range loops and mixed meter

Implemented and approved for squash merge in [PR #18](https://github.com/rudoi/strudeland/pull/18). See the [validation record](implementation.md).

## Goal

Loop a contiguous run of named sections, or loop a single section whose bars have different time signatures. Live playback and audio export should repeat the same musical span at the song's quarter-note BPM. The motivating arrangement has two F passes and four E bars. It should loop with the written rhythms and no added pickup beat.

## Scope

- Author one default loop range from the start of one named section through the end of another. A single section is the one-section case. Existing intro, repeat count, and ending behavior applies to the entire range.
- Audition a contiguous range of sections from the timeline, with the existing single-section command as a shortcut. Audition choices are temporary. The saved song defines the range for export.
- Let the time signature change at any authored bar line, including inside a section. Keep a single quarter-note BPM. Update playback, the playhead, the beat ruler, event inspection, regular renders, stems, and existing loop export presets.
- The scope is one contiguous authored loop region. Independently repeating several regions, reordering sections, reusable arrangement instructions, tempo automation, and editing meter changes graphically are outside this proposal. A short measure can represent a pickup or irregular phrase ending when it fits the supported time signatures.

## Architecture

[Song structure](../../src/lib/strudel/structure.ts) currently validates integer section lengths, allows one section with `loop: true`, and calculates duration from one CPS. [Transport](../../src/lib/strudel/transport.ts) and [song pattern queries](../../src/lib/strudel/song.ts) use whole cycles for bar boundaries and keep each section's pattern in its own local time. Preserve that musical coordinate so existing patterns and bar-based tools retain their shape. Resolve the loop to a single range and share a layout-aware mapping between musical bar cycles and a continuous clock. The mapping follows the actual playback order, including count-in, repeats and release.

Keep the authored arrangement, current live transport, and render layout distinct. The linear layout visits every section once, the clip layout repeats the authored range the requested number of times, and the live transport adds temporary starts, audition loops and releases. A single timing module should derive the duration of each of those layouts from the same validated meter map. Represent the map as runs of bars with integer sixteenth-note totals and calculate repeated ranges by their period. This protects seek and render costs on long loops and avoids adding rounded durations each pass.

## Technical design

### Song metadata and validation

Proposed literals, using one-based bar numbers relative to the written song:

```js
const meta = {
  bpm: 120,
  meter: '4/4',
  sections: [
    { name: 'opening', bars: 1 },
    { name: 'F1', bars: 3 },
    { name: 'F2', bars: 3 },
    { name: 'E', bars: 4 },
    { name: 'ending', bars: 1 },
  ],
  loop: { from: 'F1', through: 'E' },
  meterChanges: [
    { bar: 4, meter: '3/8' },
    { bar: 5, meter: '4/4' },
    { bar: 7, meter: '3/8' },
    { bar: 8, meter: '4/4' },
  ],
  loops: 2,
}
```

This synthetic arrangement makes each F pass two 4/4 bars plus one 3/8 bar. The loop is ten authored bars totaling 35 quarter notes, or 17.5 seconds at 120 BPM. That is the duration of 8¾ bars in 4/4. The example illustrates the reported duration. The actual track's meter boundaries need to come from its score.

- `meta.meter` sets bar 1 and applies until the first change. Each later change persists until the next entry. Require strictly increasing, in-range bar numbers and supported meters. Meter changes require `meta.sections`; section lengths remain whole bar counts.
- Resolve `from` and `through` by exact section name, with both names inclusive. Reject unknown names, reversed ranges, and a new range declared alongside an existing `loop: true`. Continue accepting the legacy form and resolve it through the same range logic. Keep `meta.loops` and `--loops` as counts of full passes.
- Share validation between the static [header reader](../../src/lib/project/meta.ts), evaluated `song()`, and the render planner. Report errors at the relevant metadata value, including single-quote requirements. Invalid edits must leave the last evaluated playback intact. Test conflicting declarations, boundary bars, stale edits, and unchanged legacy inputs.

### Live playback and inspection

- Convert between musical cycles and scheduler clock cycles with a piecewise mapping. Use the original meter as the scheduler's reference unit and keep the identity path for constant-meter songs. At 120 BPM, 4/4 takes 2 seconds and 3/8 takes 0.75 seconds. A two-bar hap held across those measures takes 2.75 seconds and has one onset. Query final composed patterns in musical coordinates so outer transformations and independently stacked voices follow the same bar grid. Convert both ends of each hap for playback. Preserve the source location, logical onset for seeds, and query-split behavior of held and clipped haps.
- Make local CPS available to controls that synchronize to the current measure, including pattern operators such as `loopAt` and playback effects such as `delaysync`. Separate that value from elapsed seconds and from the logical onset shared with deterministic audio. Define phase against the continuous clock for cycle-synced effects that need elapsed time. Verify equivalent timestamps and gate lengths through Superdough and Rust. Keep legacy `setcps` behavior on constant-meter songs. For mixed-meter songs, propose a clear diagnostic for conflicting code-driven CPS changes until their interaction with the map has a defined contract.
- Apply start-from, loop selection, release, backend handoff, and auto-stop through [runtime](../../src/lib/strudel/runtime.ts) and [transport](../../src/lib/strudel/transport.ts). Quantize edits to musical bar boundaries beyond scheduler lookahead. Release from any member at the end of the current eligible full pass. Starting beyond the authored loop should play through the ending. Retain a clock anchor across transport history pruning, stop/restart, and re-evaluation so elapsed time stays monotonic and scheduled haps retain their timestamps.
- In [Timeline](../../src/components/Timeline.svelte), use an accessible section-menu action to pick a loop end from the eligible sections. Highlight the full active range and show meter changes at their bar lines. Read the active signature for the current beat and for [event inspection](../../src/lib/control/events.ts). Keep bar numbers, section ownership, and legacy single-section status compatible across the control protocol and CLI.
- [Backed recording and memo alignment](../../src/lib/recording/backing.ts) currently assume one grid. Support playback independently of recording. Guard any backed-take, memo alignment, or bar lookup that would otherwise claim a fixed grid for mixed meter. Persisting a variable-grid memo format would require a separate contract. Unbacked recording and existing fixed-grid memos retain their behavior.

### Render planning and loop exports

- Feed [job planning](../../src/lib/render/job-plan.ts), [chunk assembly](../../src/lib/render/chunks.ts), [event timing](../../src/lib/render/schedule.ts), and the browser offline renderer from layout-aware times. Reuse the same schedule and musical onset identity for the opening and isolated body. Include pre-roll, release tails, synced effects, and chunks spanning a meter change. Use explicit elapsed times and effective per-hap CPS where needed by the [engine job](../../src/lib/render/engine-job.ts). Keep existing constant-meter job semantics and accept existing job data.
- Supply frame boundaries in the shared [host plan](../../crates/strudeland-host/src/planned.rs). Round each absolute edge once, then subtract edges for joins and range lengths. For the isolated repeat, reuse the opening body's authored frame origin, including its fractional-sample position. The opening pass and repeat then have identical lengths even if the intro ends between samples. Use the resolved frame counts in [join](../../crates/strudeland-render/src/join.rs) and [export](../../crates/strudeland-render/src/export.rs) instead of deriving them from a global CPS. Standard renders and stems take the same frame path.
- Preserve the [clean first-pass contract](../0097-loop-first-pass/proposal.md). The opening carries only the state of music already heard. The repeat uses isolated loop carryover. `intro-loop`, `tagged-loop`, and `folded-loop` retain their existing packaging and place decoded-frame markers around the entire repeat. Legacy loop count and range selection rules for the presets remain in force. The existing stems workflow produces aligned linear or clip stems.

## Dependencies

The static and evaluated paths, browser renderer, headless host, and Rust renderer need a shared timing contract. Rebuild the committed host bundle and its input hashes. Planner changes require refreshed engine reference provenance and the repository's parity checks. Update playback and export documentation, CLI skill and any protocol mirrors with their paired tests. Aim to keep the engine's DSP and encoders intact by supplying the timing they already consume as seconds, frames, and CPS. Investigate whether a final-pattern adapter covers query consumers before considering a change to the vendored scheduler.

## Risks and validation

- Test a held event crossing a signature change, a pattern composed outside `song()`, a section that starts with an inherited signature, an exact boundary onset, and a release armed from each member of a multi-section range. Verify no skipped or double-triggered events, phase discontinuity, or clock drift after many passes. Exercise backend switching and dense playback using the existing runtime tests and timing harnesses.
- Use the synthetic 35-quarter-note fixture to prove both a loop inside one mixed-meter section and a loop range spanning several sections. At 120 BPM its repeat must be exactly 771,750 frames at 44.1 kHz. Use a second tempo with fractional-sample boundaries to exercise the intro-to-repeat and repeat-to-repeat joins. Compare browser and headless planning, chunked and whole-window output lengths, aligned stems, and decoded loop markers. Run existing parity and playback tests to check legacy behavior.
- The meter map changes how linear and repeated requests translate to seconds, including beyond the authored end where a renderer may still request decay or overlap. Choose and test the boundary rule in the shared timing module. Verify that recording and inspection surfaces cannot silently show a wrong bar grid.
- Smoke test the timeline and preview/export a mixed-meter range from the desktop app. Confirm the loop reaches the next downbeat exactly after the final E bar.

## Open questions

- Is the proposed `meta.loop` and `meta.meterChanges` syntax suitable for editing songs directly? The timeline proposal adds temporary range selection and a display for meter changes.
- Which signatures and bar lines does the source arrangement use for the F passes and E bars? The reported duration establishes the test total. A track excerpt would let refinement verify the real arrangement against the proposed form.

Workflow complexity is large. The shared timing contract affects live scheduling, inspection and rendering, so retain fracture, singletonize, and loss check before sequencing.

## Auto-refinement decisions

The owner authorized an automated refinement pass and continuation through the pull request review gate. Code-reference reviews covered the scheduler, both live outputs, the planner, the host wire format, and the engine. The proposed metadata syntax is adopted for implementation. The source arrangement’s exact meter remains unverified, so acceptance uses the synthetic arrangement and its exact musical duration.

Use `song()` for new metadata that describes a meter map or a named range. This gives playback and export a defined arrangement to map. A legacy song without these fields retains its existing behavior. Mixed meter uses the declared quarter-note BPM and rejects conflicting CPS changes with a diagnostic.

The reference-clock mapping may extrapolate before bar 1 at the base meter and after the final authored bar at the last meter. Negative elapsed time remains outside the playable region. The live scheduler installs a timing adapter around the final pattern, keeping the authored pattern available to inspection and visuals. Rust timing data carries elapsed phase independently of the authored cycle used for seeds. Render plans carry explicit content and marker frame counts. See the subsystem contracts for compatibility and test cases.
