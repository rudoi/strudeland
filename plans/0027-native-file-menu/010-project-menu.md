# Project menu implementation

## Goal

Open or create projects from the native File menu and share the project actions with the welcome screen.

## Prerequisites

Confirmed proposal and design. Use the locked Tauri release and the existing project session.

## Implementation

Add the native items and event, a typed frontend listener, the shared project action controller, and a modal for the existing name step. Add tests for native command routing, duplicate actions, picker/form cancellation and invalid names.

## Validation

Run the focused project tests first. Then run the canonical frontend and Rust gates and native smoke described in the workflow.

## Commit

`feat(project): add native File menu project commands`
