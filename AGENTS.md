# strudeland: agent guide

A desktop Tauri 2 + Svelte 5 + TypeScript app for editing Strudel music. See [README.md](README.md)
for the overview and [CONTRIBUTING.md](CONTRIBUTING.md) for contributor guidance.

## Commands

| Task | Command |
|---|---|
| Install | `bun install` |
| Move the vendored Strudel to an upstream commit or `main` (exit 1: conflicts, 2: refused) | `bun run strudel:sync <commit\|main>` |
| Regenerate the committed Strudel docs file after a sync (reads `vendor/strudel/doc.json`) | `bun run docs:extract` |
| Run the app | `bun run tauri dev` |
| Build the debug app bundle signed with a stable identity and open it, so macOS keeps the microphone grant across rebuilds (identity: `APPLE_SIGNING_IDENTITY`, else the keychain's first "Apple Development" one; exit 1 none found or build failed, 2 bad arguments) | `bun run app:signed [--no-open]` |
| Frontend only (browser) | `bun run dev` |
| Build frontend | `bun run build` |
| Frontend tests | `bun test` |
| Typecheck | `bun run check` |
| Format check / fix | `bun run lint` / `bun run format` |
| Build the render page (`dist-render/`; `render:check` builds it when stale) | `bun run render:build` |
| Render the corpus, run gates and parity against goldens (needs Chromium once: `bunx playwright install chromium`; exit 0 pass, 1 a fixture failed or errored or a mutation was missed, 2 harness error: Chromium or the render build missing, bad arguments). `--engine rust` renders with the Rust engine (cargo-built, or `STRUDELAND_ENGINE_BIN`), judges it against the superdough goldens with `PROFILES.engine` and reports fixtures it cannot play as `unsupported`; with `--update-goldens` it exits 2. `--planner host` (requires `--engine rust`; default `page`) plans each engine job with one single-chunk `plan-render` of the headless host (cargo-built, or `STRUDELAND_HOST_BIN`) instead of the render page, and skips the render-page build; `--planner batch` plans like `host` and renders each fixture's main window through the whole Rust pipeline (a `PipelineRenderer`: `strudeland-render`, cargo-built, or `STRUDELAND_RENDER_BIN`) | `bun run render:check [fixture\|glob\|tag:<tag>]… [--engine superdough\|rust] [--planner page\|host\|batch] [--update-goldens] [--keep-wav] [--no-mutations] [--concurrency N] [--rebuild] [--out DIR]` |
| Check (or with `--write` regenerate) the engine's JS vectors (`crates/strudeland-engine/tests/vectors/`; exit 1: they differ) | `bun run engine:vectors [--write]` |
| Check (or with `--write` regenerate) the engine's Chromium references (`crates/strudeland-engine/tests/refs/`: micro cases and fixture jobs; also writes `v8-math.json` (a Chromium-evaluated JS math reference) and renders the measured-floor cases (`mod-lfo-note`, `vowel-formants`) eight times; needs Chromium; exit 1: they differ, over budget or a case failed, 2: bad arguments or harness error) | `bun run engine:refs [--write] [name…]` |
| Render an engine job with the Rust engine (the binary `render:check --engine rust` runs; exit 1: render errors, 2: bad input) | `cargo run -p strudeland-engine -- render --job <file> --out <file.wav>` |
| List what the Rust engine cannot play in a job (the unsupported signatures, one line of JSON `[{ s, keys, firstBar }]`; the only unsupported detection, called by `rust-engine.ts`; exit 0, 2: bad job) | `cargo run -p strudeland-engine -- signatures --job <file\|->` |
| Rebuild the headless host's bundle `crates/strudeland-host/js/host.js` (and `host.inputs.json`) after a Strudel sync or an edit of anything it bundles | `bun run host:build` |
| Regenerate the committed Tauri IPC bindings `src/lib/ipc/bindings.ts` (command names and argument keys, event names, command error kinds) from `src-tauri/src` after a command, event or kind change (exit 1: a command it cannot read, 2: bad arguments) | `bun run ipc:build` |
| Compare render-page and headless-host plans (needs Chromium; exit 0 equal, 1 difference or failure on one side only, 2 harness error). Each request is a single-chunk `planRender` on both sides, compared by job text byte for byte (and its info without `errors`). `--batch` compares each corpus window's whole `plan-render` plan (without `errors`) and each chunk's job against the page's single-chunk job. `--mixed-meter` compares the complete clip and loop-preset plans for the mixed-meter fixture, and two single-chunk jobs over it. | `bun run host:parity [fixture\|glob\|tag:<tag>]… [--batch] [--out DIR]` or `bun run host:parity --mixed-meter` |
| Check (or with `--write` regenerate) the render crate's TS-derived vectors (`crates/strudeland-render/tests/vectors/`: `loudness.json` from `loudness.ts`, `joiner.json` from `assembleWindow`; no browser; exit 1: they differ, 2: bad arguments) | `bun run render:vectors [--write]` |
| Time the webview scheduler on the synthetic live-pressure fixture (headless, real Cyclist, bun/JSC; prints ms per tick and per second of playback; exit 0 ok, 2 bad arguments) | `bun run bench:scheduler [--song FILE] [--seconds N] [--out FILE]` |
| Plan a song's engine jobs, query its events, or time full-song plans with the headless host (exit 0 ok, 1 host error including a planning `timeout`, 2 bad input; the binary `--planner host` and `host:parity` run) | `cargo run -p strudeland-host -- plan-render --song <file> --request <json\|file> [--out <file>]` (every chunk of a render in one call, or one chunk with `chunk`; `timeoutSec` bounds it), `… -- query --song <file> --from <c> --to <c> [--layout clip\|linear]`, `… -- bench [--bars N] [--lengths FILE] [--profile] [--out-dir DIR] <song…>` |
| Check the headless host's per-song budget (release, the largest corpus fixture's full window, planned as one chunk, under 2 s) | `cargo test --release -p strudeland-host --test perf_budget -- --ignored --nocapture` |
| Render a song file to audio with the whole pipeline, as the CLI does (plan, refuse unsupported, render, join, level, encode; harness binary: not a `strudeland` subcommand; exit 0 ok, 1 render failed, 2 bad input) | `cargo run -p strudeland-render -- render --song <file> --out <file> [--from N --to N --layout clip\|linear \| --section NAME \| --length N] [--loops N] [--format wav\|mp3\|ogg\|opus] [--bit-depth 16\|24\|32f] [--loudness on\|off] [--jobs N] [--plan-timeout SECS] [--info <file.json>]` |
| Time the whole render pipeline and every encoder over songs (JSON on stdout, table on stderr; exit 0 ok, 1 a song failed, 2 bad input) | `cargo run --release -p strudeland-render -- bench [--jobs N] [--lengths FILE] [--bars N] [--out-dir DIR] <song…>` |
| Check the live player's 48 kHz dropout budget (dense pattern under load and a sustained long-reverb workload) | `cargo test --release -p strudeland-live --test dropout -- --ignored --nocapture --test-threads=1` |
| Measure the live batch decode cost at 1/16/64/256 haps per batch (JSON to values and haps; prints a table; the app side is the invoke round trip in `strudeland state`) | `cargo test --release -p strudeland-live --test ipc_decode -- --ignored --nocapture` |
| Render a song file to audio (offline, no app needed; defaults -23 LUFS (EBU R128), -1 dBTP, wav 24-bit; exit 1 render failed, 4 bad length request) | `cargo run -p strudeland-cli -- render <song> [--out PATH] [--format F] [--preset tagged-loop\|intro-loop\|folded-loop\|one-shot] [--section NAME \| --length N] [--loops N] [--lufs DB] [--true-peak DBTP] [--loudness on\|off] [--bit-depth B] [--mp3-mode M] [--quality Q] [--jobs N] [--plan-timeout SECS]` |
| List the global instrument library and its location, or manage project pins | `cargo run -p strudeland-cli -- instrument library`, `… -- instrument list [--project DIR]`, `… -- instrument pin ID [--project DIR]`, `… -- instrument update ID --expect HASH [--project DIR]` |
| Create a validated instrument in the global library or apply a project pin to the active editor | `cargo run -p strudeland-cli -- instrument create ID --file PATH`, `… -- instrument apply ID --from N --to N --expect-file PATH [--project DIR] [--song songs/NAME.js]` (zero-based UTF-16 offsets, exclusive end) |
| Run the local instrument tools over MCP stdio | `cargo run -p strudeland-cli -- mcp` |
| Export an aligned pre-master mix and stereo float WAV stems (new directory in the project's renders folder by default) | `cargo run -p strudeland-cli -- render <song> --stems orbit\|voice\|instrument [--out DIR] [--section NAME \| --length N] [--loops N] [--jobs N] [--plan-timeout SECS]` |
| List a project's voice memos / show one's timing (offline; project from --project, the cwd, then the running app; exit 1 not found or bad sidecar, 4 no project) | `cargo run -p strudeland-cli -- memo list [--project DIR]`, `… -- memo show <name> [--project DIR]` |
| Transcribe a voice memo to Strudel code (offline; project as for memo; exit 1 not found, bad sidecar or unreadable WAV, 4 usage: bad flags, no project, `--bars` without a known grid, a grid that does not divide the bar, `--debug` without `--json`, a memo over 10 minutes without a span) | `cargo run -p strudeland-cli -- transcribe <name> [--project DIR] [--grid 4\|8\|16\|8t\|16t] [--bpm N] [--meter N/D] [--bar1 S] [--shift MS] [--key ROOT:SCALE] [--tuning CENTS] [--start S --end S \| --bars A-B] [--debug --json]` |
| Score the transcriber on the synthetic evaluation set (release; prints the per-tier table) | `cargo run --release -p strudeland-transcribe --features eval --example eval` |
| Run the transcriber's evaluation tiers alone (`cargo test --workspace` runs them too: the CLI's dev-dependency turns `eval` on) | `cargo test -p strudeland-transcribe --features eval` |
| Check the transcriber's budget (release, a minute of audio under 1 s) | `cargo test --release -p strudeland-transcribe --features eval --test perf_budget -- --ignored --nocapture` |
| Print the capture-timestamp probe for an input device (by hand, needs microphone access for the terminal; `--list` lists inputs, `--device ID` picks one, default 5 s; exit 1 permission denied or stream failed, 2 bad arguments) | `cargo run -p strudeland-record --example probe -- [--list] [--device ID] [--seconds N]` |
| Rust tests | `cargo test --workspace` |
| Rust lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Rust format check | `cargo fmt --all --check` |
| Build a local installer with its matching CLI (native Apple Silicon or Windows x64) | `bun run app:bundle dmg --adhoc` or `bun run app:bundle msi` |
| Install the CLI from source | `cargo install --locked --path crates/strudeland-cli` |
| Run the CLI | `cargo run -p strudeland-cli -- state` |
| Open a version bump PR or build a tagged release draft | `bun run release (--current\|--patch\|--minor\|--major\|--set VERSION) [--dry-run]` |

