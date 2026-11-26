# 0119: generate the Tauri IPC contract and use one command error

## Goal

The names that cross the Tauri boundary are written once, in Rust, and generated for TS. These
are the command names and their argument keys, the event names and the error kinds. The
generator is a hand-written Bun script, `bun run ipc:build`, that writes the committed
`src/lib/ipc/bindings.ts`. A `bun test` fails when the bindings are stale.

Every fallible command rejects with one shape, a **command error** `{ kind, message }`, from one
Rust type with one flat kind enum. Export adds `written`. Stale-project detection becomes a
kind, not a string sentinel. On the TS side, one `invokeCommand` turns every rejection into a
`CommandError`, which is an `Error`, so a raw object can never reach `errorMessage`: any rejection
with a string message becomes an `Error` with that message. A malformed object with no string
`message` keeps today's text, `"[object Object]"` (pinned at `recording-api.test.ts:97`).

Every visible text stays byte-identical: the status bar, dialogs, memo notices, the control
socket's wire codes and messages, and the CLI's output.

## Background

The [re-verification](reverify.md) has the evidence, with file and line.

- **46 commands are invoked by string name from 9 TS files,** with camelCase keys typed by hand.
  Two use a spread. Nothing checks a name or a key against Rust.
- **6 event names are written on both sides.** One of them is an inline TS literal.
- **Three kind enums are mirrored in TS with no cross-check.** Each converter treats an unknown
  kind differently: recording → `other`, playback → `engine`, song → cast unchecked.
- **The stale-project error is a string sentinel.** Rust's `current_for` returns
  `"the project changed"` as a `String`. `changed_or` and two `From<String>` impls sniff it back
  into a kind, and TS compares the message (`session.ts:728`).
- **The error shapes differ:**
  - 20 commands reject with a bare string.
  - Song, recording and playback reject with `{ kind, message }`, each with its own kind enum.
  - Export rejects with `{ message, written }` and no kind.
  - `errorMessage` stringifies anything that isn't an `Error`, so a raw object would show
    `"[object Object]"`. Today that only works because the String commands reject with strings.

### Current behaviour: error shapes and where the text lands

| Commands | Rejection today | Rejection after | TS reads | Where the text lands |
|---|---|---|---|---|
| `song_list`, `song_read`, `song_save`, `project_info` | bare string | `CommandError`; `projectChanged`, `noProject`, `notFound`, `invalidName` or `other` | plain `CommandError` (not a `ProjectError`) | status bar (`"<stem>: …"`, `"listing songs failed: …"`, `"reloading project.json failed: …"`); control: `code get` "cannot read …: …", `song new`'s read after create "internal error: …" |
| `song_create`, `song_rename`, `song_delete` | `SongError`: `songExists`, `songNotFound`, `invalidName`, `projectChanged`, `other` | `CommandError`: `exists`, `notFound`, `invalidName`, `projectChanged`, `noProject`, `other` | `ProjectError`, narrowed to `exists`, `notFound`, `invalidName`, `projectChanged`, `other` (`noProject` → `other`) | new-song and rename forms (inline); control: `SONG_EXISTS`, `NO_SUCH_SONG`, invalid params, "the project changed; try again", else "internal error: …" |
| `project_create`, `project_open`, `project_last`, `project_root_of`, `recent_list` | bare string | `CommandError`, mostly `other` | `CommandError` | welcome screen and open errors; control `project open` (`OPEN_FAILED`) |
| `project_instruments`, `instrument_*`, `project_instrument_entries`, `project_write_instrument_pin` | bare string | `CommandError` (`busy` for "instrument preview is busy") | `CommandError` | status bar "reading instruments failed: …", instrument browser and preview, control `instrument apply` |
| `piano_roll_inspect` | bare string | `CommandError` (`busy` for "A piano roll preview is already running.") | `CommandError` | piano roll status |
| `prefs_get`, `prefs_set` | bare string | `CommandError` `other` | `CommandError` | settings notice "prefs: …", console |
| `app_info`, `export_audio_set_available` | bare string | `CommandError` `other` | `CommandError` | console only |
| `export_audio_pick_folder`, `_preview`, `_start` | `{ message, written }` | `{ kind, message, written }` (`projectChanged`, `noProject`, `busy`, `other`) | `ExportError` (a `CommandError` with `written`) | export dialog failure and preview error, with the written paths |
| `recording_*` (10) | `RecordingError` (10 kinds) | `CommandError`, plus `noProject` | `RecordingError`, narrowed to its 10 kinds (`noProject` → `other`) | memo notices (`memo-ui.ts`), memo row errors, recording controller |
| `recording://state` `error` | `RecordingError` | `CommandError` | narrowed as above | memo notices |
| `playback_*` (5) | `PlaybackError`: `stale`, `output`, `badBatch`, `engine` | `CommandError` with the same kinds | `PlaybackError`, narrowed (unknown → `engine`) | status bar playback error; `stale` dropped |
| `app_quit_listen`, `app_quit_reply` | bare `&'static str` | unchanged | `CommandError` `other` | console only |
| `host_now_ns`, `control_attach`, `control_reply` | infallible (IPC failures only) | unchanged | `CommandError` `other` | console, or `PlaybackError` `engine` for the clock |

