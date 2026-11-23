# Authored arrangement and layout timing

[Proposal](proposal.md)

## Metadata and resolved structure

`meta.meterChanges` is a list of sorted unique one-based bar changes after bar 1, at or before the final authored bar. Each entry has `bar` and single-quoted `meter`. `meta.loop` has single-quoted `from` and `through` names in authored order, both inclusive. Validate both through the static parser and `buildStructure`. Require sections for either feature and reject a range alongside `loop: true`. An empty meter change list has constant timing.

Keep `Section.start/bars`, `totalBars`, `clipBars`, and the layout cycle integer at bar lines. Keep `loopIndex` as the first section index for older consumers. Optionally serialize `SongStructure.loopRange` with `{ first, last, start, end }`. Indices `first/last` are inclusive and authored bars `start/end` are zero-based and half-open. The helper `authoredLoop(structure)` also resolves the legacy form. `SongStructure.meterChanges` optionally exposes zero-based `{ bar, meter }` entries. Omit new properties for old songs.

## Transport and time

A loop view keeps `section` and can add `through`. A manual loop and the authored loop each cover one contiguous pass; release targets the authored loop end and is available on its full passes. `withLoop` accepts an optional through index. The runtime implements `loopRange(first, last, structure)` while keeping `loopSection`. The clip inserts one release segment at the full pass count and plays the ending once.

In `src/lib/strudel/timing.ts`, `layoutClock(structure, segments, anchor?)` provides `toClock`, `toCycle`, `meterAt`, and `nextMeterBoundary`. An optional `{ cycle, clock }` anchors live history. One clock cycle has the duration of a bar at the reference meter. Divide clock cycles by the reference CPS for seconds. Prefix sums in sixteenth notes make every supported meter boundary exact before the final division. Compress repeats by quotient and remainder; avoid allocating per repeated bar. Invert monotonic mapping across fractional positions and handle boundaries without drifting into the preceding bar. A query may split at layout segment boundaries as well as actual changes of meter.

Validate directly with structure, metadata, transport, song and timing tests. Cover mixed runs within one section, authored and manual range wrap, release and starting in the ending, clip duration, large repeat counts and negative/beyond-end conversion.
