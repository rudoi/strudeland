# Instrument library

## Goal

Give composers a reusable library of sound chains and multi-voice kits. A song can apply a
preset with defaults or pass options such as `lead({ cutoff: 900 })`. A project carries
the definitions it uses so it can play and export on another machine and keep the same
sound after changes to the global library.

Keitora task #23 depends on project folders (#2, complete). The instrument browser and
MCP tools (#24) depend on this work.

## Scope

- Define and document an editable library for one user across projects. Support single-voice
  sound chains, parameter defaults and overrides, and layered kits with named voices.
- Provide explicit commands to list library entries, pin selected entries to a project,
  inspect its pins, and update a selected pin. Require the current pin fingerprint on an explicit
  update command. Report missing entries, malformed definitions, and invalid options with
  the relevant instrument and parameter names.
- Load project pins for editor playback and queries, desktop audio and stems export, and
  CLI render. Carry an empty set for existing songs and raw-source callers without a project.
- Leave browsing, auditioning through the browser, and MCP tools to #24. Package downloads,
  sample distribution, and new audio engine functionality are outside this task.

## Architecture

### Library and pins

Use a versioned data format. Start with declarative definitions for ordered Strudel sound
and effect controls, named scalar parameters with defaults, and kits that combine voices
from the same definition. Keep definitions self-contained in the first version. This makes
pins portable and prevents a kit from changing when a separate library entry changes.

`crates/strudeland-project/src/project.rs` already creates an `instruments/` directory.
Extend that crate to resolve it, read a validated snapshot, and write pins using its
project-path and atomic-write conventions. Treat a missing directory in an older project
as empty. Use the per-user data directory and the bundle identifier
for the global library. Validate identifiers, schema version, payload size and
path containment. Reject symlink escapes and ambiguous names on case-insensitive filesystems.
Use one file per pin so replacing one entry needs a single atomic write. Detect competing
writes, and leave the prior pin intact on validation or write failure.

A pin stores the entire definition and its format version. Keep the instrument ID stable and
show a content fingerprint when comparing the pin with the library. A first pin refuses to
overwrite an existing one. An explicit update validates the proposed replacement and checks
that the pin still matches the version being replaced. Opening or rendering a project reads
its pins without consulting the global library. Evaluation never saves or refreshes a pin.

### Song bindings and evaluation

`src/lib/strudel/prebake.ts` and `src/host/prebake.ts` initialize their evaluation scope
once. Share an instrument adapter between them, and install a fixed Strudel pattern
operation with `register()` during prebake. It applies a compiled chain to a pattern and
combines kit voices using existing Strudel operations. Pass the definition or compiled
operation as an argument so the registered operation itself needs no project state.

Expose the per-evaluation definitions through a lexical `instruments` binding. A song
can opt into a short name with `const { lead } = instruments`, then use
`note("c4 e4").apply(lead({ cutoff: 900 }))`. Bind factories to the captured snapshot.
The vendored `register()` writes `Pattern.prototype` and `strudelScope`, so registering
each pinned name would make projects share mutable methods. Check the fixed operation's
name for collisions and preserve the existing song authoring surface. Bind the namespace
after transpilation so the editor and control queries continue using the original source
offsets. Preserve normal JavaScript shadowing and validate lazy Pattern evaluation when a
second project is opened or a re-evaluation fails.

### Desktop and offline entry points

- `src/lib/project/session.ts` currently calls `runtime.evaluate` with song text and
  tempo options. Load pins through a root-checked project API on open and refresh the
  snapshot after pin commands or a project pin change. Ignore stale responses using the
  session's project generation. Pass the cached snapshot synchronously to the serialized
  evaluate in `src/lib/strudel/runtime.ts`, where WebKit audio resume still runs inside the
  user gesture. A playing pattern retains its captured version.
- `src-tauri/src/export_audio.rs` receives the live editor text. Capture project pins as
  part of one export request and pass them through `strudeland-render/src/plan.rs` to
  `Host::plan_render`. Use that snapshot for every chunk and stem in the export.
- `crates/strudeland-cli/src/run.rs` resolves render input. Use project lookup for project
  songs and pass the same pin data into the renderer. Standalone songs keep an empty input.
  Route project pin commands to the same Rust store and document commands in the embedded
  CLI skill. Follow existing offline project discovery for commands that update pins.
- Carry optional instrument input through `src/lib/render/job-plan.ts` and
  `src/host/main.ts`, keeping the browser render page and headless host on the same
  evaluation contract. Keep sources and pins separate at the boundary. Regenerate the
  committed host bundle and its input hashes when the shared adapter is added.

## Technical design and verification

- Define the schema, the supported controls and the value/parameter rules before wiring
  commands. Exercise an ordinary chain, a kit, defaults, overrides, and invalid options.
  A kit's member names should remain available to existing voice-grouped stems when the
  caller has supplied no conflicting label. Verify the actual hap values and grouping.
- Confirm an explicit update changes the next evaluation and preserves the playing
  pattern. Verify two projects can pin different versions under the same ID, including
  after a failed evaluation. Verify playback and export work with the library absent.
- Check that desktop and headless evaluation produce the same resolved voices and
  controls for the same source and pins. Cover source offsets, browser/host job parity,
  legacy projects, missing pins, malformed input, and rejected competing writes.
- Keep existing Rust engine support checks in force. The library expresses ordinary
  pattern controls. The renderer should report unsupported mapped controls using its
  existing signature reporting rather than silently substituting a sound.

## Dependencies and risks

Registration must work with Strudel's shared global state and the fresh V8 context used
by the headless host. Parameter validation and error reporting must behave the same on
both paths. Pin updates and offline resolution also need to respect project ownership,
case rules, and atomic writes on macOS, Windows, and Linux.

This is a large change spanning storage, CLI, editor runtime, and render planning. Keep
Fracture, Singletonize, and Loss Check.

## Refinement decisions

- Use a manually editable JSON format. Presets accept literal controls and scalar parameter
  references. Kit voices layer the input notes or rhythm. Built-in Strudel control ordering
  still applies. Files contain data only.
- Use the song-local `lead({ cutoff })` binding through `.apply()`. The fixed prebake
  registration applies the compiled transform. Project-defined method names on
  `Pattern.prototype` are outside this release.
- Provide offline CLI commands for library and pin management. Watch pin files from the
  desktop so a CLI update is available on the next evaluation. The browser remains #24.