Every message is the same bytes. A kind only ever chooses a branch that existed before, on the
same messages.

## Scope

In:

1. **One Rust `CommandError`** in the new `src-tauri/src/command_error.rs`.
   - It has a flat `CommandErrorKind`: `projectChanged`, `noProject`, `notFound`, `exists`,
     `invalidName`, `busy`, `permissionDenied`, `noDevice`, `deviceLost`, `limitReached`,
     `stale`, `output`, `badBatch`, `engine` and `other`.
   - One `From<ProjectError>` table, with `CommandError::for_memos` for the recording commands.
   - `current`/`current_for` return a typed `CommandError`.
   - `PROJECT_CHANGED`, `NO_PROJECT`, `changed_or`, both `From<String>` impls, `SongError`,
     `SongErrorKind`, `RecordingError`, `RecordingErrorKind`, `PlaybackError` and
     `PlaybackErrorKind` are deleted.
   - Every `String` command returns `CommandError`. `ExportError` gains a `kind`.
2. **The song kind rename:** `songExists` → `exists`, `songNotFound` → `notFound`. Nothing
   outside the app reads them.
3. **One TS converter:**
   - `src/lib/ipc/command-error.ts` holds `CommandError` and `toCommandError`. It replaces
     `src/lib/command-error.ts`, and has no Tauri import.
   - `src/lib/ipc/invoke.ts` holds `invokeCommand`.
   - `ProjectError`, `RecordingError`, `PlaybackError` and `ExportError` are thin subclasses.
     `ExportError` lives in the Tauri-free `src/lib/export/error.ts`.
     Every adapter calls Tauri only through `invokeCommand`.
4. **The generator, `scripts/ipc/build.ts` (`bun run ipc:build`),** writes
   `src/lib/ipc/bindings.ts`:
   - `COMMANDS` and `CommandArgs`;
   - `EVENTS`;
   - `COMMAND_ERROR_KINDS` and `CommandErrorKind`.

   `tests/ipc-bindings.test.ts` regenerates the bindings in memory and byte-compares them.
5. **Adapters and fakes use the generated names, args, events and kinds.**
   - TS `PROJECT_CHANGED` and the TS event consts go.
   - `session.ts:728` checks the kind.
   - The fakes throw `CommandError`s.
6. **AGENTS.md:** the `ipc:build` row, the Layout entry and the bindings rule.

Out:

- **Generated payload types (ts-rs, specta, tauri-specta).** Payload interfaces stay
  hand-written (decision 2). The generator emits only names, keys, simple value types and kinds.
- **A new dependency.** The generator is plain Bun and regexes over the Rust source.
- **The control protocol** (`strudeland-ctl`, `protocol.ts`, the CLI, `SKILL.md`). Its codes and
  messages are unchanged and already cross-checked.
- **`errors.ts`.** It is a headless-host input, so it stays as is: no `host:build`.
- **`ProjectError`'s variants and messages** in `strudeland-project`.
- **Changing what any caller does with a kind,** beyond the renamed spellings and the
  `session.ts:728` predicate.

## Technical design

### Rust: `src-tauri/src/command_error.rs`

```rust
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub kind: CommandErrorKind,
    pub message: String,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CommandErrorKind {
    ProjectChanged, NoProject, NotFound, Exists, InvalidName, Busy,
    PermissionDenied, NoDevice, DeviceLost,
    /// Only ever carried by a `recording://state` event.
    LimitReached,
    Stale, Output, BadBatch, Engine,
    Other,
}

impl CommandError {
    pub fn new(kind: CommandErrorKind, message: impl Into<String>) -> Self;
    /// `.map_err(CommandError::other)`: the error's `Display` text, kind `other`.
    pub fn other(message: impl Display) -> Self;
    pub fn busy(message: impl Into<String>) -> Self;
    /// "the project changed": the command's `root` is not the open project.
    pub fn project_changed() -> Self;
    /// "no project is open".
    pub fn no_project() -> Self;
    /// A recording command's project error: as `From`, except that a song's name error is
    /// `other` (memo-ui shows `other` as a notice and hides the memo name kinds).
    pub fn for_memos(e: ProjectError) -> Self;
}

