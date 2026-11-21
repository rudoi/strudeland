# Voicing and sustained notation

## Goal

Suppress pitched output without credible voiced support and keep tied notes
sounding through the barline.

## Prerequisites

Read the proposal and contracts. Preserve baseline outputs for the local case.

## Implementation

Make independent changes in the segmentation and notation stages. Match CLI
warnings and documentation to audible behavior. Coordinate shared tests and
builds in the parent session.

## Validation

Cover unsupported and marginally supported pieces, real pickups, runs crossing
multiple bars, equal-pitch repeated attacks, and bounded spans. Preserve all
existing behavior for untied output.

## Commit message

`fix(transcribe): preserve supported attacks and sustained notes`
