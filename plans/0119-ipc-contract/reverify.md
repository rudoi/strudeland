# 0119 re-verification (step 0)

Checked on `llm/andrew/strudeland/0119-ipc-contract` at bb30ac5 (the same tree as `main`).

## Confirmed

- **`generate_handler!` registers 46 commands** (`src-tauri/src/lib.rs:61-108`, one per line
  from `:62` to `:107`).
  - 20 return `Result<_, String>`: `app_info` (`commands.rs:75`), `export_audio_set_available`
    (`project_menu.rs:70`), `project_create`, `project_open`, `project_info`, `project_last`,
    `recent_list`, `prefs_get`, `prefs_set`, `project_instruments`, `song_list`, `song_read`,
    `song_save`, `project_root_of` (`project.rs:314-437`), `instrument_library_entries`,
    `project_instrument_entries`, `instrument_definition`, `project_write_instrument_pin`,
    `instrument_preview` (`instruments.rs:14-95`), `piano_roll_inspect` (`piano_roll.rs:29`).
  - 3 return `SongError`: `song_create`, `song_rename`, `song_delete` (`project.rs:406-432`).
  - 10 return `RecordingError` (`recording.rs:1321-1412`). `host_now_ns` (`:1416`) is
    infallible.
  - 5 return `PlaybackError` (`playback.rs:225-272`).
  - 3 return `ExportError { message, written }` with no kind (`export_audio.rs:120-123`,
    commands at `:310`, `:339`, `:378`).
  - 2 return `&'static str`: `app_quit_listen`, `app_quit_reply` (`app_quit.rs:86`, `:95`).
  - 3 are infallible: `host_now_ns`, `control_attach`, `control_reply` (`control.rs:787`,
    `:793`).
- **The command names are typed as strings in 9 TS files**: `src/lib/info.ts:28`,
  `src/lib/app-quit.ts:9`, `:18`, `src/lib/piano-roll/api.ts:5`, `src/lib/prefs/api.ts:26-27`,
  `src/lib/recording/api.ts:91-106`, `src/lib/audio/playback-api.ts:105-111`,
  `src/lib/control/api.ts:21-23`, `src/lib/export/api.ts:56-65` and
  `src/lib/project/api.ts:198-217`.
  - The argument keys are camelCase by hand. Two use a spread: `{ ...version }`
    (`project/api.ts:204`, `:206`), where `InstrumentVersion` is `{ id, fingerprint, root }`.
  - Recording drops undefined keys with `defined()` (`recording/api.ts:85-86`).
- **6 event names are written on both sides:**

  | Event | Rust | TS |
  |---|---|---|
  | `recording://level` | `recording.rs:36` `LEVEL_EVENT` | `recording/api.ts:22` |
  | `recording://state` | `recording.rs:38` `STATE_EVENT` | `recording/api.ts:23` |
  | `project://changed` | `watcher.rs:27` `CHANGED_EVENT` | `project/api.ts:129` |
  | `project://menu` | `project_menu.rs:8` `PROJECT_MENU_EVENT` (private) | `project/api.ts:130` |
  | `control://request` | `control.rs:47` `REQUEST_EVENT` | `control/api.ts:17` |
  | `app://quit-requested` | `app_quit.rs:11` `REQUEST_EVENT` (private) | `app-quit.ts:5`, an inline literal |

  Two Rust consts share the name `REQUEST_EVENT`, so the generated keys can't come from the
  const names. `app_quit.rs:140` `CORE_EVENT` is a `u32`, not an event name.
- **Three kind enums are mirrored in TS with no cross-check:**
  - `RecordingErrorKind` (`recording.rs:64-78`) ↔ `RECORDING_ERROR_KINDS`
    (`recording/types.ts:9-20`). `toRecordingError` maps an unknown kind to `other`
    (`recording/api.ts:56-61`).
  - `PlaybackErrorKind` (`playback.rs:59-66`) ↔ `PLAYBACK_ERROR_KINDS` (`playback-api.ts:52`).
    `toPlaybackError` maps an unknown kind to `engine` (`:83-90`).
  - `SongErrorKind` (`project.rs:212-220`) ↔ `ProjectErrorKind` (`project/api.ts:61-62`).
    `toProjectError` casts the kind unchecked (`:181-182`).
- **`PROJECT_CHANGED` is a string sentinel:**
  - Rust: `project.rs:29` (`"the project changed"`), returned as a `String` by `current_for`
    (`:69-75`). `NO_PROJECT` (`:27`) is the other `String` from `current` (`:61-66`).
  - `changed_or` (`:239-245`) sniffs the message. Two `From<String>` impls use it:
    `SongError` (`project.rs:247-256`) and `RecordingError` (`recording.rs:97-106`).
  - `piano_roll.rs:11`, `:58` return it after the inspection.
  - TS: `project/api.ts:81`; `session.ts:728` compares `errorMessage(e) === PROJECT_CHANGED`.
  - The fakes throw the bare string (`tests/helpers/fake-project-api.ts:133`) or a
    `ProjectError` (`:137`). `fake-recording-api.ts:134` throws a `RecordingError`.
  - Rust tests compare to it: `project.rs:740-764`, `recording.rs:1720`, `:2238`, `:3044`,
    `export_audio.rs:524`, `instruments.rs:252`.