impl Display for CommandError { /* the message, nothing else */ }
impl From<ProjectError> for CommandError { /* the table */ }
```

- **The table is one `match` with no `_` arm,** so a new `ProjectError` variant fails to compile
  until someone chooses its kind:
  - `SongExists` and `RecordingExists` → `exists`;
  - `SongNotFound` and `RecordingNotFound` → `notFound`;
  - `InvalidSongName` and `InvalidRecordingName` → `invalidName`;
  - every other variant → `other`. The message is `e.to_string()`.

  `for_memos` reads the same table through a private
  `fn project_kind(&ProjectError) -> (CommandErrorKind, Option<Subject>)`, where
  `Subject` is `Song` or `Memo`, and demotes `Subject::Song` to `other`.
- **`Display` is the message alone.** Export's `problem(e)` and every `e.to_string()` caller then
  see the same bytes as before.
- **There is no `From<String>`.** Strings convert with an explicit `CommandError::other`, so a
  sentinel string can't come back by accident.
- **The private message constants** (`"the project changed"`, `"no project is open"`) live in
  this file only.

### Rust: the command modules

- **`project.rs`:**
  - `current` returns `Err(CommandError::no_project())`; `current_for` returns
    `Err(CommandError::project_changed())`.
  - `with_project` returns `Result<T, CommandError>` through `From<ProjectError>`.
  - `install`, `create_project`, `open_project`, `open_last`, `reload_info`, `root_of` and the
    `app_data_file` and `recents_path` helpers return `CommandError`. The watcher's `String`
    converts with `CommandError::other`.
  - `with_song_project` is deleted, since `with_project` now does its job. `new_song`,
    `rename_song` and `delete_song` call `with_project`.
  - `SongError`, `SongErrorKind`, `changed_or`, `PROJECT_CHANGED` and `NO_PROJECT` go.
  - The module doc (`:10-13`) names `CommandErrorKind::ProjectChanged` instead of the constant.
- **`recording.rs`:**
  - `RecordingError` becomes `CommandError`, and `RecordingErrorKind::X` becomes
    `CommandErrorKind::X`. This is a mechanical rename of about 124 uses; keep the `K` alias
    in tests.
  - The `new`, `other` and `busy` helpers move to `CommandError`.
  - `From<RecordError> for CommandError` stays in `recording.rs`, unchanged.
  - Every `ProjectError` conversion goes through `CommandError::for_memos`. Find them with
    `rg -n 'ProjectError|\?' src-tauri/src/recording.rs` around the `Project` calls.
    `project_for` returns `current_for`'s error directly.
  - `StateEvent.error` is `Option<CommandError>`.
  - The `From<String>` impl goes.
- **`playback.rs`:**
  - `PlaybackError` becomes `CommandError`, and `PlaybackErrorKind` becomes `CommandErrorKind`.
  - `stale()` becomes a private `fn stale() -> CommandError` with the same text, "the playback
    session ended". It stays the domain kind `stale`, distinct from `projectChanged`
    (decision 8).
  - `From<LiveError>` and `From<OutputError>` are implemented for `CommandError` here.
- **`export_audio.rs`:**
  - `ExportError { kind, message, written }`, with `kind` serialized first.
  - `problem(e)` → `other`; `reserve` → `busy`.
  - `From<CommandError> for ExportError` keeps the kind, so
    `projects.current_for(..)?` replaces `.map_err(problem)` at `:249` and `:345`.
  - `export_error` and `run_error` → `other`.
- **`instruments.rs`:**
  - `with_project` already returns `CommandError`.
  - `map_err(|e| e.to_string())` becomes `map_err(CommandError::other)`.
  - `reserve_preview` → `busy` with the same text.
  - `render_preview` keeps its `String` and converts at the command.
- **`piano_roll.rs`:**
  - `current_for(..)?` is typed.
  - The lock failure is `CommandError::busy("A piano roll preview is already running.")`.
  - The re-check returns `CommandError::project_changed()`.
  - The inner `String`s convert with `CommandError::other`.
- **`commands.rs`** (`app_info`) and **`project_menu.rs`** (`export_audio_set_available`):
  `CommandError::other`, with the same texts.
- **`app_quit.rs` and the infallible commands are unchanged** (decision 10).

### TS: the converter

- **`src/lib/ipc/command-error.ts`** has no Tauri import. Pure modules such as
  `recording/types.ts` can import it.

  ```ts
  export class CommandError<K extends string = CommandErrorKind> extends Error {
    constructor(message: string, readonly kind: K, options?: ErrorOptions) {
      super(message, options);
      this.name = 'CommandError';
    }
  }
  /** Any rejection as a CommandError: `{ kind, message }` keeps a known kind (unknown → other),
   *  a string or another value is its text with kind other. The raw value is the `cause`. */
  export function toCommandError(e: unknown): CommandError;
  /** `kind` if it is in `allowed`, else `fallback`. */
  export function narrowKind<K extends string>(kind: string, allowed: readonly K[], fallback: K): K;
  ```

  `toCommandError` behaves as follows:
  - a `CommandError` is returned as is (its own `cause` is kept);
  - a `{ message: string, kind }` object becomes `new CommandError(message, known, { cause: e })`,
    where `known` is its kind when that is in `COMMAND_ERROR_KINDS`, else `other`;
  - any other value becomes `new CommandError(errorMessage(e), 'other', { cause: e })`. A
    string stays itself, an `Error` gives its `.message` and `null` gives `'null'`, as
    `toProjectError` does today. A `{ kind: 'busy' }` with no string `message` is
    `"[object Object]"`, as today (`recording-api.test.ts:97` stays as is).

  **Every branch that builds a new `CommandError` sets `cause: e`, the raw rejection.** That is
  how `ExportError` reads `written`: the export commands reject with
  `{ kind, message, written }`, and `toCommandError` keeps the whole object as the `cause`.

  In stage 001, `COMMAND_ERROR_KINDS` is a hand-written list in this file. Stage 003 imports it
  from the bindings and deletes the hand list.
- **`src/lib/ipc/invoke.ts`:**

  ```ts
  export async function invokeCommand(name, args?): Promise<unknown> {
    try { return await invoke(name, args); } catch (e) { throw toCommandError(e); }
  }
  ```

  **It has no result type parameter.** It resolves to `unknown`, and each adapter types the
  result with a cast at its call, `as Promise<T>` (the result was an unchecked `invoke<T>`
  before, so this is the same trust, written once per method). The reason: stage 003 makes the
  name a type parameter `N`, and TS has no partial type-argument inference, so
  `invokeCommand<T, N>(…)` with an explicit `T` would fall back to `N = CommandName` and check
  the args against the union. With `N` as the only type parameter it is always inferred from
  the literal name.

  - **Stage 001:** `invokeCommand(name: string, args?: Record<string, unknown>):
    Promise<unknown>`. The adapters gain their casts here, so 003 only tightens the parameters.
  - **Stage 003:**

    ```ts
    type ArgsOf<N extends CommandName> =
      Record<string, never> extends CommandArgs[N] ? [args?: CommandArgs[N]] : [args: CommandArgs[N]];
    export function invokeCommand<N extends CommandName>(name: N, ...args: ArgsOf<N>): Promise<unknown>;
    ```

    An adapter call:

    ```ts
    readSong: (root, path) => invokeCommand('song_read', { root, path }) as Promise<string>,
    ```

    The domain wrappers (`songInvoke`, recording's and playback's `call`) take the same `N` and
    `...args: ArgsOf<N>`, return `Promise<unknown>`, and their callers cast.
  - **Arguments are written with explicit keys,** so each is a fresh object literal checked
    against `CommandArgs[N]`, and a misspelled key is an excess-property error.
    - `{ ...version }` (`project/api.ts:204`, `:206`) becomes
      `{ root: version.root, id: version.id, fingerprint: version.fingerprint }`. TS does not
      excess-check spread members.
    - Recording's `defined({ … })` (`recording/api.ts:96`, `:100`) moves inside recording's
      `call`, which passes `args && defined(args)` to `invokeCommand`. The call sites pass the
      plain literal, for example
      `call('recording_arm', { deviceId: opts.deviceId, channel: opts.channel }) as
      Promise<RecordingStatus>`. `defined` keeps its `Record<string, unknown>` signature; an
      `undefined` value is absent on the wire either way, and `recording-api.test.ts:145-168`
      pins the args.
- **The thin subclasses keep their names, constructors and `instanceof`:**
  - `ProjectError extends CommandError<ProjectErrorKind>`, with
    `ProjectErrorKind = 'exists' | 'notFound' | 'invalidName' | 'projectChanged' | 'other'`
    and `name = 'ProjectError'`. `toProjectError(e)` narrows with `narrowKind`.
  - `RecordingError extends CommandError<RecordingErrorKind>` (the 10 kinds, unchanged), in
    `recording/types.ts`. `toRecordingError` narrows, and `onState` narrows the event's
    `error.kind` the same way.
  - `PlaybackError extends CommandError<PlaybackErrorKind>`, keeping its `(kind, message)`
    constructor. `toPlaybackError` narrows with fallback `engine`.
  - `ExportError extends CommandError`, with `readonly written: string[]`. **It lives in the new
    `src/lib/export/error.ts`, with `toExportError`, and imports only `../ipc/command-error`.**
    `export/controller.ts` imports it from there, so the controller and
    `tests/export-dialog.test.ts` still import nothing from `@tauri-apps` at runtime (the test's
    `export/api` import is type-only). This matters because Bun's `mock.module` is
    process-global, and `project-api.test.ts:49` and `recording-api.test.ts:9` mock
    `@tauri-apps/api/core` without `Channel` or `isTauri`. `export/api.ts` imports
    `toExportError` from `./error`.
  - `toExportError(e)`: an `ExportError` is returned as is. Otherwise
    `c = e instanceof CommandError ? e : toCommandError(e)`, and `written` is `c.cause`'s
    `written` when that is an array of strings, else `[]`. The cause is read only off a
    `CommandError`, which `invokeCommand` always gives.
  - `failureFrom` becomes
    `{ message: errorMessage(error), written: error instanceof ExportError ? error.written : [] }`.
- **Which calls reject with `ProjectError`.** Only `createSong`, `renameSong` and `deleteSong`
  reject with a `ProjectError`. Every other project command rejects with a plain
  `CommandError`. This keeps decision 6:
  - `session.createSong`'s `e instanceof ProjectError ? e : new Error(…)` (`:1438`) still
    wraps a failed read;
  - `fromProjectError` still passes read errors through to "internal error: …".

  Neither check widens to `CommandError`.
- **Every adapter calls Tauri only through `invokeCommand`.** `listen` stays a direct
  `@tauri-apps/api/event` import, with the name from `EVENTS` from stage 003.

### The generator: `scripts/ipc/build.ts`

`bun run ipc:build` writes `src/lib/ipc/bindings.ts`. `export function generate(root): string`
is the pure part the test calls; `main` writes the file.

**Input.** It reads `src-tauri/src/lib.rs` and the `src-tauri/src/*.rs` files it names.

- **The command list:** the `module::name` entries inside `tauri::generate_handler![ … ]`, in
  order. Each module is `src-tauri/src/<module>.rs`.
- **Each command:** the `#[tauri::command]` attribute directly above
  `pub (async )?fn <name>(`.
  - **The attribute matches only as a whole line, `^#\[tauri::command\]$`.** The module docs
    mention it inside backticks (`project.rs:4`, `playback.rs:7`, `recording.rs:7`), and those
    must not count. `///` and `//` lines above the attribute are allowed and skipped.
  - Read the parameters up to the matching `)`, counting generic depth per character (`<>`,
    `()`, `[]`) and splitting on commas at depth 0. Lifetimes (`'_`) are plain characters:
    there is no char-literal handling, since a parameter list has none.
    `State<'_, Arc<Bridge>>` (`control.rs:787`) must not split.
  - The list may be on one line (`app_quit_reply`, `recent_list`, `prefs_set`, `playback_stop`,
    `export_audio_set_available`) or one parameter per line.
  - Drop the injected types: `State<…>`, `tauri::State<…>`, `AppHandle`, `AppHandle<…>`,
    `WebviewWindow`, and also `Window` and `Webview`, which no command takes today.
  - camelCase the rest, as Tauri does: `device_id` → `deviceId`, `on_progress` → `onProgress`.
  - Value types:
    - `String`, `&str`, `PathBuf` → `string`;
    - `bool` → `boolean`;
    - `u8`–`u64`, `i8`–`i64`, `usize`, `f32`, `f64` → `number`;
    - `Option<T>` → an optional key `?: T | null`;
    - anything else (structs, `Channel<…>`, `Vec<…>`) → `unknown`. The payload types stay
      hand-written.
  - The return type is `Result<T, E>`, with `E` one of `CommandError`, `ExportError` or
    `&'static str`, or a non-`Result`. Split `Result<…>` at its depth-0 comma: `T` may nest
    generics or paths (`Result<Vec<InstrumentEntry>, …>`, `Result<tauri::ipc::Response, …>`),
    and `E` may hold a lifetime (`Result<(), &'static str>`). A non-`Result` return
    (`host_now_ns -> String`, `control_attach -> ControlInfo`) and no `->` at all
    (`control_reply`) both give error `null`.
- **Events:** column-0 `(pub(\(crate\))? )?const [A-Z_]+_EVENT: &str = "<scheme>://<path>";`
  in every `src-tauri/src/*.rs`, keyed by value (`app_quit.rs:11` and `control.rs:47` are both
  `REQUEST_EVENT`). `app_quit.rs:140`'s indented `u32` `CORE_EVENT` doesn't match.
- **Kinds:** every column-0 `pub enum <Name>Kind {` directly under
  `#[serde(rename_all = "camelCase")]`. The body is split on commas; doc comments and
  attributes are skipped, each variant must be a unit ident, and each is camelCased.

**Precedents.** Reading Rust source from a TS test with regexes is already done by
`tests/control-protocol.test.ts:13-20` (the error codes) and `:71-80` (the `LiveStats`
fields); the byte-compare of a committed generated file is `tests/render-vectors.test.ts`'s.
There is no ESLint config to exempt the file from. `.gitattributes` already has
`* text=auto eol=lf`, so the byte-compare holds on a Windows checkout.

**It fails loudly** (it throws, and `main` exits 1 with `file:line`) on:

- a registered command whose `fn` it cannot find or parse;
- a `#[tauri::command]` fn that is not registered (`cli_install.rs` and `snapshot.rs` have none);
- a `#[tauri::command(…)]` with arguments, such as `rename_all`, which would change the keys;
- a `//` inside a parameter list;
- an error type outside the three allowed. A `Result<_, String>` command fails with "return
  CommandError";
- two event consts with the same value, or two keys that camelCase to the same name;
- a `*Kind` variant with fields or a `#[serde(rename…)]`.

**Output.** The file has fixed formatting, ends in a newline and is listed in `.prettierignore`.

```ts
// Generated by `bun run ipc:build` from src-tauri/src. Do not edit.

/** Every registered command and its invoke argument keys, in `generate_handler!` order. */
export const COMMANDS = {
  app_quit_listen: [],
  // …
  song_read: ['root', 'path'],
  // …
} as const;
export type CommandName = keyof typeof COMMANDS;

