# Pattern inspection and arrangement

[Approved proposal](proposal.md) · [Refinement audit](refinement-audit.md)

Related contracts: [Source resolution and editing](source.md), [Piano roll dock and note auditioning](dock.md).

## Meter, window and pitch

Keep exact logical-cycle start/end fractions through projection and write-back. In Strudeland
one cycle is one written bar. Use [structure.ts](../../src/lib/strudel/structure.ts) and
[timing.ts](../../src/lib/strudel/timing.ts) for meter and mapping to the reference clock. Use
written bar numbers in the in-song view, and label pattern-local cycles in Expression. Select
the relevant written use as the meter anchor. A direct selection of a definition starts at its
first known occurrence in written song order. When arriving via **Edit shared definition**,
keep the occurrence the user came from. Show the meter anchor and offer a chooser for other
known occurrences. For a definition without a known occurrence, label the base meter as the
preview meter. Edits stay in bar-relative fractions even if other uses display them against a
different beat grid.

Use quarter-note equivalents as the horizontal unit of the ruler. An eighth note takes the
same horizontal space in 4/4 and 7/8 at a given zoom, so
the 7/8 bar is seven-eighths as wide. A bar in meter N/D has N denominator
beat marks spaced 1/N of a cycle apart. In the viewport, favor nearby musical subdivisions
including triplets and offer snap bypass. Treat compound grouping as visual emphasis (for example
two dotted beats in 6/8). Use denominator beats for coordinates. Preserve existing off-grid
events exactly. Perform inverse pixel-to-time conversion against the same bar measurement and
snap to an exact rational in that bar. The [timeline](../../src/lib/project/timeline.ts) has
separate tick conventions for its compact ruler. Share the underlying meter calculations.

Use an initial bounded window of up to four bars, with a shorter window when a finite authored
phrase or section extent is known. Allow window navigation for longer and variable patterns.
Keep pattern phase separate from the meter anchor. A later section can start at local cycle
zero while its grid takes the meter of the corresponding written bar. A repeated source atom remains linked
across displayed occurrences. In song, derive the location from the linear arrangement and
section-local mapping in [song.ts](../../src/lib/strudel/song.ts). Use
[queryMetered](../../src/lib/strudel/metered-pattern.ts) to retain the scheduler's per-meter
query controls. Map a live playhead through `runtime.position()` when the session's evaluated song
and the runtime's `activeCode` match the roll and the position is in the displayed use. Add a pin
revision match when claiming preview parity with the live evaluation. Loop wraps follow the
written position. Count-in and unmatched playback hide the playhead.

For pitched haps use Strudel's [pitch conversions](../../vendor/strudel/packages/core/util.mjs).
Treat finite positive frequency and fractional MIDI as continuous pitch and show their exact
values in inspection. A raw `n` can be a sample index or scale degree. Give it a piano key only
when a pitched interpretation is established. Bare mini within a recognized pitch producer
inherits that context. Keep overlapping voices distinguishable. Retain full logical span and
clipped visible span, distinguish continuations and preserve true duplicates. Sort for display
by onset and stable occurrence order because Strudel can return haps out of time order. Use the visible
pitches to fit a padded vertical range with a minimum height and scrolling at wide ranges.
Freeze the mapping during a drag, provide edge scroll for out-of-range destinations, and refit
once the edit lands. An empty editor target gives guidance. A rest-only phrase such as
`note("~")` has an insertable grid with a conventional starting octave.

## Isolated expression inspection

Keep selection changes outside the live REPL. [Runtime.query](../../src/lib/strudel/runtime.ts)
queries the last successful full evaluation, which misses unused definitions and can represent
a different document. [The current host query](../../src/host/main.ts) is an arrangement/render
diagnostic and drops exact spans, continuation fragments, custom-routed events and source
locations through [render selection](../../src/lib/render/plan.ts). Its
[Rust query](../../crates/strudeland-host/src/host.rs) runs without a deadline. Extend the
headless host with a distinct inspection request. Reuse its inert audio backend and fresh
isolate, and require a deadline via the [watchdog](../../crates/strudeland-host/src/engine.rs).
Supply the session's current project instrument snapshot and evaluation options. Render and
playback callers retain their existing query contracts.

