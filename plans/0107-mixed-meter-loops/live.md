# Live playback and inspection

[Proposal](proposal.md) · [Arrangement and clock](arrangement.md)

The live adapter converts a scheduler query into logical cycles, supplies local CPS for mixed-meter pieces, and returns mapped haps to the scheduler. Preserve full gate and audible duration across a meter boundary and preserve context and seed identity. Keep logical queries for events, painters, and source highlighting. `runtime.query` uses the requested layout, not live transport. Use the untouched path for constant meter. New metadata describing an active meter map or authored range requires an evaluated `song()` with matching structure.

Keep the clock monotonic across safe transport edits, history pruning, re-evaluation and backend handoff. Resolve count-in from the armed start. Report a diagnostic for imperative CPS changes, patterned CPS changes, and `.cpm()` on mixed songs. On a hot edit, check the candidate at the next schedule window before replacing the playing pattern. `getCps()` follows the audible meter. Use local CPS for time-synced controls and carry elapsed phase alongside the logical onset. Live Rust already batches by CPS. Extend the hap tuple with optional trailing phase seconds and continue accepting the legacy four elements.

Add an accessible way to choose the temporary through-section in the timeline. Indicate the full range and meter changes. Use each bar's meter for beat labels. Extend control status compatibly for multi-section selection. Guard the app's fixed-grid backing and alignment features on mixed songs. Ordinary playback and unbacked recording remain available.

Validate in existing runtime/backend/transport/control/timeline/recording tests and a browser smoke run. Exercise crossing holds, seeded loops, a composed pattern outside `song()`, missed or duplicated onsets near boundaries, explicit meter labels, inspectable beat numbers, invalid CPS, engine handoff, and constancy of the legacy path.

Fixed-Hz modulators use elapsed phase. Cycle-synced rates use the musical onset cycle divided by local CPS as phase time, which preserves musical phase at the onset of each hap. Existing retrigger and onset-rate behavior remains in effect. The Superdough invocation can take optional phase seconds in addition to the musical cycle, with a recorded vendored patch for the shared live and offline path.
