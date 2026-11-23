# 2. Live playback and inspection

## Goal

Play, audition, and inspect mixed-meter ranges through both backends.

## Prerequisites

[Arrangement](arrangement.md) and [live timing interfaces](live.md). The APIs may be developed in parallel to stage 1.

## Implementation

Install the scheduler clock adapter without shifting the logical pattern offered to tools. Route logically seeded wall-time events with local CPS and optional phase seconds. Update timeline and transport controls, events, status, and recording guards. Record the Superdough patch.

## Validation

Exercise scheduler and audio backend tests, clock jumps and tempo-sensitive haps, range UI, and controls.

## Commit message

`feat(playback): schedule and inspect mixed-meter loops`
