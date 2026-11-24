# Instrument automation

See the [storage and preview contract](storage-and-preview.md) for creation and the
[browser and editor contract](browser-and-editor.md) for the shared apply operation.

- Keep `instrument library`, `instrument list`, `instrument pin`, and
  `instrument update` compatible. Add `instrument create ID --file PATH` and
  `instrument apply ID --project DIR --song SONG --from N --to N --expect-file PATH`
  with the project's existing resolution rules. Apply targets the active editor;
  return an actionable error when the app or song is unavailable. Creation accepts
  the same JSON definition as an on-disk file.
- Add a typed `instrument.apply` RPC with camel-case fields
  `root`, `song`, `id`, `from`, `to`, and `expect`. Return
  `song`, `from`, and `to` with the new replacement range.
- Add `strudeland mcp` for local stdio. Expose list, create, pin, apply, and update
  tools with the same instrument store and control socket operations. Validate
  discovery and calls using a subprocess. Keep diagnostic output on stderr.
- Update the embedded CLI skill for the commands, flags, results and exit behavior.
