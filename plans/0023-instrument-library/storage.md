# Instrument storage and commands

The [proposal](proposal.md) covers scope and alternatives. The [evaluation contract](evaluation.md)
covers how the snapshot reaches the runtime.

- Store records as `<id>.json`. The global library lives under
  `dirs::data_dir()/com.rudoi.strudeland/instruments`. A project's copies live in its
  existing `instruments/` directory. A missing directory is empty. IDs use lowercase
  ASCII letters, digits and underscores, start with a letter, and avoid reserved names
  on Windows. Ignore non-JSON files and dot-prefixed temporaries.
- Format 1 is `{"schema":1,"parameters":{"cutoff":1200},"voices":[{"name":"lead","chain":[{"control":"s","value":"sawtooth"},{"control":"lpf","value":{"param":"cutoff"}}]}]}`.
  Parameters default to finite numbers, strings, or booleans. Overrides keep the default's type.
  A chain is nonempty and ordered. A kit is the same format with multiple uniquely named voices.
  The supported control list ships with the project crate and is shared with the JS adapter.
  All named parameter references must resolve. Bound file size, entry count, string length,
  voice count and chain length before passing a snapshot to the webview or host.
  Measure compact snapshot JSON with numeric fields normalized to zero, so Rust and JS use
  the same budget even when they serialize the original numbers differently. Disk reads
  and pin writes also enforce the actual bytes on disk.
- Rust exposes a validated `InstrumentSnapshot`, a map from ID to definition.
  List project pin and global records with fingerprints. A first pin is atomic and refuses
  replacement. Updating requires the expected current pin fingerprint and uses the project
  crate's scoped write helpers. Serialize cooperating writes using a stable lock rather than
  locking the file that gets replaced. Direct filesystem editors bypass that lock.
- Add offline `strudeland instrument library`, `instrument list`, `instrument pin ID`
  and `instrument update ID --expect HASH` commands. Project commands accept `--project`
  and use the existing offline project discovery. The global library is edited as files.
  Put example definitions and commands in the contributor-facing documentation and embedded CLI skill.
- Verify portable names, invalid definitions, duplicate pins, stale updates, symlink containment,
  missing legacy directories, filesystem limits and a reopen with the global library absent.

## Binding interfaces

Expose `InstrumentDefinition`, `InstrumentSnapshot` (a string-keyed map), `InstrumentEntry`
(with `id` and `fingerprint`), `InstrumentLibrary::open_default`, and project operations
`instrument_snapshot`, `instrument_entries`, `pin_instrument`, and `update_instrument`.
Use a structured diagnostic including the offending path or ID. Keep the canonical control
names in `crates/strudeland-project/instrument-controls.json`. Pin file names and the keys
in the snapshot are the IDs. List returns stable ID ordering. Use SHA-256 of the validated
deterministic JSON, so whitespace edits do not change the fingerprint.
