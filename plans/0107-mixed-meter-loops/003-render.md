# 3. Render and export

## Goal

Render an entire mixed-meter range with exact repeat markers and aligned stems.

## Prerequisites

[Arrangement](arrangement.md) and [render interfaces](render.md). The planner and Rust work can proceed in parallel under the wire contract.

## Implementation

Map timing in browser and host planning, and use explicit frames in the batch joiner and exports. Add optional per-hap engine timing. Verify the wire bridges and avoid changing constant-meter job shapes.

## Validation

Run planner, engine and join tests. Verify clean first pass and mapped lengths for clip, one-shot, stems and repeat exports.

## Commit message

`feat(render): export sample-aligned mixed-meter loop ranges`
