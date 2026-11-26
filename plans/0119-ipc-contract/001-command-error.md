# 001: one command error

## Goal

Every fallible Tauri command except `app_quit_*` rejects with a `CommandError { kind, message }`.
Export adds `written`. The stale project is a typed error and no longer a sentinel string. The
song kinds are renamed `exists` and `notFound`. Every TS adapter calls Tauri through one
`invokeCommand`, which turns every rejection into an `Error` subclass: any rejection with a
string message becomes an `Error` with that message. A malformed object keeps today's
`"[object Object]"` (`recording-api.test.ts:97` stays as is). All visible text is unchanged.

## Prerequisites

- **The plan's "Technical design"** sections "Rust", "TS: the converter" and the behaviour
  table.
- **The reverify sections** "Behaviour the plan must keep" and "Corrected from the research".

## Implementation

### Rust

1. **Add `src-tauri/src/command_error.rs`**, as the plan specifies, and `mod command_error;` in
   `lib.rs`.
   - `CommandError` and `CommandErrorKind`. The variant order is the plan's: it becomes the
     generated array's order in 002.
   - `new`, `other(impl Display)`, `busy`, `project_changed()`, `no_project()` and `for_memos`.
   - `Display` prints the message alone, and `std::error::Error` is implemented.
   - **The `From<ProjectError>` table** is one `match` over every variant, with no `_` arm. It
     goes through a private `project_kind(&ProjectError) -> (CommandErrorKind,
     Option<Subject>)`, which `for_memos` also reads.
   - The message constants `"the project changed"` and `"no project is open"` are private here.
2. **`project.rs`:**
   - `current` and `current_for` return `CommandError`.
   - `with_project` returns `Result<T, CommandError>` and is used by the song commands too.
     Delete `with_song_project`.
   - `install`, `create_project`, `open_project`, `open_last`, `reload_info`, `root_of`,
     `app_data_file`, `recents_path`, the `list`, `read` and `save` helpers and every command
     return `CommandError`. The watcher's `String` and `e.to_string()` sites become
     `CommandError::other` or `?` through `From<ProjectError>`. Each message must be the text
     the same failure gave before.
   - Delete `NO_PROJECT`, `PROJECT_CHANGED`, `SongError`, `SongErrorKind`, `changed_or` and the
     `From` impls.
   - Update the module doc (`:10-13`) and the `reload_info` and `with_project` docs.
3. **`recording.rs`:**
   - Rename `RecordingError` to `CommandError` and `RecordingErrorKind` to `CommandErrorKind`
     (sed, then read the diff).
   - Delete the struct, enum, helpers, `From<String>` and `From<ProjectError>`.
   - `From<RecordError> for CommandError` stays, unchanged.
   - **Every `ProjectError` → error conversion uses `CommandError::for_memos`.** Check each `?`
     after a `Project` or `ProjectError`-returning call, and give it an explicit
     `.map_err(CommandError::for_memos)?`, so none silently uses the shared `From`. Today
     they are the four in `list`, `rename`, `delete` and `read` (`:1068`, `:1077`, `:1081`,
     `:1085`, the second `?` of each line), `write_sidecar_or_fallback` (`:692`, `:699`),
     `entry` (`:705`) and `start` (`:992`, `:993`). `project_for(..)?` is already a
     `CommandError` and needs no map.
   - `project_for` returns `projects.current_for(root)` directly.
   - `StateEvent.error: Option<CommandError>`.
   - Import from `crate::command_error`, no longer from `crate::project::changed_or`.
4. **`playback.rs`:**
   - The same rename. `PlaybackError::new` becomes `CommandError::new`.
   - `stale()` becomes a private fn returning
     `CommandError::new(Stale, "the playback session ended")`.
   - Move `From<LiveError>` and `From<OutputError>` onto `CommandError`.
5. **`export_audio.rs`:**
   - `ExportError { kind: CommandErrorKind, message, written }`, in that field order.
   - `problem` → `Other`.
   - `reserve`'s error → `Busy`, with the same text.
   - `export_error` and `run_error` → `Other`.
   - Add `From<CommandError> for ExportError` (it keeps the kind, `written` empty), and replace
     `projects.current_for(..).map_err(problem)?` at `:249` and `:345` with `?`.
6. **`instruments.rs`:**
   - Return types become `CommandError`.
   - `.map_err(|e| e.to_string())` → `.map_err(CommandError::other)`.
   - `reserve_preview` → `CommandError::busy("instrument preview is busy")`.
   - `render_preview` keeps `String` internally, and the command maps it with `other`.
7. **`piano_roll.rs`:**
   - The return type is `CommandError`.
   - The lock failure is `CommandError::busy("A piano roll preview is already running.")`.
   - The inner `to_string()`s map with `other`.
   - The re-check returns `Err(CommandError::project_changed())`.
   - Drop the `PROJECT_CHANGED` import.
8. **`commands.rs` `app_info` and `project_menu.rs` `export_audio_set_available`:**
   `Result<_, CommandError>`, with the same texts through `CommandError::other`.
9. **`app_quit.rs`, `host_now_ns`, `control_attach` and `control_reply` are unchanged.**

