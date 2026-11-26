# 003: adapters use the generated contract

## Goal

Every TS adapter takes its command names, argument keys, event names and error kinds from
`src/lib/ipc/bindings.ts`, so a Rust rename breaks `bun run check` instead of failing at
runtime. TS `PROJECT_CHANGED` and the hand-copied event consts go. The fakes reject the way the
real adapters do. AGENTS.md documents `ipc:build` and its rule. There is no behaviour change.

## Prerequisites

- **Stages 001 and 002 are committed,** and the bindings are current.
- **Read the plan's** "Stage 003: using the bindings".

## Implementation

1. **`src/lib/ipc/command-error.ts`:**
   - import `COMMAND_ERROR_KINDS` and `CommandErrorKind` from `./bindings`;
   - delete the hand-written list;
   - re-export the type only if an importer needs it.
2. **`src/lib/ipc/invoke.ts`:** type the parameters. `N` is the only type parameter, and the
   result stays `unknown` (the plan's "TS: the converter" has why: with an explicit result
   type, TS cannot infer `N` and would check the args against the union).

   ```ts
   type ArgsOf<N extends CommandName> =
     Record<string, never> extends CommandArgs[N] ? [args?: CommandArgs[N]] : [args: CommandArgs[N]];
   export function invokeCommand<N extends CommandName>(name: N, ...args: ArgsOf<N>): Promise<unknown>;
   ```

   - Keep the body.
   - `songInvoke` and recording's and playback's `call` take the same
     `<N extends CommandName>(name: N, ...args: ArgsOf<N>)` and still return
     `Promise<unknown>`.
   - The adapters keep their stage-001 casts, for example
     `readSong: (root, path) => invokeCommand('song_read', { root, path }) as Promise<string>`.
   - Every argument is an explicit-key object literal, so it is excess-property-checked
     against `CommandArgs[N]`: the instrument calls write `root`, `id` and `fingerprint` (001
     replaced `{ ...version }`), recording's `call` applies `defined` inside (001), and
     `export_audio_start` is `{ request, source, preview, onProgress: channel }`.
   - A command whose args are all optional (`recording_arm`) also accepts no args; that is
     harmless.
   - If a key fails to typecheck, the call was wrong. Fix the call, never the generator's
     output.
3. **Events:** `listen(EVENTS.recordingLevel, …)`, `EVENTS.recordingState`,
   `EVENTS.projectChanged`, `EVENTS.projectMenu`, `EVENTS.controlRequest` and
   `EVENTS.appQuitRequested`, in `app-quit.ts`, which replaces its literal.
   - Delete `LEVEL_EVENT`, `STATE_EVENT` (`recording/api.ts:22-23`), `PROJECT_CHANGED_EVENT` and
     `PROJECT_MENU_EVENT` (`project/api.ts:129-130`), and `CONTROL_REQUEST_EVENT`
     (`control/api.ts:17`).
   - Move their importers (`rg -n "LEVEL_EVENT|STATE_EVENT|PROJECT_(CHANGED|MENU)_EVENT|CONTROL_REQUEST_EVENT" src tests`)
     to `EVENTS`.
   - Update the `control/api.ts:11` doc comment.
4. **The domain kind lists,** each `as const satisfies readonly CommandErrorKind[]`:
   - `PROJECT_ERROR_KINDS` in `project/api.ts`. `ProjectErrorKind` is derived from it, and
     `toProjectError` narrows with it;
   - `RECORDING_ERROR_KINDS` in `recording/types.ts`, with the same contents and order;
   - `PLAYBACK_ERROR_KINDS` in `playback-api.ts`.

   `recording/types.ts` takes `CommandErrorKind` as `import type` from `../ipc/bindings`.
   `PROJECT_ERROR_KINDS` deliberately leaves out `noProject`, `busy` and the other non-song
   kinds: they narrow to `other`, as today.
5. **Delete TS `PROJECT_CHANGED`** (`project/api.ts:77-81`), and rewrite the `ProjectApi` doc
   (`:83-86`) to say a stale `root` rejects with kind `projectChanged`.
   - `session.ts`:
     - import `CommandError`;
     - `isProjectChanged(e)` becomes `e instanceof CommandError && e.kind === 'projectChanged'`;
     - `:728` becomes `if (gen !== generation && isProjectChanged(e)) return null;`.
   - The rename and delete callers keep using `isProjectChanged`.
   - **`:1225` and `:1438` keep `instanceof ProjectError`.**
6. **The fakes reject with `CommandError`s,** with the same texts:
   - `tests/helpers/fake-project-api.ts`:
     - `check` throws `new CommandError('the project changed', 'projectChanged')`;
     - `songCheck` throws the `ProjectError` with that message;
     - the missing read throws `notFound`;
     - every other string throw becomes `new CommandError(<same text>, 'other')`;
     - update the header comment at `:45-46`.
   - `fake-recording-api.ts:134` already throws a `RecordingError`, a `CommandError` subclass
     since 001. It is unchanged.
   - `fake-playback-api.ts` is unchanged. Check that it still compiles with the typed kinds.