For Expression, use source-aware capture around an eligible expression while evaluating the
file in its lexical scope once. This captures unused declarations that actually execute, and
keeps labels, `all()`, and `song()` semantics intact. Prototype an opt-in hook before choosing
whether it belongs in the existing AST walk or a separate transpilation step. Preserve original
source offsets through the hook. Capture the result of an expression without re-invoking it or
changing call receiver, execution order, short circuiting, direct eval or lvalue semantics.
An in-memory probe with the transpiler's `leave` hook captured an unused initializer and a
transformed reuse with original mini offsets while the final song still produced its unrelated
drum pattern. The mini plugin replaces bare literals earlier in the walk and drops their
original spans. Resolve a selected literal to an eligible enclosing expression or preserve its
origin through the mini rewrite. Evaluate a request-scoped hook and collision-free lexical
helper. Distinguish unsupported capture from a site that was not entered during evaluation.
Lazy callbacks can run only at query time. Multiple calls at one site may produce distinct
instance candidates. Use a whitelist of instrumentable roles and verify untouched song output
equivalence. The runtime applies the metadata seed to the final pattern, so direct capture
needs equivalent seed context. Verify with randomized pattern cases.

For In song, query the final pattern in the selected written window. Static proven paths and
a separately tested per-use marker can attribute haps to a particular occurrence when note
locations are shared. Probe the marker through `stack`, labels, constructors, value and time
transforms, and the section wrapper. Preserve hap statefulness and all existing metadata.
Context composition treats location arrays specially and can overwrite other keys, so verify
markers through merges and stripped context. A source link leads back to the specific source
expression or offers verified contributors when several source ranges explain the event. Any edit starts
from a supported target in Expression. Keep ambiguous events as context with a clear label. When
selection-specific attribution is unavailable, offer the full in-song view with any known
highlights and an attribution status. Do not infer that two identical notes came from the same
use. Compare output with the uninstrumented host for parity. Host slider movement, DOM APIs,
external triggers, live-only randomness or custom UI can prevent parity. Identify that in the
view. Preserve each raw hap's exact whole span and visible part when available. Merge query
fragments only when evidence establishes their identity. Start with whole-bar query windows
because the current meter splitting converts query boundaries to numbers. Serialize event span
endpoints as bounded rational strings. The in-song projection reflects the inspected buffer.

A request has a revision ID, target and use ID, window, pins fingerprint, and seed/controls
context. The response carries exact event arcs, normalized pitch data, source contributors,
instance/use identity where established, and typed failure or truncation status. Send only
bounded, sanitized observations across the desktop bridge. Debounce selection and typing.
Admit one active job and one replaceable pending request outside the host's unbounded FIFO.
On collapse or song/project change, drop pending work and ignore any reply to invalidated work.
An admitted job can run until its watchdog deadline. Bound query window width, execution time,
and event and response size during result construction. Measure pathological query memory
separately from response size. Put evaluation and result construction off the UI thread.
The static path works immediately for simple phrases.
Measure first-open and re-selection latency before enabling automatic headless previews for
trusted evaluated songs. When the current draft revision has yet to be evaluated, show simple
authored phrases immediately. If inspection requires executing the draft, offer **Preview
expression** and run the current revision in the headless host on activation.
Check platform coverage for the app's jitless host, and keep static editing available when a
headless capability fails.

## Interface and verification

The native request carries the current project identity, song source, query mode, optional
expression span, integer cycle window and optional written meter anchor. Resolve project
instruments once per request through the project library. Return bounded note observations
with pitches, exact logical edges as rational strings, source contributors and attributable
use evidence where available. Distinguish failure from an empty window.

The frontend projects source and host notes onto a common grid. Geometry uses the written
meter and the roll's separate cycle origin. Implement the host adapter with an explicit
deadline. Probe raw query behavior and compare ordinary song output with capture enabled.
Unknown occurrence provenance remains viewable in the In song projection. Cross-meter
dragging needs its own proof and does not block the established within-bar operations.