### TS

10. **`src/lib/ipc/command-error.ts`** (new; replaces `src/lib/command-error.ts`, which is
    deleted):
    - a hand-written `COMMAND_ERROR_KINDS` with the Rust order, and `type CommandErrorKind`;
    - `class CommandError<K extends string = CommandErrorKind>` (`name = 'CommandError'`);
    - `toCommandError` and `narrowKind`, as the plan specifies. **Every branch that builds a
      new `CommandError` sets `cause: e`, the raw value, the `{ message, kind }` object branch
      included;**
    - no Tauri import. It may import `../errors` (`errorMessage`), which is not edited.
11. **`src/lib/ipc/invoke.ts`** (new): `invokeCommand(name: string, args?: Record<string,
    unknown>): Promise<unknown>`, with no type parameter. It is the only `invoke` caller in
    `src/`.
    - Every adapter casts the result at its call: `invokeCommand('song_read', { root, path })
      as Promise<string>`. The domain wrappers (`songInvoke`, recording's and playback's
      `call`) lose their `<T>` and return `Promise<unknown>`, and their callers cast. This is
      the plan's form; 003 then only types `name` and `args`.
    - `{ ...version }` (`project/api.ts:204`, `:206`) becomes
      `{ root: version.root, id: version.id, fingerprint: version.fingerprint }`.
    - Recording's `call` applies `defined` to its args itself (`args && defined(args)`), and the
      `recording_arm` and `recording_start` call sites (`recording/api.ts:96`, `:100`) pass the
      plain literal. `recording-api.test.ts:145-168` must pass unchanged.
12. **`project/api.ts`:**
    - `ProjectErrorKind` = `'exists' | 'notFound' | 'invalidName' | 'projectChanged' | 'other'`
      (`:62`).
    - Reword the `ProjectApi` docs that describe the rejection: `info` and `rootOf`'s
      "Rejects with the error string" (`:91-92`, `:110`) become "Rejects with a
      `CommandError`"; the three song methods (`:102-106`) keep "Rejects with a
      `ProjectError`"; `toProjectError`'s doc (`:175`) names the Rust `CommandError`, not
      `SongError`. The interface doc (`:83-86`) keeps naming `PROJECT_CHANGED` until 003
      deletes it. `InstrumentBrowserApi` (`:34-46`) has no rejection doc and needs none.
    - `class ProjectError extends CommandError<ProjectErrorKind>`, keeping
      `(message, kind)` and `name`.
    - `toProjectError(e)`: `toCommandError`, then `narrowKind` to the project kinds with
      fallback `other`.
    - `songInvoke` wraps `invokeCommand`. Every other method calls `invokeCommand` directly, so
      it rejects with a plain `CommandError`.
    - `PROJECT_CHANGED` and the event consts stay until 003.
13. **`recording/types.ts`:**
    - `RecordingError extends CommandError<RecordingErrorKind>`, keeping `(message, kind)` and
      `name`.
    - Update the header: it now imports `../ipc/command-error`.
14. **`recording/api.ts`:**
    - `toRecordingError` goes through `toCommandError` and `narrowKind(…, 'other')`.
    - `call` wraps `invokeCommand`.
    - `onState` narrows `payload.error.kind` with the same rule before calling `cb`.
15. **`audio/playback-api.ts`:**
    - `PlaybackError extends CommandError<PlaybackErrorKind>`, keeping `(kind, message)` and
      `name`.
    - `toPlaybackError` narrows with fallback `engine`.
    - `call` wraps `invokeCommand`.
16. **`src/lib/export/error.ts`** (new, Tauri-free: it imports only `../ipc/command-error`):
    - `class ExportError extends CommandError`, `name = 'ExportError'`, with
      `readonly written: string[]`.
    - `toExportError(e)`: an `ExportError` as is; else `c = e instanceof CommandError ? e :
      toCommandError(e)`, and `written` is `c.cause`'s `written` when it is an array of
      strings, else `[]`.
    - **Why a separate module:** `export/controller.ts` and `tests/export-dialog.test.ts` must
      keep importing nothing from `@tauri-apps` at runtime. Bun's `mock.module` is
      process-global, and `project-api.test.ts:49` and `recording-api.test.ts:9` mock
      `@tauri-apps/api/core` with `invoke` only.
17. **`export/api.ts`:** the three commands go through `invokeCommand` and `toExportError`
    (imported from `./error`); `setExportMenuAvailable` uses `invokeCommand`. It may re-export
    `ExportError` for type users, but `controller.ts` imports from `./error`.
18. **`export/controller.ts`:** `import { ExportError } from './error'`. `failureFrom` is
    `{ message: errorMessage(error), written: error instanceof ExportError ? error.written : [] }`.
    Drop the `kindedError` import.
19. **`prefs/api.ts`, `piano-roll/api.ts`, `control/api.ts`, `info.ts` and `app-quit.ts`:**
    `invoke` → `invokeCommand`, with the same names and args.
