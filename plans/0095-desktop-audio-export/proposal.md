# Export audio from the desktop app

## Goal

Let a musician export the active song from Strudeland's File menu. The dialog should explain what
each export mode produces, preview the exact filenames, and report progress and the final result.

## Scope

Support WAV, MP3, Ogg Vorbis, and Opus with the existing encoder and loudness defaults. Offer the
project's `renders/` folder by default and a native picker for another folder. Show the output
roles, any planner or renderer warnings, and the Godot loop offset in seconds when the rendered
file has loop points. Keep advanced encoder and loudness controls, a render queue, cancellation,
and changes to the CLI or the sound of existing presets outside this task.

## Export modes

| Mode | Range | Suggested description |
| --- | --- | --- |
| Standard render | Clip, named section, or length in bars | Export the selected range as a regular audio file. |
| Intro + loop | Marked loop and any preceding bars | Play the opening file once, then repeat the loop file from its start. The opening includes the first loop pass. |
| Tagged loop | Marked loop and any preceding bars | Export one file with the opening and a marked repeat. Start at the beginning and loop the marked range. |
| Folded loop | Marked loop | Export one file with a clean first pass and a marked repeat that carries its own release, delay, and reverb across the loop boundary. |
| One-shot | Clip, named section, or length in bars | Export the selected range and let its notes and effects decay. |

Task 95's brief describes a single split-preset file when the song starts on its loop and describes
folded export as a repeat-only file. The [merged first-pass design](../0097-loop-first-pass/proposal.md)
and [renderer](../../crates/strudeland-render/src/export.rs) define the current behavior. The split
preset always produces an opening and a repeat, even when the opening contains only the first loop
pass. It retains the `-intro` and `-loop` suffixes. Folded export begins with a clean pass and
marks the following repeat. The UI should describe and verify those layouts. Changing the layouts
would change the shared export contract for CLI users as well.

## Architecture

- Extend the native File menu in [project_menu.rs](../../src-tauri/src/project_menu.rs) and route its
  export action in [App.svelte](../../src/App.svelte). Enable opening the dialog for an active
  project song. Reuse the native modal lifecycle and shared controls from
  [NewProject.svelte](../../src/components/NewProject.svelte).
- Add a desktop export API and a testable dialog controller. Capture the active song's path and
  current editor text using the [project session](../../src/lib/project/session.ts). Export that
  snapshot, including unsaved edits, without saving or re-evaluating the live editor. Keep the song
  named in the dialog and reject a stale project when starting the export.
- Add Tauri commands for preview and execution. Resolve the project through
  `ProjectState::current_for` and reuse the project library for song path validation and writes.
  Run the [shared render pipeline](../../crates/strudeland-render/src/lib.rs) or `render_export`
  off the UI thread. Use a per-request Tauri channel for progress like
  [playback.rs](../../src-tauri/src/playback.rs). Return paths, file roles, loop points and warnings.
  Keep encoded audio in Rust.

## Technical design

- Model the range using the renderer's `LengthRequest`. Standard and one-shot default to the
  clip. Offer named sections when available and validate a typed bar count. Loop presets select
  their own range and require a section marked `loop: true` in `meta.sections`. Use the captured
  source's static metadata for form hints and section choices. The
  [headless planner](../../src/lib/render/job-plan.ts) remains authoritative for evaluated metadata
  and validation. Surface its errors in the dialog.
- Resolve preview and execution targets through the same Rust path. Reuse the
  [renderer output helpers](../../crates/strudeland-render/src/output.rs) and project filename
  rules for all formats and ranges, including preset suffixes. Keep destination selection to a
  folder and use those same filenames there. Preview has no write side effects. Suppress stale
  preview responses after a field changes and require a current preview before submission.
- Identify collisions in preview and ask before replacing the displayed files. Bind approval to
  those paths and guard against a new collision at write time. Preserve project containment
  checks for the default folder. Treat a picked folder as an explicit external destination.
  Render and encode both split files before writing either. Writes are atomic per file. If a
  later write fails, report the paths already written so the user can retry knowingly.
- Allow one export job at a time. Show planning immediately, count completed render chunks,
  report encoding and writing, and display written files on success. Keep the dialog available
  during the run and let the user correct the selection and retry after failure. Show the MP3
  padding warning before starting a loop export. Report render warnings once per run and derive
  the Godot offset from the encoded file's `LoopPoints::godot_loop_offset`.

## Dependencies and risks

- Task 15 supplied the shared presets. Task 97 updated the first-pass layout and is already on
  the base branch. Use the existing encoder defaults and planning timeout. This proposal needs
  no host bundle or engine reference changes if the planner and DSP stay unchanged.
- Adding the renderer to the app links V8 and the native encoders into the desktop binary.
  Verify the app build and its CMake prerequisites on supported platforms. On macOS use the
  renderer's existing `host-jitless` feature for hardened runtime, as the bundled CLI does.
  Verify an export from a hardened app bundle to exercise that path.
- The renderer can reject sounds available in live playback. Display its structured error or
  actionable message. Keep progress indeterminate while total work is unknown, including while
  planning. Filenames and warnings must remain readable with long paths and keyboard navigation.

## Validation

- Cover dialog defaults, all five mode and range mappings, missing loop metadata, the MP3
  warning, progress and error recovery, and stale preview responses with frontend tests.
- Cover path and role parity with the renderer, no side effects during preview, project changes,
  collisions, write failures and partial split results with Rust tests. Exercise the real render
  pipeline through the app export command with a small synthetic song.
- In the desktop app, open File → Export Audio, select Intro + loop and WAV, verify the preview,
  export, and check the opening and repeat WAVs for the expected frame counts and loop metadata.
  Include an intro-less song to verify the two-file contract. Capture the dialog and result for
  visual inspection and record the app build used for the smoke test.

## Open questions

- Confirm that the dialog follows the current shared first-pass contract where the task brief
  differs.
- Confirm the current editor snapshot is the intended source. The proposal uses it so an export
  reflects edits visible in the editor without requiring a save.

Recommended workflow complexity is medium. Keep one proposal, skip fracture, and retain
singletonization and a light loss check for the project, renderer, and desktop UI interfaces.
