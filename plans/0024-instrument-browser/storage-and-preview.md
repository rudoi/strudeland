# Storage and preview

The [proposal](proposal.md) defines the product scope. The
[existing storage contract](../0023-instrument-library/storage.md) defines the file format.

- Add `InstrumentLibrary::create(id, definition_json)` and share the instrument
  validator and publication rules. Key reads by ID and expected fingerprint.
  Library and pin lists return existing `InstrumentEntry` values. Keep unqualified
  `pin_instrument` and `update_instrument` callers compatible and guard both source
  and destination versions for browser writes.
- Provide native Tauri commands `instrument_library_entries`,
  `project_instrument_entries`, `instrument_definition`, `instrument_preview`,
  and `project_write_instrument_pin`. The last command takes `root`, `id`,
  `libraryFingerprint`, and nullable `pinFingerprint` (null creates the pin).
  Definition and preview take `root` (null for global), `id`, and `fingerprint`.
  Preview returns binary WAV bytes. Finish the Tauri registration in the same commit.
- Use a fixed, generated phrase and an in-memory snapshot with the selected instrument.
  Bound audio length, level, and concurrent rendering. Return a useful error if the
  definition changed, the preview is busy, or a control is unsupported. Run render
  work outside the IPC thread.
- Test create, stale reads and writes, and real audio for both a preset and a kit.

See the [browser and editor contract](browser-and-editor.md) for audio delivery and
[automation](automation.md) for the CLI surface.

The native definition result is the schema-1 instrument itself. The native write result
is the resulting `InstrumentEntry`. Both list endpoints return `InstrumentEntry[]`.
The native layer passes a captured definition to render without a second store lookup.