20. **`project/session.ts`:**
    - Rename the kinds: `'songExists'` → `'exists'` (`:1178`), `'songNotFound'` → `'notFound'`
      (`:1225`), and the doc at `:166`.
    - **Keep `e instanceof ProjectError` at `:1165`, `:1225` and `:1438`.** Don't widen any of
      them to `CommandError` (decision 6).
21. **`control/handler.ts` `fromProjectError`:** `case 'exists'` and `case 'notFound'`. Nothing
    else changes.
22. **The fakes** (`fake-project-api.ts`): rename the two song kinds. Their string throws stay
    until 003; they still work, since `toCommandError` keeps strings as their text.

## Tests

The plan's "Tests", the Rust and "TS (stage 001)" parts.

- **Rust:**
  - `command_error.rs`: the table, `for_memos`, the wire names, `Display`, and the two
    constructors' texts.
  - `project.rs`: the typed stale test, the no-project test, the renamed kinds, and `.message`
    for `contains`. Delete `song_error_serializes_as_kind_and_message`, which the wire test
    replaces.
  - `recording.rs`: `project_errors_for_memos` and the `project_for` kinds. `PROJECT_CHANGED`
    becomes `CommandError::project_changed()`.
  - `playback.rs`: the kind asserts are `CommandErrorKind`. The wire test moves to
    `command_error.rs`, but keep the `stale` JSON check at `:506`.
  - `export_audio.rs`: the stale kind, the `reserve` `busy` kind and the serialized field
    order.
  - `instruments.rs:252`.
- **TS:**
  - `tests/ipc-command-error.test.ts` (new).
  - `tests/ipc-adapters.test.ts` (new): prefs, piano roll, export, control, info and app quit.
  - Rewrite `project-api.test.ts:136-145`. Adapt `project-api.test.ts`, `recording-api.test.ts`
    and `playback-api.test.ts` to the shared converter, keeping their cases.
  - In `export-dialog.test.ts`, the raw rejections become `ExportError`s, imported from
    `../src/lib/export/error`. The file keeps no runtime `@tauri-apps` import.
  - Rename the song kinds at every site (`rg -n "songExists|songNotFound" src tests` lists
    them; it must then show only `recording-api.test.ts:89`, which keeps `songExists` as an
    unknown recording kind):
    - `src/lib/project/api.ts:62`, `session.ts:166` (doc), `:1178`, `:1225`,
      `control/handler.ts:134`, `:138`;
    - `tests/helpers/fake-project-api.ts:33-34` (the helper names may stay; the kinds change);
    - `tests/project-api.test.ts:61-71`, `:117-126`;
    - `tests/session.test.ts:1814`, `:1939`, `:1947`, `:1953` (title), `:1958`;
    - `tests/control-handler.test.ts:1478` (title).
  - If `control-handler.test.ts` has no stale `song new` read case, add one: override
    `h.api.readSong` to throw `new CommandError('the project changed', 'projectChanged')`, in
    the style of the `h.api.createSong` override at `:1331`, and expect
    `internal error: the project changed`.
  - **`tests/ipc-command-error.test.ts`** includes
    `toCommandError({ kind: 'other', message: 'x', written: ['/a'] }).cause` having `written`
    `['/a']`, and `toExportError` of it giving `written` `['/a']`.
  - **The core mock:** add `tests/helpers/tauri-core-mock.ts`, a `mock.module` factory with
    `invoke`, `Channel` and `isTauri`, and use it in the new `ipc-adapters.test.ts` (it reaches
    `export/api.ts`, which imports `Channel`). Moving the three existing api tests to it is
    optional.

## Validation

- **The stage gates,** logged once (000-prompt).
- **`rg -n "import \{[^}]*\binvoke\b" src`** matches only `src/lib/ipc/invoke.ts`. The other
  `@tauri-apps/api/core` imports are `isTauri` and `Channel`. `rg -n "command-error'" src`
  points only at `src/lib/ipc/command-error`.
- **`rg -n "PROJECT_CHANGED|changed_or|SongError|RecordingError|PlaybackError" src-tauri/src`**
  is empty.
- **`rg -n "@tauri-apps" src/lib/export/controller.ts src/lib/export/error.ts
  src/lib/ipc/command-error.ts tests/export-dialog.test.ts`** is empty.
- **`rg -n "for_memos" src-tauri/src/recording.rs`** shows at least the nine sites of step 3,
  and a read of every remaining `?` in `recording.rs` finds none on a `ProjectError`.
- **`rg -n "Result<[^>]*, String>" src-tauri/src`** shows no `#[tauri::command]` return type.
- **`git diff --stat -- src/lib/errors.ts crates/strudeland-host crates/strudeland-ctl
  crates/strudeland-cli src/lib/control/protocol.ts`** is empty.

## Commit message

```
refactor(app): reject every Tauri command with one command error

The app's commands now reject with a CommandError { kind, message }
from one flat kind enum, export adds the written paths, and the stale
project is a typed kind instead of a sentinel string. One table maps
project errors to kinds; the memo commands keep a song's errors as
other. In the webview, invokeCommand turns every rejection into an
Error subclass with the same message, so no raw object reaches the
UI. The song kinds are now exists and notFound. Every message, notice
and control socket reply is unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
