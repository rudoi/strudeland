# Wire desktop and offline entry points

## Goal

Connect library management, desktop playback and offline export to project pins.

## Prerequisites

The [storage](storage.md) and [evaluation](evaluation.md) interfaces.

## Implementation

Wire project commands and watcher, CLI library and pin commands, and pinned Rust render
inputs. Update the CLI skill and public documentation. Regenerate the host bundle.

## Validation

Verify watcher and session changes, CLI persistence, headless playback and output using a
pinned fixture. Check browser parity, normal app entry points, and full workspace gates.

## Commit message

`feat(instruments): use project pins in playback and export`