/** The invoke arguments of each command. Struct payloads are `unknown`: their types are
 *  hand-written beside each adapter. */
export interface CommandArgs {
  app_quit_listen: Record<string, never>;
  // …
  recording_arm: { deviceId?: string | null; channel?: number | null };
  // …
}

/** Every event the app emits, keyed by the camelCased name: `project://changed` → projectChanged. */
export const EVENTS = {
  appQuitRequested: 'app://quit-requested',
  controlRequest: 'control://request',
  projectChanged: 'project://changed',
  projectMenu: 'project://menu',
  recordingLevel: 'recording://level',
  recordingState: 'recording://state',
} as const;

/** `CommandErrorKind` (src-tauri/src/command_error.rs), in declaration order. */
export const COMMAND_ERROR_KINDS = ['projectChanged', /* … */ 'other'] as const;
export type CommandErrorKind = (typeof COMMAND_ERROR_KINDS)[number];
```

- **Ordering.** Commands follow `generate_handler!` order. Events are sorted by value. Kinds
  follow the enum. A `<Name>Kind` becomes `<NAME>_KINDS` and `<Name>Kind`.
- **Keys are by event value, not const name.** `control.rs` and `app_quit.rs` both name a const
  `REQUEST_EVENT`.

### Stage 003: using the bindings

- `invokeCommand` is typed by `CommandName` and `CommandArgs`, with `N` its only type
  parameter (see "TS: the converter"). `{ ...version }` and `defined({ … })` become explicit-key
  literals; the rest of today's call shapes typecheck as they are.
- `listen(EVENTS.x, …)` replaces `LEVEL_EVENT`, `STATE_EVENT`, `PROJECT_CHANGED_EVENT`,
  `PROJECT_MENU_EVENT`, `CONTROL_REQUEST_EVENT` and the `app-quit.ts` literal. The consts go,
  and their importers switch to `EVENTS`.
- **The domain kind lists are written** as
  `['exists', …] as const satisfies readonly CommandErrorKind[]`, so a Rust rename breaks
  `bun run check`. `PROJECT_ERROR_KINDS` deliberately leaves out `noProject`, `busy` and the
  other non-song kinds: they narrow to `other`, which is what the session and the control
  handler see today.
  - This covers `PROJECT_ERROR_KINDS` (new), `RECORDING_ERROR_KINDS` and
    `PLAYBACK_ERROR_KINDS`.
  - `command-error.ts` imports `COMMAND_ERROR_KINDS`.
- TS `PROJECT_CHANGED` goes. `session.ts:728` becomes
  `gen !== generation && isProjectChanged(e)`, where `isProjectChanged` is
  `e instanceof CommandError && e.kind === 'projectChanged'`. It now covers rename, delete and
  save. That is equivalent: the message is unique to that kind.
- **The fakes throw `CommandError`s.**
  - `fake-project-api.ts`'s string throws become
    `new CommandError(<same text>, <kind>)`: `projectChanged` for `check`, `notFound` for a
    missing read, `other` for the rest.
  - The song throws use `exists` and `notFound`.
  - `fake-recording-api.ts` and `fake-playback-api.ts` already throw subclasses.

## Tests

Replace, don't layer: a test of deleted code moves to its replacement with the same assertions.
String-equality tests become kind-and-message tests.

**Rust (stage 001):**

- **`command_error.rs`:**
  - `project_errors_map_through_one_table`: one row per `ProjectError` variant, giving the
    kind and the exact message.
  - `memo_commands_keep_song_errors_as_other`: `for_memos` of `SongExists`, `SongNotFound` and
    `InvalidSongName` gives `other`. The `Recording*` variants keep their kinds.
  - `every_kind_has_its_wire_name`: replaces `song_error_serializes_as_kind_and_message`
    (`project.rs:876-894`), the playback wire test (`playback.rs:506-516`) and the recording one.
  - `display_is_the_message`, and the `project_changed()` and `no_project()` texts.
- **`project.rs`:**
  - `commands_for_a_stale_root_fail_and_touch_nothing` becomes the typed stale test. Save,
    read, list, info and the song commands all equal
    `CommandError { kind: ProjectChanged, message: "the project changed" }`.
  - `song_commands_without_a_project_error` asserts `NoProject` and "no project is open" for
    every command.
  - `song_error_kinds_follow_the_project_error` uses the renamed kinds.
  - The `err.contains(…)` assertions read `err.message`.
- **`recording.rs`:**
  - `errors_from_strings_and_project_errors` becomes `project_errors_for_memos`, with the
    `for_memos` rows `:1727-1754`.
  - Its first two asserts move to a `project_for` test: stale → `ProjectChanged`, no project →
    `NoProject`, with the same messages.
  - The `PROJECT_CHANGED` uses (`:2238`, `:3044`) use `CommandError::project_changed()`.
- **`playback.rs` and `export_audio.rs`:**
  - The kind asserts use `CommandErrorKind`.
  - Export's stale test (`:524`) checks kind `projectChanged` and the message.
  - A `reserve` test checks `busy`.
  - `ExportError` serializes as `{"kind":…,"message":…,"written":[…]}`.
- **`instruments.rs:252`:** checks for `CommandError::project_changed()`.

**TS (stage 001):**

- **`tests/ipc-command-error.test.ts` (new):**
  - `toCommandError` keeps a known kind; turns an unknown kind into `other` with the message;
    turns a string into `other`; gives an `Error`'s message; gives `null` as `'null'`; returns
    a `CommandError` as is; and keeps the raw value as the `cause` in every branch, including
    the object one: `toCommandError({ kind: 'other', message: 'x', written: ['/a'] }).cause`
    has `written` equal to `['/a']`.
  - `toExportError` of that `CommandError` has `written` `['/a']`; of a `written` holding a
    non-string, `[]`.
  - `narrowKind`.
  - `invokeCommand`, with `@tauri-apps/api/core` mocked as in `tests/project-api.test.ts:49`: a
    `{ kind, message }` rejection is an `Error` with that message (the `"[object Object]"`
    guard), and a string rejection is `other`.
- **One guard per adapter:** a rejection is an `instanceof Error` with the same message, and
  `errorMessage` of it is that message.
  - Project, recording and playback: in their existing files. `project-api.test.ts:136-145`
    ("keep rejecting with the raw value") is rewritten. `saveSong` and `projectRootOf` reject a
    `CommandError` that is not a `ProjectError`, with kind `projectChanged` and message
    "the project changed".
  - Prefs, piano roll, export (`written` survives as `ExportError.written`), control, info
    (`getAppInfo` still resolves `null`) and app quit: in a new `tests/ipc-adapters.test.ts`.
    Its `@tauri-apps/api/core` mock has the whole surface the app imports (`invoke`,
    `Channel`, `isTauri`), from a shared `tests/helpers/tauri-core-mock.ts`; the three existing
    api tests may move to it.
- **The song kind rename** reaches `project-api.test.ts`, `session.test.ts:1939-1953`,
  `control-handler.test.ts:1478` and `recording-api.test.ts:89`. That last test's
  `songExists` stays as the example of an unknown recording kind.
- **`export-dialog.test.ts`:** the raw `{ message, written }` rejections become `ExportError`s.
  The asserted failures are unchanged.
- **`control-handler.test.ts`:**
  - It is unchanged apart from the rename.
  - Its stale and create-then-read cases must still give the same wire codes and messages. If
    no case covers `song new` whose read fails stale, add one: `internal error: the project
    changed`. Write it by overriding `h.api.readSong` to throw
    `new CommandError('the project changed', 'projectChanged')`, in the style of the
    `h.api.createSong` override at `:1331`.

**Stage 002:**

- **`tests/ipc-bindings.test.ts`:**
  - "the committed bindings are current (run `bun run ipc:build`)" compares
    `generate(REPO_ROOT)` to the committed file byte for byte.
  - Parser cases on inline Rust snippets:
    - the injected parameters are dropped;
    - `State<'_, A>` doesn't split;
    - keys are camelCased, and `Option` becomes optional;
    - `Channel<…>` is kept as `unknown`;
    - events are keyed by value;
    - kinds are camelCased and doc comments skipped.
  - Each refusal in the fails-loudly list throws with its file and line.
  - On the real tree: 46 commands, 6 events, and the kinds of `command_error.rs`.