- **`errorMessage` is `e instanceof Error ? e.message : String(e)`** (`src/lib/errors.ts:5`).
  A raw `{ kind, message }` object reaching it gives `"[object Object]"`. Today every String
  command's rejection is a bare string, so it works by accident.
- **`kindedError`** (`src/lib/command-error.ts:3-7`) is the shared reader. Its importers are
  `project/api.ts:5`, `recording/api.ts:3`, `playback-api.ts:2` and `export/controller.ts:2`.

## Behaviour the plan must keep

- **memo-ui suppresses three kinds** (`src/lib/recording/memo-ui.ts:361-365`): `invalidName`,
  `exists` and `notFound`. Recording's `From<ProjectError>` maps only the `Recording*` variants
  to them; every other variant, `SongExists` included, is `other` (`recording.rs:108-118`,
  pinned at `:1743-1747`).
  - No recording path reaches a `Song*` variant today. `default_recording_stem` filters an
    invalid song stem to `memo` (`crates/strudeland-project/src/recordings.rs:757-767`), and
    the recording functions call only memo and sidecar methods. Keeping the rule is a guard,
    not a visible change.
- **"No project is open" is kind `other`** in both structured errors today
  (`project.rs:701-709`, `recording.rs:1723-1726`).
- **The control socket keeps read errors apart from song-command errors:**
  - `fromProjectError` (`src/lib/control/handler.ts:131-147`) acts only on
    `e instanceof ProjectError`. `songExists` → `SONG_EXISTS`, `invalidName` → invalid params,
    `songNotFound` → `NO_SUCH_SONG` or "removed", `projectChanged` → "the project changed; try
    again" (`:123-124`). Anything else passes through to "internal error: …" (`:697-702`).
  - `session.createSong` rethrows a `ProjectError` as is, and wraps any other rejection (its
    `readText`) as a plain `Error` (`session.ts:1436-1438`). A stale read after a create
    reaches the CLI as `internal error: the project changed`. If the read rejected with a
    `ProjectError`, it would become "the project changed; try again".
  - Rename and delete drop a `projectChanged` silently (`session.ts:1164-1165`, `:1187`,
    `:1223`). A delete treats `songNotFound` as done (`:1225`).
- **Export reads `written` off the raw rejection** (`src/lib/export/controller.ts:143-151`). The
  dialog tests reject with raw `{ message, written }` objects
  (`tests/export-dialog.test.ts:220`, `:285`, `:333`, `:349`).
- **Playback drops `stale`** (`src/lib/audio/rust.ts:91`, `e instanceof PlaybackError`).
- **`errors.ts` is a headless-host input** (`crates/strudeland-host/js/host.inputs.json`). The
  new error class must not live there, or `host:build` and the bundle staleness test come into
  scope. `command-error.ts` is not an input.

## Corrected from the research

- **20 commands, not 24, return `Result<_, String>`.** The other 26 are the 3 song, 10
  recording, 5 playback and 3 export commands, the 2 `&'static str` and 3 infallible ones.
- **The playback commands take `channel: Channel<Event>`** (`playback.rs:229`), and export
  takes `on_progress: Channel<ExportProgress>` (`export_audio.rs:384`). Both are invoke keys
  (`channel`, `onProgress`), so the generator keeps `Channel` parameters.
- **`WebviewWindow` is also an injected parameter** (`export_audio.rs:311`, `:379`,
  `app_quit.rs:86`). The generator drops it with `State`, `AppHandle` and `Window`.

## Precedents

- **Regex cross-check:** `tests/control-protocol.test.ts:13-20` reads
  `crates/strudeland-ctl/src/protocol.rs` and matches `pub const NAME: i64 = N;`. `:73-80` reads
  struct fields with a regex and camelCases them.
- **Committed generated file plus a staleness test:**
  - `tests/host-bundle.test.ts:24-26` (`run \`bun run host:build\` if not`);
  - `tests/render-vectors.test.ts:27-40` regenerates in memory and byte-compares, with the
    `REGENERATE` hint in the message.
  - Both generators are `scripts/<area>/<name>.ts` with `export function main` and
    `if (import.meta.main)` (`scripts/render/loudness-vectors.ts:342-352`).
- **Generated files are prettier-ignored** (`.prettierignore`: `crates/strudeland-host/js/`,
  the vectors and the golden).
- **`scripts/**/*.ts` is typechecked** (`tsconfig.json` `include`).

## Persisted bytes

- **None.** No golden, vector, ref, sidecar or project file embeds an IPC error. The control
  wire's codes and messages come from `handler.ts`, not from the Tauri rejection's shape.

## Conclusion

Worth doing as three stages. The kind rename and the move from `String` to typed errors have to
land in one commit with the TS converter. Otherwise a raw object reaches `errorMessage`.
