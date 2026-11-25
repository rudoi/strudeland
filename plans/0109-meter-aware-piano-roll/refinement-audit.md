# Piano roll refinement audit

This audit checked the existing Strudeland contracts at `e7f7b9d`. The proposal incorporates
the corrections. The checks exercise existing code and isolated examples of the proposed
behavior.

## Code references and consequences

| Area | Verified contract | Consequence |
| --- | --- | --- |
| Selection and document changes | [Editor setup](../../src/lib/editor/setup.ts) listens for transactions. [Session `show`](../../src/lib/project/session.ts) attaches a replacement editor state and calls `onShown`. | Observe ordinary edits and selections separately from song swaps. Invalidate a gesture across a switch or undo even if the text returns to its earlier value. |
| Source writes | [Session `applyInstrument`](../../src/lib/project/session.ts) checks project identity and expected text and dispatches a range transaction with full history isolation. `saveAndEvaluate` captures the buffer and applies save formatting if enabled. | Use a session-owned transaction for the gesture. Keep formatting, saving and playback changes in the existing commands. |
| Source recognition | [Note audition syntax](../../src/lib/editor/note-audition.ts) distinguishes pitched leaves from arguments. [The formatter](../../src/lib/editor/format-song.ts) checks literal spellings and mini directives and gates layout formatting by meter. [The transpiler mini plugin](../../vendor/strudel/packages/transpiler/plugin-mini.mjs) lowers supported strings to helper calls. | Reuse individual recognition rules. The formatter as a whole has a different contract. Check shadowed pitch producers and transpiled helpers, and validate replacement ranges against exact source slices. |
| Arrangement and meter | [Song queries](../../src/lib/strudel/song.ts) shift each section into local time and clip whole haps to that occurrence. [Layout clock](../../src/lib/strudel/timing.ts) maps written bars to physical lengths. [Metered queries](../../src/lib/strudel/metered-pattern.ts) split at meter boundaries and supply local CPS. | Track local pattern phase and written meter separately. Carry whole and visible spans. Use the same physical scale in the grid and hit testing. |
| Inspection | [Runtime query](../../src/lib/strudel/runtime.ts) reads the last successfully evaluated final pattern. The [JS host query](../../src/host/main.ts) shapes events for render diagnostics. The [Rust host](../../crates/strudeland-host/src/host.rs) has a synchronous response and an unbounded worker queue. Its existing query passes no deadline to [the V8 engine](../../crates/strudeland-host/src/engine.rs). | Add an inspection request with separate admission, timeout, serialization and stale-reply handling. Preserve the host’s macOS jitless configuration from the [app crate](../../src-tauri/Cargo.toml). |
| Evaluated context | The [runtime](../../src/lib/strudel/runtime.ts) publishes successful source and seed. The [session](../../src/lib/project/session.ts) caches instrument pins and refreshes them through project changes. The [host README](../../crates/strudeland-host/README.md) documents deterministic globals, inert timers and initial slider values. | Carry revision, controls, seed and pins explicitly. Distinguish the inspected buffer from a matching live evaluation. Surface host limitations before claiming in-song parity. |
| Dock persistence | [Frontend preferences](../../src/lib/prefs/store.ts) merge early edits with the loaded settings and serialize writes. [Native preferences](../../crates/strudeland-project/src/prefs.rs) default missing fields while retaining schema 1. [The app](../../src/App.svelte) mounts editor before timeline. Its [window configuration](../../src-tauri/tauri.conf.json) allows 720 by 480. | Extend the mirrored app preference with a collapsed default. Protect a toggle before settings load. Test the roll and its disclosure at the supported minimum size. |
| Audition | [App composition](../../src/lib/app.ts) gates previews on transport, recording and memo state. [The typed-note queue](../../src/lib/editor/note-audition-queue.ts) debounces edits. [Runtime preparation](../../src/lib/strudel/runtime.ts) readies the Web Audio preview graph for either playback engine. [The player](../../src/lib/audio/note-audition.ts) bounds voices and verifies registry identities. | Give deliberate roll gestures a direct preview route with shared voice ownership. Reuse the automatic preference for add and pitch change. Ensure the stock fallback remains usable after a song overrides a registry name. |

## Focused probes

- A regular call and a labeled pattern body can be captured at the original AST span without
  shifting mini offsets or changing the composition returned by the song. The same late hook
  misses an already-lowered bare mini literal. A short-circuited argument can be instrumentable
  and remain unevaluated. Lazy callback targets need a separately bounded discovery rule if
  supported.
- Source probes confirmed that `note("~")` is valid silence while an empty mini string fails
  parsing. A zero weight can be treated as a default and a negative weight can fail during
  query. The source editor should emit only bounded positive weights. A local `m` function can
  intercept the generated mini call, and a template interpolation can be dropped by the pinned
  mini transform. Those cases require conservative edit capabilities.
- For the anchored 4/4 → 7/8 example, rewriting
  `<[c4 e4 ~@2] [~@5 g4 ~]>` as `<[c4 ~@3] [~@3 e4@2 g4 ~]>` moves just E4 from
  `[1/4, 1/2)` to `[10/7, 12/7)`. Both intervals represent two eighth notes in their
  respective bars. C4 remains at `[0, 1/4)` and G4 remains at `[12/7, 13/7)`.
  A second probe checked 4/4, 7/8 and 6/8 bar edges at 0, 16, 30 and 42 sixteenths.
- A sustained two-voice unison crossing a meter boundary returned two fragments on either
  side with matching whole spans. Deduplicating by whole span and pitch alone would remove a
  voice. An alternating phrase also returned different pitches at absolute and section-local
  phases. Use both as acceptance fixtures.

## Existing checks

- 291 Bun tests passed across 17 repository suites for editor hooks, source formatting, note
  auditioning, preferences, timing, transport, timeline, song arrangement, instrument evaluation
  and the host. `HOST_BUILD_CHECK=1` rebuilt the current host bundle in memory and matched it to
  the committed bundle and input manifest.
- `cargo test --locked --offline -p strudeland-host --test host --test plan_render` passed all
  28 tests, including fresh-isolate state, timer settlement, watchdog recovery and heap failure.
  Those checks cover the current host's planning operations. The proposed inspection request
  needs its own coverage.

## Remaining gates

1. Prove capture and attribution on the chosen transpilation seam, including labels, scope,
   short circuits, seed, pin changes and output parity. Confirm that any use markers preserve
   stateful behavior and existing context. Report uncertain attribution in the roll.
2. Measure signed desktop startup and preview responsiveness, memory, timeouts and dense
   results. The existing V8 near-heap-limit callback gives a termination path. Serialized
   response caps and allocator limits need separate validation.
3. Prove a bounded cross-bar serializer and gesture against overlap, snap, subdivisions, undo,
   retained meter anchors, neighboring events, and other uses of a shared definition before
   including cross-bar editing in the release.
4. Validate gesture routing and sound by ear on both playback engine settings. Test first-use
   Web Audio initialization, explicit audition with the preference off, automatic audition
   with it on, silence during transport/memo/recording, source switches, and a replaced
   `triangle` registry entry.
