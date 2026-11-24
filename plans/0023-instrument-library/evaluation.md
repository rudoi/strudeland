# Instrument evaluation contract

The [storage contract](storage.md) defines serialized records. The [proposal](proposal.md)
describes the feature boundary.

- Install a single fixed Pattern operation at desktop and host prebake with `register()`.
  Its input contains the compiled transform and carries no mutable project registry. Expose
  lexical `instruments` factories in evaluation. A factory returns a transform passed to
  `.apply()`, and it captures its definition so a later evaluation cannot affect it.
- Validate the definition and option names and types before executing controls. Use canonical
  Strudel control functions and `pure` for literal values. An ordered chain applies successive
  controls and a kit stacks the resulting voices. Use the existing hap value `label` for kit
  stem names, qualified by instrument ID. Respect existing explicit labels. Keep source offsets
  attached to song haps as they were without instruments.
- Wrap transpiler output with the lexical binding. Do so inside the serialized runtime
  evaluation. Leave original source and mini locations unchanged and keep authored local
  bindings legal. Preserve the ordinary evaluator path when the snapshot is empty apart
  from availability of the empty namespace.
- Cache pins through project open and the root-checked project watcher in desktop session
  state. Awaiting the filesystem before requesting audio resume would break the WebKit gesture
  path. A failed refresh surfaces an error on evaluation and cannot change current playback.
- Export and CLI rendering resolve pins once per operation and pass the snapshot through the
  Rust planner and JS host. Keep source, pins, and chunk requests separate. The browser render
  page accepts the same optional snapshot. Regenerate and verify the checked-in host bundle.
- Verify voice control values and locations, option/default behavior, lazy evaluation isolation,
  browser/headless parity, stems grouping, and the empty snapshot path through existing checks.

## Binding interfaces

`src/lib/strudel/instruments.ts` exports the structural TypeScript `InstrumentSnapshot` and
the common prebake and transpiler helpers. The runtime option is `instruments?: InstrumentSnapshot`.
Build a per-evaluation namespace before running the script, after prebake resolves. Use the
core setters with explicit literal patterns and a kit label of `<id>/<voice>` only when the
hap has no label. Freeze the namespace and capture compiled controls before the song runs.
A runtime requiring a fallback scope getter clears it as soon as evaluation settles. It passes
the snapshot into the lexical binding before running any user code.

Project IPC provides `project_instruments(root)`. The `project://changed` event carries an
`instruments` flag. The session clears the old project's snapshot at install and uses the same
serialized queue as other watcher updates. A snapshot load error surfaces on attempted
evaluation, with a subsequent successful refresh clearing it. Explicit updates change the next
evaluation and do not restart the scheduler.

`RenderOptions` and the host render request carry a validated snapshot with an empty default.
`src/host/main.ts` and `src/render/main.ts` pass it through their planner runtime adapters
into evaluate options. Repeat and stems paths share the same captured input as normal rendering.
