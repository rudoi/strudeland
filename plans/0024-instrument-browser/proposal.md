# Instrument browser and tools

## Goal

Let someone find a reusable preset or kit, hear its sound, and apply it to a song that
can travel with its project. Give CLI and MCP clients the same instrument operations.
This is Keitora task #24.

## Scope

- Add a desktop browser for global library entries and project pins. Show parameter
  defaults, kit voices, and whether a pin differs from the current library entry. Let
  users audition either version and explicitly pin, update, and apply an instrument.
- Keep the existing CLI list, pin, and update commands. Add a command to create a
  validated library definition and one to apply a project pin to an open song. Expose
  list, create, apply, and update as instrument tools over local MCP, with pinning
  available so a client can complete the workflow.
- A general-purpose MCP catalog, a graphical instrument builder, library distribution,
  sample packaging, and new synthesis controls are outside this proposal.

## Architecture

Use the [instrument storage contract](../0023-instrument-library/storage.md) for IDs,
fingerprints, validation, and complete project copies. Extend
[`InstrumentLibrary`](../../crates/strudeland-project/src/instruments.rs) for reading a
selected definition and creating a global entry. Desktop actions go through
[`ProjectApi`](../../src/lib/project/api.ts) and the native
[`with_project`](../../src-tauri/src/project.rs) check for project data. The
[`ProjectSession`](../../src/lib/project/session.ts) remains responsible for refreshing
pins and passing a snapshot to the next evaluation.

Open the browser from the project workspace in [`App.svelte`](../../src/App.svelte).
Use a filterable list with Global and Project views and a detail panel for the selected
definition. Show the source version beside Preview and require a current fingerprint
when updating a pin. Keep a pin browsable and playable if its library entry is gone.
Show read, preview, and write errors where the action was initiated.

Keep the CLI and MCP handlers over the same Rust operations. For edits to the open song,
route both through a typed method in
[`strudeland-ctl`](../../crates/strudeland-ctl/src/methods.rs), mirrored in the
[webview handler](../../src/lib/control/handler.ts). The browser calls the same editor
operation from the session. Limit the first MCP server to instrument tools over
[stdio](https://ts.sdk.modelcontextprotocol.io/v2/get-started/first-client) and keep
protocol output on stdout.

## Technical design

### Browse and write

The browser identifies an entry by source, ID, and fingerprint. Use those identifiers
to read and preview the same version the user chose. Keep global and project read
errors independent, so an unavailable library still leaves valid project pins
accessible. Clear project-specific selections on project change and reject stale
responses when a project or selection changes.

Create takes an ID and a schema-1 definition and refuses an existing ID. Reuse the
store's validation, ID checks, scoped file access, and atomic publication. Use the
same checks for desktop, CLI, and MCP operations. Pin uses the existing first-copy
operation. Update refers to the current library entry and uses the existing expected
pin fingerprint. Make a stale row refreshable and keep the existing pinned definition
accessible after a failed update.

### Audition

Render a fixed short phrase with the selected definition and its defaults through
[`render_audio`](../../crates/strudeland-render/src/lib.rs) and the shared
[instrument adapter](../../src/lib/strudel/instruments.ts). Pass the definition as an
in-memory snapshot and use a fixed source and window. Play the bounded result on the
existing Web Audio graph at a controlled level. Apply a short fade if a tail is cut at
the preview boundary. If the Rust renderer cannot play the definition, display the
unsupported sound or control.

Start audio preparation from the Preview gesture and reuse the lifecycle conventions
in [`runtime.prepareAudition`](../../src/lib/strudel/runtime.ts) and
[`memo-player.ts`](../../src/lib/audio/memo-player.ts). Keep at most one preview
audible and discard superseded render results. Stop preview when the panel closes,
the selection or project changes, or song playback or recording starts. Keep audition
available while transport and recording are idle. The live transport, song buffer,
and pin files are untouched by Preview.

### Apply to a song

Require a pinned instrument and a complete pattern expression selected in the active
song. Wrap the expression as
`(<expression>).apply(instruments["<id>"]())` using an encoded ID. Check the selection
against the existing script parser so unrelated statements, comments, and mini-notation
tokens are outside the replacement. Preserve the selected source and its surrounding
text. Commit the edit to the buffer as one undoable transaction. Saving and evaluating
remain deliberate editor actions.

For CLI and MCP, propose a named song or the active song, a documented source range,
and an expected source snapshot. Reuse the conditional edit behavior of
[`code.set`](../../src/lib/control/handler.ts) and verify the project, song, range,
source, and pin at the edit boundary. Surface stale edits as failures the caller can
recover from by reading the song again. Keep pinning and applying as separate actions
so an edit failure has no project write to roll back. Report the edited song and range.

### Validation

- Cover existing IDs, malformed definitions, competing creation, missing library
  entries, stale pin fingerprints, and stale project or song responses.
- Verify a single preset and a layered kit produce audible previews with their
  declared controls and defaults. Cover unsupported controls, level and duration
  bounds, rapid selection changes, and interruption by song transport or recording.
- Exercise nested selections, Unicode and multiline coordinates, stale buffers and
  wrong projects, one-step undo, and application from the desktop, CLI, and MCP.
  Verify the saved song plays and renders with its pin after the library is unavailable.
- Run the canonical frontend and Rust gates, exercise the desktop browser and inspect
  a capture, and test the MCP initialization, discovery, and calls through a subprocess.
  Update the README and embedded CLI skill with the new command and error contracts.

## Dependencies

Tasks #23 (instrument library) and #5 (control socket and CLI) are complete. Task #6
tracks a general MCP server and is in the backlog. Task #24's description calls for
MCP tools even though the dependency on #6 was removed. This proposal includes the
minimum local server needed for instruments and leaves the broader catalog with #6.

## Risks

Kit voices and effect chains can make previews expensive or loud. Use bounded preview
rendering, a single active preview request, and controlled playback gain. Verify a late
response cannot restart audio. Some validated Strudel definitions use features the Rust
engine still reports as unsupported. An explicit error preserves the distinction between
a stored definition and what the chosen preview engine can play.

Source ranges cross a Rust/JavaScript boundary. Define one coordinate convention before
exposing CLI and MCP signatures, and check the expected buffer before interpreting the
range. Test astral characters, line endings, and concurrent editor changes. Keep MCP
transport and error handling out of the instrument semantics so the CLI behaves the
same way when the app is closed or the requested project changes.

## Refinement decisions

- Shelve the broader catalog in #6. Include the five instrument tools over stdio here.
- Preview defaults. Keep temporary parameter editing out of the browser so the same
  Preview button always plays the stored version indicated on screen.
- Apply wraps a complete pattern expression selected in the editor. Keep Pin to Project
  and Apply to Selection separate. Apply is enabled for project pins, and the browser
  explains how to select a pattern when Apply is unavailable.
- Use a compact, searchable browser with a Global Library and Project Pins switch.
  Give Preview and Stop consistent positions, put the selected preset name above its
  voice and parameter details, and place project actions together. Preserve the filter
  and selection on refresh when possible. Use the app's typography, surfaces, focus
  states and controls. Keep the layout usable by keyboard and at narrow widths.
- Refresh the list after each write. An update is an explicit action and identifies
  the two versions it affects. Offer project actions only for an open project.

This is a large change across storage, desktop UI, audio, editor, CLI, and MCP.
Keep Fracture, Singletonize, and Loss Check.