**Stage 003:**

- `bun run check` enforces the typed names, args and kind lists.
- A test asserts each domain list's runtime contents against `COMMAND_ERROR_KINDS` (a subset),
  and recording's list against its stage-001 value.
- The fakes' behaviour tests (`session.test.ts`, `recording-controller.test.ts`) pass unchanged
  except for the thrown type.

## Dependencies

- **After #118** (merged as bb30ac5). It touches no render, planner or host file.
- **Not needed:** `host:build`, `engine:refs`, `render:vectors`, `render:check`,
  `host:parity`. `errors.ts` and every `host.inputs.json` file stay untouched.

## Risks

- **A raw object reaches `errorMessage`.** Stage 001 changes the Rust shapes and every adapter in
  one commit, with a guard test per adapter. Grep `@tauri-apps/api/core` afterwards: only
  `src/lib/ipc/invoke.ts` imports `invoke`, and `info.ts` and `app.ts` import `isTauri`.
- **A read error turns into a `ProjectError`,** which would change the control socket's text for
  a stale read (decision 6). Only the three song commands produce `ProjectError`, and `session.ts`
  and `handler.ts` keep `instanceof ProjectError`.
- **A recording `?` uses the shared table** instead of `for_memos`. No recording path reaches a
  `Song*` variant today (reverify), so a missed site is not visible. The test pins the rule, and
  every `ProjectError` `?` gets an explicit `.map_err(CommandError::for_memos)?`, so none
  silently uses `From`: the `list`, `rename`, `delete` and `read` calls (`recording.rs:1068`,
  `:1077`, `:1081`, `:1085`, the second `?` of each), and `:692`, `:699`, `:705`, `:992`,
  `:993`.