7. **AGENTS.md:**
   - **Commands table**, a row after `host:build`:
     `| Regenerate the committed Tauri IPC bindings src/lib/ipc/bindings.ts (command names and
     argument keys, event names, command error kinds) from src-tauri/src after a command,
     event or kind change (exit 1: a command it cannot read, 2: bad arguments) | bun run
     ipc:build |`
   - **Layout**, beside the `src-tauri/` bullet: `src/lib/ipc/` is the IPC contract.
     - `bindings.ts` is generated by `scripts/ipc/build.ts`.
     - `command-error.ts` holds `CommandError` and `toCommandError`, with no Tauri import.
     - `invoke.ts` holds `invokeCommand`, the only `invoke` caller.
     - `src-tauri/src/command_error.rs` holds the Rust `CommandError`, its kind enum and the
       `ProjectError` table.
   - **Update the stale error names** in the existing text:
     - `:129`: "the `SongError`-returning song commands" becomes "the song commands (they
       reject with a `CommandError`)";
     - `:277`: "the recording commands, `RecordingError`, events" becomes "the recording
       commands, events";
     - `:302-303`: "a stale id is `PlaybackError` kind `stale`" becomes "a stale id is kind
       `stale`", and "`PlaybackError` (`stale`, `output`, `badBatch`, `engine`)" becomes "the
       playback kinds (`stale`, `output`, `badBatch`, `engine`, the TS `PlaybackError`)".
   - **Rules**, one bullet:
     - `src/lib/ipc/bindings.ts` is written only by `bun run ipc:build`, never hand-edited.
     - A change to a `#[tauri::command]`'s name or parameters, to `generate_handler!`, to an
       `*_EVENT` const or to `CommandErrorKind` reruns it in the same change.
       `tests/ipc-bindings.test.ts` fails until then.
     - Every fallible command returns `CommandError` (or `ExportError`), never `String`.
     - Every adapter calls Tauri through `invokeCommand`, and narrows to its domain's kinds.
     - A stale `root` is kind `projectChanged`, never a compared message.
   - **The existing per-feature "only X calls Tauri" rules** (project, prefs, recording,
     control, playback) still hold. They now mean "only X calls `invokeCommand` for it". Add
     that parenthetical once in the new bullet; leave those rules' text alone.

## Tests

- **`bun run check`** is the main gate. It catches the typed names, args, events and kind lists.
- **`tests/ipc-bindings.test.ts`:** one new test checks that each domain list is a subset of
  `COMMAND_ERROR_KINDS`, at runtime. `satisfies` covers the types, and this covers a list built
  some other way.
  - `PROJECT_ERROR_KINDS` is `['exists', 'notFound', 'invalidName', 'projectChanged', 'other']`.
  - `RECORDING_ERROR_KINDS` equals its 001 value.
  - `PLAYBACK_ERROR_KINDS` is `['stale', 'output', 'badBatch', 'engine']`.
- **`tests/project-api.test.ts`:**
  - the `PROJECT_CHANGED` import goes, and its cases use the literal message
    `'the project changed'`;
  - one case checks that `onChange` listens on `EVENTS.projectChanged`, through a mocked
    `@tauri-apps/api/event`.
- **`tests/recording-api.test.ts`:** `onLevel` and `onState` listen on `EVENTS.recordingLevel`
  and `EVENTS.recordingState`. Extend its `@tauri-apps/api/event` mock (`:15`) to record the
  names.
- **`tests/session.test.ts`:** a save that goes stale while a newer project opens is still
  dropped silently. It now goes through the fake's `CommandError`, not a string match. That
  test must pass unchanged; if none covers it, add it.
- **Every other test passes unchanged.** If a test compared a fake's thrown string with `toBe`,
  it compares `.message` now. That is a rewrite, not a new layer.

## Validation

- **The stage gates,** logged once.
- **`rg -n "'(recording|project|control|app)://" src`** matches nothing outside
  `src/lib/ipc/bindings.ts`.
- **`rg -n "PROJECT_CHANGED" src tests src-tauri`** is empty.
- **`rg -n "invokeCommand\('" src`:** every name is a key of `COMMANDS` (tsc already proves it).
- **`bun run ipc:build`** leaves `git status` clean.
- **`rg -n "SongError|RecordingError|PlaybackError" AGENTS.md`** names only the TS thin
  subclasses (`RecordingError`, `PlaybackError`), never a Rust type; `SongError` is gone.

## Commit message

```
refactor(app): take Tauri command, event and kind names from the bindings

Every adapter now invokes commands and listens to events by the names
generated from the Rust source, with typed argument keys, and narrows
rejections to kind lists checked against the generated command error
kinds. The stale project is recognised by its kind, the hand-copied
event names and the project-changed message constant are gone, and the
test fakes reject with command errors like the real adapters. AGENTS.md
documents ipc:build and when to run it.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
