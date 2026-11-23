# 1. Structure and clock

## Goal

Normalize authored section ranges and map logical bars to reference-clock time.

## Prerequisites

[Arrangement contract](arrangement.md).

## Implementation

Build shared metadata validation, normalize legacy and explicit loop ranges, extend transport, and compute the bounded piecewise meter clock, including the inverse and next timing boundary. Keep compiler and serialized outputs compatible for constant-meter metadata.

## Validation

Run metadata, song, transport, and timing tests. Check range boundaries, authored and temporary looping, and mapped duration.

## Commit message

`feat(transport): add section ranges and a meter map`