- **`Display` drift.** If `CommandError`'s `Display` ever adds the kind, export's `problem` and
  every `to_string` change text. `display_is_the_message` pins it.
- **Error ordering on the wire.** Serde writes struct fields in order. Nothing parses the JSON
  order, but `ExportError` declares `kind` first for consistency.
- **The generator's regexes are brittle** against new Rust syntax. Failing loudly is the
  mitigation: a command it can't read stops `ipc:build` and the staleness test.
- **Windows clippy (`result_large_err`).** `CommandError` is a one-byte kind plus a `String`,
  about 32 B, and `ExportError` adds a `Vec<String>`, about 56 B. Clippy's default threshold is
  128 B, so this is not a risk.

## Complexity

Medium to large: about 37 files, one of them generated.

- Rust: 9 `src-tauri` modules plus the new one.
- TS: 10 adapters, `export/error.ts`, and `session.ts`, `handler.ts` and `controller.ts`.
- Tests: 4 fakes, the new `tauri-core-mock.ts` helper and about 10 test files.
- Docs: AGENTS.md only. README and CONTRIBUTING list no generator commands
  (`CONTRIBUTING.md:41` names only `bun run check`), so they need no `ipc:build` mention.

Stage 001 is the widest, because the wire change and the TS converter must land together. The
loss check is light.