`scripts/app-bundle.ts` builds the locked CLI into the existing Cargo target and feeds it to
Tauri as a macOS sidecar or to the Windows MSI WiX fragment. The macOS release prepares the
CLI before importing the signing identity and bundles it during the signing step.

The [macOS workflow](.github/workflows/macos.yml) runs frozen Bun and locked Rust checks on
Apple Silicon. The [native workflow](.github/workflows/native.yml) checks Windows x64 and Ubuntu
24.04 amd64. Both workflows restore Cargo caches and save new entries from successful `main`
builds. Browser parity, live timing benchmarks, and installer builds run separately on demand.
The [contributor guide](CONTRIBUTING.md#verify-your-changes) lists the local commands.

`cargo test`/`clippy`/`check` do not need a built frontend (`dist/`). Only `tauri build` does, and it
runs `bun run build` itself. The first cargo build takes a few minutes. Building `strudeland-render` (and so `strudeland-app` and `strudeland-cli`)
needs `cmake` and a C compiler (libopus, LAME, libvorbis), and `cargo test -p strudeland-cli` needs
the V8 download the host brings.

## Layout

- `src/lib/strudel/` runtime (wraps `repl()`, owns tempo, the song transport and auto-stop),
  prebake, and song structure: `structure.ts` (`meta` rules, `buildStructure`, cps from bpm and
  meter; no Strudel, Tauri or DOM imports, so the renderer and headless host can reuse it),
  `transport.ts` (pure logical-cycle to song-position mapping: `locate`, segments, section
  ranges, quantization, `LINEAR_SEGMENTS` for the linear song timeline), `timing.ts` (maps
  logical layout cycles to reference-clock cycles for mixed meter, including repeated ranges),
  `song.ts` (the `song(meta, parts)` pattern, registered in the eval scope; it follows the live
  transport segments unless a query passes its own in `controls._segments`, as `runtime.query`
  does with `LINEAR_SEGMENTS`). `src/lib/audio/` the `AudioBackend` seam (and `scope-tap.ts`, the
  master-output analyser behind `readScope`; `playback-api.ts`, the only Tauri caller for live
  playback, and `rust.ts`, the Rust backend over it, tested with `tests/helpers/fake-playback-api.ts`). The master meter and limiter: `master-dsp.ts`
  (the limiter and level maths), `master-worklet.ts`, `master-tap.ts` (the superdough tap) and
  `limiter-pref.ts` (the stored preference); `src/lib/levels/levels.ts` (pure: frames, hold, the
  meter's view model; outside `src/lib/audio/` because the runtime imports it and the host bundle
  must not reach `src/lib/audio/`).
  `src/lib/editor/` CodeMirror setup, visuals and
  `theme.ts` (the strudeland theme, registered in Strudel's theme registry), `vim-mode.ts` (the vim
  mode store, `vimChip`, the `:w` ex command). `note-audition.ts` resolves typed pitch atoms
  and fixed synths from the local syntax. `note-audition-queue.ts` settles the edit before
  `src/lib/audio/note-audition.ts` plays a bounded preview on the Web Audio graph.
  `visuals.ts` outlines the sounding audition token independently of evaluated highlights.
  `format-script.ts` computes token-gap edits for chains and song structures, and `format-song.ts`
  combines them with localized mini-notation edits. The project session applies both on save.
- Piano roll (`src/lib/piano-roll/`) resolves caret, selections and aliases in `source.ts`.
  The direct codecs are `phrase.ts` and `authored-grid.ts`. The latter shares row decoding with
  `src/lib/strudel/note-grid.ts`. `event-source.ts` writes scoped edits for evaluated notes.
  For cross-bar moves, `move.ts` resolves exact quarter-note spans and checks section repetitions.
  `source.ts` proves written placements before committing. See the
  [cross-bar move contract](plans/0110-cross-bar-piano-roll-moves/contract.md) for placement and
  source-meter rules.
  `src/lib/strudel/event-edits.ts` applies scoped edits to logical-time haps before clock mapping.
  The session commits, reveals and marks changed ranges in one editor transaction. See the
  [editor](plans/0111-caret-driven-piano-roll/editor-experience.md),
  [grid](plans/0111-caret-driven-piano-roll/authored-grids.md) and
  [event edit](plans/0111-caret-driven-piano-roll/event-edits.md) contracts.
- `src/components/`
  Svelte UI, built from the primitives in `src/components/ui/` (`Button`, `Chip`, `Dot`, `Notice`,
  `Field`, `ListRow`, `Link`, `Menu`, `MenuItem`, `MasterMeter`, `Scope`). `src/styles/` the global CSS:
  `tokens.css` (palette, roles, domain sets and scales; the only file with colour literals),
  `base.css` (tones and their derived tints, utilities including the shared `.pressable` and
  `.control` states, focus), `editor.css` (CodeMirror chrome, docs popup), `fonts.css`.
  `src/lib/theme/` `resolveColor` (computed colours for canvas code) and the `Tone` type.
  `src/lib/scope/` the scope's pure maths (`trace.ts`, `draw.ts`) and the shared animation loop
  (`clock.ts`).
- `src/lib/play-arbiter.ts` the play arbiter: what play, stop, evaluate, section arming and record
  mean while a song, an audible memo, a take or an audition sounds. The toolbar, Cmd-Enter,
  the Timeline, the recording controls, the instrument browser, the audition gating and
  the control handler go through it.
- `src/lib/shell.ts` the app shell: the open dialog and where focus returns, the Quit decision,
  the window-close predicate it hands `session.init`, the gate table for menu items, shortcuts
  and the welcome screen, and the Export Audio item's availability. `src/lib/shell-ports.ts`
  wraps `app-quit.ts`, `onProjectMenu` and `setExportMenuAvailable`, which stay the only Tauri
  callers for them; tests use `tests/helpers/fake-shell-ports.ts`.
- `src/lib/project/` projects: `song-header.ts` (`readSongHeader`: the static song header read,
  its structure and evaluate options), `api.ts` (the only module that calls Tauri for projects:
  commands, events, dialogs, window), `session.ts` (open project, songs, dirty state, hot-reload
  and conflict rules; tested with the fake in `tests/helpers/fake-project-api.ts`),
  `song-playback.ts` (the playing song and its transport: the evaluated song, the reset when
  another song takes over, arming sections by name; the session builds it as `session.playback`
  and calls its verbs, never `runtime.transport`), `actions.ts` (shared open and create actions
  for the File menu, keyboard shortcut, and welcome screen),
  `meta-write.ts` (minimal-diff edits of header values), `timeline.ts` (pure helpers for
  `Timeline.svelte`).
- `src/lib/prefs/` app preferences: `api.ts` (the only module that calls Tauri for prefs) and
  `store.ts` (the prefs store, the one frontend copy of a pref).
- `src-tauri/` the Rust app crate (`strudeland-app`): `project.rs` (commands, open-project state, and
  the song commands (they reject with a `CommandError`), and the `prefs_get`/`prefs_set` commands),
  `project_menu.rs` (native project commands in File), `watcher.rs` (`project://changed` events),
  `command_error.rs` (the Rust `CommandError`, its kind enum and the `ProjectError` table).
  Further crates go in `crates/` and are added to the root `Cargo.toml` workspace.
- `src/lib/ipc/` the IPC contract: `bindings.ts` (command names and argument keys, event names,
  command error kinds; generated by `scripts/ipc/build.ts`), `command-error.ts` (`CommandError`
  and `toCommandError`, no Tauri import), `invoke.ts` (`invokeCommand`, the only `invoke` caller).
- Desktop audio export in `src/lib/export/` and `src-tauri/src/export_audio.rs` runs
  `strudeland_render::export` with the create-new replace policy. The dialog captures the active editor source, previews resolved
  output paths, and receives progress over a channel. Only the export API calls Tauri for this
  feature. The native command confirms replacements and atomically publishes new files without
  replacing a competing file. The macOS app enables the renderer's `host-jitless` feature.
  For stems, the dialog previews a new set directory and reports the published paths.
- Application Quit on macOS starts in `src-tauri/src/app_quit.rs`. The AppKit callback defers to
  the webview through `src/lib/app-quit.ts` and the shell's `onQuit`. The export dialog shows the wait decision and final
  outcome. The project session's `guardShutdown` shares the unsaved prompt with window close.
  An approved native exit holds the export permit through Tauri shutdown.
- `crates/strudeland-project/` the project folder library (layout, `project.json`, path scoping,
  atomic writes, templates, recents, app preferences (`prefs.json`)). No Tauri dependency, so the CLI can reuse it.
- The instrument library store and project pins live in `crates/strudeland-project`.
  `src/lib/strudel/instruments.ts` binds the pinned snapshot as `instruments` at
  evaluation. The project session refreshes the snapshot through the watcher, and the desktop
  export and CLI each capture the project pins for the entire render.
  `InstrumentBrowser.svelte` and `src/lib/project/instrument-browser.ts` browse and compare
  versions through the project API. `src-tauri/src/instruments.rs` renders bounded WAV previews,
  played by `src/lib/audio/instrument-preview.ts`. The session and `instrument.apply` control
  method share a conditional selection edit. `strudeland-cli/src/run/instruments.rs` shares
  instrument operations with its local stdio MCP server in `src/run/mcp.rs`.
- `crates/strudeland-ctl/` the control protocol (JSON-RPC types, framing, error codes, `socket_path`,
  blocking `Client`). Windows uses per-user named pipes. Linux uses private XDG Unix sockets
  with peer credentials. No Tauri dependency. `tests/wire/` the golden wire examples
  (hand-written, prettier-formatted), read by `tests/wire.rs`, the TS control tests
  (`tests/helpers/wire-examples.ts`) and, through the dev-only `wire-fixtures` feature, the
  CLI's and the app's tests.
- `crates/strudeland-cli/` the `strudeland` command (clap, output, exit codes) over
  `strudeland-ctl`. `src/docs.rs` plus `strudel-docs.json` are the embedded Strudel docs (`strudeland
  docs`); `scripts/extract-strudel-docs.ts` generates the JSON from
  `vendor/strudel/doc.json`. `skill/SKILL.md` is the Claude Code skill, embedded by `src/skill.rs`
  (`strudeland skill print/install`).
- `src-tauri/src/control.rs` the control socket listener; it bridges requests to the webview and
  replies, and answers `ping` and `screenshot` itself. `src-tauri/src/snapshot.rs` the native webview
  screenshot behind the `Snapshotter` trait (WKWebView, WebView2 and WebKitGTK). `src/lib/control/` the webview side: `protocol.ts`, `handler.ts` (methods on the
  session and runtime), `events.ts` (haps to inspectable events with bar, beat, section and source
  line/col; pure), `api.ts`. `src/lib/editor/claude-mark.ts` marks lines changed by Claude.
- `vendor/strudel/` the vendored Strudel fork (workspace packages, consumed from source):
  `UPSTREAM.json` (upstream repo, commit and date), `upstream-manifest.json` (hash of every
  vendored file at that commit, the base for merges and the integrity test), `PATCHES.md` (every
  local edit), `doc.json` (generated jsdoc output), `packages/` (core, codemirror, superdough, …).
  `scripts/strudel-sync.ts` moves it to another upstream commit (three-way merge per file) and
  `scripts/strudel-patches.ts` checks `PATCHES.md` against the manifest.
  `vendor/strudel/packages/superdough/rng.mjs` is the seeded RNG; it is imported directly as
  `superdough/rng.mjs` and deliberately not re-exported from superdough's index.
- `bunfig.toml` sets the hoisted linker (one copy of each dependency, so the single-copy tests
  hold) and preloads `tests/helpers/audioworklet-plugin.ts` (with `audioworklet-stub.ts`) so
  `?audioworklet` imports resolve under `bun test`. `package.json` overrides `vite` to one
  version; `vite.config.ts` uses the workspace `vite-plugin-bundle-audioworklet`.
- `strudel-pins.json` (schema 2) the vendored commit plus the npm pins that remain (kabelsalat);
  read by `tests/pins.test.ts` and embedded into the Rust binary (`app_info`, used by About &
  licenses in Settings).
- `src/lib/render/` the pure render analysis (no Strudel, DOM, Tauri or Playwright imports): `contract.ts`
  (`Renderer`, `RenderRequest`, `RenderResult`), `schedule.ts`, `wav.ts`, `dsp.ts`, `loudness.ts`,
  `align.ts`, `features.ts` (golden features), `parity.ts`, `lookback.ts`, `chunks.ts`,
  `logged-errors.ts` (core logger failures as `RenderError`s, shared by the page and the host),
  `thresholds.ts` (every gate constant, citing its calibration evidence), `gates/` (seam, twin,
  stems, bass, mix), `corpus.ts` (manifest), `check-options.ts` (CLI args), `report.ts`.
  `src/lib/audio/offline.ts` is the offline superdough backend (`renderOffline`; it turns
  superdough's node pool off for the render, so pool reuse cannot depend on `ended` timing). `src/render/` and
  `render/index.html` are the render page (`window.strudelandRender`; `guard.ts` makes a live
  `AudioContext` throw; the `math` page kind, `math.ts` and `math-worklet.ts`, evaluates V8
  `Math.*` inside an AudioWorklet for `engine:refs`), built by `vite.render.config.ts` into
  `dist-render/`. `scripts/render/` the Playwright driver (`chromium.ts`,
  `superdough-renderer.ts`, `transfer.ts`), `check.ts` (`render:check`), `mutations.ts` (known
  defects the gates must catch), `make-samples.ts`, and `probes/*.probe.ts` (calibration). Use
  `PROBE_EVIDENCE_DIR=<dir>` and `PROBE_LABEL=<suffix>` to direct calibration captures.
  `stem-twin.probe.ts` bisects a twin difference by stem and prints its results.
  `tests/render/` the corpus (`corpus.json`, `corpus/*.js`, `samples/`) and `goldens/*.json`.
- `crates/strudeland-engine/` the single-threaded Rust renderer of engine jobs, ported from
  superdough and Blink at the Chromium tag Playwright drives. `job` (schema 2 and validation),
  `capability` (what the engine plays: the key allowlist audited in `controls.md`, the
  sounds, chain and modulator keys, the unsupported detection behind the binary's `signatures`
  subcommand, called by `scripts/render/rust-engine.ts`, and the `controls --json` dump),
  `params` (the typed controls a voice reads, superdough's defaults and its mutation of a hap
  value), `coerce` (JS `Number`, `String`, truthiness and `Math.max`/`min`), `js` (superdough's seeding and `String(number)`),
  `jsmath` (V8's `Math.*` bit for bit: CORE-MATH and V8 ports, the host `pow`/`tanh`),
  `render_core` (the render core both drivers share: voices, buses, orbits, caches, the IR
  settler seam), `render` (the offline driver: the pause loop and frame conversions), `voice` (one
  hap's graph), `chain` (the voice's stereo stage chain, FX entries, the node table), `param`,
  `adsr`, `periodic`, `osc`, `fft`, `biquad`,
  `panner`, `noise`, `worklet` (how Chromium runs an AudioWorklet processor), `supersaw`, `pulse`,
  `shaper` (coarse, crush, shape, distort), `transient`, `ladder`, `djf`, `lfo`, `envelope`,
  `compressor`, `vowel`, `modulate` (`lfo`/`env`/`bmod` modulators), `bus`, `orbit` (buses,
  stems routing, duck), `delay`, `reverb` (IR generation and the convolver), `wav`, `live`
  (`LiveEngine`, `LiveHap`, `LiveEvent`, the IR hooks `IrRequest`/`IrBuilt`/`build_ir`; streaming
  mode on the shared core); the binary in `src/bin/`. `tests/` holds the integration tests
  (`micro.rs`, `fixtures.rs`, `never_panics.rs`, `v8_math.rs`, `licence.rs`,
  `live_equals_offline.rs`, ...), `tests/data/` (the signature table), `tests/vectors/` (from
  `engine:vectors`) and `tests/refs/` (from `engine:refs`, with `v8-math.json`; `thresholds.json`
  holds each micro case's null threshold and the fixture envelope tolerances). The README lists
  the ported Blink files, the re-port procedure and the known differences.
- `src/lib/render/plan.ts` the planning helpers both renderers share (hap selection, window and
  channel plans, pauses, orbit creation order); `src/lib/render/engine-job.ts` the engine job (`planEngineJob`, `$num`/`$unsupported`/`$set`
  encoding, `validateEngineJob`, mutations, the `UnsupportedSignature` type; unsupported detection
  lives in the binary). `scripts/render/rust-engine.ts`
  (build, run and parse the binary, `signatures`) and `rust-renderer.ts` (the `Renderer`: jobs from the render
  page, audio from the binary). `scripts/engine/` `engine:vectors` and `engine:refs` (pure parts
  in `refs-lib.ts`, `provenance.ts`); `tests/render/engine-micro/*.js` the micro cases (one DSP
  rule each, at most two sounding inputs per summing junction, so Chromium renders them
  identically every run; the measured-floor cases are the named exceptions). `scripts/render/probes/engine-null.probe.ts` calibrates
  `PROFILES.engine`.
- The headless host: `src/lib/render/job-plan.ts` (`planRender`,
  the planner the render page and the host both run: evaluate, schedule query, select, plan,
  `PlanJobError` kinds `meta`/`eval`/`plan`) and `page-job.ts` (the engine job as text, `asciiJson`).
  `src/host/` the host bundle's entry (`main.ts`, the `PlannerRuntime` adapter), the inert audio
  backend, the prebake and the shims of codemirror's and webaudio's value-affecting names
  (`shims.ts`, `superdough-names.ts`, `not-reachable.ts`); the bundle imports no superdough,
  webaudio or codemirror. `crates/strudeland-host/` the Rust host (raw `v8` crate: a fresh isolate
  per request, deterministic natives, `HostError`; the `strudeland-host` binary with
  `plan-render`, `query`, `bench`) and its committed bundle `js/host.js` with `js/host.inputs.json` (the hashes
  of every bundled file); its README has the runtime choice, the `Math` classification and the
  known differences. `scripts/host/build.ts` runs `host:build` with Bun's bundler.
  For [piano roll inspection](plans/0109-meter-aware-piano-roll/inspection.md), `src/host/inspection.ts`
  captures expressions, exact event spans and identifiers needed for write-back.
  `src-tauri/src/piano_roll.rs` resolves project
  pins and admits one preview. The frontend request queue and IPC live in `src/lib/piano-roll/`.
  `scripts/render/job-source.ts` (`JobSource`: where `RustRenderer` gets its jobs),
  `host-planner.ts` (runs the host binary), `host-parity.ts`
  (`host:parity`, pure parts in `src/lib/render/host-parity-lib.ts`).
- The render pipeline: `src/lib/render/job-plan.ts`
  `planRender` (resolve the length request, evaluate once, query the schedule once, plan every
  chunk's job, or one chunk for the harness with `chunk`), `batch-window.ts` (reads the pipeline's `--info` file, its chunk frames included, and
  builds the `AssembledWindow` for `--planner batch`), `chunks.ts` `renderChunkPlans`. `crates/strudeland-host/` also has
  `plan_render` (the `plan-render` subcommand), the planning timeout (a watchdog thread,
  `HostError::Timeout`) and `HostError::MetaFlag`. `crates/strudeland-render/` the pipeline as a
  library: `plan` (host to jobs, refusal of unsupported material; the only V8 caller) and
  `render_planned` (in `pipeline_audio`: a plan to joined audio through an injected chunk renderer),
  `render` (the worker pool), `join` (sample-exact joiner), `finish` (the one finishing loop: the
  `Leveling` switch between Limited and RepeatSafeStatic, through the `Codec` seam), `splice` (the
  loop splice and its file cuts), `level/` (`loudness.rs`, `true_peak.rs`, `limiter.rs`), `encode/`
  (`wav`, `mp3`, `ogg`, `opus`, `verify` decoders, `EncodeRequest` with the explicit MP3 flush),
  `loop_export` (the presets' public types and `finish_export`: one-time loop openings, isolated
  repeats with post-roll folding, and shared static gain),
  `export/` (the export module both the CLI and the app run: request, destinations, the role
  table, preview, the replace policies, ordered publish and the partial write, behind the
  `Pipeline` seam; `testing::FakePipeline` is its fake),
  `stems` (directory and buffer limits, pre-master float WAV sets and manifest), `output`
  (the suffixes, `OutputTarget` and the atomic write; `export/` resolves destinations), `options`, `summary` (progress and the summary), `bench`, and the `strudeland-render` harness
  binary (`render`, `bench`); its README has the formats, defaults and licences, and
  `tests/vectors/` is written by `render:vectors`. `crates/strudeland-cli/src/render.rs` is
  `strudeland render` (flag rules, the export adapter, progress, summary, exit codes). `scripts/render/batch-render.ts`
  (`BatchRenderer` and `PipelineRenderer`, the `Renderer` with `renderWindow`) and
  `loudness-vectors.ts` are the harness side (`--planner batch`, `render:vectors`);
  `crates/strudeland-project` also gives songs their project lookup and `renders/` paths.
  The engine's `stems` module captures orbit, labeled voice and resolved instrument groups
  from the same offline graph that renders the reference mix.
- `tests/bench/` (`live-pressure.js`, the synthetic scheduler and live-test fixture) and `scripts/bench/scheduler.ts`
  (the scheduler bench); `vendor/strudel/PATCHES.md` already covers the `fraction.mjs` patch.
- Voice memos: `crates/strudeland-record/` microphone capture
  (cpal input to a mono f32 WAV through rtrb rings and a writer thread, levels, anchors from frame
  index to mach host time; `Source` seam with `CpalSource` and `testing::FakeSource`; its README has
  the threads, format and the capture-timestamp decision). `crates/strudeland-project/src/recordings.rs`
  names, lists, renames and deletes memos, owns the sidecar (schema 1) and repairs `.partial` files.
  `src-tauri/src/recording.rs` the recording commands, events, `RecorderState`
  and the microphone permission (device prefs in `recording.json` in the app data dir);
  `src-tauri/Info.plist` (`NSMicrophoneUsageDescription`) and `Entitlements.plist` (audio input).
  `src/lib/recording/`: `api.ts` (the only Tauri caller for recording), `controller.ts` (the take
  state machine), `backing.ts`, `sync.ts`, `memos.ts`, `memo-ui.ts`, `types.ts`. `src/lib/audio/`
  `metronome.ts`, `memo-player.ts`, `output-clock.ts`; UI in `RecordControls`, `MemoList`,
  `DevicePicker` and `ui/Meter`. Tests use `tests/helpers/fake-recording-api.ts` and
  `fake-audio-graph.ts`.
- Melody transcription: `crates/strudeland-transcribe/`
  (pure library, no Tauri, V8 or audio I/O; stages `load`, `pitch` (`PitchTracker`, pYIN),
  `tuning`, `onset`, `segment`, `grid`, `quantize`, `key`, `notation`, `report`, `suggest` (grid
  division and shift suggestions), shared maths in `util`, every threshold in `consts.rs`; `eval` (the `eval` feature, off by default so the shipped binary lacks it; on in
  the crate's unit tests and through `strudeland-cli`'s dev-dependency) the seeded synthetic
  generator, the evaluation cases and the COnP scorer; its
  README has the constants, the evaluation table, the tuning log and the known differences).
  `crates/strudeland-cli/src/run.rs` `run_transcribe` and `output.rs` the transcription formatters.
- Live playback: `crates/strudeland-live/` `Player` (engine
  thread, IR worker, rtrb ring, resampler, `master.rs` the level meter and safety limiter), the `Output` seam with `CpalOutput` and
  `testing::FakeOutput` (the `strudeland-record` pattern), `Batch`, `Event`, `Stats`; the README
  is the place for the ring depth, the anchor rule and the thread scheduling (`rt`: on macOS the
  engine thread and the fake output's pull thread run under the Mach time-constraint policy, best
  effort with a QoS fallback; the engine renders in slices of at most 4 quanta and blocks between
  them, so it stays inside its computation budget). Tests `tests/player.rs`, `tests/rt_alloc.rs`,
  `tests/dropout.rs`, `tests/ipc_decode.rs` and the fixture `tests/data/live-pressure.job.json`.
  `src-tauri/src/playback.rs` the app side: `playback_start/schedule/stop/seed/now`,
  `PlaybackState` (one session, ids from 1, a stale id is kind `stale`),
  the playback kinds (`stale`, `output`, `badBatch`, `engine`, the TS `PlaybackError`) and the forwarder thread that moves the
  player's events onto a `Channel<Event>` through the `EventSink` seam; a page reload and
  `RunEvent::Exit` stop the session. Its tests use `FakeOutput`, never the device.
- Tests: frontend in `tests/*.test.ts` (shared fakes and fixtures in `tests/helpers/`); Rust unit
  tests inline in `src-tauri/src` and `crates/*/src`.

## Rules

- New logic ships with tests.
- Strudel is vendored in `vendor/strudel/`. Move it with `bun run strudel:sync <commit|main>`:
  conflicts leave markers and exit 1, refusals (uncommitted changes to `vendor/strudel/` or the
  derived files, conflict markers left in a vendored file, bad state) exit 2 and write nothing.
  Resolve conflicts, update `vendor/strudel/PATCHES.md`, run `bun test` and `cargo test
  --workspace`, and commit with the log the script prints. The sync regenerates `doc.json`,
  `upstream-manifest.json`, `strudel-pins.json` and `crates/strudeland-cli/strudel-docs.json`.
  Edit vendored files only for a patch recorded in `PATCHES.md`; the integrity test
  (`tests/vendor-integrity.test.ts`) fails on any unrecorded difference from the manifest. The
  kabelsalat pins stay in `package.json` and `strudel-pins.json` `npm`; bump them with `bun install`
  and `shasum -a 256`.
- Only `src/lib/strudel/runtime.ts` talks to `repl()`. Only `src/lib/audio/*` talks to superdough's
  audio context. Never create an `AudioContext` at module load; it must be resumed from a user
  gesture (`runtime.resumeAudio()`).
- Strudel's `evalScope` and `repl()` mutate globals; one runtime per window, tests must not assume
  isolation.
- Run `bun run format` before committing frontend changes. Prettier does not touch Rust or
  Markdown; use `cargo fmt` for Rust.
- All project file access goes through `crates/strudeland-project` (it keeps paths inside the
  project). The frontend gets no `fs:` permission. Song commands take the open project's `root`
  and reject a stale one with `the project changed`.
- `src/lib/ipc/bindings.ts` is written only by `bun run ipc:build`, never hand-edited. A change to
  a `#[tauri::command]`'s name or parameters, to `generate_handler!`, to an `*_EVENT` const or to
  `CommandErrorKind` reruns it in the same change; `tests/ipc-bindings.test.ts` fails until then.
  Every fallible command returns `CommandError` (or `ExportError`), never `String`. Every adapter
  calls Tauri through `invokeCommand` and narrows to its domain's kinds (so the per-feature "only
  X calls Tauri" rules mean only X calls `invokeCommand` for it). A stale `root` is kind
  `projectChanged`, never a compared message.
- A change to the CLI's commands, flags, output or exit codes updates
  `crates/strudeland-cli/skill/SKILL.md` in the same change. A test checks that every subcommand
  and long flag is mentioned, but it cannot check what the text says about them.
- Only `src/lib/control/api.ts` calls Tauri for the control socket. `strudeland-ctl` owns the
  method table (`Method`, one `Rpc` marker per method), the error codes and the `events`
  limits; the CLI calls only through `Rpc` markers. The wire examples in
  `crates/strudeland-ctl/tests/wire/` (one JSON file per method, plus `errors.json` and
  `limits.json`) are the cross-check: Rust round-trips them through the typed params and
  results, the webview handler accepts every params example, refuses every invalid one with
  its exact code and message, and produces their result shape, and the CLI and app tests
  take their fake replies from them. `protocol.ts`'s `ErrorCode`, `METHOD_NAMES` and
  `handler.ts`'s limits are checked against them. The webview stays the only params
  validator (Rust checks only `screenshot` and `project.open`'s absolute path). A change to
  a method's params, result or refusal texts updates its example in the same change; a
  change to the `state` result's fields updates `methods.rs`, `state.json`, `protocol.ts`,
  `handler.ts`, the CLI formatter and `SKILL.md` together.
- Song files are a plain Strudel script, evaluated exactly as on disk. The `meta` header is
  `const meta = { … }`, never `export const`: the transpiler parses code as a script. Strings in
  `meta` that the app reads (meters, section names, loop endpoints) use single quotes: the transpiler turns
  double-quoted strings into mini-notation.
- Songs may set `meta.seed` (an integer from -2147483648 to 2147483647, default 0). Audio
  randomness (noise, reverb, zzfx, oscillator phases) is fixed per hap from it; change `meta.seed`
  to vary it.
- Tempo comes from `meta.bpm` (quarter notes) and `meta.meter`; one pattern cycle is one written
  bar. `meta.meterChanges` sets the signature from each stated one-based bar. `meta.loop` selects
  a contiguous range through the end of a named section. New timing and range fields require
  `song()`. Templates never call `setcps`/`setcpm`. Constant-meter songs can override tempo with a
  status warning. Mixed-meter songs use the declared BPM.
- Render harness: nothing that imports Playwright is a `*.test.ts` (`bun test` stays browser-free;
  probes are `*.probe.ts`, run by hand). Goldens in `tests/render/goldens/` are written only by
  `render:check --update-goldens` (after a Playwright or Strudel bump, or an intended sound change);
  review the printed summary before committing. Corpus `tests/render/corpus/*.js` are in
  `.prettierignore`: prettier's `singleQuote` would turn mini-notation `"..."` into plain strings.
  Gate constants live in `thresholds.ts` with calibration evidence; re-run the probes after a bump.
- Rust engine: engine renders never write goldens (`render:check --engine rust --update-goldens`
  exits 2). Engine references and vectors are written only by `bun run engine:refs --write` and
  `bun run engine:vectors --write` (without `--write` they print a diff summary); regenerate both
  after a Strudel sync (the sync prints the reminder), a Playwright bump, a patch of
  `vendor/strudel/PATCHES.md` (added or edited), an edit to `scripts/engine/v8-math-cases.ts`, or
  any edit to `src/lib/render/plan.ts`, `engine-job.ts`, `lookback.ts`, `schedule.ts`, `job-plan.ts`
  or `page-job.ts`, or either of the render and Strudel `timing.ts` modules (`PLANNER_FILES`: the refs' provenance hashes them, so the staleness test fails until
  `engine:refs --write`); after a Playwright bump re-port the Blink changes (crate README) and
  re-run `engine-null.probe.ts`. Schema 1 jobs are rejected, not migrated: regenerate them. Floor
  mode is only for the named hash-set junctions (`FLOOR_CASES`), never to absorb a defect. The
  engine ports `offlineDuck` (`src/lib/audio/offline.ts`): a change to it moves the engine, the
  goldens and the refs together. Every file ported from Blink carries the header naming its Blink
  sources and the Chromium tag (`tests/licence.rs` checks it, `LICENSE-chromium` holds the
  licence); a file ported from V8 or CORE-MATH carries its family's header (`// Port of V8 (` or
  `// Port of CORE-MATH (`) and `LICENSE-v8` or `LICENSE-core-math` holds the licence (the same
  test). `jsmath`'s `pow` and `tanh` call the host C library as V8 does, so their exact tests
  (`v8_math.rs`) are judged on macOS only. Micro-case thresholds and `PROFILES.engine` cite
  evidence; never loosen one to absorb a defect. The engine's binary is not a `strudeland`
  subcommand (`crates/strudeland-cli/skill/SKILL.md` does not mention it); its library is behind
  `strudeland render`.
- Styling: colours, font sizes, radii, z-indexes and durations come from `src/styles/tokens.css`
  (`tests/theme-tokens.test.ts` enforces it). Components use role and domain tokens and
  `data-tone`/`data-cat`, never palette tokens. Build UI from `src/components/ui/`; add a primitive
  when an element appears in two components. UI copy is lowercase except proper nouns, user content,
  error text, licence identifiers and contextual help sentences. Canvas code gets colours from `resolveColor()`.
- Contextual help: Shared controls take a `help` prop. Custom controls use the `helpTarget` action
  on the interactive element. `src/lib/help/targets.ts` owns pointer/focus selection and each
  control's accessible description. `Help.svelte` presents the status and dialog slots. Read [Writing contextual help](docs/writing-contextual-help.md) when adding,
  changing or reviewing hover and focus hints. Verify each hint against the control's behavior and states.
- Headless host: `crates/strudeland-host/js/host.js` and `host.inputs.json` are committed and
  written only by `bun run host:build`, never hand-edited; rebuild after a Strudel sync (the sync
  prints the reminder) or an edit of any file listed in `host.inputs.json` (a `bun test` staleness
  test fails until then), then run `bun run host:parity` and `cargo test -p strudeland-host`.
  `host:parity` is the check that the host and the render page plan the same jobs (byte-equal job
  text, and `info` deep-equal without `errors`; `errors` is not compared: the page-only and
  host-only error sources differ, crate README).
  In the host `Math.random` throws and the clocks read 0; `Math` transcendentals are `jsmath`
  overrides or on the verified-native list (`crates/strudeland-host/src/math.rs`), checked against
  `v8-math.json`. The host's binary is not a `strudeland` subcommand (`SKILL.md` does not mention
  it; a test checks); its library is behind `strudeland render`. `host:parity --batch` is the
  check that `plan-render` plans the page's plans and the page's single-chunk jobs; a `job-plan.ts` edit also needs
  `host:build`, `engine:refs --write` and `host:parity --batch`.
- Render pipeline: `strudeland-render`'s binary is not a `strudeland` subcommand either (`SKILL.md`
  does not mention it; a test checks). `render:vectors --write` after an edit of `loudness.ts` or
  `chunks.ts` (`bun test` byte-compares the committed vectors). True peak is the BS.1770-4 Annex 2
  48-tap filter, checked against EBU Tech 3341 (+0.2/-0.4 dB). A lossy file is decoded again and
  its true peak corrected, at most twice and then a warning: a limited render (plain or one-shot)
  lowers its limiter ceiling, and a loop preset lowers the static gain its files share. Never
  loosen either to absorb a defect. Every encoder's licence file in `crates/strudeland-render/` is
  checked by `tests/licence.rs`; LAME (LGPL) is statically linked into the AGPL binary, so keep
  the build instructions public. Renders are byte-reproducible (fixed dither and stream serials):
  a test that compares bytes breaks when an encoder crate is bumped.
  `crates/strudeland-render/tests/golden/finish.json` pins finished bytes, summaries, warnings
  and progress; it is written only by `STRUDELAND_WRITE_FINISH_GOLDEN=1 cargo test -p
  strudeland-render characterization` (on macOS aarch64, where it is judged) after an encoder
  crate bump, and `.prettierignore` keeps prettier off it.
- Voice memos: only `src/lib/recording/api.ts` calls Tauri for recording; the frontend reads WAV
  bytes only through `recording_read`. A take is finalized into the project captured at start.
  The new-project `.gitignore` covers `/recordings/*` and keeps `recordings/.gitkeep` addable.
  The [README](README.md#sharing-voice-memos) documents how to adopt the rules in existing projects.
  Capture tests never touch hardware (`FakeSource`, a fake recorder port and permission provider).
  The sidecar is schema 1, owned by `strudeland-project`; the CLI and app read it through that
  crate. Keep `objc2-*` features explicit in `src-tauri/Cargo.toml`. In `bun run tauri dev` macOS
  charges the microphone permission to the terminal (a denied or missing grant records silence);
  test the app's own permission with `bun run app:signed` (a debug bundle signed with a stable
  identity, so the grant survives rebuilds; an ad-hoc signed build is a new app to macOS every time). `DEFAULT_CAPTURE_TIME` is
  provisional until it is validated by loopback measurement. See the
  [capture-timestamp probe](crates/strudeland-record/README.md#the-capture-timestamp-question).
  Change the constant and the pinned test together.
- Transcription: constants live in `crates/strudeland-transcribe/src/consts.rs`; the provisional
  ones are retuned only against real memos and every change goes in the crate README's tuning log.
  Segmentation requires a pitched frame at the voicing threshold. The report keeps per-bar tie
  parts, and both forms of generated notation play the joined note as one event.
  The evaluation pins in `tests/eval.rs` are measured, then pinned, and never loosened to absorb a
  defect. `strudeland transcribe` never writes a song and never reads the song file; Claude proposes a
  transcription by writing it unsaved into the song for the user to audition and correct by ear,
  and saves only when asked (`SKILL.md`). The crate has no binary: its library
  is behind `strudeland transcribe`.
- Only `src/lib/audio/playback-api.ts` calls Tauri for live playback; `rust.ts` and the runtime see
  only the `PlaybackApi` type, and tests use `tests/helpers/fake-playback-api.ts`. The Rust backend
  creates no `AudioContext`.
- Songs may set `meta.engine` (`'superdough'`, the default, or `'rust'`, single quotes): which backend
  plays the song live. Render, the headless host and `strudeland render` always use the Rust engine
  and ignore it; a recording take with song backing plays on superdough whatever it says, and so
  does every song in a plain browser (`bun run dev`, no rust backend). Changing it while playing
  switches at the next bar line: the old engine plays out what it was already sent and the new one
  takes over from the scheduler's queried end, so no hap is doubled or lost. Live playback is not
  byte-reproducible.
- `strudeland-live` real-time rule: the output callback and everything it calls never allocate,
  lock, block or log; it pops the ring, bumps atomics and pushes anchor samples, nothing else.
  Voice construction, resampling and reverb IRs happen on the engine thread or the IR worker. The
  ring depth and the dropout test's pass condition (zero underruns, zero late haps) are never
  loosened to absorb a defect.
- Repo is AGPL-3.0-or-later; keep the Strudel credit in the UI.