## Decisions

1. **A hand-written Bun generator, with no new dependency.** It writes the committed
   `src/lib/ipc/bindings.ts`, a `bun test` regenerates it and byte-compares, and it fails
   loudly on what it can't parse.
2. **Payload types stay hand-written for now.**
3. **One Rust `CommandError { kind, message }` in `src-tauri/src/command_error.rs`,** with one
   flat kind enum and one `From<ProjectError>` table.
   - `current` and `current_for` return a typed error.
   - `changed_or` and the `From<String>` impls are deleted.
   - Each TS adapter narrows to its domain's kinds.
4. **Rename `songExists`/`songNotFound` to `exists`/`notFound`.** They are internal only.
5. **Recording commands keep their kind mapping.** `Song*` variants are `other` there, through
   `for_memos`, so memo-ui's suppression (`memo-ui.ts:361-365`) is unchanged.
6. **Stale read, save, list and info errors keep their control-socket routing,** so the CLI text
   is unchanged. Only the three song commands reject with `ProjectError`.
7. **A `noProject` kind, with the text "no project is open".** Each adapter narrows it to
   `other`, which is today's kind.
8. **Playback `stale` stays a domain kind,** distinct from `projectChanged`.
9. **`ExportError` gains a kind and keeps `{ message, written }`.**
10. **The `app_quit_*` `&'static str` errors and the infallible commands stay as they are.** The
    bindings list them by name and keys only.
11. **Messages stay byte-identical.** Every message comes from `e.to_string()` or the same
    constant, including "A piano roll preview is already running.".
    - Every adapter goes through `invokeCommand`, which turns every rejection into a
      `CommandError extends Error`: an unknown kind or a plain string is `other`.
    - `ProjectError`, `RecordingError`, `PlaybackError` and `ExportError` are thin subclasses,
      so `instanceof` keeps working.
    - `PROJECT_CHANGED` goes on both sides, and the fakes throw `CommandError`.
12. **The bindings are prettier-ignored, with fixed formatting.** A prettier bump can't make them
    stale.
13. **`CommandError` lives outside `errors.ts`,** which the headless host bundles.
